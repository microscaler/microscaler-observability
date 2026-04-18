# `ShutdownGuard` — RAII flush handle

> **Status:** DRAFT
> **Last-synced:** 2026-04-18 — against `../../../src/shutdown.rs` + `../../PRD.md` Phase O.5.
> **Authority:** `../../../src/shutdown.rs` (the type) + `../../PRD.md` §Phase O.5 (the flush sequence it will execute once Phase O.1 lands).
> **Related:** [`../flows/init-flow.md`](../flows/init-flow.md), [`../topics/hexagonal-architecture.md`](../topics/hexagonal-architecture.md).

## What this page covers

The RAII handle returned by [`crate::init`](../../../src/lib.rs). What it does today (nothing — v0.0.1 scaffold). What it will do once Phase O.1 implements it. How callers are expected to hold it. What happens on `Drop`.

## Today (v0.0.1)

```rust
// src/shutdown.rs
#[must_use = "dropping the ShutdownGuard immediately triggers telemetry flush; hold it for the process lifetime"]
pub struct ShutdownGuard {
    _private: (),
}

impl ShutdownGuard {
    pub(crate) const fn noop() -> Self { Self { _private: () } }
}

impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        // Phase O.1 fills in the real flush sequence.
    }
}
```

`init()` panics in v0.0.1, so no `ShutdownGuard` is ever actually constructed outside `noop()` (which is dead code until Phase O.1). The public type is pinned so consumers' integration shapes can validate.

## Target shape (Phase O.1)

```rust
pub struct ShutdownGuard {
    // Held so the BSP background thread survives.
    _tracer_provider: Arc<TracerProvider>,
    // Held so the BLRP background thread survives.
    _logger_provider: Arc<LoggerProvider>,
    // Held so the tracing_appender background thread survives.
    _fmt_layer_guard: Option<WorkerGuard>,
    // Held so the Pyroscope sampler thread survives (Phase O.12).
    #[cfg(feature = "profiling")]
    _pyroscope_handle: Option<PyroscopeAgent<PyroscopeAgentRunning>>,
}

impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        // 1. Force-flush the span exporter (5s timeout).
        let _ = self._tracer_provider.force_flush();
        // 2. Force-flush the log exporter (5s timeout).
        let _ = self._logger_provider.force_flush();
        // 3. Shutdown both providers.
        let _ = self._tracer_provider.shutdown();
        let _ = self._logger_provider.shutdown();
        // 4. Drop the non-blocking writer guard (flushes any buffered lines).
        drop(self._fmt_layer_guard.take());
        // 5. Stop the Pyroscope agent.
        #[cfg(feature = "profiling")]
        if let Some(agent) = self._pyroscope_handle.take() {
            let _ = agent.stop();
        }
    }
}
```

Rationale for the order:

1. Flushing first means any in-flight batches get sent before the exporter's transport shuts down.
2. Shutting down the providers gives them a chance to signal their background threads cleanly.
3. Dropping the log appender guard is last because, if OTLP export has failed catastrophically, the stdout-fallback fmt layer may be the only record we have of the shutdown itself.
4. Pyroscope stop is independent of OTLP.

## How callers hold it

From a host `main()`:

```rust
fn main() -> anyhow::Result<()> {
    // (1) observability adapter goes up first
    let _obs = microscaler_observability::init(
        microscaler_observability::ObservabilityConfig::from_env()
            .with_service_name("hauliage-bff")
            .with_service_version(env!("CARGO_PKG_VERSION"))
    )?;
    //   ^ the _ prefix + `#[must_use]` on the type means:
    //     - cargo clippy complains if you don't bind it
    //     - the leading underscore says "I know I'm not reading it,
    //       I'm keeping it alive for drop-time side effects"

    // (2) then the other adapters
    let pool = lifeguard::Pool::from_env()?;
    let router = brrtrouter::build_router_from_spec("openapi.yaml")?;

    // (3) run — this is a blocking call in BRRTRouter's may-coroutine model
    brrtrouter::server::HttpServer::new(router).start("0.0.0.0:8080")?;

    // (4) _obs dropped here, flush happens before Ok(()) returns to the runtime
    Ok(())
}
```

## What NOT to do

### Don't re-bind the guard to `()`

```rust
// BAD — drops the guard immediately, triggering flush, then running the server
// with no telemetry pipeline.
let () = microscaler_observability::init(config)?;
```

`#[must_use]` should catch this at compile time; the diagnostic is "value implementing `Drop` is held for less than the function's lifetime".

### Don't install a SIGTERM handler that *kills* the process before `main()` returns

If you `std::process::exit(0)` directly on SIGTERM, `Drop` impls in `main()`'s scope don't run. Use a flag + a cooperative loop exit instead, so control flow returns to `main()` and the guard's `Drop` fires naturally. BRRTRouter's `HttpServer::start` supports this via its `ServerHandle::stop()` API (see BRRTRouter `llmwiki/` for the server shutdown flow).

### Don't call `init()` twice in tests

For tests that need a subscriber scoped to a single test, use `tracing::subscriber::set_default(subscriber)` — it's process-scoped, returns a guard, and cleans up when dropped. `init()` is for `main()` only. Calling it twice returns [`ObservabilityError::AlreadyInitialized`](../../../src/error.rs).

### Don't `mem::forget` the guard to "extend" its lifetime

Leaking it defeats the whole point: `Drop` never runs, pending batches never flush, shutdown never closes the exporter. The process hangs or loses the last few seconds of telemetry. If you want "alive for the entire process" semantics, just let `main()` own the binding — that's already what the pattern produces.

## Test-harness pattern (future `microscaler_observability::testing`)

For integration tests, Phase O.1 adds a helper:

```rust
// In a test:
let _test_guard = microscaler_observability::testing::TestSubscriber::builder()
    .with_otlp_endpoint_mock()   // spawn an in-process mock collector
    .install();
// _test_guard lifetime is the test; install() uses set_default internally,
// not set_global_default, so per-test parallelism works.
```

Separate from the production `ShutdownGuard` but analogous in spirit.

## Open questions

> **Open:** Should `Drop` log its own "observability shut down, last spans flushed at HH:MM:SS" line? Trade-off: useful for SRE post-mortems vs noise in well-behaved processes. Probably yes, with a trace! level.

> **Open:** What's the right flush timeout? Phase O.5 draft says 5 s. Verify against BRRTRouter's typical SIGTERM-to-hard-kill window (from Kubernetes) and Jaeger's ingest latency tolerance.
