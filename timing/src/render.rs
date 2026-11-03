//! Prometheus text exposition of every site that has recorded.
//!
//! Runs on the scrape (`/metrics`), not on the record path, so it may allocate. It reads the
//! atomics without locking: a scrape that races a record can see a bucket incremented but not
//! yet the sum (or the reverse), which Prometheus tolerates; `_count` is derived from the
//! buckets, so it always equals the `+Inf` bucket.

use std::fmt::Write as _;
use std::sync::atomic::Ordering;

use crate::site::{linked_sites, Outcome, BUCKETS, BUCKET_BOUNDS_NANOS};

/// The histogram family every timed operation is rendered under.
pub const METRIC: &str = "op_duration_seconds";
/// Levels kept beside the timings ([`crate::Gauge`], [`crate::GaugeFn`]).
pub const GAUGE_METRIC: &str = "op_gauge";
/// Gauge of calls in flight per operation (rendered only when asked for: see [`RenderOptions`]).
pub const IN_FLIGHT_METRIC: &str = "op_in_flight";

/// `le` label values, matching [`BUCKET_BOUNDS_NANOS`].
const LE: [&str; 11] = [
    "0.001", "0.0025", "0.005", "0.01", "0.025", "0.05", "0.1", "0.25", "0.5", "1", "2.5",
];

/// One (component, op, outcome) series as of now. Several sites with the same labels (the same
/// op timed in two places) are merged into one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpSnapshot {
    /// `component` label (`::` rendered as `.`).
    pub component: String,
    /// `op` label.
    pub op: &'static str,
    /// `outcome` label.
    pub outcome: Outcome,
    /// Non-cumulative bucket counts: one per bound in [`crate::BUCKET_BOUNDS_NANOS`], then `+Inf`.
    pub buckets: [u64; BUCKETS],
    /// Sum of all durations, nanoseconds.
    pub sum_nanos: u64,
}

impl OpSnapshot {
    /// Calls recorded.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.buckets.iter().sum()
    }
}

/// What [`render_prometheus_with`] adds beyond the histogram.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderOptions {
    /// Also render `op_in_flight{component,op}` (one more series per operation; off by default
    /// because Managed Prometheus bills every series on every scrape).
    pub in_flight: bool,
}

impl RenderOptions {
    /// From the environment: `MICROSCALER_TIMING_IN_FLIGHT=1` turns the in-flight gauge on.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            in_flight: std::env::var("MICROSCALER_TIMING_IN_FLIGHT")
                .is_ok_and(|v| matches!(v.trim(), "1" | "true" | "on" | "yes")),
        }
    }
}

fn component_label(component: &str) -> String {
    component.replace("::", ".")
}

