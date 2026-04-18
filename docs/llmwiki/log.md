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

> **Open:** Once Phase O.1 lands and `init()` is no longer a scaffold stub, remove the two `#[expect(clippy::unimplemented, reason = ...)]` carve-outs in `lib.rs` and `config.rs`. Grep helper: `rg 'clippy::unimplemented' src/` should return zero matches after Phase O.1 merges. (`#[expect]` was adopted over `#[allow]` in the 2026-04-18 coding-standards import — see the log entry below.)

> **Open:** Set up GitHub branch protection on `main` to require the `CI` job (the aggregate gate) + require PRs (no direct pushes) when the user's ready. Not scripted — GitHub UI task.

## [2026-04-18] ingest | JSF AV Rules + Microsoft Pragmatic Rust Guidelines

Imported the two workspace-standard coding references per the user directive "in the BRRTRouter repo you will find JSF Coding rules, these need to be added here. as well as rust-guidelines.txt".

Raw references added under `../references/`:

- `rust-guidelines.md` — moved from repo root (the 90 KB Microsoft Pragmatic Rust Guidelines dropped in earlier; renamed `.txt` → `.md` since it IS markdown and gets proper GitHub rendering).
- `jsf-writeup.md` — copied from `../../../BRRTRouter/docs/JSF/JSF_WRITEUP.md` (the authoritative 1300-line distillation).
- `jsf-audit-opinion.md` — copied from `../../../BRRTRouter/docs/JSF/JSF_AUDIT_OPINION.md`.
- `jsf-compliance.md` — copied from `../../../BRRTRouter/docs/JSF_COMPLIANCE.md`.

*Not* copied: the 800 KB `JSF-AV-rules.pdf`. The PDF is the original Lockheed Martin source; BRRTRouter owns one copy. We reference the Stroustrup-hosted public copy instead.

Wiki synthesis pages added:

- `topics/coding-standards-jsf-inspired.md` — six JSF principles we inherit (bounded complexity / allocation discipline / no exceptions / strong types / no recursion / test coverage), plus the three we deliberately decline (C++-specific or handled by Rust). Tabular mapping from JSF AV rules → our enforcement mechanism.
- `topics/pragmatic-rust-guidelines.md` — grouped by verification state: rules already honoured at v0.0.1 (M-PANIC-IS-STOP, M-PUBLIC-DEBUG, M-UNSAFE, M-LINT-OVERRIDE-EXPECT, M-STATIC-VERIFICATION, etc.), rules adopted but not verifiable until Phase O.1 (M-HOTPATH, M-THROUGHPUT, M-LOG-STRUCTURED, M-CANONICAL-DOCS), rules explicitly declined with rationale (M-APP-ERROR — this is a library, not an app).

Enforcement mechanisms landed:

- `../../../clippy.toml` — mirrored from BRRTRouter's JSF-inspired thresholds verbatim: `cognitive-complexity-threshold = 30`, `too-many-lines-threshold = 200`, `too-many-arguments-threshold = 8`, `stack-size-threshold = 512000`, `enum-variant-size-threshold = 256`, `type-complexity-threshold = 300`. With `nursery` denied in `Cargo.toml`, the complexity / line / argument thresholds are compile errors, not warnings.
- `../../AGENTS.md` — extended the "Golden rules" section with a new **G4 — JSF-inspired hot-path + complexity discipline** row-by-row mapping of JSF / Microsoft rules to our enforcement. G3 title clarified to enumerate `todo!` / `unimplemented!` / `unreachable!` alongside `unwrap` / `expect` / `panic!`.
- `src/lib.rs::init` and `src/config.rs::from_env` — converted `#[allow(clippy::unimplemented)]` → `#[expect(clippy::unimplemented, reason = "…")]` per Microsoft's M-LINT-OVERRIDE-EXPECT. When Phase O.1 removes the `unimplemented!()` macros, the compiler will warn that the `expect` attribute is now stale, reminding the engineer to remove it too.

Touched pages:

- `../../../clippy.toml` — created.
- `../../../AGENTS.md` — G3 title tweak + new G4 block.
- `../../../src/lib.rs` — `#[expect]` migration.
- `../../../src/config.rs` — `#[expect]` migration.
- `topics/coding-standards-jsf-inspired.md` — created.
- `topics/pragmatic-rust-guidelines.md` — created.
- `docs-catalog.md` — new "Imported reference documents" section listing the four reference files + their synthesising wiki pages.

Verification: all CI gates still pass (`cargo fmt --check`, `cargo clippy` against 3 feature legs at `-D warnings`, `cargo test --all-targets --all-features`, `cargo doc` with `-D warnings`).

> **Open:** Lifeguard hasn't adopted the same `clippy.toml` thresholds (no JSF-style compliance there yet). Worth a follow-up PR on the Lifeguard side to align — but out of scope for this crate.

> **Open:** The Microsoft M-CANONICAL-DOCS rule suggests specific doc-comment sections (`# Arguments`, `# Errors`, `# Panics`, `# Examples`, `# Safety`). Audit during Phase O.1 code review to ensure every new public item follows.
