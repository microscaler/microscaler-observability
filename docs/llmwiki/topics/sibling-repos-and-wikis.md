# Sibling repos and their wikis — responsibility split and navigation

> **Status:** DRAFT
> **Last-synced:** 2026-04-18 — against PRD v0.4 §1 "Summary" and readings of each sibling's top-level `AGENTS.md` / `AGENT.md`.
> **Authority:** `../../PRD.md` §1 (target repos table) + each sibling's agent-rules file.
> **Related:** [`hexagonal-architecture.md`](./hexagonal-architecture.md), [`../docs-catalog.md`](../docs-catalog.md) (sibling-repo sources section).

## What this page covers

The four-repo constellation: BRRTRouter, Lifeguard, Hauliage, this crate. Which wiki to consult for which question. Why agents checking out just one of these repos will struggle — the PRD is cross-repo and the wikis are mutually cross-referenced.

## The four repos, on one page

| Repo | Role | Wiki entry-point | AGENTS file |
|---|---|---|---|
| **[`../../../BRRTRouter/`](../../../../BRRTRouter/)** | HTTP input + output adapter — the router, dispatcher, server, middleware stack, OpenAPI codegen. | [`BRRTRouter/llmwiki/`](../../../../BRRTRouter/llmwiki/) (at repo root — outlier from the majority). | [`BRRTRouter/AGENTS.md`](../../../../BRRTRouter/AGENTS.md) |
| **[`../../../lifeguard/`](../../../../lifeguard/)** | Postgres output adapter — ORM, pool, transaction, migration engine. | [`lifeguard/docs/llmwiki/`](../../../../lifeguard/docs/llmwiki/) | [`lifeguard/AGENT.md`](../../../../lifeguard/AGENT.md) (singular — outlier from the majority). |
| **[`../../../hauliage/`](../../../../hauliage/)** | Real composition root — ~17 microservices with real domain code, real Lifeguard DB usage, real dashboards. | [`hauliage/docs/llmwiki/`](../../../../hauliage/docs/llmwiki/) | [`hauliage/AGENTS.md`](../../../../hauliage/AGENTS.md) |
| **this crate** | OTEL output adapter. | [`docs/llmwiki/`](..) | [`AGENTS.md`](../../../AGENTS.md) |

## Standard sibling checkout layout

All four wikis use `../../../` relative paths assuming:

```
microscaler/
├── BRRTRouter/
├── lifeguard/
├── hauliage/
└── microscaler-observability/
```

If any of these are missing from a given checkout, cross-repo links 404 gracefully — the wikis are designed so single-repo navigation still works; sibling navigation is an optional enhancement.

## Who owns what (functional responsibilities)

| Concern | Home |
|---|---|
| HTTP routing, dispatcher, server, OpenAPI codegen | **BRRTRouter** |
| Incoming HTTP parse, response encode | **BRRTRouter** |
| W3C `traceparent` extraction from requests | **BRRTRouter** (Phase O.2) |
| Request-lifecycle metrics (`http_server_request_duration_seconds` etc.) | **BRRTRouter** (Phase O.6) |
| `lifeguard_*` Prometheus metrics | **Lifeguard** (`prometheus_scrape_text()`) |
| ORM, `LifeModel`, `SelectQuery`, pool, transactions | **Lifeguard** |
| SQL schema migrations | **Lifeguard** (`lifeguard-migrate`) |
| `tracing::span!` spans for query / pool / transaction operations | **Lifeguard** (7 existing span sites — see [lifeguard wiki](../../../../lifeguard/docs/llmwiki/)) |
| Domain logic: bookings, fleet, quotes, consignments, etc. | **Hauliage** |
| Domain `tracing::span!` spans (e.g. `hauliage.bookings.create`) | **Hauliage** (Phase O.3-Hauliage) |
| Grafana dashboards (the 5 existing + new ones) | **Hauliage** `k8s/observability/dashboards/` |
| Deployment Helm charts | **Hauliage** `helm/` |
| OTLP egress (traces + logs) | **this crate** |
| `TracerProvider` / `LoggerProvider` / propagator install | **this crate** |
| `ShutdownGuard` RAII flush | **this crate** |
| Pyroscope continuous profiling install | **this crate** (Phase O.12, behind `profiling` feature) |

## Who reads what (navigation by question)

| You want to know | Read |
|---|---|
| How to write a span in domain code | Hauliage `AGENTS.md` + `docs/llmwiki/` |
| How the router matches a path | BRRTRouter `llmwiki/flows/` (routing / dispatch flows) |
| How to add a new Lifeguard model | Lifeguard `docs/llmwiki/topics/` |
| Why logs go over OTLP, not stdout | *this wiki* → [`../flows/init-flow.md`](../flows/init-flow.md) and `../../PRD.md` §5 |
| How OTLP version 0.29 got picked | *this wiki* → [`otel-version-pinning.md`](./otel-version-pinning.md) |
| Why `microscaler-observability` exists as a separate crate | *this wiki* → [`hexagonal-architecture.md`](./hexagonal-architecture.md) |
| How to add a new dashboard panel | Hauliage `k8s/observability/README.md` + Hauliage wiki |
| What changes in each Hauliage microservice's `main.rs` for Phase O.1 | `../../PRD.md` §Phase O.1 + future `hauliage/docs/observability-migration.md` |

## Cross-repo citation hygiene

Wiki pages cite sibling-repo pages with relative paths as described in [`../SCHEMA.md`](../SCHEMA.md). When a sibling wiki is restructured (e.g. BRRTRouter moves `llmwiki/` to `docs/llmwiki/` to match the majority convention), every cross-repo citation in this wiki breaks. Lint pass after any sibling restructure.

Grep helper to find all cross-repo citations from this wiki:

```bash
rg '\.\./\.\./\.\.\/(BRRTRouter|lifeguard|hauliage)' docs/llmwiki/
```

## Where PRDs live

Historically BRRTRouter hosted `docs/PRD_OBSERVABILITY_AND_TRACING.md` (v0.1–v0.3). v0.4 moved the master cross-repo PRD into this repo (`../../PRD.md`) because its scope outgrew any one repo. BRRTRouter's copy stays in-place but will get a header note in Phase O.0 saying "superseded by microscaler-observability/docs/PRD.md".

The *single authoritative source* for cross-repo observability plans: `microscaler-observability/docs/PRD.md`.
Hauliage-specific, BRRTRouter-specific, and Lifeguard-specific PRDs stay in their own repos and cite this master when observability-adjacent.

## Open questions

> **Open:** BRRTRouter's `llmwiki/` is at repo root (outlier); the majority convention is `docs/llmwiki/`. Worth filing a BRRTRouter issue to relocate? Low priority — citations work either way, but alignment is marginally cleaner.

> **Open:** Lifeguard's `AGENT.md` is singular (outlier); majority is `AGENTS.md`. Same question, same low priority.
