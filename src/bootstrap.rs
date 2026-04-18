//! OTLP pipeline construction and [`crate::init`] implementation.

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};

use opentelemetry::global;
use opentelemetry::trace::TracerProvider;
use opentelemetry::KeyValue;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{LogExporter, SpanExporter, WithExportConfig};
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::resource::Resource;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::trace::Sampler as OtelSampler;
use opentelemetry_semantic_conventions::attribute::{SERVICE_NAME, SERVICE_VERSION};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

use crate::config::{ObservabilityConfig, OtlpProtocol, Sampler};
use crate::error::{ObservabilityError, ObservabilityResult};
use crate::shutdown::ShutdownGuard;

static INIT_DONE: AtomicBool = AtomicBool::new(false);

/// See `docs/PRD.md` Phase O.1.
pub fn init_internal(config: &ObservabilityConfig) -> ObservabilityResult<ShutdownGuard> {
    if INIT_DONE.swap(true, Ordering::SeqCst) {
        return Err(ObservabilityError::AlreadyInitialized);
    }

    let env_filter = build_env_filter(config);

    let use_stdout_fmt = should_install_stdout_fmt(config);

    match config.endpoint.as_deref() {
        None => install_subscriber_fmt_only(env_filter, use_stdout_fmt),
        Some(endpoint) if endpoint.trim().is_empty() => {
            install_subscriber_fmt_only(env_filter, use_stdout_fmt)
        }
        Some(endpoint) => install_subscriber_otlp(env_filter, config, endpoint, use_stdout_fmt),
    }
}

#[allow(clippy::missing_const_for_fn)] // ObservabilityConfig is not a const type
fn should_install_stdout_fmt(config: &ObservabilityConfig) -> bool {
    config.dev_logs_to_stdout_override || config.endpoint.is_none()
}

fn build_env_filter(config: &ObservabilityConfig) -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        let base = config.rust_log.as_deref().unwrap_or("info");
        FromStr::from_str(base).unwrap_or_else(|_| {
            FromStr::from_str("error").unwrap_or_else(|_| EnvFilter::default())
        })
    })
}

fn install_subscriber_fmt_only(
    env_filter: EnvFilter,
    with_stdout: bool,
) -> ObservabilityResult<ShutdownGuard> {
    let registry = Registry::default().with(env_filter);

    if with_stdout {
        let fmt = tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_writer(std::io::stdout);
        if registry.with(fmt).try_init().is_err() {
            INIT_DONE.store(false, Ordering::SeqCst);
            return Err(ObservabilityError::SubscriberAlreadyInstalled);
        }
    } else if registry.try_init().is_err() {
        INIT_DONE.store(false, Ordering::SeqCst);
        return Err(ObservabilityError::SubscriberAlreadyInstalled);
    }

    Ok(ShutdownGuard::noop())
}

#[allow(clippy::significant_drop_tightening)] // exporters move into batch processors inside the SDK
fn install_subscriber_otlp(
    env_filter: EnvFilter,
    config: &ObservabilityConfig,
    endpoint: &str,
    with_stdout: bool,
) -> ObservabilityResult<ShutdownGuard> {
    validate_endpoint(endpoint)?;

    let resource = build_resource(config);

    let span_exporter = build_span_exporter(config, endpoint)?;
    let log_exporter = build_log_exporter(config, endpoint)?;

    let trace_sampler = map_sampler(&config.sampler);

    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter)
        .with_sampler(trace_sampler)
        .with_resource(resource.clone())
        .build();

    let logger_provider = SdkLoggerProvider::builder()
        .with_batch_exporter(log_exporter)
        .with_resource(resource)
        .build();

    global::set_tracer_provider(tracer_provider.clone());

    global::set_text_map_propagator(TraceContextPropagator::new());

    let tracer = tracer_provider.tracer("microscaler");
    let otel_trace_layer = OpenTelemetryLayer::new(tracer);

    let otel_log_layer = OpenTelemetryTracingBridge::new(&logger_provider);

    if with_stdout {
        let fmt = tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_writer(std::io::stdout);
        if Registry::default()
            .with(env_filter)
            .with(otel_trace_layer)
            .with(otel_log_layer)
            .with(fmt)
            .try_init()
            .is_err()
        {
            INIT_DONE.store(false, Ordering::SeqCst);
            return Err(ObservabilityError::SubscriberAlreadyInstalled);
        }
    } else if Registry::default()
        .with(env_filter)
        .with(otel_trace_layer)
        .with(otel_log_layer)
        .try_init()
        .is_err()
    {
        INIT_DONE.store(false, Ordering::SeqCst);
        return Err(ObservabilityError::SubscriberAlreadyInstalled);
    }

    Ok(ShutdownGuard::new_otlp(tracer_provider, logger_provider))
}

