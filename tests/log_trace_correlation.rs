//! A log emitted inside a tracing span carries that span's trace and span ids.
//!
//! Until opentelemetry-appender-tracing 0.31 this needed its `experimental_use_tracing_span_context`
//! feature. From 0.32 the bridge reads the current OpenTelemetry context, which tracing-opentelemetry
//! activates when a span is entered (`with_context_activation`, on by default). This pins that the
//! layers `init()` composes still correlate logs with traces.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
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

    let subscriber = tracing_subscriber::registry()
        .with(
            tracing_opentelemetry::layer().with_tracer(tracer_provider.tracer("correlation-test")),
        )
        .with(OpenTelemetryTracingBridge::new(&logger_provider));

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
