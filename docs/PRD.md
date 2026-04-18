# PRD: microscaler cross-repo observability & tracing (O-series)

**Document version:** 0.4 (DRAFT — for review, authoritative home)
**Date:** 2026-04-18
**Status:** Master cross-repo plan. Supersedes `../BRRTRouter/docs/PRD_OBSERVABILITY_AND_TRACING.md` v0.1–v0.3, which remain as historical drafts. No phase of this PRD has landed yet.
**Owner:** microscaler platform (all four repos below).
**Target repos:**

| Repo | Role | Target branch |
|---|---|---|
| [`microscaler-observability`](..) (this repo) | OTEL output adapter — owns all OpenTelemetry globals | `main` |
| [`BRRTRouter`](../../BRRTRouter) | HTTP input/output adapter | `pre_BFF_work` |
| [`lifeguard`](../../lifeguard) | Postgres output adapter | `address-migration-issues` |
| [`hauliage`](../../hauliage) | Real composition root — ~17 microservices, 5 existing dashboards, real domain code | `main` |

**Primary driver:** Hauliage — the rubber-meets-road consumer — has surfaced two architectural smells that can't be fixed inside either BRRTRouter or Lifeguard alone:

1. **BRRTRouter and Lifeguard both try to own OpenTelemetry globals.** BRRTRouter's `init_logging_with_config` is the single subscriber init (benign but presumptive). Lifeguard's `LifeguardMetrics::init()` calls `opentelemetry::global::set_meter_provider` via `OnceCell`. Any service that embeds both — and every Hauliage service does — is subject to a silent first-caller-wins race.
2. **Pure-CLI services using only Lifeguard** (migrations, reflectors, workers) must either depend on BRRTRouter to get a working OTEL init or reimplement it. This is the library-framework-coupling smell that hexagonal architecture is supposed to prevent.

Moving OTEL init into a neutral, peer-of-everything crate fixes both. This PRD is the migration plan.

## 1. Summary

We are creating a new crate [`microscaler-observability`](../) as an outbound adapter peer to BRRTRouter and Lifeguard. All four OpenTelemetry globals move there. BRRTRouter and Lifeguard become pure emitters. Hauliage's microservices (and BRRTRouter's pet_store example) call the new crate's `init()` from `main()` once per process. Existing dashboards keep working end-to-end. Jaeger starts receiving traces for the first time. Promtail's runtime role is eliminated in favour of OTLP-native logs.

## 2. Goals

1. **G1 — OTLP traces reach Jaeger.** A Hauliage service request produces a span tree in Jaeger within 30 s.
2. **G2 — OTLP logs reach Loki via Collector, not Promtail.** Runtime `tracing::event!` records appear in Loki with `trace_id` / `span_id` as first-class OTLP fields. Promtail only tails startup stdout.
3. **G3 — Cross-service trace context.** Hauliage's BFF → downstream-service HTTP calls carry `traceparent`; the downstream service's spans appear as children in Jaeger.
4. **G4 — Hauliage's existing five dashboards remain green.** `hauliage-overview`, `hauliage-bff`, `hauliage-postgres`, `hauliage-lifeguard`, `hauliage-cluster-logs` keep serving data throughout the migration, not just at the end.
5. **G5 — BRRTRouter's per-route SLO queries work.** `histogram_quantile(0.95, sum by (http_route, le) (rate(http_server_request_duration_seconds_bucket[1m])))` returns a series per route.
6. **G6 — No OTEL-global race.** Neither BRRTRouter nor Lifeguard installs any `opentelemetry::global::set_*_provider`. Enforced by `deny(clippy::disallowed_methods)` in both crates.
7. **G7 — Stdout is startup-only under load.** After boot, `kubectl logs <service>` shows only panics, aborts, and graceful-shutdown output. All runtime observability is in Grafana / Jaeger / Pyroscope.
8. **G8 — Safe to partially adopt.** If a Hauliage service deploys the new crate before some other service has migrated, neither breaks.

## 3. Non-goals

