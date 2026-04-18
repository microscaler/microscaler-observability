//! Configuration for the observability adapter.
//!
//! Values are primarily read from OTEL-standard environment variables
//! (see the OpenTelemetry specification's "Environment variable
//! specification"). This crate's own env-var names are prefixed `MICROSCALER_`
//! only when no suitable OTEL-standard variable exists.

use std::time::Duration;

/// Protocol for the OTLP exporter connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtlpProtocol {
    /// OTLP/gRPC over HTTP/2. Default. Port 4317.
    Grpc,
    /// OTLP/HTTP with protobuf body. Port 4318.
    HttpProto,
    /// OTLP/HTTP with JSON body. Port 4318.
    HttpJson,
}

impl Default for OtlpProtocol {
    fn default() -> Self {
        Self::Grpc
    }
}

/// Sampler configuration. Follows the OTEL spec's
/// `OTEL_TRACES_SAMPLER` / `OTEL_TRACES_SAMPLER_ARG` conventions.
#[derive(Debug, Clone, PartialEq)]
pub enum Sampler {
    /// Always sample. Good for dev and for low-traffic services.
    ParentBasedAlwaysOn,
    /// Sample a fraction of root spans; honour parent context otherwise.
    ParentBasedTraceIdRatio(f64),
    /// Never sample. Useful for a canary that should only carry propagated
    /// context without generating its own traces.
    AlwaysOff,
}

impl Default for Sampler {
    fn default() -> Self {
        Self::ParentBasedAlwaysOn
    }
}

/// Top-level configuration for [`crate::init`].
///
/// Prefer [`Self::from_env`] — it honours the OTEL-spec env vars exactly —
/// and override with the builder methods only when you need to.
#[derive(Debug, Clone, Default)]
pub struct ObservabilityConfig {
    /// OTLP endpoint. If `None`, no OTLP exporter is constructed and
    /// the crate falls back to a stdout `fmt::Layer` (see
    /// `dev-stdout-fallback` cargo feature).
    ///
    /// Read from `OTEL_EXPORTER_OTLP_ENDPOINT` (example
    /// `http://otel-collector:4317`).
    pub endpoint: Option<String>,

    /// Transport protocol. Read from `OTEL_EXPORTER_OTLP_PROTOCOL`.
    pub protocol: OtlpProtocol,

    /// Per-request timeout for the exporter. Read from
    /// `OTEL_EXPORTER_OTLP_TIMEOUT` (parsed as seconds, default 10s).
    pub timeout: Duration,

    /// `service.name` resource attribute. Read from `OTEL_SERVICE_NAME`.
    /// Required for useful traces — Jaeger groups by this.
    pub service_name: String,

    /// `service.version` resource attribute. Read from
    /// `OTEL_SERVICE_VERSION`, defaults to the value of
    /// `CARGO_PKG_VERSION` at the call site (caller must pass it).
    pub service_version: Option<String>,

    /// `deployment.environment` resource attribute. Conventionally
    /// `dev` / `staging` / `prod`. Read from a well-known subset of
    /// `OTEL_RESOURCE_ATTRIBUTES` or from `DEPLOYMENT_ENVIRONMENT`.
    pub deployment_environment: Option<String>,

    /// Extra resource attributes parsed from `OTEL_RESOURCE_ATTRIBUTES`
    /// (`k=v,k=v`).
    pub extra_resource_attributes: Vec<(String, String)>,

    /// Sampler.
    pub sampler: Sampler,

    /// `BatchSpanProcessor` schedule delay. Read from
    /// `OTEL_BSP_SCHEDULE_DELAY` (milliseconds, default 5000).
    pub bsp_schedule_delay: Duration,

    /// `BatchSpanProcessor` max export batch size. Read from
    /// `OTEL_BSP_MAX_EXPORT_BATCH_SIZE` (default 512).
    pub bsp_max_batch_size: usize,

    /// `BatchLogRecordProcessor` schedule delay.
    pub blrp_schedule_delay: Duration,

    /// `BatchLogRecordProcessor` max export batch size.
    pub blrp_max_batch_size: usize,

    /// `tracing` log-level filter string (merged into `EnvFilter`).
    /// Read from `RUST_LOG`.
    pub rust_log: Option<String>,

    /// Honour `BRRTR_DEV_LOGS_TO_STDOUT=1` as an escape hatch forcing the
    /// stdout fallback even when OTLP is configured. For break-glass local
    /// debugging only.
    pub dev_logs_to_stdout_override: bool,
}

