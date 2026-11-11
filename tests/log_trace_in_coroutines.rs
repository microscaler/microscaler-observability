//! A log emitted on a `may` coroutine inside a `may_tracing` span carries that span's trace and span ids.
//!
//! This is how hosts run: `may_tracing` never enters spans. It keeps the coroutine's span in a coroutine-local slot
//! and parents events on it explicitly (`info_in_current!`), and the coroutine can resume on another worker thread
//! after any yield. The layers `init()` installs (`otel_layers`) turn OpenTelemetry context activation off, so the
//! log bridge, which reads the current OpenTelemetry context, must be given the span's context another way. With
//! opentelemetry-appender-tracing 0.33 and nothing else, every such record lost its trace (pp, 11 Nov 2026).
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    unsafe_code
)]

use opentelemetry::trace::TraceContextExt as _;
use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use tracing_subscriber::layer::SubscriberExt;

#[test]
fn a_log_on_a_coroutine_carries_its_may_tracing_span_trace_and_span_ids() {
    // coroutine stacks are in words; exporting a span from inside one needs more than may's default
    may::config().set_stack_size(0x8000);
    let spans = InMemorySpanExporter::default();
    let logs = InMemoryLogExporter::default();
    let tracer_provider = SdkTracerProvider::builder()
        .with_simple_exporter(spans.clone())
        .build();
    let logger_provider = SdkLoggerProvider::builder()
        .with_simple_exporter(logs.clone())
        .build();
    // global: the coroutine runs on may's worker threads
    tracing::subscriber::set_global_default(tracing_subscriber::registry().with(
        microscaler_observability::otel_layers(&tracer_provider, &logger_provider),
    ))
    .expect("the only subscriber in this test binary");

    let request = tracing::info_span!("request");
    // SAFETY: may's spawn is unsafe because of thread-locals inside the coroutine; this one uses only its own data
    // and may_tracing's coroutine-local slot.
    let handle = unsafe {
        may_tracing::spawn_with(may::coroutine::Builder::new(), request.clone(), || {
            // yield so the coroutine can resume on another worker, as a handler does around I/O
            for _ in 0..8 {
                may::coroutine::yield_now();
            }
            may_tracing::info_in_current!(operation = "quote", "pw business event");
        })
    }
    .expect("spawn the coroutine");
    handle.join().expect("the coroutine finished");
    drop(request);

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
    // and nothing was left attached on this thread
    assert!(!opentelemetry::Context::current()
        .span()
        .span_context()
        .is_valid());
}
