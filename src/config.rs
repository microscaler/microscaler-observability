//! Configuration for the observability adapter.
//!
//! Values are primarily read from OTEL-standard environment variables
//! (see the OpenTelemetry specification's "Environment variable
//! specification"). This crate's own env-var names are prefixed `MICROSCALER_`
//! only when no suitable OTEL-standard variable exists.

use std::env;
use std::time::Duration;

/// Protocol for the OTLP exporter connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OtlpProtocol {
    /// OTLP/gRPC over HTTP/2. Default. Port 4317.
    #[default]
    Grpc,
    /// OTLP/HTTP with protobuf body. Port 4318.
    HttpProto,
    /// OTLP/HTTP with JSON body. Port 4318.
    HttpJson,
}

/// Sampler configuration. Follows the OTEL spec's
/// `OTEL_TRACES_SAMPLER` / `OTEL_TRACES_SAMPLER_ARG` conventions.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Sampler {
    /// Always sample. Good for dev and for low-traffic services.
    #[default]
    ParentBasedAlwaysOn,
    /// Sample a fraction of root spans; honour parent context otherwise.
    ParentBasedTraceIdRatio(f64),
    /// Never sample. Useful for a canary that should only carry propagated
    /// context without generating its own traces.
    AlwaysOff,
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
    #[must_use]
    pub fn from_env() -> Self {
        let endpoint = env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let protocol = parse_otlp_protocol(env::var("OTEL_EXPORTER_OTLP_PROTOCOL").ok().as_deref());

        let timeout_ms = env::var("OTEL_EXPORTER_OTLP_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10_000_u64);
        let timeout = Duration::from_millis(timeout_ms);

        let service_name = env::var("OTEL_SERVICE_NAME")
            .unwrap_or_else(|_| "unknown_service".to_string());

        let service_version = env::var("OTEL_SERVICE_VERSION").ok();

        let deployment_environment = env::var("DEPLOYMENT_ENVIRONMENT").ok();

        let mut extra_resource_attributes = parse_otel_resource_attributes(
            env::var("OTEL_RESOURCE_ATTRIBUTES").ok().as_deref(),
        );

        let sampler = parse_otel_traces_sampler(
            env::var("OTEL_TRACES_SAMPLER").ok().as_deref(),
            env::var("OTEL_TRACES_SAMPLER_ARG").ok().as_deref(),
        );

        let bsp_schedule_delay_ms = env::var("OTEL_BSP_SCHEDULE_DELAY")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(5_000_u64);
        let bsp_schedule_delay = Duration::from_millis(bsp_schedule_delay_ms);

        let bsp_max_batch_size = env::var("OTEL_BSP_MAX_EXPORT_BATCH_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(512_usize);

        let blrp_schedule_delay_ms = env::var("OTEL_BLRP_SCHEDULE_DELAY")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1_000_u64);
        let blrp_schedule_delay = Duration::from_millis(blrp_schedule_delay_ms);

        let blrp_max_batch_size = env::var("OTEL_BLRP_MAX_EXPORT_BATCH_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(512_usize);

        let rust_log = env::var("RUST_LOG").ok();

        let dev_logs_to_stdout_override = env::var("BRRTR_DEV_LOGS_TO_STDOUT").is_ok_and(|s| {
            matches!(
                s.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        });

        // If resource attrs duplicate deployment.environment, prefer explicit DEPLOYMENT_ENVIRONMENT.
        if deployment_environment.is_some() {
            extra_resource_attributes.retain(|(k, _)| k != "deployment.environment");
        }

        Self {
            endpoint,
            protocol,
            timeout,
            service_name,
            service_version,
            deployment_environment,
            extra_resource_attributes,
            sampler,
            bsp_schedule_delay,
            bsp_max_batch_size,
            blrp_schedule_delay,
            blrp_max_batch_size,
            rust_log,
            dev_logs_to_stdout_override,
        }
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

fn parse_otlp_protocol(raw: Option<&str>) -> OtlpProtocol {
    let Some(s) = raw else {
        return OtlpProtocol::Grpc;
    };
    match s.trim().to_ascii_lowercase().as_str() {
        "http/protobuf" | "http_proto" | "http-proto" => OtlpProtocol::HttpProto,
        "http/json" | "http-json" => OtlpProtocol::HttpJson,
        _ => OtlpProtocol::Grpc,
    }
}

fn parse_otel_resource_attributes(raw: Option<&str>) -> Vec<(String, String)> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((k, v)) = part.split_once('=') {
            out.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

fn parse_otel_traces_sampler(name: Option<&str>, arg: Option<&str>) -> Sampler {
    let Some(name) = name else {
        return Sampler::ParentBasedAlwaysOn;
    };
    match name.trim().to_ascii_lowercase().as_str() {
        "always_off" => Sampler::AlwaysOff,
        "traceidratio" | "parentbased_traceidratio" => {
            let ratio = arg.and_then(|s| s.parse().ok()).unwrap_or(1.0_f64);
            Sampler::ParentBasedTraceIdRatio(ratio)
        }
        _ => Sampler::ParentBasedAlwaysOn,
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
        let config = ObservabilityConfig::default()
            .with_sampler(Sampler::ParentBasedTraceIdRatio(0.5));
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

}