- **N1** — Refactoring Lifeguard's ORM model layer. Only its OTEL init is touched.
- **N2** — Rewriting Hauliage's business-metric dashboards. We add new BRRTRouter+Lifeguard-level dashboards, we don't retire the existing five.
- **N3** — Vendor-specific APM (Datadog, Honeycomb). OTLP is the contract.
- **N4** — Installing an OTEL `MeterProvider` from this crate. Metrics stay on the Prometheus-text `/metrics` path (BRRTRouter serves, Lifeguard contributes via `prometheus_scrape_text()` concat). The Collector's Prometheus receiver handles OTLP metric re-export if downstream consumers ever need it.
- **N5** — Deleting Lifeguard's `channel_layer()` in this PRD. Deprecate it (Phase O.13) and remove in a follow-up Lifeguard PRD.
- **N6** — Rewriting Hauliage's portal / Flutter frontend observability. Backend only.
- **N7** — Kubernetes operator / sidecar auto-injection of OTEL. The SDK lives in the application process.
- **N8** — Integrating Pyroscope in the same release train as the Jaeger fix. Pyroscope (Phase O.12) ships independently on its own cadence.

## 4. Current state — audit findings (2026-04-18)

### 4.1 `microscaler-observability` scaffold

- v0.1.0: `init()` wires OTLP traces + logs via `microscaler-observability` (see `src/bootstrap.rs`). BRRTRouter's `init_logging_with_config` delegates to this crate when `OTEL_EXPORTER_OTLP_ENDPOINT` is set.
- Cargo.toml pins `opentelemetry = "0.31"` (coordinated with BRRTRouter and Lifeguard; git patch for reqwest 0.13 — see `[patch.crates-io]`).

### 4.2 BRRTRouter — see historical PRD v0.3 §4.1–4.8

Summary of what BRRTRouter's pre-v0.4 PRD found:

- Cluster OTEL stack (Jaeger, OTEL Collector, Prometheus, Grafana, Loki, Promtail, Pyroscope) is correctly wired.
- `src/otel.rs::init_logging_with_config` installs only a `tracing_subscriber::fmt` layer — no OTLP exporter, no TracerProvider, no propagator.
- 3 `info_span!` sites in the whole codebase; no `#[instrument]` anywhere.
- `brrtrouter_request_duration_seconds_bucket` has no `method` / `route` labels.
- Memory middleware threshold change (100 MB → 500 MB) over-corrected — user reports real RSS drift the new threshold now hides.

### 4.3 Lifeguard — see `OBSERVABILITY_APP_INTEGRATION.md`

- Already documents the "one `TracerProvider`, one subscriber, Lifeguard declines OTel globals, `channel_layer()` optional" contract — but then violates rule 3 itself by calling `global::set_meter_provider` via `OnceCell`.
- 7 hot-path `tracing::span!` sites on query / pool / transaction operations. Well-named (`lifeguard.execute_query`, `lifeguard.acquire_connection`, etc.).
- Exposes `lifeguard::metrics::prometheus_scrape_text()` — the concat entry point BRRTRouter's `/metrics` handler uses.
- `channel_layer()` exists, drains `may`-mpsc to stderr. No Hauliage service actually installs it today.

### 4.4 Hauliage — the real consumer

Single Cargo workspace at `hauliage/microservices/Cargo.toml`. ~17 microservices, each with a `gen/` binary (generated) and an `impl/` binary (hand-maintained). Path-dependent on BRRTRouter and Lifeguard as peer crates.

**Current telemetry state:**

- **Working today:** Structured logging via BRRTRouter's `init_logging_with_config` (stdout → Promtail → Loki). Prometheus scraping of `/metrics` (BRRTRouter's `brrtrouter_*` series, plus Lifeguard's `lifeguard_*` via `set_extra_prometheus` concat in 13 of the 17 services). Five Grafana dashboards in `k8s/observability/dashboards/`.
- **Half-built:** `OTEL_EXPORTER_OTLP_ENDPOINT` + `OTEL_SERVICE_NAME` set in Helm for every service — but consumed by *nothing* because BRRTRouter's `init_logging_with_config` ignores them.
- **Missing completely:** Distributed tracing (no Jaeger data for Hauliage). Domain-level span emission (no `hauliage.booking.create` / `hauliage.quote.accept` spans — only `consignments/impl` has any `tracing::info!` in domain code). Business-metric dashboards (bookings/day, quote conversion, fleet util). Alerting rules.
- **Outliers:** `hauliage_iot_worker` uses `tracing_subscriber::fmt::init()` directly — bypasses BRRTRouter's init entirely. `email_reminder_worker` uses only `eprintln!`. `reviews` and `storage` don't wire Lifeguard scrape (storage has no Lifeguard dep at all — object storage).

