# microscaler-observability

**Hexagonal observability adapter for the microscaler platform.**

Cluster OTLP endpoints and env vars: see [`docs/CLUSTER_OBSERVABILITY.md`](docs/CLUSTER_OBSERVABILITY.md).

This crate is the single place in the workspace that owns OpenTelemetry
global state — `TracerProvider`, `LoggerProvider`, `MeterProvider`, and
the W3C propagator. It sits alongside [BRRTRouter](../BRRTRouter/)
(HTTP adapter) and [Lifeguard](../lifeguard/) (Postgres adapter) as a
**peer** in the ports-and-adapters architecture, not as a child of either.

```
┌──────────────────────────────────────────────────────────┐
│                     Host app (main.rs)                   │
│  ┌────────────────────────────────────────────────────┐  │
│  │                  DOMAIN (core)                     │  │
│  │    handler impls, business logic, domain types     │  │
│  │         emits: tracing::info!, tracing::span!      │  │
│  └────────────────────────────────────────────────────┘  │
│        ▲                    │                   │        │
│        │                    │                   │        │
│     input              output (DB)         output (OTEL) │
│        │                    │                   │        │
│  ┌─────┴──────┐    ┌────────┴──────┐    ┌───────┴──────┐ │
│  │ BRRTRouter │    │   Lifeguard   │    │  THIS CRATE  │ │
│  │ (HTTP in)  │    │ (Postgres out)│    │  (OTEL out)  │ │
│  │  also out: │    │   emits:      │    │              │ │
│  │  HTTP resp │    │ tracing::*    │    │ owns:        │ │
│  │  emits:    │    │               │    │ TracerProvid.│ │
│  │ tracing::* │    │               │    │ LoggerProvid.│ │
│  │            │    │               │    │ MeterProvid. │ │
│  │            │    │               │    │ Propagator   │ │
│  └────────────┘    └───────────────┘    └──────────────┘ │
└──────────────────────────────────────────────────────────┘
```

## Why it exists

Originally (pre-v0.4 of the cross-repo PRD), BRRTRouter's
`init_logging_with_config` did double duty as the observability adapter.
Lifeguard separately installed its own `MeterProvider` via an `OnceCell`.
When [Hauliage](../hauliage/) grew up into a real domain app composing both,
two smells emerged:

1. **Wrong coupling.** Services that use only BRRTRouter (without a DB) still
   carried Lifeguard's meter-provider installer in the dep graph. Services
   that use only Lifeguard (CLI tools, migrations) had nowhere clean to init
   OTEL without pulling in the whole HTTP framework.
2. **Global-state race.** Lifeguard's `set_meter_provider` call fired
   whenever `LifeguardMetrics::init()` ran; if BRRTRouter ever also tried to
   set it, whoever ran first won — silently.

Pulling all OTEL init into a dedicated, neutral crate fixes both. BRRTRouter
and Lifeguard become pure emitters (`tracing::span!` / `tracing::info!`) that
never touch `opentelemetry::global::*`. This crate owns init; `main()` owns
lifecycle.

See `docs/PRD.md` for the complete architectural rationale and the cross-repo
migration plan.

## Who uses it

As of v0.0.1, **nothing yet** — the crate is a scaffold. `init()` deliberately
panics with an instruction pointing at the PRD, so integration shape can be
validated without accidental success against a stub.

Once Phase O.1 of `docs/PRD.md` ships, consumers look like:

- **Hauliage** (the primary driver — real domain, real DB, real dashboards):
  Each of the ~17 microservices' `main.rs` calls `init()` first, holds the
  `ShutdownGuard` for process lifetime, then wires its BRRTRouter server +
  Lifeguard pool + domain handlers.
- **BRRTRouter examples** (pet_store, etc.): Same pattern.
- **Lifeguard CLI tools** (migrations, reflector, health-checks): Same
  pattern, just without the HTTP layer.

## Version coupling

**`opentelemetry = "0.29"`** is pinned to match Lifeguard's
[`Cargo.toml`](../lifeguard/Cargo.toml) (currently `opentelemetry = "0.29.1"`,
`opentelemetry_sdk = "0.29.0"`, `opentelemetry-prometheus = "0.29.1"`).

If both crates ever see different `opentelemetry` majors, their global-state
slots are *different slots at the same name* — traces emit fine from one side
but are invisible from the other. Any bump is a coordinated cross-repo change,
not a unilateral one. See `docs/PRD.md` §Phase O.0 for the pinning contract.

