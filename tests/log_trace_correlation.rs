//! A log emitted inside an entered span, on a plain thread, carries that span's trace and span ids, with the
//! layers `init()` installs (`otel_layers`). Spans on may coroutines, never entered, are covered by
//! `tests/log_trace_in_coroutines.rs`.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use tracing_subscriber::layer::SubscriberExt;

#[test]
fn a_log_inside_a_span_carries_the_span_trace_and_span_ids() {
    let spans = InMemorySpanExporter::default();
    let logs = InMemoryLogExporter::default();
    let tracer_provider = SdkTracerProvider::builder()
        .with_simple_exporter(spans.clone())
        .build();
    let logger_provider = SdkLoggerProvider::builder()
        .with_simple_exporter(logs.clone())
        .build();

    let subscriber = tracing_subscriber::registry().with(microscaler_observability::otel_layers(
        &tracer_provider,
        &logger_provider,
    ));
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("request");
        let _entered = span.enter();
        tracing::info!("inside the span");
    });

    let finished = spans.get_finished_spans().expect("spans");
    assert_eq!(finished.len(), 1, "one span");
    let span = &finished[0].span_context;

    let emitted = logs.get_emitted_logs().expect("logs");
    assert_eq!(emitted.len(), 1, "one log");
    let ctx = emitted[0]
        .record
        .trace_context()
        .expect("the log carries a trace context");
    assert_eq!(ctx.trace_id, span.trace_id());
    assert_eq!(ctx.span_id, span.span_id());
}
