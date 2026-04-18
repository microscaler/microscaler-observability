# Cluster observability (shared Kind) and OTLP endpoints

The shared local stack lives in [`shared-kind-cluster`](https://github.com/microscaler/shared-kind-cluster) (Tilt in that repo, namespace `observability`).

## Services (in-cluster DNS)

| Component | Kubernetes Service | Ports | Use from app pods |
|-----------|-------------------|-------|-------------------|
| OpenTelemetry Collector | `otel-collector` | 4317 gRPC, 4318 HTTP | `http://otel-collector.observability.svc.cluster.local:4317` |
| Jaeger (collector receives OTLP) | `jaeger` | 4317 (internal to collector config) | Apps talk to **Collector**, not Jaeger directly |
| Prometheus | (see `k8s/observability/prometheus.yaml`) | scrape | `/metrics` on your service |
| Loki | `loki` | 3100 | Push via Collector when OTLP logs pipeline is enabled |
| Grafana | `grafana` | 3000 | UI |

Embedded collector config (`shared-kind-cluster/k8s/observability/embedded/otel-collector-config.yml`) routes **traces** OTLP → `otlp/jaeger` → `jaeger:4317`. **Metrics** pipeline receives OTLP and exposes Prometheus on `:9464` on the collector.

## Environment variables (apps)

Set on Deployments (Helm / Tilt) before calling `microscaler_observability::init`:

| Variable | Example | Notes |
|----------|---------|--------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | `http://otel-collector.observability.svc.cluster.local:4317` | Required for OTLP export |
| `OTEL_SERVICE_NAME` | `hauliage-fleet` | Appears in Jaeger |
| `OTEL_SERVICE_VERSION` | Chart app version | Resource attribute |
| `OTEL_RESOURCE_ATTRIBUTES` | `deployment.environment=dev,service.namespace=hauliage` | Comma-separated `k=v` |
| `RUST_LOG` | `info,brrtrouter=debug` | `EnvFilter` for tracing |

Optional: `BRRTR_DEV_LOGS_TO_STDOUT=1` forces a `fmt` layer even when OTLP is set (break-glass debugging).

## OpenTelemetry version alignment (required)

**All of the following agree on the same `opentelemetry` / `opentelemetry_sdk` line (0.31.x, git-patched in lockstep with BRRTRouter for reqwest 0.13):**

- `microscaler-observability` (this repo)
- `lifeguard`
- `BRRTRouter`

## BRRTRouter and Hauliage wiring

When **`OTEL_EXPORTER_OTLP_ENDPOINT`** is non-empty, **`brrtrouter::otel::init_logging_with_config`** delegates to **`microscaler_observability::init`** (merged `RUST_LOG` / `BRRTR_LOG_LEVEL`, `may_minihttp::http_server=warn`, and optional debug-session directives). The OTLP `microscaler_observability::ShutdownGuard` is stored inside BRRTRouter and flushed from **`brrtrouter::otel::shutdown`**.

**Process lifecycle:** host binaries should use **`brrtrouter::server::ServerHandle::run_until_shutdown`** instead of **`join()`** so Kubernetes **SIGTERM** (rollouts, scale-down) stops the HTTP listener and calls **`brrtrouter::otel::shutdown`** after the server stops. Tune **`terminationGracePeriodSeconds`** if in-flight requests need more time than the default.

Services that do not use BRRTRouter can still call `microscaler_observability::init(ObservabilityConfig::from_env()...)` directly from `main()` and hold the returned guard.