fn validate_endpoint(endpoint: &str) -> ObservabilityResult<()> {
    let t = endpoint.trim();
    if t.is_empty() {
        return Err(ObservabilityError::InvalidEndpoint {
            value: endpoint.to_string(),
            reason: "endpoint is empty after trim".to_string(),
        });
    }
    if !(t.starts_with("http://") || t.starts_with("https://")) {
        return Err(ObservabilityError::InvalidEndpoint {
            value: endpoint.to_string(),
            reason: "must start with http:// or https://".to_string(),
        });
    }
    Ok(())
}

fn build_resource(config: &ObservabilityConfig) -> Resource {
    let mut attrs: Vec<KeyValue> = vec![
        KeyValue::new(SERVICE_NAME, config.service_name.clone()),
        KeyValue::new("service.namespace", "microscaler"),
    ];
    if let Some(v) = &config.service_version {
        attrs.push(KeyValue::new(SERVICE_VERSION, v.clone()));
    }
    if let Some(env) = &config.deployment_environment {
        attrs.push(KeyValue::new("deployment.environment", env.clone()));
    }
    for (k, v) in &config.extra_resource_attributes {
        attrs.push(KeyValue::new(k.clone(), v.clone()));
    }
    Resource::builder_empty().with_attributes(attrs).build()
}

fn map_sampler(sampler: &Sampler) -> OtelSampler {
    match sampler {
        Sampler::ParentBasedAlwaysOn => OtelSampler::ParentBased(Box::new(OtelSampler::AlwaysOn)),
        Sampler::ParentBasedTraceIdRatio(r) => {
            OtelSampler::ParentBased(Box::new(OtelSampler::TraceIdRatioBased(*r)))
        }
        Sampler::AlwaysOff => OtelSampler::ParentBased(Box::new(OtelSampler::AlwaysOff)),
    }
}

fn build_span_exporter(
    config: &ObservabilityConfig,
    endpoint: &str,
) -> ObservabilityResult<SpanExporter> {
    let built = match config.protocol {
        OtlpProtocol::Grpc => SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint.to_string())
            .with_timeout(config.timeout)
            .build(),
        OtlpProtocol::HttpProto | OtlpProtocol::HttpJson => {
            return Err(ObservabilityError::ExporterConstruction(
                "HTTP OTLP for traces requires opentelemetry-otlp http features; use grpc (default) or extend Cargo.toml"
                    .to_string(),
            ));
        }
    };
    built.map_err(|e| ObservabilityError::ExporterConstruction(e.to_string()))
}

fn build_log_exporter(
    config: &ObservabilityConfig,
    endpoint: &str,
) -> ObservabilityResult<LogExporter> {
    let built = match config.protocol {
        OtlpProtocol::Grpc => LogExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint.to_string())
            .with_timeout(config.timeout)
            .build(),
        OtlpProtocol::HttpProto | OtlpProtocol::HttpJson => {
            return Err(ObservabilityError::ExporterConstruction(
                "HTTP OTLP for logs not wired yet; use grpc".to_string(),
            ));
        }
    };
    built.map_err(|e| ObservabilityError::ExporterConstruction(e.to_string()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn validate_endpoint_accepts_http() {
        assert!(validate_endpoint("http://otel-collector.observability:4317").is_ok());
    }

    #[test]
    fn validate_endpoint_rejects_bare_host() {
        assert!(validate_endpoint("otel-collector:4317").is_err());
    }
}
