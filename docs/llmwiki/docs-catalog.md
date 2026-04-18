# Docs catalog

Inventory of every source this wiki synthesises. The wiki is the *compiled* layer; these are the *raw* files. Agent rule: when you read one of these sources and the matching wiki page's `Last-synced` is older than the file's `git log` shows, the wiki is stale — sync it.

---

## In-repo sources

| Path | Role | Synthesised by |
|---|---|---|
| [`../PRD.md`](../PRD.md) | **Master cross-repo PRD.** Single authoritative plan for the 13 phases. | [`topics/hexagonal-architecture.md`](./topics/hexagonal-architecture.md), [`topics/otel-version-pinning.md`](./topics/otel-version-pinning.md), [`flows/init-flow.md`](./flows/init-flow.md). |
| [`../../README.md`](../../README.md) | Public-facing crate README (hexagonal diagram, env-var contract, feature flags). | [`topics/hexagonal-architecture.md`](./topics/hexagonal-architecture.md), [`topics/sibling-repos-and-wikis.md`](./topics/sibling-repos-and-wikis.md). |
| [`../../AGENTS.md`](../../AGENTS.md) | Agent operational rules. | (Not synthesised — agents read it directly.) |
| [`../../Cargo.toml`](../../Cargo.toml) | Dep pins, feature flags. | [`topics/otel-version-pinning.md`](./topics/otel-version-pinning.md). |
| [`../../src/lib.rs`](../../src/lib.rs) | Public API stub (`init()`, re-exports). | [`flows/init-flow.md`](./flows/init-flow.md). |
| [`../../src/config.rs`](../../src/config.rs) | `ObservabilityConfig`, `OtlpProtocol`, `Sampler`. | *(future: `entities/entity-observability-config.md` when Phase O.1 lands.)* |
| [`../../src/error.rs`](../../src/error.rs) | `ObservabilityError`, `ObservabilityResult`. | *(future: `entities/entity-observability-error.md`.)* |
| [`../../src/shutdown.rs`](../../src/shutdown.rs) | `ShutdownGuard` RAII handle. | [`entities/entity-shutdown-guard.md`](./entities/entity-shutdown-guard.md). |

---

## Sibling-repo sources

### BRRTRouter (`../../../BRRTRouter/`)

