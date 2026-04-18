# Hexagonal architecture — why this crate exists as a peer

> **Status:** DRAFT
> **Last-synced:** 2026-04-18 — against `../../PRD.md` v0.4 §1-§5, `../../README.md` (hexagonal diagram), and the BRRTRouter PRD v0.3 it supersedes.
> **Authority:** `../../PRD.md` §1 "Summary" + §5.1 "Ownership matrix".
> **Related:** [`otel-version-pinning.md`](./otel-version-pinning.md), [`sibling-repos-and-wikis.md`](./sibling-repos-and-wikis.md), [`../flows/init-flow.md`](../flows/init-flow.md).

## What this page covers

Why `microscaler-observability` was created as a standalone crate instead of extending [`BRRTRouter/src/otel.rs`](../../../../BRRTRouter/src/otel.rs). The hexagonal (ports-and-adapters) rationale, the smell that surfaced when Hauliage grew up into a real consumer, and the ownership contract this crate enforces.

## The architecture, in one diagram

```
┌──────────────────────────────────────────────────────────┐
│                     Host app (main.rs)                   │
│  ┌────────────────────────────────────────────────────┐  │
│  │                  DOMAIN (core)                     │  │
│  │    handler impls, business logic, domain types     │  │
│  └────────────────────────────────────────────────────┘  │
│        ▲                    │                   │        │
│     input              output (DB)         output (OTEL) │
│        │                    │                   │        │
│  ┌─────┴──────┐    ┌────────┴──────┐    ┌───────┴──────┐ │
│  │ BRRTRouter │    │   Lifeguard   │    │  THIS CRATE  │ │
│  │ (HTTP in)  │    │ (Postgres out)│    │  (OTEL out)  │ │
│  │  also out: │    │               │    │              │ │
│  │  HTTP resp │    │               │    │              │ │
│  └────────────┘    └───────────────┘    └──────────────┘ │
└──────────────────────────────────────────────────────────┘
```

BRRTRouter is an HTTP adapter (inbound request, outbound response). Lifeguard is a Postgres adapter (outbound query, inbound result). This crate is an OTEL adapter (outbound telemetry). **All three are peers** of the domain core. None of them is upstream of any other.

## The smell that forced this

Pre-v0.4, the workspace had two adapters trying to own OpenTelemetry globals:

1. [`BRRTRouter/src/otel.rs::init_logging_with_config`](../../../../BRRTRouter/src/otel.rs) was the single subscriber init. Benign-seeming but presumptive — any non-HTTP service (CLI, migration tool) that wanted the same subscriber behaviour had to pull in BRRTRouter.
2. [`lifeguard/src/metrics.rs::LifeguardMetrics::init()`](../../../../lifeguard/src/metrics.rs) calls `opentelemetry::global::set_meter_provider(...)` via `OnceCell::call_once`. Whichever adapter ran first won; the other silently lost.

When Hauliage grew up — seventeen microservices all embedding BRRTRouter + Lifeguard — the racing `set_meter_provider` became a production concern, and the "BRRTRouter owns observability init" pattern meant Hauliage's CLI tooling (migration runner, reflector) had to either pull BRRTRouter in as a dep they never used or reimplement the init from scratch.

Pulling OTEL globals out of both adapters into a neutral peer fixes both symptoms. The contract becomes "no adapter owns global OTEL state; only the observability adapter does".

## The ownership contract

See [`../../PRD.md`](../../PRD.md) §5.1 for the authoritative matrix. Summary:

| Global | Owner | Notes |
|---|---|---|
| `opentelemetry::global::set_tracer_provider` | **this crate** | Called exactly once per process from `init()`. |
| `opentelemetry::global::set_text_map_propagator` | **this crate** | Same. |
| `opentelemetry::global::set_logger_provider` (or the `tracing_opentelemetry` bridge equivalent) | **this crate** | Same. |
| `opentelemetry::global::set_meter_provider` | **nobody today** | PRD §Phase O.6 keeps metrics on Prometheus-text; no `MeterProvider` install until a future PRD proves one is needed. |
| `tracing_subscriber::registry().try_init()` | **this crate** | Single subscriber. |
| `tracing::span!` / `tracing::event!` emission | **every adapter emits; this crate exports** | BRRTRouter, Lifeguard, and Hauliage domain code all emit `tracing::*`. This crate's subscriber converts those into OTLP spans + log records. |

## Enforcement mechanism

Phase O.1 lands `clippy.toml` entries in BRRTRouter and Lifeguard (and enables them by default in this crate) with `disallowed-methods` listing all four `opentelemetry::global::set_*_provider` functions. Any agent reintroducing a global install outside this crate produces a compile error — not a runtime bug, not a "first-one-wins" race, just a clear stop.

The single exemption is this crate itself; the clippy rule lives in `../../../BRRTRouter/clippy.toml` and `../../../lifeguard/clippy.toml` only, not here.

## How this pattern compares to v0.3

The BRRTRouter PRD v0.3 (retained at [`BRRTRouter/docs/PRD_OBSERVABILITY_AND_TRACING.md`](../../../../BRRTRouter/docs/PRD_OBSERVABILITY_AND_TRACING.md)) proposed BRRTRouter owning `set_tracer_provider` + `set_logger_provider` + `set_text_map_propagator`, while Lifeguard kept `set_meter_provider`. That "each adapter owns its piece" model was simpler to sketch but failed the hexagonal test:

- A pure-Lifeguard CLI (migration runner, reflector) still had no home for OTEL init.
- A new third-party adapter (e.g. a Redis client with its own tracing) would need yet another `set_*_provider` race resolution.

v0.4 resolves by making the observability adapter peer-of-everything and giving it the *sole* responsibility for globals. Adapters become pure emitters. Host `main()` is the composition root — the only place that calls `init()`, the only place that holds the `ShutdownGuard`.

## What "pure emitter" means in practice

BRRTRouter and Lifeguard, under the Phase O.1 contract:

- MAY call `tracing::span!`, `tracing::event!`, `#[tracing::instrument]` freely.
- MAY call `opentelemetry::global::tracer(...)` / `meter(...)` to obtain a handle, but only to *read* the currently-installed provider, never to *set* one.
- MUST NOT call `opentelemetry::global::set_tracer_provider`, `set_logger_provider`, `set_meter_provider`, or `set_text_map_propagator`.
- MUST NOT call `tracing_subscriber::registry().try_init()` or `tracing::subscriber::set_global_default`.

Tests inside those crates use `tracing::subscriber::set_default` (scoped, not global) to install per-test subscribers. This is fine; it doesn't touch globals.

## Open questions

> **Open:** Does a service that depends on `microscaler-observability` *optionally* — i.e. can initialise OTEL or skip it — work correctly if the crate dependency is present but `init()` is never called? Phase O.1 must handle this: if no subscriber is installed, `tracing::*` macros still no-op cleanly, and BRRTRouter / Lifeguard don't panic.

> **Open:** What happens when two unit tests in the same binary each call `init()`? It must return `ObservabilityError::AlreadyInitialized` on the second call. Verify in Phase O.1 integration tests.
