//! # microscaler-observability
//!
//! Hexagonal observability adapter for the microscaler platform. This crate is
//! the **single place** in the workspace that calls the process-global
//! installers `opentelemetry::global::set_tracer_provider`,
//! `opentelemetry::global::set_logger_provider` (via the `tracing` bridge),
//! `opentelemetry::global::set_meter_provider`, and
//! `opentelemetry::global::set_text_map_propagator`.
//!
//! ## Hexagonal role
//!
//! In ports-and-adapters terms:
//!
//! - `brrtrouter` — HTTP **input + output** adapter.
//! - `lifeguard`  — Postgres **output** adapter.
//! - **this crate** — OTEL **output** adapter (logs / traces / metrics egress).
//!
//! All three adapters are **peers** of each other. None of them owns the
//! OpenTelemetry globals; this crate does. The host application's `main()` is
//! the composition root and is the only place that calls [`init`].
//!
//! ```ignore
//! fn main() -> anyhow::Result<()> {
//!     // (1) observability adapter goes up first
//!     let _obs = microscaler_observability::init(
//!         microscaler_observability::ObservabilityConfig::from_env()
//!             .with_service_name("hauliage-bff")
//!             .with_service_version(env!("CARGO_PKG_VERSION"))
//!     )?;
//!
//!     // (2) then the other adapters
//!     let pool = lifeguard::Pool::from_env()?;
//!     let router = brrtrouter::build_router_from_spec("openapi.yaml")?;
//!     brrtrouter::server::HttpServer::new(router).start("0.0.0.0:8080")?;
//!     Ok(())
//! }
//! ```
//!
//! ## What `BRRTRouter` and `Lifeguard` do
//!
//! `BRRTRouter` emits `tracing::span!` and `tracing::event!` from the HTTP
//! pipeline. `Lifeguard` emits the same from the ORM / pool / transaction
//! layers. Neither crate touches any `opentelemetry::global::*` function. The
//! subscriber installed by [`init`] — specifically the
//! [`tracing_opentelemetry::OpenTelemetryLayer`] — converts their
//! `tracing::span!` events into OTEL spans automatically.
//!
//! ## Egress invariants
//!
//! - **Logs** — flow via [`opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge`]
//!   → `LoggerProvider` → OTLP/gRPC → Collector → Loki. **Never** via stdout
//!   under load (see `docs/PRD.md` §5.1). Startup-only and panic output keep
//!   using `println!` / `eprintln!` so `kubectl logs` still shows boot +
//!   failures.
//! - **Traces** — flow via [`tracing_opentelemetry`] → `TracerProvider` →
//!   OTLP/gRPC → Collector → Jaeger (or Tempo).
//! - **Metrics** — stay on the Prometheus-text `/metrics` endpoint `BRRTRouter`
//!   serves. This crate does **not** install a `MeterProvider` today; see
//!   `docs/PRD.md` Phase O.6 for the rationale. `Lifeguard`'s
//!   `lifeguard::metrics::prometheus_scrape_text()` is concatenated into
//!   `BRRTRouter`'s scrape response by the host.
//!
//! ## When [`init`] does nothing
//!
//! If `OTEL_EXPORTER_OTLP_ENDPOINT` is unset, [`init`] installs a minimal
//! `tracing_subscriber::fmt` layer to stdout (behind the `dev-stdout-fallback`
//! feature — on by default) and returns a no-op [`ShutdownGuard`]. This keeps
//! `cargo test` and `cargo run` showing logs locally without contacting any
//! Collector.
//!
//! ## Shutdown
//!
//! The [`ShutdownGuard`] returned by [`init`] flushes the `BatchSpanProcessor`
//! and `BatchLogRecordProcessor` and then shuts the providers down on `Drop`.
//! Keep it alive for the process lifetime; drop it last before
//! `std::process::exit` so `SIGTERM` doesn't truncate telemetry.
//!
//! ## Coordinated version pins
//!
//! This crate pins `opentelemetry = "0.29"` to match
//! `lifeguard`'s pins exactly. Both crates must see the same
//! `opentelemetry::global::*` state or logs and metrics cross-talk silently.
//! Any version bump in this crate requires a coordinated bump in `lifeguard`.
//! See `docs/PRD.md` §Phase O.0.

// Crate-wide lints live in `Cargo.toml` `[lints.*]` sections (AGENTS.md
// "Golden rules"). Only top-level module attributes that the [lints] table
// cannot express go here.
#![deny(missing_docs)]

