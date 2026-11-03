//! The record path must never block an OS thread (the services run on `may` coroutines, where a
//! blocked worker stalls every coroutine on it). Two proofs:
//!
//! 1. Static: the record-path source (`src/site.rs`, outside its tests) uses no blocking or
//!    lazily-initialised primitive and no thread-local state.
//! 2. Dynamic: thousands of `may` coroutines on two workers time calls that yield (and so may
//!    resume on another worker) between start and finish, and every call is counted once, in
//!    the right bucket.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::let_underscore_must_use,
    missing_docs,
    unsafe_code
)]

use std::time::Duration;

use microscaler_timing::{snapshot, Outcome};

const FORBIDDEN: &[&str] = &[
    "Mutex",
    "RwLock",
    "Condvar",
    "Barrier",
    "OnceLock",
    "OnceCell",
    "LazyLock",
    "Lazy<",
    "Once::",
    "thread_local",
    "park(",
    "sleep(",
    "recv(",
    ".lock(",
    ".read(",
    ".write(",
    "spin_loop",
];

#[test]
fn record_path_source_has_no_blocking_primitives() {
    let src = include_str!("../src/site.rs");
    let record_path = src.split("#[cfg(test)]").next().unwrap();
    for (n, line) in record_path.lines().enumerate() {
        let code = line.split("//").next().unwrap();
        for word in FORBIDDEN {
            assert!(
                !code.contains(word),
                "src/site.rs:{}: `{word}` on the record path: {line}",
                n + 1
            );
        }
    }
    // and it is the record path: the guard and the recorder live there
    assert!(record_path.contains("pub fn record("));
    assert!(record_path.contains("pub fn start("));
    assert!(record_path.contains("impl Drop for Timer"));
}

#[test]
fn coroutines_migrating_between_workers_are_timed_exactly_once() {
    may::config().set_workers(2);
    let site = microscaler_timing::site!("test.may", "yielding_call");
    let n = 2_000;
    let handles: Vec<_> = (0..n)
        .map(|i| {
            // SAFETY (may): the closure uses no thread-local storage.
            unsafe {
                may::coroutine::spawn(move || {
                    let t = site.start();
                    may::coroutine::yield_now();
                    may::coroutine::sleep(Duration::from_millis(3));
                    may::coroutine::yield_now();
                    t.finish(if i % 10 == 0 {
                        Outcome::Err
                    } else {
                        Outcome::Ok
                    });
                })
            }
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let rows: Vec<_> = snapshot()
        .into_iter()
        .filter(|s| s.component == "test.may" && s.op == "yielding_call")
        .collect();
    let ok = rows.iter().find(|s| s.outcome == Outcome::Ok).unwrap();
    let err = rows.iter().find(|s| s.outcome == Outcome::Err).unwrap();
    assert_eq!(ok.count() + err.count(), n);
    assert_eq!(err.count(), n / 10);
    // each call slept 3 ms: nothing in the 1 ms or 2.5 ms buckets
    assert_eq!(ok.buckets[0] + ok.buckets[1], 0);
    assert_eq!(site.in_flight(), 0);
}