**The five existing dashboards** (must survive migration):

| Dashboard | UID | Covers |
|---|---|---|
| Overview | `hauliage-overview` | HTTP metrics by `app`/`path`, BFF summary, Postgres `pg_up` + `numbackends`, cross-links |
| BFF | `hauliage-bff` | BFF-only traffic, 2xx vs 5xx by path, p50/p95/p99, per-path latency table |
| PostgreSQL | `hauliage-postgres` | Exporter health, connections, DB size / xact rates |
| Lifeguard | `hauliage-lifeguard` | `lifeguard_*` pool, query latency, waits, errors, acquire timeouts |
| Cluster logs | `hauliage-cluster-logs` | Loki cluster-wide pod logs with namespace/pod/container variables |

All five consume `brrtrouter_*`, `lifeguard_*`, or Loki label-filtered pod logs. None of them depend on *how* those signals reach their backend — so they survive a transport change (stdout→Promtail → OTLP direct) transparently.

### 4.5 Gaps Hauliage has exposed that drive this PRD

- **Postmortem `postmortem-consignments-list-jobs-empty-2026-04.md`** — `list_jobs` returned `[]` on a DB error with no log trail. Could have been diagnosed in 30 s with a trace; took hours without one. *This is the single clearest "why we need this" anchor in the Hauliage repo.*
- **BFF gap analysis** — relies on metrics + logs for upstream attribution; no way to follow a single request from BFF through to the downstream Fleet service.
- **Bespoke `main.rs` concerns** — `PRD_BFF_SCAFFOLDING_REMEDIATION.md` calls out that `set_extra_prometheus` is hand-wired in every `impl/src/main.rs`, so codegen regenerating `gen/src/main.rs` doesn't risk losing it. Hoisting init into a shared crate *and* updating the codegen template eliminates this class of regeneration risk.

## 5. Target architecture

### 5.1 Ownership matrix

| Concern | Owner | Invariant |
|---|---|---|
| `opentelemetry::global::set_tracer_provider` | **microscaler-observability** | Called exactly once per process by `init()`. Any other crate that calls it is a bug. |
| `opentelemetry::global::set_text_map_propagator` | **microscaler-observability** | Same. |
| `opentelemetry::global::set_logger_provider` *(or equivalent under `tracing_opentelemetry`)* | **microscaler-observability** | Same. |
| `opentelemetry::global::set_meter_provider` | **microscaler-observability (future)** or **noone (today)** | Not installed by any crate in v0.4. Phase O.6 keeps metrics on Prometheus-text. If a future phase moves to OTEL Metrics SDK, this crate installs it. |
| `tracing_subscriber::registry().try_init()` | **microscaler-observability** | Single subscriber. Everything else adds `Layer`s via the crate's API. |
| `EnvFilter`, `fmt::Layer`, redaction, sampling, `OpenTelemetryLayer`, `OpenTelemetryTracingBridge` | **microscaler-observability** | Composed in `init()`. |
| `tracing::span!` / `#[instrument]` emission from HTTP pipeline | **BRRTRouter** | Emits only; never touches globals. |
| `tracing::span!` emission from DB / pool / transaction layers | **Lifeguard** | Emits only; never touches globals. |
| `tracing::span!` emission from domain code | **Hauliage (per service)** | Emits only. |
| W3C `traceparent` extraction from incoming HTTP requests | **BRRTRouter** | Only it sees the raw request. |
| W3C `traceparent` injection on handler-originated outbound HTTP calls | **BRRTRouter HTTP client** (once that lands) or **Hauliage-side HTTP clients** | Whoever makes the outbound call. Uses the globally-installed propagator. |
| `prometheus_scrape_text()` concat in `/metrics` response | **BRRTRouter serves**, **Lifeguard contributes** | Host enables `lifeguard-integration` cargo feature; BRRTRouter calls Lifeguard's helper. |
| SIGTERM / flush orchestration | **microscaler-observability's `ShutdownGuard`** | Host holds guard in `main()`; `Drop` does the flush. |

### 5.2 Three telemetry streams, one exporter each

See `microscaler-observability/src/lib.rs` rustdoc for the diagram. Summary:

