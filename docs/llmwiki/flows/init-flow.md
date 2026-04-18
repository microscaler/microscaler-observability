# `init()` flow — what happens when the observability adapter is installed

> **Status:** DRAFT
> **Last-synced:** 2026-04-18 — against `../../../src/lib.rs` (v0.0.1 scaffold) and `../../PRD.md` §Phase O.1 (the target implementation).
> **Authority:** `../../../src/lib.rs::init` + `../../PRD.md` §Phase O.1 "Code (in this repo)".
> **Related:** [`../entities/entity-shutdown-guard.md`](../entities/entity-shutdown-guard.md), [`../topics/hexagonal-architecture.md`](../topics/hexagonal-architecture.md).

## What this page covers

Step-by-step of what `microscaler_observability::init(config)` does today (panics intentionally) and what it will do once Phase O.1 lands (build the tracer + logger pipelines, install globals, compose the `tracing` subscriber, return a `ShutdownGuard`).

## Today (v0.0.1)

```rust
// src/lib.rs
pub fn init(_config: ObservabilityConfig) -> ObservabilityResult<ShutdownGuard> {
    unimplemented!(
        "microscaler-observability v0.0.1 is a scaffold. \
         Phase O.1 of docs/PRD.md lands the real implementation. \
         Do not call init() yet — use BRRTRouter's legacy \
         brrtrouter::otel::init_logging_with_config() until Phase O.1 ships."
    );
}
```

Any caller panics with an explicit instruction to switch to BRRTRouter's current stub. This is deliberate: we don't want services to accidentally "succeed" against a no-op init and ship with silent observability.

## Target flow (Phase O.1)

```
init(config)
  │
  ▼
┌──────────────────────────────────────────────────────────────────┐
│ 1. Install-once guard                                            │
│    if INITIALIZED.compare_exchange(false, true, …).is_err() {    │
│        return Err(ObservabilityError::AlreadyInitialized);       │
│    }                                                             │
└──────────────────────────────────────────────────────────────────┘
  │
  ▼
┌──────────────────────────────────────────────────────────────────┐
│ 2. Build Resource (service.name, service.version, deployment…)   │
│    Resource::new([                                               │
│        KeyValue::new("service.name", config.service_name),       │
│        KeyValue::new("service.version", config.service_version), │
│        KeyValue::new("deployment.environment", config.env),      │
│        // + extras parsed from OTEL_RESOURCE_ATTRIBUTES          │
│    ])                                                            │
└──────────────────────────────────────────────────────────────────┘
  │
  ├─── config.endpoint is Some(url)? ──────────────────────────────┐
  │                                                                │
  │                     YES (OTLP wire-up)                NO (dev) │
  │                                                                │
  ▼                                                                ▼
┌───────────────────────────────────────────┐   ┌───────────────────────────┐
│ 3a. Build SpanExporter (gRPC-tonic)       │   │ 3b. No exporter; compose  │
│     + BatchSpanProcessor                  │   │     subscriber with only  │
│     + TracerProvider                      │   │     a fmt::Layer to       │
│     global::set_tracer_provider(…)        │   │     stdout (if the        │
│                                           │   │     `dev-stdout-fallback` │
│ 3c. Build LogExporter (gRPC-tonic)        │   │     feature is on).       │
│     + BatchLogRecordProcessor             │   │                           │
│     + LoggerProvider                      │   │ Return noop ShutdownGuard.│
│                                           │   │                           │
│ 3d. global::set_text_map_propagator(      │   │ Exit.                     │
│         TraceContextPropagator::new())    │   │                           │
└───────────────────────────────────────────┘   └───────────────────────────┘
  │
  ▼
┌──────────────────────────────────────────────────────────────────┐
│ 4. Compose the single tracing_subscriber::Registry               │
│                                                                  │
│    Registry::default()                                           │
│        .with(EnvFilter::try_from_default_env()                   │
│                .unwrap_or_else(|_| EnvFilter::new("info")))      │
│        .with(RedactionLayer::new(config.redact_level))           │
│        .with(SamplingLayer::new(config.sampling_mode,            │
│                                 config.sampling_rate))           │
│        // -- span bridge:                                        │
│        .with(tracing_opentelemetry::layer()                      │
│                .with_tracer(tracer_provider.tracer("microscaler")))│
│        // -- log bridge (replaces the old fmt::Layer):           │
│        .with(OpenTelemetryTracingBridge::new(&logger_provider))  │
│        // -- optional stdout fallback for break-glass debugging: │
│        .with(config.dev_logs_to_stdout_override                  │
│                .then(|| fmt::Layer::new().json().with_writer(…)))│
│        .try_init()?;                                             │
└──────────────────────────────────────────────────────────────────┘
  │
  ▼
┌──────────────────────────────────────────────────────────────────┐
│ 5. Build and return ShutdownGuard                                │
│    ShutdownGuard { _tracer_provider, _logger_provider, … }       │
│    — see `../entities/entity-shutdown-guard.md`                  │
└──────────────────────────────────────────────────────────────────┘
  │
  ▼
Ok(ShutdownGuard)
```

