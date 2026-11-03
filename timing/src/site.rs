//! The record path: a [`Site`] per timed operation, its histogram as plain atomics, and the
//! [`Timer`] guard.
//!
//! **Nothing in this file may block an OS thread.** The services run on the `may` coroutine
//! runtime, where a blocked worker thread stalls every coroutine scheduled on it. Recording is a
//! fixed number of relaxed atomic adds on a `'static`; the first record of a site links it into
//! the process-wide list with a compare-and-swap loop (lock-free: some thread always makes
//! progress, nobody waits). There is no mutex, no `RwLock`, no `OnceLock`/`LazyLock`
//! initialisation (those block concurrent initialisers) and no thread-local state, so a coroutine
//! that migrates to another worker between [`Site::start`] and [`Timer::finish`] is measured
//! correctly: the start [`Instant`] lives in the [`Timer`] on the caller's stack.
//! `tests/no_blocking.rs` scans this file for blocking primitives.

use std::ptr;
use std::sync::atomic::{AtomicI64, AtomicPtr, AtomicU64, AtomicU8, Ordering};
use std::time::{Duration, Instant};

/// Upper bounds of the histogram buckets, in nanoseconds (1 ms ... 2.5 s), then `+Inf`.
///
/// Eleven buckets tuned to request-path latencies. 50 ms and 100 ms (the service SLO lines) are
/// bucket bounds, so `histogram_quantile` is exact at the targets. Managed Prometheus bills a
/// histogram point as 2 samples plus 1 per non-empty bucket, so few buckets keep the cost down.
pub const BUCKET_BOUNDS_NANOS: [u64; 11] = [
    1_000_000,
    2_500_000,
    5_000_000,
    10_000_000,
    25_000_000,
    50_000_000,
    100_000_000,
    250_000_000,
    500_000_000,
    1_000_000_000,
    2_500_000_000,
];

/// Number of bucket counters per series: the bounded buckets plus `+Inf`.
pub const BUCKETS: usize = BUCKET_BOUNDS_NANOS.len() + 1;

/// How a timed operation ended. The `outcome` label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Outcome {
    /// Completed normally (`Ok`, or a function that does not return a `Result`).
    Ok = 0,
    /// Returned `Err`, panicked, or (async) was dropped before completing.
    Err = 1,
    /// Returned an error that says it timed out (see [`crate::IsTimeout`]).
    Timeout = 2,
}

impl Outcome {
    /// All outcomes, in label order.
    pub const ALL: [Self; 3] = [Self::Ok, Self::Err, Self::Timeout];

    /// The label value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Err => "err",
            Self::Timeout => "timeout",
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// One histogram series: non-cumulative bucket counts (cumulated when rendered) and the sum.
///
/// The count is the sum of the buckets, so `_count` always equals the `+Inf` bucket in a scrape.
pub struct Series {
    pub buckets: [AtomicU64; BUCKETS],
    pub sum_nanos: AtomicU64,
}

impl Series {
    const fn new() -> Self {
        Self {
            buckets: [const { AtomicU64::new(0) }; BUCKETS],
            sum_nanos: AtomicU64::new(0),
        }
    }
}

const NEW: u8 = 0;
const LINKING: u8 = 1;
const LINKED: u8 = 2;

/// Head of the process-wide list of sites that have recorded at least once. Sites are only ever
/// pushed (never removed), and every pointer in the list was made from a `&'static Site`.
static HEAD: AtomicPtr<Site> = AtomicPtr::new(ptr::null_mut());

/// A timed operation: its static labels and its histograms, one per [`Outcome`].
///
/// Normally declared by `#[timed]`, `timed_block!` or `site!` as a `static`. A site renders
/// nothing until it has recorded once, so unused code paths cost no series.
pub struct Site {
    component: &'static str,
    op: &'static str,
    state: AtomicU8,
    next: AtomicPtr<Self>,
    in_flight: AtomicI64,
    pub(crate) series: [Series; 3],
}

impl Site {
    /// A site for `op` in `component`. `const`, so it can initialise a `static`.
    ///
    /// Both labels must be static, low-cardinality names (never ids, tickers, URLs or user
    /// data): see the crate docs.
    #[must_use]
    pub const fn new(component: &'static str, op: &'static str) -> Self {
        Self {
            component,
            op,
            state: AtomicU8::new(NEW),
            next: AtomicPtr::new(ptr::null_mut()),
            in_flight: AtomicI64::new(0),
            series: [const { Series::new() }; 3],
        }
    }

    /// The `component` label as declared (`::` is rendered as `.`).
    #[must_use]
    pub const fn component(&self) -> &'static str {
        self.component
    }