- **Logs** — `tracing::event!` → `OpenTelemetryTracingBridge` → `LoggerProvider` → OTLP/gRPC → Collector → Loki. No stdout under load.
- **Traces** — `tracing::span!` → `tracing_opentelemetry::layer` → `TracerProvider` → OTLP/gRPC → Collector → Jaeger.
- **Metrics** — Prometheus-text via BRRTRouter's `/metrics` (+ Lifeguard concat). Scraped by Prometheus. Collector's Prometheus receiver re-exports via OTLP if needed downstream.

### 5.3 Stdout invariant

Stdout under load is **startup-only and panic-only**. Three paths write there:

1. Boot `println!` (route registration, "server listening", OTEL init ack).
2. `panic!` / `abort!` / OTEL init failure — via plain `eprintln!`.
3. Graceful shutdown completion.

`BRRTR_DEV_LOGS_TO_STDOUT=1` is the escape hatch for break-glass local dev — it additionally installs the `fmt::Layer` even when OTLP is configured, so a developer can `kubectl logs` or `tilt logs` during a debugging session.

## 6. Phases

Each phase is one or more PRs per repo, coordinated via this PRD's PR merge order. Each phase's acceptance criteria are specific and testable.

### Phase O.0 — Contract ratification (documentation)

**Scope:** all four repos pick up this PRD as their source of truth for observability.

**Per-repo deliverables:**

- **microscaler-observability:** this PRD file committed at `docs/PRD.md`. README landed. Scaffolded crate committed.
- **BRRTRouter:** `docs/PRD_OBSERVABILITY_AND_TRACING.md` v0.3 gets a header note saying "superseded by microscaler-observability/docs/PRD.md; retained for historical record." No phase work removed — the v0.3 phases map onto the v0.4 phases and readers can follow either.
- **Lifeguard:** `docs/OBSERVABILITY_APP_INTEGRATION.md` amended. The "BRRTRouter (reference host)" paragraph points at `microscaler-observability` instead. Rule 3 ("Lifeguard does not own OTel globals") gets an explicit "including `MeterProvider` — to be removed in Phase O.1.5" note.
- **Hauliage:** new file `docs/observability-migration.md` summarising the migration from the consumer's perspective — what changes in each microservice's `main.rs`, what dashboards stay, what env vars need adjusting, what to expect in Jaeger.

**Acceptance:**

- All four repos have synchronised docs. A new engineer can start from any of the four and follow pointers to the master.

**Commit scope:** doc-only. Same-day across all four repos.

### Phase O.1 — microscaler-observability v0.1.0 lands 🚨 UNBLOCKS JAEGER END-TO-END

**Scope:** this crate's `init()` does real work. Tracer + logger pipelines are built. A host calling `init()` with `OTEL_EXPORTER_OTLP_ENDPOINT` set sees spans in Jaeger.

**Code (in this repo):**

1. `src/lib.rs::init` — replace the `unimplemented!` with the real build:
   - Parse `ObservabilityConfig::from_env()`.
   - If `config.endpoint.is_some()`:
     - Build `opentelemetry_otlp::SpanExporter` (gRPC, tonic).
     - Build `BatchSpanProcessor` with config's delay/batch-size.
     - Build `opentelemetry_sdk::trace::TracerProvider` with the exporter, sampler, and resource attributes.
     - `global::set_tracer_provider(provider.clone())`.
     - Build `opentelemetry_otlp::LogExporter` (gRPC).
     - Build `BatchLogRecordProcessor`.
     - Build `opentelemetry_sdk::logs::LoggerProvider`.
     - `global::set_text_map_propagator(TraceContextPropagator::new())`.
   - Compose the `tracing_subscriber::Registry`:
     - `EnvFilter` from `RUST_LOG`.
     - `tracing_opentelemetry::layer().with_tracer(...)` — bridges spans.
     - `opentelemetry_appender_tracing::OpenTelemetryTracingBridge::new(&logger_provider)` — bridges events to OTLP logs.
     - (feature-gated) `fmt::Layer` to stdout when endpoint unset or `BRRTR_DEV_LOGS_TO_STDOUT=1`.
   - `registry.try_init()?`.
   - Return a real `ShutdownGuard` that stashes the provider handles for `Drop`-time flush.

2. `src/shutdown.rs::ShutdownGuard::Drop` — actual flush sequence (5 s timeout on each).

3. Integration test `tests/otlp_roundtrip.rs` — spin up an in-process OTLP gRPC receiver (using `tonic`), call `init()` against it, emit a span and an event, assert both arrive at the receiver.

**Code (in BRRTRouter):**

