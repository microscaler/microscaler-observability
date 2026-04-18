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
//! This crate pins **`opentelemetry = "0.31"`** (git patch aligned with
//! **`BRRTRouter`** / **`Lifeguard`**). All crates must see the same
//! `opentelemetry::global::*` types or telemetry cross-talks silently.
//! See `docs/PRD.md` §Phase O.0.
//!
//! ## Shared Kind cluster (local dev)
//!
//! With the stack in [`shared-kind-cluster`](https://github.com/microscaler/shared-kind-cluster)
//! (`tilt up` in that repo), the OTLP gRPC endpoint is typically
//! `http://otel-collector.observability.svc.cluster.local:4317` from in-cluster
//! pods, or port-forward the `otel-collector` service in namespace `observability`.

// Crate-wide lints live in `Cargo.toml` `[lints.*]` sections (AGENTS.md
// "Golden rules"). Only top-level module attributes that the [lints] table
// cannot express go here.
#![deny(missing_docs)]

mod bootstrap;
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
/// Returns [`ObservabilityError::AlreadyInitialized`] if called twice in one
/// process, [`ObservabilityError::InvalidEndpoint`] if the OTLP URL is
/// unusable, [`ObservabilityError::ExporterConstruction`] if the OTLP client
/// cannot be built, [`ObservabilityError::SubscriberAlreadyInstalled`] if
/// another global `tracing` subscriber was installed first, or
/// [`ObservabilityError::Shutdown`] only from [`ShutdownGuard`] drop (not from
/// `init` itself).
#[allow(clippy::needless_pass_by_value)] // Call sites pass freshly built `ObservabilityConfig` from `from_env()` + builders
pub fn init(config: ObservabilityConfig) -> ObservabilityResult<ShutdownGuard> {
    bootstrap::init_internal(&config)
}

#[cfg(test)]
mod tests {
    //! Public-API smoke tests.
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

    // Compile-time assertions that the public API surface hasn't drifted.
    const _INIT_SIGNATURE_IS_STABLE:
        fn(ObservabilityConfig) -> ObservabilityResult<ShutdownGuard> = init;

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
