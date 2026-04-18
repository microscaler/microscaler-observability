# microscaler-observability — agent rules

Strict operational rules for AI assistants working in this repository. **Knowledge about *how* this crate works is in [`docs/llmwiki/`](./docs/llmwiki/), not here.** This file only holds rules the agent must obey.

---

## Golden rules — foundational, enforced at CI

These three rules are the non-negotiable foundations of this crate. They are enforced mechanically so human review and agent diligence can focus on design. Codebases that add testing, linting, or panic-hygiene as afterthoughts never recover; we are starting with them in place at v0.0.1 so they never become someone's later problem.

### G1 — Testing is not an afterthought

- **Every new public item lands with its unit test in the same PR.** No "I'll add tests later". Later never arrives.
- **Every bug fix lands with a regression test that would have caught the bug.** The test exists to prove the fix works and to prevent the bug from returning.
- **CI runs `cargo test --all-targets` and `cargo test --all-features --doc` on every PR.** See `.github/workflows/ci.yml`.
- The v0.0.1 scaffold already has **19 unit tests** across `src/{lib,config,error,shutdown}.rs` despite the crate having no real implementation — because the day we land code *without* a test is the day the rule starts eroding.

### G2 — Pedantic clippy at deny level

`Cargo.toml` `[lints.clippy]` enforces:

| Group / lint | Level | Why |
|---|---|---|
| `clippy::pedantic`  | **deny** (with documented carve-outs) | Strict code-quality baseline. No legacy debt to grandfather. |
| `clippy::nursery`   | **deny** (with documented carve-outs) | Catches emerging anti-patterns. Cheaper to deny now than to triage later. |
| `unsafe_code`       | **forbid** (rust-level, not clippy) | Pure library; no unsafe is ever needed. Forbid cannot be overridden by `#[allow]`. |

Carve-outs exist for three specific pedantic lints that are noisy on legitimate patterns (`module_name_repetitions`, `missing_errors_doc`, `must_use_candidate`), each documented in the `Cargo.toml` comments next to the lint.

**CI runs** `cargo clippy --all-targets --all-features -- -D warnings` on three feature matrix legs (`default`, `--no-default-features`, `--all-features`). A lint warning anywhere breaks the build.

### G3 — No `unwrap()` / `expect()` / `panic!()` on any non-test path

`Cargo.toml` `[lints.clippy]` declares `deny` for:

- `unwrap_used`
- `expect_used`
- `panic`
- `unreachable`
- `todo`
- `unimplemented`

The crate-wide `deny` means the compiler rejects any of these in library code. The only legitimate exceptions are:

1. **Tests** — the pattern `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` at the top of a `#[cfg(test)] mod tests` block. `assert!(result.unwrap())` is idiomatic in tests and `deny`-ing it there is the wrong trade-off.
2. **Deliberate scaffold stubs** — `#[allow(clippy::unimplemented)]` on the `init()` / `from_env()` scaffold functions in v0.0.1. The local `#[allow]` + comment forces any agent removing the `unimplemented!()` to see the allow and the PRD reference right next to it.

Any new `unwrap()` / `expect()` / `panic!` in production code requires an explicit `#[allow(…)]` with a comment justifying the panic. In practice that should almost never happen — propagate a `Result<T, ObservabilityError>` instead. When an operator's process is being killed by a panic in observability init, the fact that the panic was "obvious" at the author's desk is no comfort.

### What these three rules buy us

Libraries that ship with all three from day 1 stay robust as they grow. Libraries that add them later drown in fixing their own accumulated violations. The dependency on this crate from Hauliage's ~17 microservices means that a panic or hidden `Err` here stops ~17 services; we owe Hauliage the rigour that makes that unlikely.

Authority for each rule: `Cargo.toml` `[lints]` table + `.github/workflows/ci.yml`.

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

*(These layer on top of the §Golden rules above — those are the invariants, these are the context-specific design rules.)*

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