/// Every series with at least one call, sorted by (component, op, outcome).
#[must_use]
pub fn snapshot() -> Vec<OpSnapshot> {
    let mut out: Vec<OpSnapshot> = Vec::new();
    for site in linked_sites() {
        for outcome in Outcome::ALL {
            let Some(series) = site.series.get(outcome as usize) else {
                continue;
            };
            let mut buckets = [0u64; BUCKETS];
            for (slot, counter) in buckets.iter_mut().zip(series.buckets.iter()) {
                *slot = counter.load(Ordering::Relaxed);
            }
            if buckets.iter().all(|&b| b == 0) {
                continue;
            }
            out.push(OpSnapshot {
                component: component_label(site.component()),
                op: site.op(),
                outcome,
                buckets,
                sum_nanos: series.sum_nanos.load(Ordering::Relaxed),
            });
        }
    }
    out.sort_by(|a, b| {
        (a.component.as_str(), a.op, a.outcome).cmp(&(b.component.as_str(), b.op, b.outcome))
    });
    // Same labels from different sites: one series.
    out.dedup_by(|later, kept| {
        if later.component == kept.component && later.op == kept.op && later.outcome == kept.outcome
        {
            for (k, l) in kept.buckets.iter_mut().zip(later.buckets.iter()) {
                *k = k.saturating_add(*l);
            }
            kept.sum_nanos = kept.sum_nanos.saturating_add(later.sum_nanos);
            true
        } else {
            false
        }
    });
    out
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// Append `op_duration_seconds` (and, if asked, `op_in_flight`) in Prometheus text format.
/// Appends nothing until some operation has recorded.
pub fn render_prometheus_with(out: &mut String, options: RenderOptions) {
    render_gauges(out);
    let series = snapshot();
    if series.is_empty() {
        return;
    }
    writeln!(
        out,
        "# HELP {METRIC} Duration of timed operations (microscaler-timing), by component, op and outcome."
    ).unwrap_or(());
    writeln!(out, "# TYPE {METRIC} histogram").unwrap_or(());
    for s in &series {
        let labels = format!(
            "component=\"{}\",op=\"{}\",outcome=\"{}\"",
            escape(&s.component),
            escape(s.op),
            s.outcome.as_str()
        );
        let mut cumulative = 0u64;
        for (le, count) in LE.iter().zip(s.buckets.iter()) {
            cumulative = cumulative.saturating_add(*count);
            writeln!(out, "{METRIC}_bucket{{{labels},le=\"{le}\"}} {cumulative}").unwrap_or(());
        }
        let total = s.count();
        writeln!(out, "{METRIC}_bucket{{{labels},le=\"+Inf\"}} {total}").unwrap_or(());
        #[allow(clippy::cast_precision_loss)] // seconds as f64, as Prometheus wants
        let sum = s.sum_nanos as f64 / 1e9;
        writeln!(out, "{METRIC}_sum{{{labels}}} {sum}").unwrap_or(());
        writeln!(out, "{METRIC}_count{{{labels}}} {total}").unwrap_or(());
    }
    if options.in_flight {
        render_in_flight(out);
    }
    debug_assert_eq!(LE.len(), BUCKET_BOUNDS_NANOS.len());
}

/// Append `op_duration_seconds` in Prometheus text format, with [`RenderOptions::from_env`].
pub fn render_prometheus(out: &mut String) {
    render_prometheus_with(out, RenderOptions::from_env());
}

fn render_in_flight(out: &mut String) {
    let mut rows: Vec<(String, &'static str, i64)> = linked_sites()
        .map(|s| (component_label(s.component()), s.op(), s.in_flight()))
        .collect();
    rows.sort_by(|a, b| (a.0.as_str(), a.1).cmp(&(b.0.as_str(), b.1)));
    rows.dedup_by(|later, kept| {
        if later.0 == kept.0 && later.1 == kept.1 {
            kept.2 = kept.2.saturating_add(later.2);
            true
        } else {
            false
        }
    });
    writeln!(
        out,
        "# HELP {IN_FLIGHT_METRIC} Timed operations currently in flight (microscaler-timing)."
    )
    .unwrap_or(());
    writeln!(out, "# TYPE {IN_FLIGHT_METRIC} gauge").unwrap_or(());
    for (component, op, n) in rows {
        writeln!(
            out,
            "{IN_FLIGHT_METRIC}{{component=\"{}\",op=\"{}\"}} {n}",
            escape(&component),
            escape(op)
        )
        .unwrap_or(());
    }
}

fn render_gauges(out: &mut String) {
    let mut rows = crate::gauge::gauge_values();
    if rows.is_empty() {
        return;
    }
    rows.sort_unstable_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    writeln!(
        out,
        "# HELP {GAUGE_METRIC} Levels kept beside the operation timings (microscaler-timing)."
    )
    .unwrap_or(());
    writeln!(out, "# TYPE {GAUGE_METRIC} gauge").unwrap_or(());
    for (component, op, v) in rows {
        writeln!(
            out,
            "{GAUGE_METRIC}{{component=\"{}\",op=\"{}\"}} {v}",
            escape(&component_label(component)),
            escape(op)
        )
        .unwrap_or(());
    }
}