1. **Delete** `src/otel.rs::init_logging` and `init_logging_with_config` stubs. They're replaced by `microscaler_observability::init`.
2. **Delete** `src/otel.rs::LogConfig::from_env` — replaced by `microscaler_observability::ObservabilityConfig::from_env`. The `BRRTR_LOG_*` env vars either map 1:1 to new `MICROSCALER_*` or OTEL-standard vars (migration table in this phase's PR description) or are deprecated.
3. **Retain** `src/otel.rs::SamplingLayer` and `RedactionLayer` — move them into `microscaler-observability/src/layers/` and expose from there. BRRTRouter re-exports the names for one release for API continuity.
4. **Delete** `tests/tracing_util.rs::TestTracing` — replaced by `microscaler_observability::testing::TestSubscriber` (new module).
5. **Add** `Cargo.toml` dep: `microscaler-observability = { path = "../microscaler-observability" }` (for now; crates.io publish later).
6. **Cargo.toml lint rule:** `deny(clippy::disallowed_methods)` with `disallowed-methods` in `clippy.toml` listing `opentelemetry::global::set_tracer_provider`, `set_logger_provider`, `set_meter_provider`, `set_text_map_propagator`. Anyone re-introducing global install gets a compile error.

**Code (in Lifeguard):**

1. **Remove** the `global::set_meter_provider` call from `src/metrics.rs::LifeguardMetrics::init()`. The `OnceCell` guard stays (still protects against double-init of the Prometheus registry).
2. Lifeguard metrics continue to flow through `lifeguard::metrics::prometheus_scrape_text()` — but now they go via a locally-held `SdkMeterProvider` that is **not** installed globally. Nothing else in the workspace observes Lifeguard metrics via `opentelemetry::global::meter()` — nobody does today — so no external change.
3. **Cargo.toml lint rule:** same `disallowed-methods` list as BRRTRouter.

**Code (in Hauliage):**

1. **In every microservice's `impl/src/main.rs`:** replace
   ```rust
   brrtrouter::otel::init_logging_with_config(&brrtrouter::otel::LogConfig::from_env())
   ```
   with
   ```rust
   microscaler_observability::init(
       microscaler_observability::ObservabilityConfig::from_env()
           .with_service_name(env!("CARGO_PKG_NAME"))
           .with_service_version(env!("CARGO_PKG_VERSION"))
   )
   ```
   Holding the returned `ShutdownGuard` for the lifetime of `main()`.

2. **In the codegen template** that produces `gen/src/main.rs` (BRRTRouter's `templates/main.rs.txt`): same replacement. Re-run the generator for every service.

3. **Helm chart** (`helm/hauliage-microservice/templates/deployment.yaml`):
   - `OTEL_EXPORTER_OTLP_ENDPOINT` — if pointing at `otel-collector` in a different namespace, qualify to FQDN `otel-collector.observability.svc.cluster.local:4317`.
   - Add `OTEL_SERVICE_VERSION: "{{ .Chart.AppVersion }}"`.
   - Add `OTEL_RESOURCE_ATTRIBUTES: "deployment.environment={{ .Values.environment }},service.namespace=hauliage"`.

4. **Outliers remediated in the same phase:**
   - `hauliage_iot_worker`: replace `tracing_subscriber::fmt::init()` with `microscaler_observability::init(...)`.
   - `email_reminder_worker`: add `init()` at top of `main()`.
   - `reviews`, `storage`: same (they use BRRTRouter but didn't wire `set_extra_prometheus` — they can still use Lifeguard-less variants of the init if they don't depend on Lifeguard).

**Acceptance criteria (cross-repo):**

- A Hauliage Fleet service request to `GET /fleet/vehicles/{id}` produces a span tree in Jaeger within 30 s, service name `hauliage-fleet`, with `http.request.method=GET`, `url.path=/fleet/vehicles/{id}` (route-templated), `http.response.status_code=200`.
- A matching log record for the request appears in Loki under the OTLP-native stream with `trace_id` / `span_id` attributes populated.
- `kubectl logs deploy/hauliage-fleet` shows only startup banner + OTEL-init-ack lines. Hitting the endpoint 100 times produces zero new lines on stdout.
- All five existing dashboards (`hauliage-overview`, `hauliage-bff`, `hauliage-postgres`, `hauliage-lifeguard`, `hauliage-cluster-logs`) continue to serve data with no panel breakage.
- `cargo clippy --all -D warnings` in BRRTRouter and Lifeguard reports no `disallowed-methods` violations.

**Rollback plan:** if Phase O.1 destabilises Hauliage, `git revert` the Hauliage `main.rs` edits and re-deploy. BRRTRouter + Lifeguard changes can stay (they're removing code, not changing behaviour) but require redeploy.

**Commit scope:** Four PRs across four repos, merged in order: microscaler-observability first (publish API), then BRRTRouter + Lifeguard in parallel (both depend on O.1 being merged in this repo), then Hauliage last (uses the new `init()`).

### Phase O.1.5 — Lifeguard meter-provider removal cleanup

Strictly speaking this was part of O.1, but it warrants its own listing because it's a Lifeguard PRD-level concern and may pick up extra cleanup (the unused `release_connection_span` identified in the audit, the doc drift in `docs/OBSERVABILITY.md`).

**Scope:** Lifeguard repo only.

**Deliverables:**
- Remove `global::set_meter_provider` call (done in O.1).
- Delete unused `release_connection_span` helper.
- Update `docs/OBSERVABILITY.md` to match the current API (drift from `METRICS.exporter.registry()` → `LifeguardMetrics { registry, ... }` + `prometheus_scrape_text()`).
- Extend `docs/OBSERVABILITY_APP_INTEGRATION.md` with an explicit "migrated to microscaler-observability — see ../microscaler-observability/docs/PRD.md" section.

### Phase O.2 — W3C propagation

**Scope:** BRRTRouter-only (mostly). Incoming `traceparent` is extracted; outgoing HTTP calls from handlers pick up the current span's context.

See BRRTRouter PRD v0.3 §Phase O.2 for the code detail. No changes in this v0.4 other than:
- The propagator install already happened in Phase O.1 (moved here from the old BRRTRouter-owned location).
- The extractor + injector helpers live in `brrtrouter::server::trace_context` and use whatever propagator `opentelemetry::global::get_text_map_propagator` returns — no direct SDK dependence in BRRTRouter.

### Phase O.3 — Span catalog

**Scope:** BRRTRouter adds child spans for every significant request phase. Lifeguard already has its 7 spans and they automatically become children.

The span table is unchanged from BRRTRouter PRD v0.3 §Phase O.3. Adds spans for:

- `brrtrouter.parse_request`
- `brrtrouter.router.match`
- `brrtrouter.middleware.before` / `.after`
- `brrtrouter.dispatcher.dispatch`
- `brrtrouter.handler.execute`
- `brrtrouter.schema.validate_request` / `.validate_response`
- `brrtrouter.response.encode`

Plus, new in v0.4: a separate Hauliage-side sub-phase for domain-level spans.

#### Phase O.3-Hauliage — Domain span seeding

Hauliage adds `tracing::span!` or `#[instrument]` at domain-boundary functions:

- `hauliage.consignments.list_jobs` — the one place in the repo that already has logging; spans make the existing structured info/error events children of the span.
- `hauliage.bff.view.{fleet,quote,booking}.*` — BFF view-composition operations.
- `hauliage.fleet.vehicles.lookup`, `hauliage.bookings.create`, `hauliage.quotes.accept` — the core domain verbs.

The list is incremental; not all services need spans on day one. Prioritise by "post-mortem would have been shorter with a span here" — starting with consignments per the April postmortem.

### Phase O.4 — Resource attributes & service metadata

**Scope:** this crate only. Already partly in Phase O.1; this phase adds `container.id`, `host.name`, and refines `OTEL_RESOURCE_ATTRIBUTES` parsing.

### Phase O.5 — Graceful shutdown

**Scope:** this crate + Hauliage.

- This crate's `ShutdownGuard::Drop` does the flush sequence (already scaffolded; Phase O.5 completes the implementation).
- Hauliage's `main()` in every service holds the guard until last, installs a SIGTERM handler that drops it before `std::process::exit`.
- BRRTRouter's codegen template does the same.

### Phase O.6 — Metrics improvements

**Scope:** BRRTRouter + Hauliage. Unchanged from PRD v0.3 §Phase O.6 in substance — no OTEL Metrics SDK, hand-rolled Prometheus text, concat Lifeguard.

Add to Hauliage: `hauliage_*` business metrics. Start small:
- `hauliage_consignments_active_total{status}` — exposed by the consignments service.
- `hauliage_bookings_created_total` — counter.
- `hauliage_fleet_vehicles_assigned` — gauge by tenant.

One metric per service to start, expandable.

### Phase O.7 — Log → trace correlation

**Scope:** YAML-only (Grafana datasource config).

OTLP-native log records already carry `trace_id` / `span_id`. Phase O.7 wires Grafana's Loki datasource `derivedFields` and Jaeger's `tracesToLogsV2` mapping so click-through works both directions.

### Phase O.8 — Promtail eviction

**Scope:** `hauliage/k8s/observability/` YAML. Narrow Promtail to startup-stdout-only. Runtime logs arrive via OTEL Collector's Loki exporter.

### Phase O.9 — Dashboard overhaul

**Scope:** mainly this repo + hauliage.

- **microscaler-observability** exposes canonical BRRTRouter + Lifeguard dashboard JSONs via `dashboards/` directory — reusable across services. Existing Hauliage dashboards reference them instead of duplicating.
- **Hauliage** adds: `hauliage-jaeger-service-map` (Grafana Tempo/Jaeger datasource panel), `hauliage-business-metrics` (bookings/day, etc.), `hauliage-telemetry-health` (OTLP queue depth, dropped spans, scrape freshness).
- **BRRTRouter** ships in its repo: `brrtrouter-request-level` (per-route latency heatmap, schema failures, auth failures, CORS, worker pool, client disconnects) — referenced by Hauliage dashboards via includes.
- **Lifeguard** continues to own `hauliage-lifeguard.json` (or renames to `lifeguard-overview.json` since it's Lifeguard-generic).

### Phase O.10 — Memory middleware tuning

**Scope:** BRRTRouter only. Unchanged from PRD v0.3 §Phase O.10. Warmup window + per-100 MB bucket warn. Restores sensitivity to real RSS drift without firing on ramp.

### Phase O.11 — Perf-PRD integration

**Scope:** BRRTRouter's `scripts/run_goose_tests.py`. Unchanged from PRD v0.3 §Phase O.11.

### Phase O.12 — Pyroscope continuous profiling

**Scope:** this crate. `pyroscope-rs` behind the `profiling` cargo feature (scaffolded already). Feature-gated so non-profiling consumers pay zero cost.

Hauliage services that want profiling enable the feature in their `Cargo.toml`; the Helm chart sets `PYROSCOPE_SERVER_ADDRESS=http://pyroscope.observability:4040`.

### Phase O.13 — Lifeguard `channel_layer()` deprecation

**Scope:** Lifeguard only, and later than the rest.

Per the earlier "(a) → (c), skip (b)" analysis:

- **This release cycle (with O.1):** `channel_layer()` stays, marked `#[deprecated]` with a note pointing at `microscaler_observability::init`. Feature-flagged in Hauliage host mains (default off).
- **Next minor Lifeguard release:** delete the module. No Hauliage service uses it today (audit confirmed).

## 7. Cross-repo migration order

```
┌─────────────────────────────────────────────────────────┐
│  O.0 — docs in all four repos (same day)                │
│  ┌──────────────────────────────────────────────────┐   │
│  │  microscaler-observability — PRD + README + scaf │   │
│  │  BRRTRouter — historical-note header on v0.3 PRD │   │
│  │  lifeguard — doc amendment                       │   │
│  │  hauliage — observability-migration.md           │   │
│  └──────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────┐
│  O.1 — v0.1.0 crate release + consumers                 │
│                                                         │
│  1. microscaler-observability: init() real implementation│
│     (separate PR, merged + tagged v0.1.0 first)         │
│                                                         │
│  2. PARALLEL:                                           │
│     BRRTRouter: delete old otel.rs, depend on new crate │
│     Lifeguard: remove set_meter_provider call           │
│                                                         │
│  3. Hauliage: update every main.rs, redeploy            │
│     (per-service rolling, validate Jaeger sees traces)  │
│                                                         │
│  Rollback plan: per-service — revert Hauliage main.rs   │
└─────────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────┐
│  O.2  W3C propagation        ─┐                         │
│  O.3  Span catalog            │                         │
│  O.4  Resource attrs          │  any order,             │
│  O.5  Graceful shutdown       │  independent            │
│  O.6  Metrics improvements    │  per-repo PRs           │
│  O.10 Memory middleware tune ─┘                         │
└─────────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────┐
│  O.7  Log→trace correlation  (YAML only)                │
│  O.8  Promtail eviction       (YAML only)               │
│  O.9  Dashboards              (mostly JSON)             │
└─────────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────┐
│  O.11 Perf-PRD integration                              │
│  O.12 Pyroscope                                         │
│  O.13 Lifeguard channel_layer deprecation               │
└─────────────────────────────────────────────────────────┘
```

## 8. Risk & trade-offs

| Risk | Likelihood | Repo blast radius | Mitigation |
|---|---|---|---|
| Version skew between this crate's `opentelemetry` pin and Lifeguard's | **High** | All four repos | Pin to `"0.29"` in this crate's `Cargo.toml` with inline comment referencing Lifeguard. Any bump is a coordinated cross-repo change, gated by PRD §Phase O.0. |
| BRRTRouter's codegen template lags the hand-edited `impl/src/main.rs` | Medium | Hauliage | Phase O.1 updates the template in the same PR. Re-run codegen per service. `PRD_BFF_SCAFFOLDING_REMEDIATION.md` already called this out. |
| Some Hauliage service migrates to `microscaler_observability::init` while others still use `brrtrouter::otel::init_logging_with_config` | Medium | Hauliage | Phase O.1 keeps the BRRTRouter function as a deprecated shim for one release that re-exports the new crate's API. Hauliage services migrate at their own pace. |
| OTLP export backpressure drops log records | Medium | Any service under load | `BatchLogRecordProcessor` drops on overflow; expose `brrtrouter_otel_log_dropped_records_total` counter (Phase O.6). Grafana alert on sustained drops. |
| Dashboards break because `brrtrouter_*` metric names change | Low | Hauliage dashboards | Phase O.6 deprecates old metrics alongside new; both emit for one release. |
| Silencing stdout makes `kubectl logs` useless for in-flight incidents | Medium-High (by design) | Hauliage SRE workflow | Runbooks reference Grafana/Jaeger. Panics still hit stderr. `BRRTR_DEV_LOGS_TO_STDOUT=1` escape hatch. |
| A third-party crate (not BRRTRouter/Lifeguard) pulled in by Hauliage calls `global::set_*` | Low | Any service | The `disallowed-methods` clippy rule catches first-party violations; third-party crates need manual review during dep bumps. |
| Lifeguard's `channel_layer()` deprecation surprises some consumer | Low | Lifeguard users outside Hauliage | Phase O.13 — deprecation warning for one release before removal. |

## 9. Success criteria (whole PRD)

- **G1–G8** from §2 all satisfied.
- Hauliage's five existing dashboards are still green.
- At least two new dashboards exist covering BRRTRouter per-route latency + telemetry health.
- Jaeger shows a service-graph for Hauliage — BFF at the center, downstream services as nodes, requests carry `traceparent` end-to-end.
- The `consignments/list_jobs` postmortem scenario is reproducible *and diagnosable* with a Jaeger trace in under 60 s (the original incident took hours).
- `cargo clippy --workspace -- -D warnings` across all four repos reports zero `disallowed-methods` violations.
- `microscaler-observability` is at v0.1.0 on crates.io *or* stable as a workspace path dependency with a tagged release.

## 10. Revision history

| Version | Date | Change |
|---|---|---|
| 0.1 | 2026-04-18 | Initial DRAFT in `BRRTRouter/docs/PRD_OBSERVABILITY_AND_TRACING.md`. 11 phases. Proposed OTEL deps at 0.27. Missed Lifeguard composition contract. |
| 0.2 | 2026-04-18 | Folded Lifeguard composition findings. Added Phase O.0 (BRRTRouter ↔ Lifeguard ownership table). Bumped OTEL pins to 0.29. Phase O.6 stays on Prometheus-text. |
| 0.3 | 2026-04-18 | Replaced stdout→Promtail→Loki with OTLP-native logs. Phase O.7 collapsed; Phase O.8 descoped to startup-only. Added Phase O.12 (Pyroscope). |
| 0.4 | 2026-04-18 | **Cross-repo restructure.** Created this crate as neutral peer; extracted all OTEL globals from BRRTRouter and Lifeguard; moved PRD to this repo as authoritative home. Hauliage called out as the real driver. Phase O.1.5 added for Lifeguard cleanup. Phase O.3 split into BRRTRouter-side + Hauliage-domain sub-phases. Migration order diagram added. |