## Feature flags

| Feature                   | Default | Purpose |
|---------------------------|:-------:|---------|
| `dev-stdout-fallback`     | ✅      | When `OTEL_EXPORTER_OTLP_ENDPOINT` is unset, install a plain `tracing_subscriber::fmt` layer to stdout so `cargo test` / `cargo run` still show logs locally. Disable in release builds that want the "stdout is startup-only" invariant enforced at compile time. |
| `profiling`               | ❌      | Bring in `pyroscope-rs` for push-mode continuous profiling (flamegraphs). Activated by setting `PYROSCOPE_SERVER_ADDRESS` at runtime. |
| `http-proto` / `http-json`| ❌      | Alternate OTLP transports. Default is `grpc-tonic`. |

## Env var contract

All OTEL-standard variables per the [OpenTelemetry specification][otel-spec]
are honoured. A subset that matters most:

| Env var                           | Default             | Effect |
|-----------------------------------|---------------------|--------|
| `OTEL_EXPORTER_OTLP_ENDPOINT`     | unset               | When unset, OTLP is disabled and stdout-fallback is used (if the feature is on). When set (e.g. `http://otel-collector:4317`), all three pipelines route through OTLP. |
| `OTEL_EXPORTER_OTLP_PROTOCOL`     | `grpc`              | `grpc` / `http/protobuf` / `http/json` |
| `OTEL_SERVICE_NAME`               | *(required)*        | Appears as the service name in Jaeger / Loki. |
| `OTEL_SERVICE_VERSION`            | `CARGO_PKG_VERSION` | Resource attr; set by caller via `.with_service_version()`. |
| `OTEL_RESOURCE_ATTRIBUTES`        | empty               | Extra `k=v,k=v` resource attrs (e.g. `deployment.environment=dev`). |
| `OTEL_TRACES_SAMPLER`             | `parentbased_always_on` | Standard OTEL sampler name. |
| `OTEL_TRACES_SAMPLER_ARG`         | `1.0`               | Ratio for ratio-based samplers. |
| `RUST_LOG`                        | `info`              | `tracing` filter. Merged into the subscriber's `EnvFilter`. |
| `BRRTR_DEV_LOGS_TO_STDOUT`        | `0`                 | Break-glass override: even if OTLP is configured, also install the stdout fallback layer so operators can see logs in `kubectl logs` during a debugging session. Set to `1` to enable. |

One crate-specific knob with a `MICROSCALER_` prefix exists only where no
OTEL-standard variable covers the concern.

## Non-goals (for now)

- **Installing a `MeterProvider`.** v0.0.1 and Phase O.1 deliberately leave
  metrics in BRRTRouter's existing Prometheus-text `/metrics` endpoint,
  concatenated with Lifeguard's `prometheus_scrape_text()`. If downstream OTLP
  metrics ever become necessary, the OTEL Collector's Prometheus receiver
  re-exports them via OTLP without any application change. See `docs/PRD.md`
  Phase O.6.
- **Vendor-specific APM integrations** (Datadog, Honeycomb, New Relic). OTLP
  is the contract; vendor translation is the Collector's job.
- **Custom sampling policies beyond the standard OTEL-SDK samplers.** Tail
  sampling is a Collector concern.

## Related docs

- [`docs/PRD.md`](docs/PRD.md) — **Cross-repo master PRD.** Authoritative source
  for the migration plan, phase sequencing, and ownership contract between
  this crate, BRRTRouter, Lifeguard, and Hauliage.
- [`../BRRTRouter/docs/PRD_OBSERVABILITY_AND_TRACING.md`](../BRRTRouter/docs/PRD_OBSERVABILITY_AND_TRACING.md)
  — Historical draft (v0.1–v0.3) that predates this crate's creation. Retained
  for context; the master plan lives here.
- [`../lifeguard/docs/OBSERVABILITY_APP_INTEGRATION.md`](../lifeguard/docs/OBSERVABILITY_APP_INTEGRATION.md)
  — Lifeguard's pre-existing integration contract. This crate honours its four
  rules (one TracerProvider, one subscriber, Lifeguard declines OTel globals,
  `channel_layer()` is optional).
- [`../hauliage/k8s/observability/README.md`](../hauliage/k8s/observability/README.md)
  — The five existing Grafana dashboards Hauliage already ships. This crate's
  output must satisfy and extend them without breakage.

[otel-spec]: https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/

## License

Apache-2.0.
