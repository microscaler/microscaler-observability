# Content catalog

The full inventory of wiki pages. Read [`SCHEMA.md`](./SCHEMA.md) first if you don't yet know how this wiki is structured. Each entry has a one-line purpose + `Status` + `Last-synced`.

---

## Structural files

| Page | Purpose |
|---|---|
| [`README.md`](./README.md) | Wiki entry point. Four-file orientation. |
| [`SCHEMA.md`](./SCHEMA.md) | Directory layout, page template, agent workflow. |
| [`index.md`](./index.md) | This file. Content catalog. |
| [`log.md`](./log.md) | Chronological session log. |
| [`docs-catalog.md`](./docs-catalog.md) | Inventory of `../` sources this wiki synthesises. |

---

## Topics

Concept / subsystem explanations. Answer "why" and "how".

| Page | Status | Last-synced | Purpose |
|---|---|---|---|
| [`topics/hexagonal-architecture.md`](./topics/hexagonal-architecture.md) | DRAFT | 2026-04-18 | Why this crate is a peer of BRRTRouter / Lifeguard, not a child. |
| [`topics/otel-version-pinning.md`](./topics/otel-version-pinning.md) | DRAFT | 2026-04-18 | Why `opentelemetry = "0.29"` is coupled to Lifeguard and how any bump is coordinated. |
| [`topics/sibling-repos-and-wikis.md`](./topics/sibling-repos-and-wikis.md) | DRAFT | 2026-04-18 | Cross-repo responsibility split and how to navigate between the four wikis. |

*(future — will be added as phases land):*

- `topics/otlp-egress-pipeline.md` — Phase O.1: how spans + logs reach the Collector.
- `topics/stdout-invariant.md` — Phase O.1: "stdout is startup-only" and the `BRRTR_DEV_LOGS_TO_STDOUT` escape hatch.
- `topics/w3c-propagation.md` — Phase O.2: `traceparent` extract / inject.
- `topics/span-catalog.md` — Phase O.3: the full span tree across BRRTRouter + Lifeguard + Hauliage.
- `topics/lifeguard-channel-layer.md` — Phase O.13: deprecation plan.

## Entities

Specific named things — types, env vars, cargo features, files.

| Page | Status | Last-synced | Purpose |
|---|---|---|---|
| [`entities/entity-shutdown-guard.md`](./entities/entity-shutdown-guard.md) | DRAFT | 2026-04-18 | The RAII handle returned by `init()`. |

*(future):*

- `entities/entity-observability-config.md` — Phase O.1: the config struct + env var mapping.
- `entities/entity-observability-error.md` — Phase O.1: error taxonomy.
- `entities/entity-feature-flag-profiling.md` — Phase O.12.
- `entities/entity-feature-flag-dev-stdout-fallback.md` — Phase O.1.

## Flows

Time-ordered process descriptions.

| Page | Status | Last-synced | Purpose |
|---|---|---|---|
| [`flows/init-flow.md`](./flows/init-flow.md) | DRAFT | 2026-04-18 | What happens (and doesn't yet happen) when `init()` is called. |

*(future):*

- `flows/shutdown-flow.md` — Phase O.5: `ShutdownGuard::Drop` flush sequence.
- `flows/span-emission-flow.md` — Phase O.3: from `tracing::span!` to OTLP.
- `flows/log-emission-flow.md` — Phase O.1: from `tracing::info!` to Loki.

## Reference

Lookup tables. Dense, scannable.

*(future, Phase O.1 seeds them):*

- `reference/env-vars.md` — every env var this crate reads.
- `reference/cargo-features.md` — every cargo feature and its effect.
- `reference/metric-names.md` — metrics this crate and its dependants expose.
- `reference/span-names.md` — all `brrtrouter.*`, `lifeguard.*`, `hauliage.*` span names observable in Jaeger when this stack is running.

---

## Quick links by question

| You want to know... | Read |
|---|---|
| Why a new crate instead of extending BRRTRouter? | [`topics/hexagonal-architecture.md`](./topics/hexagonal-architecture.md) |
| Why `opentelemetry = "0.29"` and not newer? | [`topics/otel-version-pinning.md`](./topics/otel-version-pinning.md) |
| How does this crate relate to Lifeguard's existing `lifeguard::metrics`? | [`topics/sibling-repos-and-wikis.md`](./topics/sibling-repos-and-wikis.md) |
| What does `init()` actually do today? | [`flows/init-flow.md`](./flows/init-flow.md) — spoiler: panics with an instruction message. |
| What will `ShutdownGuard::Drop` do when Phase O.1 lands? | [`entities/entity-shutdown-guard.md`](./entities/entity-shutdown-guard.md) |
| How is the master observability PRD organised? | [`../PRD.md`](../PRD.md) §6 (13 phases, O.0–O.13). |

---

> **Open:** As of 2026-04-18 this index is seeded by hand from the scaffold. Once Phase O.1 lands and pages are added for the real implementation, the update cadence moves to "every commit that changes `src/` updates the matching wiki page in the same commit".
