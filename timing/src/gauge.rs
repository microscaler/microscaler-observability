//! Gauges beside the timings: `op_gauge{component, op}`, for the few levels that explain a
//! latency (open connections, an accept queue), rendered by the same `render_prometheus` call so a
//! host needs no extra wiring.
//!
//! Two kinds, both `const`-constructible statics and lock-free like [`crate::Site`]:
//!
//! - [`Gauge`]: a level the code keeps up to date (`add` / `sub` / `set`).
//! - [`GaugeFn`]: a level read when `/metrics` is scraped, from a function (`register` it once).
//!
//! A gauge renders once it has been touched (`Gauge`) or registered (`GaugeFn`).

use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicPtr, Ordering};

static GAUGES: AtomicPtr<Gauge> = AtomicPtr::new(ptr::null_mut());
static GAUGE_FNS: AtomicPtr<GaugeFn> = AtomicPtr::new(ptr::null_mut());

/// A level kept by the code: `op_gauge{component, op}`.
pub struct Gauge {
    component: &'static str,
    op: &'static str,
    value: AtomicI64,
    linked: AtomicBool,
    next: AtomicPtr<Self>,
}

impl Gauge {
    /// A gauge for `op` in `component` (static, low-cardinality names, as for a site).
    #[must_use]
    pub const fn new(component: &'static str, op: &'static str) -> Self {
        Self {
            component,
            op,
            value: AtomicI64::new(0),
            linked: AtomicBool::new(false),
            next: AtomicPtr::new(ptr::null_mut()),
        }
    }

    /// Add `n` (e.g. a connection opened).
    #[inline]
    pub fn add(&'static self, n: i64) {
        self.link();
        self.value.fetch_add(n, Ordering::Relaxed);
    }

    /// Subtract `n` (e.g. a connection closed).
    #[inline]
    pub fn sub(&'static self, n: i64) {
        self.link();
        self.value.fetch_sub(n, Ordering::Relaxed);
    }

    /// Set the level.
    #[inline]
    pub fn set(&'static self, v: i64) {
        self.link();
        self.value.store(v, Ordering::Relaxed);
    }

    /// The current level.
    #[must_use]
    pub fn get(&self) -> i64 {
        self.value.load(Ordering::Relaxed)
    }

    #[inline]
    fn link(&'static self) {
        if self.linked.load(Ordering::Acquire) {
            return;
        }
        if self.linked.swap(true, Ordering::AcqRel) {
            return;
        }
        let me = ptr::from_ref(self).cast_mut();
        let mut head = GAUGES.load(Ordering::Acquire);
        loop {
            self.next.store(head, Ordering::Relaxed);
            match GAUGES.compare_exchange_weak(head, me, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => break,
                Err(current) => head = current,
            }
        }
    }
}

/// A level read at scrape time from `read` (`None`: not available now, nothing rendered).
pub struct GaugeFn {
    component: &'static str,
    op: &'static str,
    read: fn() -> Option<i64>,
    linked: AtomicBool,
    next: AtomicPtr<Self>,
}

impl GaugeFn {
    /// A scrape-time gauge for `op` in `component`, read by `read`.
    #[must_use]
    pub const fn new(component: &'static str, op: &'static str, read: fn() -> Option<i64>) -> Self {
        Self {
            component,
            op,
            read,
            linked: AtomicBool::new(false),
            next: AtomicPtr::new(ptr::null_mut()),
        }
    }

    /// Start rendering it (idempotent).
    pub fn register(&'static self) {
        if self.linked.swap(true, Ordering::AcqRel) {
            return;
        }
        let me = ptr::from_ref(self).cast_mut();
        let mut head = GAUGE_FNS.load(Ordering::Acquire);
        loop {
            self.next.store(head, Ordering::Relaxed);
            match GAUGE_FNS.compare_exchange_weak(head, me, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => break,
                Err(current) => head = current,
            }
        }
    }
}

/// (component, op, value) of every touched gauge and registered gauge function.
pub fn gauge_values() -> Vec<(&'static str, &'static str, i64)> {
    let mut out = Vec::new();
    let mut cursor = GAUGES.load(Ordering::Acquire);
    // SAFETY: as for sites: every pointer was made from a `&'static Gauge` / `&'static GaugeFn`
    // (statics, never freed) and is never unlinked; all mutable state is atomic.
    #[allow(unsafe_code)]
    while let Some(g) = unsafe { cursor.as_ref() } {
        out.push((g.component, g.op, g.get()));
        cursor = g.next.load(Ordering::Acquire);
    }
    let mut cursor = GAUGE_FNS.load(Ordering::Acquire);
    #[allow(unsafe_code)]
    while let Some(g) = unsafe { cursor.as_ref() } {
        if let Some(v) = (g.read)() {
            out.push((g.component, g.op, v));
        }
        cursor = g.next.load(Ordering::Acquire);
    }
    out
}

#[cfg(test)]
#[allow(clippy::unnecessary_wraps)]
mod tests {
    use super::*;

    static OPEN: Gauge = Gauge::new("test.gauge", "open");
    fn seven() -> Option<i64> {
        Some(7)
    }
    fn absent() -> Option<i64> {
        None
    }
    static SEVEN: GaugeFn = GaugeFn::new("test.gauge", "seven", seven);
    static ABSENT: GaugeFn = GaugeFn::new("test.gauge", "absent", absent);

    #[test]
    fn gauges_render_once_touched_or_registered() {
        let has = |op: &str| {
            gauge_values()
                .iter()
                .any(|(c, o, _)| *c == "test.gauge" && *o == op)
        };
        assert!(!has("open"));
        OPEN.add(2);
        OPEN.sub(1);
        SEVEN.register();
        SEVEN.register();
        ABSENT.register();
        let v = gauge_values();
        assert!(v.contains(&("test.gauge", "open", 1)));
        assert_eq!(
            v.iter()
                .filter(|(c, o, _)| *c == "test.gauge" && *o == "seven")
                .count(),
            1
        );
        assert!(v.contains(&("test.gauge", "seven", 7)));
        assert!(!has("absent"));
    }
}