mod config;
mod error;
mod shutdown;

pub use config::{ObservabilityConfig, OtlpProtocol, Sampler};
pub use error::{ObservabilityError, ObservabilityResult};
pub use shutdown::ShutdownGuard;

/// Initialize the observability adapter: OTLP tracer, OTLP logger (if
/// endpoint is set), W3C propagator, and the single global `tracing`
/// subscriber.
///
/// # Contract
///
/// - **Called exactly once per process**, from `main()` in the composition
///   root. Calling twice is a programming error and returns
///   [`ObservabilityError::AlreadyInitialized`].
/// - If `config.endpoint` is `None` (typically because
///   `OTEL_EXPORTER_OTLP_ENDPOINT` is unset), only the stdout fallback
///   subscriber is installed and no network traffic is emitted.
/// - Sets the four global OpenTelemetry state slots this crate owns.
///   **No other crate in the workspace is permitted to touch these slots.**
///
/// # Errors
///
/// Returns [`ObservabilityError`] if the OTLP exporter cannot be constructed
/// (invalid endpoint, missing TLS config, etc.) or if another subscriber has
/// already claimed the global slot.
///
/// # Phase 1 implementation status
///
/// **Not yet implemented.** This signature is the committed API; the body
/// will be filled in as part of Phase O.1 (see `docs/PRD.md`). For the current
/// scaffolding phase, this function panics with a deliberate message so that
/// any caller can validate the integration shape without accidentally
/// succeeding against a placeholder.
///
/// # Panics
///
/// In the v0.0.1 scaffold, always panics. The shape is stable; the body lands
/// with Phase O.1.
// Crate-wide lint `clippy::unimplemented` is `deny` — we carve out here so
// this single deliberate scaffold stub compiles. Phase O.1 removes the
// `unimplemented!` and the allow.
#[allow(clippy::unimplemented)]
pub fn init(_config: ObservabilityConfig) -> ObservabilityResult<ShutdownGuard> {
    unimplemented!(
        "microscaler-observability v0.0.1 is a scaffold. \
         Phase O.1 of docs/PRD.md lands the real implementation. \
         Do not call init() yet — use BRRTRouter's legacy \
         brrtrouter::otel::init_logging_with_config() until Phase O.1 ships."
    );
}

#[cfg(test)]
mod tests {
    //! Public-API smoke tests — what v0.0.1 can prove about the scaffold
    //! without the real `init()` body.
    //!
    //! Tests deliberately allow `unwrap_used` / `expect_used` / `panic`:
    //! `assert!(result.unwrap())` is idiomatic in tests and the `deny` rule
    //! that exists for library code is the wrong trade-off in a test module.
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::missing_panics_doc
    )]

    use super::*;

    /// The v0.0.1 scaffold must panic — and the panic message must point
    /// an operator at the PRD. Regression guard: if someone accidentally
    /// merges a half-implemented `init()` that silently succeeds into a
    /// no-op, this test fails.
    #[test]
    fn init_panics_with_prd_pointer_in_scaffold() {
        let config = ObservabilityConfig::default();
        let result = std::panic::catch_unwind(|| init(config));
        let panic_payload = result.err().expect("init() must panic in v0.0.1");

        let msg = panic_payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic_payload.downcast_ref::<&str>().copied())
            .unwrap_or("<non-string panic payload>");

        assert!(
            msg.contains("docs/PRD.md"),
            "scaffold panic must point callers at the PRD; got: {msg}"
        );
        assert!(
            msg.contains("Phase O.1"),
            "scaffold panic must reference the phase that will land the real implementation; got: {msg}"
        );
    }

    // Compile-time assertions that the public API surface hasn't drifted.
    // These are `const _` items so they're fully evaluated at type-check
    // time — no runtime binding, no `no_effect_underscore_binding`.
    const _INIT_SIGNATURE_IS_STABLE: fn(ObservabilityConfig) -> ObservabilityResult<ShutdownGuard> =
        init;

    /// Ensure the canonical error and enum variants still exist under
    /// their documented names.
    #[test]
    fn public_enum_variants_still_exist() {
        assert!(matches!(
            ObservabilityError::AlreadyInitialized,
            ObservabilityError::AlreadyInitialized
        ));
        assert!(matches!(OtlpProtocol::Grpc, OtlpProtocol::Grpc));
        assert!(matches!(
            Sampler::ParentBasedAlwaysOn,
            Sampler::ParentBasedAlwaysOn
        ));
    }
}
