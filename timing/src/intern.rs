//! Sites whose labels are known only at run time, from a bounded set fixed at startup (a router's
//! `operationId`s from its `OpenAPI` spec, a pool's name from its configuration).
//!
//! **Not for the hot path.** [`intern`] takes a mutex and may allocate: call it once, where the
//! handler / pool / client is set up, and keep the returned `&'static Site` for recording. The
//! record path itself ([`Site::record`], [`Site::start`]) stays lock-free.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use crate::site::Site;

/// Most distinct interned sites.
///
/// Past this every new label pair shares one overflow site (`op="_other"`), so a caller that
/// interns something unbounded by mistake cannot blow up the series count (and the Managed
/// Prometheus bill).
pub const MAX_INTERNED: usize = 2048;

static INTERNED: Mutex<Option<HashMap<(String, String), &'static Site>>> = Mutex::new(None);

/// The process-wide site for (`component`, `op`), created and leaked on first use; the same
/// labels always return the same site.
///
/// Both labels must come from a bounded, static set (spec `operationId`s, configured names): never
/// ids, tickers, URLs or user data.
#[must_use]
pub fn intern(component: &str, op: &str) -> &'static Site {
    let mut guard = INTERNED.lock().unwrap_or_else(PoisonError::into_inner);
    let map = guard.get_or_insert_with(HashMap::new);
    let site = intern_in(map, component, op);
    drop(guard);
    site
}

fn intern_in(
    map: &mut HashMap<(String, String), &'static Site>,
    component: &str,
    op: &str,
) -> &'static Site {
    if let Some(site) = map.get(&(component.to_owned(), op.to_owned())) {
        return site;
    }
    let (component, op) = if map.len() >= MAX_INTERNED {
        ("_overflow", "_other")
    } else {
        (component, op)
    };
    let key = (component.to_owned(), op.to_owned());
    if let Some(site) = map.get(&key) {
        return site;
    }
    let component: &'static str = Box::leak(component.to_owned().into_boxed_str());
    let op: &'static str = Box::leak(op.to_owned().into_boxed_str());
    let site: &'static Site = Box::leak(Box::new(Site::new(component, op)));
    map.insert(key, site);
    site
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn same_labels_same_site() {
        let a = intern("test.intern", "listPets");
        let b = intern("test.intern", "listPets");
        let c = intern("test.intern", "getPet");
        assert!(std::ptr::eq(a, b));
        assert!(!std::ptr::eq(a, c));
        assert_eq!(a.op(), "listPets");
    }
}
