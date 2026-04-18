# OpenTelemetry version pinning — why `"0.29"` and how bumps are coordinated

> **Status:** DRAFT
> **Last-synced:** 2026-04-18 — against `../../Cargo.toml`, `../../../lifeguard/Cargo.toml` (`opentelemetry = "0.29.1"`), and PRD v0.4 §Phase O.0.
> **Authority:** `../../PRD.md` §Phase O.0 "Dependency pin" + `../../README.md` "Version coupling".
> **Related:** [`hexagonal-architecture.md`](./hexagonal-architecture.md), [`sibling-repos-and-wikis.md`](./sibling-repos-and-wikis.md).

## What this page covers

Why `Cargo.toml` says `opentelemetry = "0.29"` and not a newer version. Why any bump is a coordinated cross-repo change, never unilateral. What breaks if the pin is wrong.

## The pin

`../../Cargo.toml`:

```toml
[dependencies]
opentelemetry = "0.29"
opentelemetry_sdk = { version = "0.29", features = ["rt-tokio-current-thread", "trace", "logs"] }
opentelemetry-otlp = { version = "0.29", default-features = false, features = ["grpc-tonic", "trace", "logs"] }
opentelemetry-semantic-conventions = "0.29"
opentelemetry-appender-tracing = "0.29"
tracing-opentelemetry = "0.30"   # pairs with opentelemetry 0.29
```

Lifeguard (`../../../lifeguard/Cargo.toml`):

```toml
opentelemetry = { version = "0.29.1", features = ["testing"], optional = true }
opentelemetry-prometheus = { version = "0.29.1", optional = true }
opentelemetry_sdk = { version = "0.29.0", default-features = false, features = ["metrics"], optional = true }
```

Same major (`0.29`). Same minor family. Differ only in optional features. That's the constraint — same **major** is mandatory, same **minor** is strongly preferred, patch version may drift if Lifeguard's `0.29.1` pin hasn't been updated since this crate bumped.

## What happens if the pin is wrong

`opentelemetry` uses process-global state: `global::set_tracer_provider`, `global::set_meter_provider`, `global::tracer()`, `global::meter()`. Those globals are **keyed by the `opentelemetry` crate's own types** — `TracerProvider` trait objects, `Meter` handles, propagator references.

If two crates in the same binary depend on two different **majors** of `opentelemetry`, cargo resolves them as two separate dependencies, duplicated in the dep tree. The `opentelemetry::global::*` slot installed by one major is **a different slot** from the one the other major reads. So:

- This crate (on `0.29`) installs `TracerProvider` via its `0.29::global::set_tracer_provider`.
- Lifeguard (on `0.30` hypothetically) tries to obtain a tracer via its `0.30::global::tracer("lifeguard")`. Different global slot. Gets `NoopTracer`.
- Lifeguard's spans emit fine at the `tracing::span!` call site — but never reach this crate's OTLP exporter because the `tracing_opentelemetry` bridge sees the `0.29` tracer, and Lifeguard's code sees the `0.30` one.

The symptom is subtle: everything compiles, no runtime panic, some spans show up in Jaeger (BRRTRouter's), others don't (Lifeguard's), nobody can tell why. It would take hours to diagnose.

## Cross-repo bump procedure

Because unilateral bumps silently break the above invariant, bumps are coordinated:

1. A single human (the person running the bump) opens a PR against one of the four repos with a `bump(deps): opentelemetry 0.29 → 0.30` style title. Commit touches only `Cargo.toml` + `Cargo.lock`.
2. That PR CI must pass.
3. In parallel, matching PRs open against the other three repos (including this one). Each uses `{ version = "0.30", path = "../microscaler-observability" }` style pins during the transition if a workspace-path checkout is being used.
4. All four PRs merge in this order: `microscaler-observability` first (so the coordinated API surface is available), then `lifeguard` + `BRRTRouter` in parallel, then `hauliage` last.
5. A single commit in each repo with a back-reference to this wiki page in the commit body.

This is the same pattern as PRD phase O.1 itself — the PRD explicitly models this as a coordinated multi-PR landing (see PRD v0.4 §7 "Cross-repo migration order").

## Why not chase the latest OTEL release?

As of 2026-04-18, `opentelemetry-rust` is still pre-1.0 — the crate's changelog shows recent breaking changes across 0.26 → 0.27 → 0.28 → 0.29. Chasing the latest before the API stabilises means paying the coordination cost every quarter. The 0.29 pin was chosen because:

- Lifeguard landed on it first (see `../../../lifeguard/Cargo.toml` history).
- It has the first stable `opentelemetry_sdk::logs::LoggerProvider` that Phase O.1 needs.
- It has `tracing-opentelemetry = "0.30"` pairing that's known to work.

Next probable bump window: when `opentelemetry 1.0` lands upstream and the API stabilises.

## Features alignment

Both crates need the same *major*; the *feature* set may differ. Current alignment:

| Feature | `microscaler-observability` | Lifeguard |
|---|---|---|
| `trace` (SDK) | ✅ | ❌ (Lifeguard doesn't export traces) |
| `logs` (SDK) | ✅ | ❌ |
| `metrics` (SDK) | ❌ | ✅ (for its `SdkMeterProvider`) |
| `rt-tokio-current-thread` | ✅ (for `BatchSpanProcessor` / `BatchLogRecordProcessor`) | ❌ |
| `testing` | ❌ | ✅ (for Lifeguard's unit tests) |

No collision. Both crates pulling the same `opentelemetry` major but different feature sets compiles cleanly — cargo unifies features per-crate.

## Open questions

> **Open:** If Lifeguard ever bumps to `opentelemetry 0.30.x` ahead of this crate, what's the grace window? Answer tentatively: zero — this crate must land the same day. Document explicitly when the first real bump happens.

> **Open:** Hauliage has no `opentelemetry` in its own `Cargo.toml` today (it inherits via BRRTRouter + Lifeguard path deps). After Phase O.1 adds `microscaler-observability` as a direct path-dep, Hauliage will have THREE transitive paths to `opentelemetry` — check `cargo tree -d` after Phase O.1 to confirm cargo's feature unification keeps it to one real compile of the crate.
