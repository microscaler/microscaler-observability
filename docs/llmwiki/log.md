# Wiki log

Append-only chronological history. Each entry follows the format in [`SCHEMA.md`](./SCHEMA.md) → "log.md entry format".

Useful queries:

```bash
# Last 5 entries:
grep "^## \[" docs/llmwiki/log.md | tail -5

# All scaffold events:
grep -A2 "^## \[.*\] scaffold" docs/llmwiki/log.md

# Everything in April 2026:
grep "^## \[2026-04-" docs/llmwiki/log.md
```

---

## [2026-04-18] scaffold | wiki and AGENTS.md imported from organisation pattern

Seeded the wiki for the new `microscaler-observability` crate, following:

- [Karpathy's LLM-wiki gist](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f) for the conceptual model (three layers — raw sources / wiki / schema).
- The sibling repos' established conventions: [`../BRRTRouter/AGENTS.md`](../../../BRRTRouter/AGENTS.md), [`../hauliage/AGENTS.md`](../../../hauliage/AGENTS.md), [`../lifeguard/AGENT.md`](../../../lifeguard/AGENT.md). Two of three use `AGENTS.md` (plural); two of three put the wiki under `docs/llmwiki/`. This repo matches that majority.

Pages created:

- Structural: `README.md`, `SCHEMA.md`, `index.md`, `log.md` (this file), `docs-catalog.md`.
- `topics/hexagonal-architecture.md` — the foundational concept that makes this crate exist as a peer, not a child.
- `topics/otel-version-pinning.md` — the `0.29` coupling to Lifeguard.
- `topics/sibling-repos-and-wikis.md` — cross-repo responsibility split.
- `entities/entity-shutdown-guard.md` — RAII flush handle.
- `flows/init-flow.md` — what `init()` does today (panics with an instruction) and the target shape Phase O.1 will implement.

All seven non-structural pages are `Status: DRAFT` — synthesised from a single session's reading of the scaffold code + `docs/PRD.md` v0.4. First lint pass should happen after Phase O.1 lands and there's a second source (working `init()`) to validate claims against.

Paired with [`../../AGENTS.md`](../../AGENTS.md) at repo root.

> **Open:** No `reference/` pages yet. Those seed with Phase O.1 when env-var contract + metric names + span names become concrete.

> **Open:** Cross-repo wiki citations (`[[../../../lifeguard/docs/llmwiki/…](…)]`) depend on the standard `microscaler/` sibling checkout layout. If the directory layout ever changes, every wiki needs a lint pass to fix the relative paths.

## [2026-04-18] scaffold | CI + golden rules landed at v0.0.1

Per user directive "codebases that add testing as an afterthought are always problematic — this must be a golden rule", three foundational disciplines were installed in-tree *before* any phase work begins:

- **G1 — Testing from day 1.** 19 unit tests across `src/{lib,config,error,shutdown}.rs` covering builder methods, error display, RAII shutdown semantics, public API stability, and the deliberate-panic scaffold regression guard.
- **G2 — Pedantic clippy.** `[lints.clippy]` table in `Cargo.toml` at `deny` level for `pedantic` + `nursery` groups, with three documented carve-outs (`module_name_repetitions`, `missing_errors_doc`, `must_use_candidate`). `[lints.rust]` at `forbid` for `unsafe_code`.
- **G3 — No hot-path panics.** `unwrap_used`, `expect_used`, `panic`, `unreachable`, `todo`, `unimplemented` all at `deny`. Local `#[allow(clippy::unimplemented)]` on the two scaffold stubs (`init()` + `ObservabilityConfig::from_env()`) so the Phase O.1 engineer removing the `unimplemented!` calls sees the allow next to them.

Pages touched:

- `../../AGENTS.md` — new "Golden rules" section at the top with §G1-G3. "Core rules" section prefix note acknowledging the layer structure.
- `../../Cargo.toml` — `[lints.rust]` + `[lints.clippy]` tables.
- `../../src/lib.rs`, `src/config.rs`, `src/error.rs`, `src/shutdown.rs` — local `#[allow]` carve-outs + unit tests.
- `../../.github/workflows/ci.yml` — new CI workflow: fmt → clippy matrix (3 feature legs) → test matrix (2 feature legs) → docs → MSRV → aggregate `ci-success` gate.
- `../../.github/workflows/audit.yml` — weekly `cargo audit` against RustSec.
- `../../.github/dependabot.yml` — weekly Cargo + GitHub-Actions updates, `opentelemetry` family grouped so coordinated-bump PRs stay single.

All gates green locally: `cargo fmt --check` + `cargo clippy --all-targets --all-features -- -D warnings` + `cargo clippy --all-targets --no-default-features -- -D warnings` + `cargo test --all-features` (19 passed) + `cargo doc --no-deps` with `-D warnings -D rustdoc::broken_intra_doc_links`.

> **Open:** Once Phase O.1 lands and `init()` is no longer a scaffold stub, remove the two `#[allow(clippy::unimplemented)]` carve-outs in `lib.rs` and `config.rs`. Grep helper: `rg 'clippy::unimplemented' src/` should return zero matches after Phase O.1 merges.

> **Open:** Set up GitHub branch protection on `main` to require the `CI` job (the aggregate gate) + require PRs (no direct pushes) when the user's ready. Not scripted — GitHub UI task.
