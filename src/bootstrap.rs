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
use opentelemetry_sdk::trace::Sampler as OtelSampler;
use opentelemetry_sdk::trace::SdkTracerProvider;
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

    #[allow(unused_mut)] // mutated only with the cloud-profiler feature
    let mut guard = match config.endpoint.as_deref() {
        None => install_subscriber_fmt_only(env_filter, use_stdout_fmt),
        Some(endpoint) if endpoint.trim().is_empty() => {
            install_subscriber_fmt_only(env_filter, use_stdout_fmt)
        }
        Some(endpoint) => install_subscriber_otlp(env_filter, config, endpoint, use_stdout_fmt),
    }?;
    // Cloud Profiler is independent of OTLP: a service with tracing off is still profiled when asked.
    #[cfg(all(feature = "cloud-profiler", target_os = "linux"))]
    {
        guard.profiler =
            crate::profiler::start(&config.service_name, config.service_version.as_deref());
    }
    Ok(guard)
}

#[allow(clippy::missing_const_for_fn)] // ObservabilityConfig is not a const type
fn should_install_stdout_fmt(config: &ObservabilityConfig) -> bool {
    config.dev_logs_to_stdout_override || config.endpoint.is_none()
}

fn build_env_filter(config: &ObservabilityConfig) -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        let base = config.rust_log.as_deref().unwrap_or("info");
        FromStr::from_str(base)
            .unwrap_or_else(|_| FromStr::from_str("error").unwrap_or_else(|_| EnvFilter::default()))
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

    // Tonic OTLP and `opentelemetry_sdk` batch processors use `tokio::spawn`. Hauliage / BRRTRouter
    // `main` runs on the `may` stack, not inside a Tokio runtime — build exporters and providers
    // under a dedicated multi-thread runtime we keep for the process lifetime (see `ShutdownGuard`).
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("obs-otlp")
        .build()
        .map_err(|e| {
            ObservabilityError::ExporterConstruction(format!(
                "tokio runtime (required for OTLP): {e}"
            ))
        })?;

    let (tracer_provider, logger_provider) = {
        let _enter = tokio_runtime.enter();
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

        (tracer_provider, logger_provider)
    };

    global::set_tracer_provider(tracer_provider.clone());

    global::set_text_map_propagator(TraceContextPropagator::new());

    if with_stdout {
        let fmt = tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_writer(std::io::stdout);
        if Registry::default()
            .with(env_filter)
            .with(otel_layers(&tracer_provider, &logger_provider))
            .with(fmt)
            .try_init()
            .is_err()
        {
            INIT_DONE.store(false, Ordering::SeqCst);
            return Err(ObservabilityError::SubscriberAlreadyInstalled);
        }
    } else if Registry::default()
        .with(env_filter)
        .with(otel_layers(&tracer_provider, &logger_provider))
        .try_init()
        .is_err()
    {
        INIT_DONE.store(false, Ordering::SeqCst);
        return Err(ObservabilityError::SubscriberAlreadyInstalled);
    }

    Ok(ShutdownGuard::new_otlp(
        tokio_runtime,
        tracer_provider,
        logger_provider,
    ))
}

/// The OpenTelemetry layers [`crate::init`] installs over these providers: spans ([`trace_layer`]) and logs.
///
/// Public so tests can check the composition a host actually gets; hosts call [`crate::init`].
#[doc(hidden)]
#[must_use]
pub fn otel_layers<S>(
    tracer_provider: &SdkTracerProvider,
    logger_provider: &SdkLoggerProvider,
) -> impl tracing_subscriber::Layer<S>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    use tracing_subscriber::Layer as _;
    trace_layer(tracer_provider).and_then(log_layer(logger_provider))
}

/// Logs → OpenTelemetry, each record with the trace context of the span its event belongs to.
fn log_layer(
    logger_provider: &SdkLoggerProvider,
) -> SpanContextLogs<
    OpenTelemetryTracingBridge<SdkLoggerProvider, opentelemetry_sdk::logs::SdkLogger>,
> {
    SpanContextLogs {
        inner: OpenTelemetryTracingBridge::new(logger_provider),
        dispatch: std::sync::OnceLock::new(),
    }
}

/// Gives the log bridge the OpenTelemetry context of the span an event belongs to.
///
/// `opentelemetry-appender-tracing` (0.32 on) takes a record's trace and span ids from the *current* OpenTelemetry
/// context. `tracing-opentelemetry` sets that only while a span is entered with context activation on, which
/// [`trace_layer`] turns off, and our hosts do not enter spans on `may` anyway: `may_tracing` parents events on the
/// coroutine's span explicitly. So, per event, this looks up the event's span (its explicit parent, else the
/// thread's current span) and attaches that span's context around the bridge's `on_event` only. The guard is pushed
/// and popped inside one synchronous call, so, unlike an entered span's context, it cannot be left on a thread when
/// a coroutine yields.
pub struct SpanContextLogs<L> {
    inner: L,
    /// The subscriber this layer is part of, for `get_otel_context` (set when it becomes a `Dispatch`).
    dispatch: std::sync::OnceLock<tracing::dispatcher::WeakDispatch>,
}

