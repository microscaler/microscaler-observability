//! `#[timed]`, `timed_block!` and `site!`: what each records.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::let_underscore_must_use,
    missing_docs
)]

use std::time::Duration;

use microscaler_timing::{snapshot, timed, timed_block, IsTimeout, OpSnapshot, Outcome};

fn find(component: &str, op: &str, outcome: Outcome) -> Option<OpSnapshot> {
    snapshot()
        .into_iter()
        .find(|s| s.component == component && s.op == op && s.outcome == outcome)
}

fn count(component: &str, op: &str, outcome: Outcome) -> u64 {
    find(component, op, outcome).map_or(0, |s| s.count())
}

#[timed(component = "test.macros")]
fn plain_sync(x: u32) -> u32 {
    std::thread::sleep(Duration::from_millis(2));
    x * 2
}

#[test]
fn sync_fn_defaults_op_to_its_name_and_records_ok() {
    assert_eq!(plain_sync(21), 42);
    let s = find("test.macros", "plain_sync", Outcome::Ok).expect("recorded");
    assert_eq!(s.count(), 1);
    assert!(
        s.sum_nanos >= 2_000_000,
        "slept 2 ms, sum {} ns",
        s.sum_nanos
    );
    // 2 ms falls in the (1 ms, 2.5 ms] bucket or later, never the first.
    assert_eq!(s.buckets[0], 0);
}

#[timed]
fn default_component() {}

#[test]
fn component_defaults_to_the_module_path_dotted() {
    default_component();
    // this integration test's crate is `macros`; `::` would be rendered as `.`
    assert_eq!(count("macros", "default_component", Outcome::Ok), 1);
}

#[derive(Debug)]
struct MyError;

#[timed(component = "test.macros", op = "fallible")]
fn fallible(fail: bool) -> Result<u8, MyError> {
    if fail {
        return Err(MyError);
    }
    let v: u8 = "7".parse().map_err(|_| MyError)?;
    Ok(v)
}

#[test]
fn result_sets_outcome_from_ok_and_err() {
    assert_eq!(fallible(false).unwrap(), 7);
    assert!(fallible(true).is_err());
    assert!(fallible(true).is_err());
    assert_eq!(count("test.macros", "fallible", Outcome::Ok), 1);
    assert_eq!(count("test.macros", "fallible", Outcome::Err), 2);
    assert_eq!(count("test.macros", "fallible", Outcome::Timeout), 0);
}

#[timed(component = "test.macros", op = "io_call")]
fn io_call(kind: std::io::ErrorKind) -> std::io::Result<()> {
    Err(std::io::Error::new(kind, "x"))
}

#[test]
fn io_timeouts_are_recorded_as_timeout() {
    let _ = io_call(std::io::ErrorKind::TimedOut);
    let _ = io_call(std::io::ErrorKind::ConnectionRefused);
    assert_eq!(count("test.macros", "io_call", Outcome::Timeout), 1);
    assert_eq!(count("test.macros", "io_call", Outcome::Err), 1);
}

#[derive(Debug)]
struct Deadline;
impl IsTimeout for Deadline {
    fn is_timeout(&self) -> bool {
        true
    }
}

#[timed(component = "test.macros", op = "own_timeout")]
fn own_timeout() -> Result<(), Deadline> {
    Err(Deadline)
}

#[test]
fn own_error_types_can_report_timeouts() {
    let _ = own_timeout();
    assert_eq!(count("test.macros", "own_timeout", Outcome::Timeout), 1);
}

type MyResult = Result<u8, String>;

#[timed(component = "test.macros", op = "alias_result")]
fn alias_result() -> MyResult {
    Err("no".into())
}

#[timed(component = "test.macros", op = "not_a_result", result = false)]
fn not_a_result() -> NotAResult {
    NotAResult
}
struct NotAResult;

#[test]
fn result_aliases_and_opt_out() {
    let _ = alias_result();
    let _ = not_a_result();
    assert_eq!(count("test.macros", "alias_result", Outcome::Err), 1);
    assert_eq!(count("test.macros", "not_a_result", Outcome::Ok), 1);
}

#[timed(component = "test.macros", op = "borrowing")]
fn borrowing(s: &str) -> &str {
    &s[1..]
}

#[timed(component = "test.macros", op = "impl_trait")]
fn impl_trait(n: u32) -> impl Iterator<Item = u32> {
    0..n
}

#[test]
fn references_and_impl_trait_return_types_compile_and_record() {
    assert_eq!(borrowing("xyz"), "yz");
    assert_eq!(impl_trait(3).count(), 3);
    assert_eq!(count("test.macros", "borrowing", Outcome::Ok), 1);
    assert_eq!(count("test.macros", "impl_trait", Outcome::Ok), 1);
}

struct Repo {
    hits: u32,
}

impl Repo {
    #[timed(component = "test.macros", op = "method_mut")]
    fn bump(&mut self) -> u32 {
        self.hits += 1;
        self.hits
    }

    #[timed(component = "test.macros", op = "method_async")]
    async fn fetch(&self, fail: bool) -> Result<u32, MyError> {
        yield_once().await;
        if fail {
            Err(MyError)
        } else {
            Ok(self.hits)
        }
    }
}

#[test]
fn methods_sync_and_async() {
    let mut r = Repo { hits: 0 };
    assert_eq!(r.bump(), 1);
    assert_eq!(block_on(r.fetch(false)).unwrap(), 1);
    assert!(block_on(r.fetch(true)).is_err());
    assert_eq!(count("test.macros", "method_mut", Outcome::Ok), 1);
    assert_eq!(count("test.macros", "method_async", Outcome::Ok), 1);
    assert_eq!(count("test.macros", "method_async", Outcome::Err), 1);
}

