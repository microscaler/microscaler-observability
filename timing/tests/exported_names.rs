//! The Prometheus text this crate renders has exactly one suffix per series: no `_total_total`,
//! `_seconds_seconds` or `_bytes_bytes` (which an exporter appending suffixes to already-suffixed
//! names would produce). Hosts append this text to their `/metrics`.

#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use microscaler_timing::{render_prometheus, timed};

#[timed(component = "test.exported_names")]
fn recorded() -> u32 {
    7
}

#[test]
fn rendered_names_have_exactly_one_suffix() {
    assert_eq!(recorded(), 7);
    let mut text = String::new();
    render_prometheus(&mut text);
    let mut names: Vec<&str> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| l.split(['{', ' ']).next())
        .collect();
    names.sort_unstable();
    names.dedup();
    for want in [
        "op_duration_seconds_bucket",
        "op_duration_seconds_sum",
        "op_duration_seconds_count",
    ] {
        assert!(names.contains(&want), "{want} missing from {names:?}");
    }
    for n in names {
        for doubled in ["_total_total", "_seconds_seconds", "_bytes_bytes", "_ms_ms"] {
            assert!(!n.contains(doubled), "doubled suffix in {n}");
        }
        assert!(
            n.matches("_total").count() <= 1,
            "more than one _total in {n}"
        );
    }
}