| Path | Role | Synthesised by |
|---|---|---|
| [`BRRTRouter/docs/PRD_OBSERVABILITY_AND_TRACING.md`](../../../BRRTRouter/docs/PRD_OBSERVABILITY_AND_TRACING.md) | Historical PRD v0.1–v0.3 (superseded by this repo's `PRD.md` but retained for context). | [`topics/hexagonal-architecture.md`](./topics/hexagonal-architecture.md) for the "how v0.4 differs from v0.3" framing. |
| [`BRRTRouter/src/otel.rs`](../../../BRRTRouter/src/otel.rs) | BRRTRouter's current (stub) logging init. Phase O.1 deletes this file. | [`topics/hexagonal-architecture.md`](./topics/hexagonal-architecture.md), [`topics/sibling-repos-and-wikis.md`](./topics/sibling-repos-and-wikis.md). |
| [`BRRTRouter/AGENTS.md`](../../../BRRTRouter/AGENTS.md) | Sibling agent rules — the pattern this repo's `AGENTS.md` follows. | (Pattern source, not content.) |
| [`BRRTRouter/llmwiki/`](../../../BRRTRouter/llmwiki/) | Sibling wiki — deeper technical reference on routing, dispatch, hot-path. | [`topics/sibling-repos-and-wikis.md`](./topics/sibling-repos-and-wikis.md). |
| [`BRRTRouter/Cargo.toml`](../../../BRRTRouter/Cargo.toml) | Sibling dep pins — the other half of the `opentelemetry = "0.29"` coupling. | [`topics/otel-version-pinning.md`](./topics/otel-version-pinning.md). |

### Lifeguard (`../../../lifeguard/`)

| Path | Role | Synthesised by |
|---|---|---|
| [`lifeguard/Cargo.toml`](../../../lifeguard/Cargo.toml) | **Authoritative OTEL version pin.** `opentelemetry = "0.29.1"`, `opentelemetry_sdk = "0.29.0"`, `opentelemetry-prometheus = "0.29.1"`. This repo tracks it. | [`topics/otel-version-pinning.md`](./topics/otel-version-pinning.md). |
| [`lifeguard/src/metrics.rs`](../../../lifeguard/src/metrics.rs) | Lifeguard's `LifeguardMetrics::init()` — currently owns `global::set_meter_provider`. Phase O.1.5 removes that call. | [`topics/hexagonal-architecture.md`](./topics/hexagonal-architecture.md), [`topics/sibling-repos-and-wikis.md`](./topics/sibling-repos-and-wikis.md). |
| [`lifeguard/src/logging/tracing_layer.rs`](../../../lifeguard/src/logging/tracing_layer.rs) | `channel_layer()` — may-mpsc → stderr drain. Phase O.13 deprecates. | *(future: `topics/lifeguard-channel-layer.md`.)* |
| [`lifeguard/docs/OBSERVABILITY_APP_INTEGRATION.md`](../../../lifeguard/docs/OBSERVABILITY_APP_INTEGRATION.md) | Pre-existing Lifeguard-side contract (4 rules) — this repo honours rules 1-4. | [`topics/hexagonal-architecture.md`](./topics/hexagonal-architecture.md). |
| [`lifeguard/AGENT.md`](../../../lifeguard/AGENT.md) | Sibling agent rules (singular file, outlier). | (Pattern source.) |
| [`lifeguard/docs/llmwiki/`](../../../lifeguard/docs/llmwiki/) | Sibling wiki. | [`topics/sibling-repos-and-wikis.md`](./topics/sibling-repos-and-wikis.md). |

### Hauliage (`../../../hauliage/`)

| Path | Role | Synthesised by |
|---|---|---|
| [`hauliage/k8s/observability/dashboards/`](../../../hauliage/k8s/observability/dashboards/) | The five existing Grafana dashboards — must stay green throughout the migration. | *(future: `topics/hauliage-dashboards.md` when Phase O.9 is implemented.)* |
| [`hauliage/k8s/observability/README.md`](../../../hauliage/k8s/observability/README.md) | Operational pre-reqs (shared observability stack). | [`topics/sibling-repos-and-wikis.md`](./topics/sibling-repos-and-wikis.md). |
| [`hauliage/docs/postmortems/postmortem-consignments-list-jobs-empty-2026-04.md`](../../../hauliage/docs/postmortems/postmortem-consignments-list-jobs-empty-2026-04.md) | The "why we need this" anchor — a DB error with no observability trail. | [`topics/hexagonal-architecture.md`](./topics/hexagonal-architecture.md) (motivation section). |
| [`hauliage/microservices/Cargo.toml`](../../../hauliage/microservices/Cargo.toml) | The workspace that path-depends on `../../BRRTRouter/` + `../../lifeguard/` (and eventually `../../microscaler-observability/`). | [`topics/sibling-repos-and-wikis.md`](./topics/sibling-repos-and-wikis.md). |
| [`hauliage/AGENTS.md`](../../../hauliage/AGENTS.md) | Sibling agent rules. | (Pattern source.) |
| [`hauliage/docs/llmwiki/`](../../../hauliage/docs/llmwiki/) | Sibling wiki. | [`topics/sibling-repos-and-wikis.md`](./topics/sibling-repos-and-wikis.md). |

---

## External sources

| URL | Role | Cited by |
|---|---|---|
| [Karpathy's LLM Wiki gist](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f) | The pattern this wiki follows. | [`README.md`](./README.md), [`SCHEMA.md`](./SCHEMA.md). |
| [OpenTelemetry SDK Environment Variables spec](https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/) | Authoritative env-var contract. This crate honours it. | *(future: `reference/env-vars.md`.)* |
| [W3C TraceContext specification](https://www.w3.org/TR/trace-context/) | Propagation format. | *(future: `topics/w3c-propagation.md` in Phase O.2.)* |

---

> **Open:** When Phase O.1 lands, every `(future: …)` item above gets promoted to a real wiki page in the same commit, and the `log.md` entry notes the promotions.