## Why the ordering matters

1. **Install-once guard before anything else** — if a second call races (e.g. a unit test that forgot to use `set_default` + an embedded binary that also initialises), we want a clean `Err` instead of two providers fighting.
2. **Resource before providers** — both the tracer and logger providers inherit the same resource. Building it first lets us avoid constructing two identical resources.
3. **Providers + propagator before subscriber** — the subscriber's `tracing_opentelemetry::layer()` needs the tracer handle; the `OpenTelemetryTracingBridge` needs the logger provider. Those must exist first.
4. **Subscriber last** — `tracing_subscriber::Registry::try_init()` is the point-of-no-return. Once it returns, global subscriber is installed and can't be replaced process-wide. We want every previous step validated before we burn that single slot.
5. **ShutdownGuard after subscriber** — the guard is the receipt that init succeeded. If any earlier step returns `Err`, the caller gets a clean error and nothing is installed.

## What happens in failure modes

| Failure | Effect |
|---|---|
| Endpoint string is not a valid URL | `Err(InvalidEndpoint)` before any global is touched. Subscriber not installed. Process has no OTEL. |
| gRPC exporter can't reach Collector on first RPC | Exporter constructs fine (gRPC is lazy); subsequent BSP flushes log failures via the exporter's own internal `tracing::warn!` — which by that point is going *through the log bridge* to the same exporter. Infinite loop risk; the `opentelemetry` crate guards against this by never instrumenting its own export path with `tracing`. |
| `tracing_subscriber::try_init()` fails because another subscriber is already installed | `Err(SubscriberAlreadyInstalled)`. Providers installed in steps 3a–3d are still globally set — that's a partial init. Phase O.1 tests must cover this; the mitigation is probably a rollback of steps 3a–3d on subscriber failure, but it's not trivial because the OTEL 0.29 API doesn't have a documented "unset" for globals. |
| `OTEL_EXPORTER_OTLP_ENDPOINT` unset and `dev-stdout-fallback` feature off | No OTLP pipeline, no stdout pipeline. `tracing::*` macros no-op. `tracing_subscriber::fmt::init()` is NOT called. Return a noop `ShutdownGuard`. |

## Where `BRRTR_DEV_LOGS_TO_STDOUT=1` fits

If set, step 4 adds a `fmt::Layer` to stdout **in addition to** the OTLP bridge. This is the break-glass override: an operator debugging a live incident via `kubectl logs` gets both streams simultaneously, accepting the log volume cost. Default is off, per PRD v0.4 §5.3 (stdout invariant).

## Test strategy (Phase O.1 deliverable)

`tests/otlp_roundtrip.rs` plan:

1. Start an in-process OTLP gRPC receiver (a tonic service implementing `TraceService::export` and `LogsService::export` — store the received batches in a `Mutex<Vec<ExportTraceServiceRequest>>`).
2. Call `init(ObservabilityConfig::for_test(endpoint))`.
3. Inside an `info_span!("test_span", foo = "bar")`, emit `tracing::info!(result = "ok", "test message")`.
4. Drop the `ShutdownGuard`.
5. Assert the receiver got exactly one `Span` with name `test_span`, field `foo=bar`.
6. Assert the receiver got exactly one `LogRecord` with body `"test message"`, field `result="ok"`, and populated `trace_id` / `span_id` matching the span from step 3.

The roundtrip gives us end-to-end evidence of correctness without depending on a live Collector + Jaeger.

## Open questions

> **Open:** Step 4's partial-init on subscriber failure — do we attempt rollback (unset globals) or document that `init()` returning `Err` leaves a half-installed state? Phase O.1 decides. Current bias: document the half-install and recommend callers `std::process::exit(1)` on any `init()` error, rather than attempting a clean-but-unreliable unset.

> **Open:** `BatchSpanProcessor` in `opentelemetry_sdk` 0.29 wants a Tokio runtime (`rt-tokio-current-thread`). BRRTRouter runs on `may` coroutines, not Tokio. Needs verification: does pulling in `rt-tokio-current-thread` for the BSP's background flusher coexist cleanly with the main thread being driven by `may`? Likely yes (Tokio current-thread runtime on its own OS thread), but Phase O.1 tests must confirm no deadlocks.
