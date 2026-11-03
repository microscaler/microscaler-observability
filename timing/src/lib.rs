//! # microscaler-timing
//!
//! Annotate a function and its p50/p95/p99 appear on the dashboard: no metric to declare, no
//! dashboard to edit. Every timed operation lands in **one** histogram family,
//!
//! ```text
//! op_duration_seconds{component, op, outcome}
//! ```
//!
//! rendered on the service's `/metrics` (`BRRTRouter` appends it to its scrape) and read by one
//! generic dashboard templated on `component` and `op`.
//!
//! ```
//! use microscaler_timing::{timed, timed_block, Outcome};
//!
//! #[timed(component = "billing.stripe")]            // op = "create_invoice"
//! fn create_invoice(amount: u64) -> Result<u64, std::io::Error> {
//!     Ok(amount)                                       // Ok -> outcome="ok", Err -> "err"
//! }
//!
//! #[timed(component = "orders", op = "load")]         // async works too
//! async fn load_order() -> u32 { 7 }
//!
//! fn decode(rows: &[u8]) -> usize {
//!     timed_block!("decode_rows", { rows.len() })     // component = this module's path
//! }
//!
//! // A guard for spans the macros do not fit:
//! static CHECKOUT: microscaler_timing::Site = microscaler_timing::Site::new("pool", "checkout");
//! let timer = CHECKOUT.start();
//! /* ... */
//! timer.finish(Outcome::Ok);
//! # let _ = create_invoice(1); let _ = decode(&[]);
//! ```
//!
//! ## Labels: static only
//!
//! `component` and `op` are `&'static str` (string literals in the macros) and must name *code*,
//! never *data*: no ids, tickers, URLs, user names, SQL text. Each distinct (component, op,
//! outcome) is one histogram per pod, billed by Managed Prometheus on every scrape. Runtime names
//! from a bounded set fixed at startup (`OpenAPI` `operationId`s) go through [`intern`] once, at
//! setup.
//!
//! - `component`: crate / subsystem, dotted (`brrtrouter.client`, `lifeguard.pool`). Defaults to
//!   the module path (`::` is rendered as `.`).
//! - `op`: the operation, `snake_case` or the spec's operationId. Defaults to the function name.
//! - `outcome`: `ok` | `err` | `timeout` (set by the macros from `Result`, see [`IsTimeout`]).
//!
//! ## Safe on `may` coroutines
//!
//! Recording is a few relaxed atomic adds on a `static` and never blocks an OS thread: no mutex,
//! no lazy initialisation, no thread-locals. The start time lives in the [`Timer`] on the
//! caller's stack, so a coroutine that migrates between workers mid-call is timed correctly.
//! Only [`intern`] (setup time) takes a lock.
//!
//! ## One copy per binary
//!
//! Sites register in a process-wide list owned by this crate. Two copies of the crate in one
//! binary (two different git revisions) would each keep their own list and the second would never
//! be rendered: depend on it as `{ git = "...microscaler-observability.git", branch = "main" }`
//! everywhere, and check with `cargo tree -d -i microscaler-timing`.

mod intern;
mod render;
mod site;

pub use intern::{intern, MAX_INTERNED};
pub use microscaler_timing_macros::timed;
pub use render::{
    render_prometheus, render_prometheus_with, snapshot, OpSnapshot, RenderOptions,
    IN_FLIGHT_METRIC, METRIC,
};
pub use site::{Outcome, Site, Timer, BUCKET_BOUNDS_NANOS};

/// An error that can say it was a timeout.
///
/// `#[timed]` then records `outcome="timeout"` instead of `"err"`. Implemented for [`std::io::Error`] (`TimedOut`, `WouldBlock`); implement it for your
/// own error types.
pub trait IsTimeout {
    /// Whether this error means the operation ran out of time.
    fn is_timeout(&self) -> bool;
}

impl IsTimeout for std::io::Error {
    fn is_timeout(&self) -> bool {
        matches!(
            self.kind(),
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
        )
    }
}

impl<E: IsTimeout + ?Sized> IsTimeout for Box<E> {
    fn is_timeout(&self) -> bool {
        (**self).is_timeout()
    }
}

/// A `&'static Site` for (`component`, `op`), declared in place.
///
/// ```
/// let site = microscaler_timing::site!("lifeguard.pool", "checkout");
/// let t = site.start();
/// t.finish(microscaler_timing::Outcome::Ok);
/// let here = microscaler_timing::site!("decode"); // component = module path
/// # let _ = here;
/// ```
#[macro_export]
macro_rules! site {
    ($op:literal) => {{
        static __MS_TIMING_SITE: $crate::Site = $crate::Site::new(::core::module_path!(), $op);
        &__MS_TIMING_SITE
    }};
    ($component:literal, $op:literal) => {{
        static __MS_TIMING_SITE: $crate::Site = $crate::Site::new($component, $op);
        &__MS_TIMING_SITE
    }};
}

/// Time a block: `timed_block!("op", { ... })` or `timed_block!("component", "op", { ... })`.
///
/// Evaluates to the block's value and records `ok` (also on an early `return`/`?` out of it,
/// which leaves the block normally as far as the timer is concerned); `err` if it panics. Use
/// [`Site::start`] and [`Timer::finish`] when the outcome depends on the value. The block is
/// inlined, so `.await` inside it works.
#[macro_export]
macro_rules! timed_block {
    ($op:literal, $body:block) => {{
        let __ms_timer = $crate::site!($op).start();
        let __ms_value = $body;
        __ms_timer.finish($crate::Outcome::Ok);
        __ms_value
    }};
    ($component:literal, $op:literal, $body:block) => {{
        let __ms_timer = $crate::site!($component, $op).start();
        let __ms_value = $body;
        __ms_timer.finish($crate::Outcome::Ok);
        __ms_value
    }};
}

/// Support for the `#[timed]` expansion. Not public API.
#[doc(hidden)]
pub mod __private {
    use crate::{IsTimeout, Outcome};

    /// Wraps an error so the expansion can ask whether it is a timeout without requiring every
    /// error type to implement [`IsTimeout`] (autoref specialisation).
    pub struct Probe<'a, E: ?Sized>(pub &'a E);

    /// Chosen when `E: IsTimeout`.
    pub trait ViaTimeout {
        fn __ms_outcome(&self) -> Outcome;
    }
    impl<E: IsTimeout + ?Sized> ViaTimeout for Probe<'_, E> {
        fn __ms_outcome(&self) -> Outcome {
            if self.0.is_timeout() {
                Outcome::Timeout
            } else {
                Outcome::Err
            }
        }
    }

    /// Fallback for any other error type.
    pub trait ViaPlain {
        fn __ms_outcome(&self) -> Outcome;
    }
    impl<E: ?Sized> ViaPlain for &Probe<'_, E> {
        fn __ms_outcome(&self) -> Outcome {
            Outcome::Err
        }
    }
}