#[timed(component = "test.macros", op = "async_sleep")]
async fn async_sleep() -> u8 {
    yield_once().await;
    std::thread::sleep(Duration::from_millis(3));
    9
}

#[test]
fn async_fn_times_the_whole_future_not_its_construction() {
    let fut = async_sleep();
    std::thread::sleep(Duration::from_millis(30)); // not timed: the future has not been polled
    assert_eq!(block_on(fut), 9);
    let s = find("test.macros", "async_sleep", Outcome::Ok).unwrap();
    assert!(
        s.sum_nanos >= 3_000_000 && s.sum_nanos < 30_000_000,
        "{}",
        s.sum_nanos
    );
}

#[timed(component = "test.macros", op = "async_cancelled")]
async fn async_cancelled() {
    std::future::pending::<()>().await;
}

#[test]
fn dropped_async_call_records_err() {
    let mut fut = Box::pin(async_cancelled());
    let waker = noop_waker();
    let mut cx = std::task::Context::from_waker(&waker);
    assert!(fut.as_mut().poll(&mut cx).is_pending());
    drop(fut);
    assert_eq!(count("test.macros", "async_cancelled", Outcome::Err), 1);
}

#[timed(component = "test.macros", op = "panics")]
fn panics() {
    panic!("boom");
}

#[test]
fn panic_records_err() {
    assert!(std::panic::catch_unwind(panics).is_err());
    assert_eq!(count("test.macros", "panics", Outcome::Err), 1);
}

#[test]
fn block_macro_records_and_returns_the_value() {
    let v = timed_block!("test.macros", "block", { 40 + 2 });
    assert_eq!(v, 42);
    let w = timed_block!("block_default_component", { "x" });
    assert_eq!(w, "x");
    assert_eq!(count("test.macros", "block", Outcome::Ok), 1);
    assert_eq!(count("macros", "block_default_component", Outcome::Ok), 1);
}

#[test]
fn guard_records_chosen_outcome() {
    let site = microscaler_timing::site!("test.macros", "guard");
    site.start().finish(Outcome::Timeout);
    let t = site.start();
    assert_eq!(site.in_flight(), 1);
    drop(t);
    assert_eq!(site.in_flight(), 0);
    assert_eq!(count("test.macros", "guard", Outcome::Timeout), 1);
    assert_eq!(count("test.macros", "guard", Outcome::Ok), 1);
}

#[test]
fn render_emits_only_observed_series_with_cumulative_buckets() {
    let site = microscaler_timing::site!("test.render", "rendered");
    site.record(Outcome::Ok, Duration::from_millis(30)); // (25 ms, 50 ms]
    site.record(Outcome::Ok, Duration::from_secs(5)); // +Inf
    let mut out = String::new();
    microscaler_timing::render_prometheus_with(
        &mut out,
        microscaler_timing::RenderOptions::default(),
    );
    let l = r#"component="test.render",op="rendered",outcome="ok""#;
    assert!(out.contains("# TYPE op_duration_seconds histogram"));
    assert!(out.contains(&format!(
        "op_duration_seconds_bucket{{{l},le=\"0.025\"}} 0\n"
    )));
    assert!(out.contains(&format!(
        "op_duration_seconds_bucket{{{l},le=\"0.05\"}} 1\n"
    )));
    assert!(out.contains(&format!("op_duration_seconds_bucket{{{l},le=\"2.5\"}} 1\n")));
    assert!(out.contains(&format!(
        "op_duration_seconds_bucket{{{l},le=\"+Inf\"}} 2\n"
    )));
    assert!(out.contains(&format!("op_duration_seconds_count{{{l}}} 2\n")));
    assert!(out.contains(&format!("op_duration_seconds_sum{{{l}}} 5.03\n")));
    // never-recorded outcomes are absent
    assert!(!out.contains(r#"op="rendered",outcome="err""#));
    assert!(!out.contains("op_in_flight"));
    // HELP/TYPE once
    assert_eq!(out.matches("# TYPE op_duration_seconds").count(), 1);
}

#[test]
fn same_labels_in_two_places_render_as_one_series() {
    let a = microscaler_timing::site!("test.render", "twice");
    let b = microscaler_timing::site!("test.render", "twice");
    assert!(!std::ptr::eq(a, b));
    a.record(Outcome::Ok, Duration::from_millis(1));
    b.record(Outcome::Ok, Duration::from_millis(1));
    let mut out = String::new();
    microscaler_timing::render_prometheus_with(
        &mut out,
        microscaler_timing::RenderOptions { in_flight: true },
    );
    let l = r#"component="test.render",op="twice",outcome="ok""#;
    assert_eq!(
        out.matches(&format!("op_duration_seconds_count{{{l}}}"))
            .count(),
        1
    );
    assert!(out.contains(&format!("op_duration_seconds_count{{{l}}} 2\n")));
    assert_eq!(
        out.matches(r#"op_in_flight{component="test.render",op="twice"} 0"#)
            .count(),
        1
    );
}

#[test]
fn interned_sites_render_like_static_ones() {
    let s = microscaler_timing::intern("test.intern", "getPetById");
    s.record(Outcome::Err, Duration::from_millis(7));
    assert_eq!(count("test.intern", "getPetById", Outcome::Err), 1);
}

// --- tiny executor, so the tests need no async runtime -------------------------------------

async fn yield_once() {
    let mut yielded = false;
    std::future::poll_fn(move |cx| {
        if yielded {
            std::task::Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    })
    .await;
}

fn noop_waker() -> std::task::Waker {
    std::task::Waker::noop().clone()
}

fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    let mut fut = std::pin::pin!(fut);
    let waker = noop_waker();
    let mut cx = std::task::Context::from_waker(&waker);
    loop {
        if let std::task::Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

use std::future::Future as _;