impl<S, L> tracing_subscriber::Layer<S> for SpanContextLogs<L>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
    L: tracing_subscriber::Layer<S>,
{
    fn on_register_dispatch(&self, subscriber: &tracing::Dispatch) {
        self.dispatch.get_or_init(|| subscriber.downgrade());
        self.inner.on_register_dispatch(subscriber);
    }

    fn on_layer(&mut self, subscriber: &mut S) {
        self.inner.on_layer(subscriber);
    }

    fn register_callsite(
        &self,
        metadata: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        self.inner.register_callsite(metadata)
    }

    fn enabled(
        &self,
        metadata: &tracing::Metadata<'_>,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        self.inner.enabled(metadata, ctx)
    }

    fn event_enabled(
        &self,
        event: &tracing::Event<'_>,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        self.inner.event_enabled(event, ctx)
    }

    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        self.inner.on_new_span(attrs, id, ctx);
    }

    fn on_record(
        &self,
        id: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        self.inner.on_record(id, values, ctx);
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        use opentelemetry::trace::TraceContextExt as _;
        let span_cx = ctx
            .event_span(event)
            .map(|span| span.id())
            .and_then(|id| {
                let dispatch = self.dispatch.get()?.upgrade()?;
                tracing_opentelemetry::get_otel_context(&id, &dispatch)
            })
            .filter(|cx| cx.span().span_context().is_valid());
        let _attached = span_cx.map(opentelemetry::Context::attach);
        self.inner.on_event(event, ctx);
    }
}

/// The `tracing` → OpenTelemetry span layer, without OpenTelemetry context activation.
///
/// Since `tracing-opentelemetry` 0.32, entering a span also attaches its OpenTelemetry `Context`
/// to a thread-local stack and parks the guard on a thread-local stack of the layer; exiting pops
/// it again — on the thread that exits. Our hosts run requests on `may` coroutines, which the
/// scheduler moves between worker threads whenever they yield (database, Redis, HTTP I/O). A span
/// entered before a yield and exited after it leaves its context, its guard and the span data
/// they hold on the first thread for good: memory grows with every such request until
/// OpenTelemetry's stack refuses pushes at 65 535 entries per thread (`PriceWhisperer`
/// docs/incidents/2026-10-23_otel_context_leak_PLAN.md; identity-login-service reached ~720 MB).
///
/// Nothing here needs the thread-local context: `may_tracing` propagates through the coroutine
/// context (`may_tracing::otel`), and `OpenTelemetrySpanExt::context()` reads the span itself.
fn trace_layer<S>(
    provider: &SdkTracerProvider,
) -> OpenTelemetryLayer<S, opentelemetry_sdk::trace::SdkTracer>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    OpenTelemetryLayer::new(provider.tracer("microscaler")).with_context_activation(false)
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
    // Do not invent a fake service.namespace (there is no "microscaler" k8s
    // namespace). Set it only via OTEL_RESOURCE_ATTRIBUTES, or let the
    // collector k8sattributes processor inject k8s.namespace.name.
    let mut attrs: Vec<KeyValue> = vec![KeyValue::new(SERVICE_NAME, config.service_name.clone())];
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

    /// A span entered on one thread and exited on another (a may coroutine that yields while the
    /// span is entered and resumes elsewhere) must leave no OpenTelemetry context behind on the
    /// first thread. With context activation on, `Context::current()` there keeps the span (and
    /// the stack grows by one entry per request).
    #[test]
    fn span_exited_on_another_thread_leaves_no_context_behind() {
        use opentelemetry::trace::TraceContextExt;

        let provider = SdkTracerProvider::builder().build();
        let dispatch = tracing::Dispatch::new(Registry::default().with(trace_layer(&provider)));

        for _ in 0..100 {
            let span = tracing::dispatcher::with_default(
                &dispatch,
                || tracing::info_span!(parent: None, "request"),
            );
            let id = span.id().expect("span enabled");
            dispatch.enter(&id);
            let d = dispatch.clone();
            std::thread::spawn(move || {
                d.exit(&span.id().expect("span enabled"));
                drop(span);
            })
            .join()
            .unwrap();
        }

        assert!(
            !opentelemetry::Context::current().has_active_span(),
            "a span exited on another thread is still this thread's OpenTelemetry context"
        );
    }
}