impl ObservabilityConfig {
    /// Construct from environment variables per the OpenTelemetry spec.
    ///
    /// **Unimplemented** in v0.0.1 — see `docs/PRD.md` Phase O.1.
    ///
    /// # Panics
    ///
    /// Always panics in the v0.0.1 scaffold. Callers should build a
    /// [`Self`] via [`Self::default`] + the builder methods until Phase O.1
    /// lands the real env-parsing body.
    // Crate-wide `clippy::unimplemented` is `deny`. Allowed locally on the
    // deliberate scaffold stub. Phase O.1 removes the `unimplemented!` and
    // the allow.
    #[allow(clippy::unimplemented)]
    #[must_use]
    pub fn from_env() -> Self {
        unimplemented!("Phase O.1 of docs/PRD.md implements ObservabilityConfig::from_env.")
    }

    /// Override the service name. Overrides `OTEL_SERVICE_NAME`.
    #[must_use]
    pub fn with_service_name(mut self, name: impl Into<String>) -> Self {
        self.service_name = name.into();
        self
    }

    /// Override the service version.
    #[must_use]
    pub fn with_service_version(mut self, version: impl Into<String>) -> Self {
        self.service_version = Some(version.into());
        self
    }

    /// Override the deployment environment (`dev` / `staging` / `prod`).
    #[must_use]
    pub fn with_deployment_environment(mut self, env: impl Into<String>) -> Self {
        self.deployment_environment = Some(env.into());
        self
    }

    /// Override the sampler.
    #[must_use]
    pub const fn with_sampler(mut self, sampler: Sampler) -> Self {
        self.sampler = sampler;
        self
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::missing_panics_doc
    )]

    use super::*;

    #[test]
    fn otlp_protocol_default_is_grpc() {
        assert_eq!(OtlpProtocol::default(), OtlpProtocol::Grpc);
    }

    #[test]
    fn sampler_default_is_parent_based_always_on() {
        assert_eq!(Sampler::default(), Sampler::ParentBasedAlwaysOn);
    }

    #[test]
    fn config_default_has_no_endpoint_and_grpc_protocol() {
        let config = ObservabilityConfig::default();
        assert!(
            config.endpoint.is_none(),
            "default config must not auto-enable OTLP (preserves behaviour for services with no env vars)"
        );
        assert_eq!(config.protocol, OtlpProtocol::Grpc);
        assert_eq!(config.sampler, Sampler::ParentBasedAlwaysOn);
        assert!(!config.dev_logs_to_stdout_override);
    }

    #[test]
    fn builder_sets_service_name() {
        let config = ObservabilityConfig::default().with_service_name("hauliage-bff");
        assert_eq!(config.service_name, "hauliage-bff");
    }

    #[test]
    fn builder_sets_service_version() {
        let config = ObservabilityConfig::default().with_service_version("1.2.3");
        assert_eq!(config.service_version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn builder_sets_deployment_environment() {
        let config = ObservabilityConfig::default().with_deployment_environment("prod");
        assert_eq!(config.deployment_environment.as_deref(), Some("prod"));
    }

    #[test]
    fn builder_sets_sampler() {
        let config =
            ObservabilityConfig::default().with_sampler(Sampler::ParentBasedTraceIdRatio(0.5));
        assert_eq!(config.sampler, Sampler::ParentBasedTraceIdRatio(0.5));
    }

    #[test]
    fn builders_chain() {
        let config = ObservabilityConfig::default()
            .with_service_name("hauliage-fleet")
            .with_service_version("0.1.0")
            .with_deployment_environment("dev")
            .with_sampler(Sampler::AlwaysOff);
        assert_eq!(config.service_name, "hauliage-fleet");
        assert_eq!(config.service_version.as_deref(), Some("0.1.0"));
        assert_eq!(config.deployment_environment.as_deref(), Some("dev"));
        assert_eq!(config.sampler, Sampler::AlwaysOff);
    }

    #[test]
    fn from_env_panics_in_scaffold() {
        // v0.0.1 scaffold regression guard: from_env() panics with a
        // pointer at the PRD. Phase O.1 replaces this with real parsing.
        let result = std::panic::catch_unwind(ObservabilityConfig::from_env);
        assert!(result.is_err(), "from_env must panic in v0.0.1 scaffold");
    }
}
