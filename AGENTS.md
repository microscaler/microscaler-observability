# microscaler-observability — agent rules

Strict operational rules for AI assistants working in this repository. **Knowledge about *how* this crate works is in [`docs/llmwiki/`](./docs/llmwiki/), not here.** This file only holds rules the agent must obey.

---

## Before you do anything

1. Read [`docs/llmwiki/README.md`](./docs/llmwiki/README.md) — wiki entry point.
2. Read [`docs/llmwiki/SCHEMA.md`](./docs/llmwiki/SCHEMA.md) — wiki conventions and agent workflow.
3. Skim [`docs/llmwiki/index.md`](./docs/llmwiki/index.md) — content catalog.
4. Tail [`docs/llmwiki/log.md`](./docs/llmwiki/log.md) for recent context.
5. Open [`docs/PRD.md`](./docs/PRD.md) — the cross-repo master PRD that drives all phase work in this crate and the three siblings (BRRTRouter, Lifeguard, Hauliage).

Sessions that skip step 1 waste work. The wiki accumulates what earlier sessions learned; not reading it means repeating work. See [Karpathy's LLM-wiki gist](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f) for the underlying pattern.

---

## Repository shape

- **Purpose:** OTEL output adapter for the microscaler platform. Peer of BRRTRouter and Lifeguard, not a child of either. See [`llmwiki/topics/hexagonal-architecture.md`](./docs/llmwiki/topics/hexagonal-architecture.md).
- **Status:** v0.0.1 scaffold. `init()` deliberately panics. Real implementation lands with Phase O.1 of [`docs/PRD.md`](./docs/PRD.md).
- **Primary language:** Rust (single crate, no workspace yet).
- **Sibling repos** (typical `microscaler/` checkout):
  - [`../BRRTRouter/`](../BRRTRouter/) — HTTP adapter — wiki at [`../BRRTRouter/llmwiki/`](../BRRTRouter/llmwiki/).
  - [`../lifeguard/`](../lifeguard/) — Postgres adapter — wiki at [`../lifeguard/docs/llmwiki/`](../lifeguard/docs/llmwiki/).
  - [`../hauliage/`](../hauliage/) — real domain composition root — wiki at [`../hauliage/docs/llmwiki/`](../hauliage/docs/llmwiki/).

See [`llmwiki/topics/sibling-repos-and-wikis.md`](./docs/llmwiki/topics/sibling-repos-and-wikis.md) for the responsibility split.

---

## Build, lint, test

- `cargo check` — quick type-check (seconds).
- `cargo build` — full compile.
- `cargo fmt` — format before committing. Always.
- `cargo clippy --all-targets -- -D warnings` — lint.
- `cargo test` — full test suite (currently scaffold; Phase O.1 adds the OTLP round-trip integration test).
- `cargo test --features profiling` — include Pyroscope-enabled tests.

No `justfile` yet. Will be added when Phase O.1 lands.

---

## Core rules the agent must obey

### 1. This crate owns OpenTelemetry globals — nobody else does

The hexagonal contract of the wider platform is that **every call to `opentelemetry::global::set_tracer_provider`, `set_logger_provider`, `set_meter_provider`, or `set_text_map_propagator` lives in this crate and only this crate.** BRRTRouter and Lifeguard must never call these. Hauliage host-app `main.rs` files must never call these. Only `microscaler_observability::init()` does.

Enforcement: a `clippy.toml` `disallowed-methods` rule will land in Phase O.1 across all four repos. Any agent reintroducing a global install outside this crate is creating a bug that clippy will reject.

Authority: [`docs/PRD.md`](./docs/PRD.md) §5.1 ownership matrix. Wiki: [`llmwiki/topics/hexagonal-architecture.md`](./docs/llmwiki/topics/hexagonal-architecture.md).

### 2. OpenTelemetry version is pinned to 0.29 to match Lifeguard

`Cargo.toml` pins `opentelemetry = "0.29"`. Lifeguard uses `opentelemetry = "0.29.1"`, `opentelemetry_sdk = "0.29.0"`, `opentelemetry-prometheus = "0.29.1"` (see `../lifeguard/Cargo.toml`). Any bump in this crate requires a coordinated bump in Lifeguard — it is never unilateral.

Authority: [`docs/PRD.md`](./docs/PRD.md) §Phase O.0 dependency-pinning section. Wiki: [`llmwiki/topics/otel-version-pinning.md`](./docs/llmwiki/topics/otel-version-pinning.md).

### 3. Stdout is startup-only under load (consumers' invariant, not this crate's)

This crate's default configuration **never** writes runtime events to stdout. Only startup `println!`, panic `eprintln!`, and graceful-shutdown output hit stdout. The `dev-stdout-fallback` cargo feature (default on) installs a stdout `fmt::Layer` only when `OTEL_EXPORTER_OTLP_ENDPOINT` is unset, or when `BRRTR_DEV_LOGS_TO_STDOUT=1` forces break-glass local debugging.

Authority: [`docs/PRD.md`](./docs/PRD.md) §5.3.

### 4. No metric SDK install without explicit PRD update

Phase O.1 does **not** install a `MeterProvider`. Metrics stay on BRRTRouter's Prometheus-text `/metrics` endpoint, concatenated with Lifeguard's `prometheus_scrape_text()`. Any future PR that introduces `set_meter_provider` must first land a PRD amendment explaining why the hand-rolled path is insufficient.

Authority: [`docs/PRD.md`](./docs/PRD.md) Non-goal N4.

### 5. `init()` is called exactly once per process

From `main()` in the host composition root, before any HTTP server or DB pool. Returns a `ShutdownGuard` that must be held for process lifetime. Tests use `tracing::subscriber::set_default` instead of `init()` — they're process-scoped, not global. See [`llmwiki/entities/entity-shutdown-guard.md`](./docs/llmwiki/entities/entity-shutdown-guard.md).

---

## Rust style

- `snake_case` fns / modules, `CamelCase` types, `SCREAMING_SNAKE_CASE` consts.
- Public API requires doc-comments with `# Errors` / `# Panics` / `# Examples` sections where applicable.
- `Result<T, ObservabilityError>` in library paths. No `unwrap()` / `expect()` in non-test code.
- Sort imports: std → external → internal.
- `#![deny(missing_docs)]` is set — any new public item needs rustdoc.

---

## Commit discipline

- Commits follow Conventional Commits (`feat(scope):`, `fix(scope):`, `docs(scope):`, `chore(scope):`, `refactor(scope):`, `test(scope):`).
- **Never push** without explicit human authorization.
- **Never use `--no-verify`** or `--no-verify-commit`. Let pre-commit hooks run.
- **Never commit secrets** (`.env`, credentials, tokens).
- **Never commit with a Cursor co-author trailer** — clients do not permit it. The human user is the sole author.

---

## Explicit instruction: read the wiki

**Every session starts with reading [`docs/llmwiki/`](./docs/llmwiki/).** This is not optional.

End of session: update the wiki pages your work touched, append an entry to [`log.md`](./docs/llmwiki/log.md) in the form `## [YYYY-MM-DD] <ingest|query|lint|scaffold|phase-O.x> | <one-line summary>`, flag any `> **Open:**` questions. Leave the wiki one step more useful than you found it.

---

## Useful files

- [`README.md`](./README.md) — project overview (the hexagonal diagram, env-var contract, why this crate exists).
- [`docs/PRD.md`](./docs/PRD.md) — master cross-repo PRD (13 phases).
- [`Cargo.toml`](./Cargo.toml) — dep pins, feature flags.
- [`src/lib.rs`](./src/lib.rs) — public API (stub in v0.0.1).
- [`docs/llmwiki/docs-catalog.md`](./docs/llmwiki/docs-catalog.md) — inventory of this crate's `docs/` sources.