    /// The `op` label.
    #[must_use]
    pub const fn op(&self) -> &'static str {
        self.op
    }

    /// Start timing one call. The returned guard records when it is finished or dropped.
    #[inline]
    pub fn start(&'static self) -> Timer {
        self.in_flight.fetch_add(1, Ordering::Relaxed);
        Timer {
            site: self,
            started: Instant::now(),
            finished: false,
            drop_outcome: Outcome::Ok,
        }
    }

    /// Record one call that took `elapsed` (for durations measured elsewhere, e.g. reported by a
    /// client's lifecycle events).
    #[inline]
    pub fn record(&'static self, outcome: Outcome, elapsed: Duration) {
        self.ensure_linked();
        let nanos = u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX);
        let Some(series) = self.series.get(outcome.index()) else {
            return;
        };
        if let Some(bucket) = series.buckets.get(bucket_index(nanos)) {
            bucket.fetch_add(1, Ordering::Relaxed);
        }
        series.sum_nanos.fetch_add(nanos, Ordering::Relaxed);
    }

    /// Calls currently between [`Site::start`] and their record.
    #[must_use]
    pub fn in_flight(&self) -> i64 {
        self.in_flight.load(Ordering::Relaxed)
    }

    #[inline]
    fn ensure_linked(&'static self) {
        if self.state.load(Ordering::Acquire) != LINKED {
            self.link();
        }
    }

    /// Push this site onto [`HEAD`]. Lock-free: a failed compare-and-swap means another site was
    /// pushed meanwhile, and we retry against the new head; nobody waits on anybody.
    #[cold]
    fn link(&'static self) {
        if self
            .state
            .compare_exchange(NEW, LINKING, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            // Another caller is linking it (or has): this record still lands in the atomics.
            return;
        }
        let me = ptr::from_ref(self).cast_mut();
        let mut head = HEAD.load(Ordering::Acquire);
        loop {
            self.next.store(head, Ordering::Relaxed);
            match HEAD.compare_exchange_weak(head, me, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => break,
                Err(current) => head = current,
            }
        }
        self.state.store(LINKED, Ordering::Release);
    }
}

/// The index of the bucket `nanos` falls in (the last one is `+Inf`).
#[inline]
pub fn bucket_index(nanos: u64) -> usize {
    let mut i = 0;
    while i < BUCKET_BOUNDS_NANOS.len() {
        if let Some(&bound) = BUCKET_BOUNDS_NANOS.get(i) {
            if nanos <= bound {
                return i;
            }
        }
        i += 1;
    }
    BUCKET_BOUNDS_NANOS.len()
}

/// Every site that has recorded, most recently linked first.
pub fn linked_sites() -> impl Iterator<Item = &'static Site> {
    let mut cursor = HEAD.load(Ordering::Acquire);
    std::iter::from_fn(move || {
        // SAFETY: every pointer on the list was made from a `&'static Site` in `link` (sites are
        // statics, or leaked by `intern`, and never freed), and sites are never unlinked, so a
        // non-null pointer read from `HEAD` or a `next` is valid for `'static` and only ever
        // accessed through shared references (all its mutable state is atomic).
        #[allow(unsafe_code)]
        let site: &'static Site = unsafe { cursor.as_ref() }?;
        cursor = site.next.load(Ordering::Acquire);
        Some(site)
    })
}

/// Times one call of a [`Site`]; records when finished or dropped.
///
/// Holds the start [`Instant`] itself (no thread-local), so it is correct across coroutine
/// migration and `.await` points. Dropped without [`Timer::finish`]: records the drop outcome
/// (`ok` unless set otherwise), or `err` if the thread is panicking.
#[must_use = "a timer records when it is finished or dropped; binding it to `_` records immediately"]
pub struct Timer {
    site: &'static Site,
    started: Instant,
    finished: bool,
    drop_outcome: Outcome,
}

impl Timer {
    /// Record the call with `outcome`; returns its duration.
    #[inline]
    #[allow(clippy::needless_pass_by_value)] // consuming the guard is the point: it cannot record twice
    #[allow(clippy::must_use_candidate)] // the duration is a convenience; the call is recorded
    pub fn finish(mut self, outcome: Outcome) -> Duration {
        self.complete(outcome)
    }

    /// Record `ok` for `Ok`, `err` for `Err`; returns the duration.
    #[inline]
    pub fn finish_result<T, E>(self, result: &Result<T, E>) -> Duration {
        self.finish(if result.is_ok() {
            Outcome::Ok
        } else {
            Outcome::Err
        })
    }

    /// The outcome recorded if the guard is dropped unfinished (default `ok`). The `#[timed]`
    /// attribute sets `err` on async functions, so a future dropped mid-flight (cancelled, or
    /// timed out by its caller) is not counted as a success.
    #[inline]
    pub const fn set_drop_outcome(&mut self, outcome: Outcome) {
        self.drop_outcome = outcome;
    }

    /// Time since the call started.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    #[inline]
    fn complete(&mut self, outcome: Outcome) -> Duration {
        let elapsed = self.started.elapsed();
        if !self.finished {
            self.finished = true;
            self.site.record(outcome, elapsed);
            self.site.in_flight.fetch_sub(1, Ordering::Relaxed);
        }
        elapsed
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if !self.finished {
            let outcome = if std::thread::panicking() {
                Outcome::Err
            } else {
                self.drop_outcome
            };
            self.complete(outcome);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn buckets_are_upper_bounds_inclusive() {
        assert_eq!(bucket_index(0), 0);
        assert_eq!(bucket_index(1_000_000), 0);
        assert_eq!(bucket_index(1_000_001), 1);
        assert_eq!(bucket_index(50_000_000), 5);
        assert_eq!(bucket_index(100_000_000), 6);
        assert_eq!(bucket_index(2_500_000_000), 10);
        assert_eq!(bucket_index(2_500_000_001), 11);
        assert_eq!(bucket_index(u64::MAX), BUCKETS - 1);
    }

    #[test]
    fn a_site_links_once_however_often_it_records() {
        static S: Site = Site::new("test.site", "links_once");
        for _ in 0..5 {
            S.record(Outcome::Ok, Duration::from_millis(3));
        }
        let n = linked_sites().filter(|s| ptr::eq(*s, &raw const S)).count();
        assert_eq!(n, 1);
    }

    #[test]
    fn timer_tracks_in_flight() {
        static S: Site = Site::new("test.site", "in_flight");
        let t = S.start();
        assert_eq!(S.in_flight(), 1);
        drop(t);
        assert_eq!(S.in_flight(), 0);
    }
}
