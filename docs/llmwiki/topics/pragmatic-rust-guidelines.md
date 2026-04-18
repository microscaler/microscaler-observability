# Pragmatic Rust Guidelines — adaptations applied to this crate

> **Status:** DRAFT
> **Last-synced:** 2026-04-18 — against `../references/rust-guidelines.md` (Microsoft Pragmatic Rust Guidelines, MIT-licensed).
> **Authority:** Microsoft [Pragmatic Rust Guidelines](https://microsoft.github.io/pragmatic-rust-guidelines/) — the raw copy in `docs/references/rust-guidelines.md`.
> **Related:** [`coding-standards-jsf-inspired.md`](./coding-standards-jsf-inspired.md).

## What this page covers

Microsoft publishes a ~90 KB guide of opinionated, named Rust rules (`M-*` tags like `M-PANIC-IS-STOP`, `M-HOTPATH`, `M-PUBLIC-DEBUG`). It's broader in scope than JSF — it covers API design, documentation, error handling, and AI-friendliness as well as performance and safety. This page synthesises the subset that matters to this crate, keyed by rule id so you can jump from the synthesis straight into the source if you need the long form.

The full file is `../references/rust-guidelines.md`. This page is the **compiled-and-kept-current** synthesis; the file is the **immutable raw source** per the Karpathy three-layer model (see `../SCHEMA.md`).

## Rules this crate already honours (verified as of v0.0.1)

| Rule id | What it says | Where we honour it |
|---|---|---|
| **M-DESIGN-FOR-AI** | APIs easy for humans are easy for agents. Use strong types, thorough docs, testable surfaces. | The whole public API — `init()` / `ObservabilityConfig` / `ShutdownGuard` / typed `ObservabilityError`. Every public item has rustdoc. |
| **M-PANIC-IS-STOP** | A panic means "the process should stop". Library code must not panic on recoverable input. | `[lints.clippy]` denies `unwrap_used` / `expect_used` / `panic` crate-wide (G3 golden rule in `AGENTS.md`). |
| **M-PANIC-ON-BUG** | Detected programming bugs *should* be panics, not errors. Errors are for runtime conditions the caller could plausibly handle. | `ObservabilityError::AlreadyInitialized` is an error (runtime mis-order, caller can handle), not a panic. The two `unimplemented!()` scaffold stubs panic because they are programmer-bugs-waiting-to-happen — an agent trying to use them before Phase O.1 *is* a bug. |
| **M-PUBLIC-DEBUG** | Every public type derives `Debug`. | `ObservabilityConfig`, `ObservabilityError`, `OtlpProtocol`, `Sampler`, `ShutdownGuard` all `#[derive(Debug)]`. (`ShutdownGuard`'s Debug is via `_private: ()` — opaque but compiles.) |
| **M-REGULAR-FN** | Prefer regular functions over methods when the body doesn't depend on `self`. | `ObservabilityConfig::from_env()` is `-> Self`, not `&self` — constructor-pattern. |
| **M-UNSAFE**, **M-UNSAFE-IMPLIES-UB**, **M-UNSOUND** | Avoid unsafe; when unsafe, document soundness. | `[lints.rust]` sets `unsafe_code = "forbid"`. Stronger than `deny` — can't be overridden with `#[allow]`. A future agent proposing `unsafe` needs to remove the forbid from Cargo.toml, which shows up in PR review. |
| **M-CONCISE-NAMES** | Names are free of weasel words. No `DataManagerHandler` / `UtilityHelper`. | `init()` / `ShutdownGuard` / `ObservabilityConfig` — nouns and verbs, no vague suffixes. |
| **M-LINT-OVERRIDE-EXPECT** | Use `#[expect(lint)]` instead of `#[allow(lint)]`. `expect` warns if the lint stops firing, catching stale attributes. | The two deliberate scaffold stubs (`init()` in `src/lib.rs`, `from_env()` in `src/config.rs`) use `#[expect(clippy::unimplemented, reason = "...")]`. When Phase O.1 removes the `unimplemented!()` call, the compiler warns that the `expect` is now stale, reminding the engineer to also delete the attribute. |
| **M-STATIC-VERIFICATION** | Prefer type-system / compile-time checks over runtime checks. | `Sampler::ParentBasedTraceIdRatio(f64)` encodes the ratio arg in the type. `OtlpProtocol` as an enum prevents typos. `#[must_use]` on `ShutdownGuard` ensures callers don't drop it accidentally. |

## Rules we adopt but can't verify at v0.0.1 (will verify at Phase O.1)

| Rule id | What it says | When/how we adopt |
|---|---|---|
| **M-HOTPATH** | Identify, profile, optimize the hot path early. | PRD Phase O.1 tests include a roundtrip timing test. Phase O.3 (span catalog) + Phase O.11 (perf-PRD integration) close the loop by wiring bench metrics into the cross-repo perf work. |
| **M-THROUGHPUT** | Optimise for throughput; avoid empty cycles. | Phase O.1 uses `BatchSpanProcessor` / `BatchLogRecordProcessor` (batched async export), not sync-per-span. The BSP background thread is the only added CPU cycle — expected <1% overhead. |
| **M-YIELD-POINTS** | Long-running tasks should have yield points. | Our only "long-running task" is the BSP flusher, owned by `opentelemetry_sdk::trace` — already has yield points built in. |
| **M-LOG-STRUCTURED** | Use structured logging with message templates, not `format!`-style. | Enforced by using `tracing::info!(field = value, "message")` instead of `tracing::info!("{}...", value)`. Phase O.3 span-catalog work explicitly adopts the structured-event pattern for all new spans. |
| **M-MODULE-DOCS** | Every module has `//!` docs explaining its purpose and examples. | `src/lib.rs` has full module docs (the hexagonal diagram, env-var contract, when `init()` no-ops). `src/config.rs`, `src/error.rs`, `src/shutdown.rs` all have `//!` module docs. Verified at v0.0.1. |
| **M-CANONICAL-DOCS** | Docs have canonical sections (`# Arguments`, `# Errors`, `# Panics`, `# Examples`, `# Safety`). | Already partly done — `init()` has `# Errors` + `# Panics` + `# Contract`. Future public items in Phase O.1 will match. |
| **M-FIRST-DOC-SENTENCE** | First sentence of every rustdoc is one line, ~15 words, standalone-parseable. | Partly done — verify during Phase O.1 code review. |

## Rules we explicitly decline

| Rule id | What it says | Why we decline |
|---|---|---|
| **M-APP-ERROR** | Applications may use `anyhow` / similar. | This is a **library**, not an application. `anyhow::Error` would leak into our public API — errors should be typed. We use `thiserror`'s derive macros to keep `ObservabilityError` exhaustive and pattern-matchable. |
| **M-MIMALLOC-APPS** | Apps should opt into `mimalloc` for performance. | Host apps choose their allocator. This crate doesn't pin one. |
| **M-SMALLER-CRATES** | When in doubt, split the crate. | We *are* the result of splitting. This crate exists specifically because it was wrong to bundle observability into BRRTRouter. We won't split further until there's a concrete consumer forcing it. |

## Rules worth revisiting at future phases

- **M-ISOLATE-DLL-STATE** — only matters if this crate ever ships as a dylib. Not planned.
- **M-DOCUMENTED-MAGIC** — Phase O.1 adds default timeout / batch-size / queue-size constants. Each one gets a doc comment with its rationale, not just its value.
- **M-DOC-INLINE** — `pub use` re-exports (e.g. `pub use config::ObservabilityConfig;` in `src/lib.rs`) should get `#[doc(inline)]` so rustdoc attaches the original type's docs to the re-export. Worth a small Phase O.1 polish pass.

## How this page grows

- When a new Microsoft guideline is cited in a PR or commit, add a row to one of the three tables above.
- If a PR deviates from a guideline, **document the deviation in the PR description** and add a "Rules we explicitly decline" entry here with a back-reference.
- When Phase O.1 lands, the "Rules we adopt but can't verify at v0.0.1" table promotes verified entries to the first table.

## Open questions

> **Open:** Microsoft guidelines include a full `[lints.clippy]` `restriction` group recommendation (commented section in `../references/rust-guidelines.md`). Our `Cargo.toml` has pedantic + nursery + selective `restriction` lints (`unwrap_used` / `expect_used` / `panic`). A one-time audit of the full Microsoft `restriction` list to pull in any others that match our discipline.

> **Open:** M-LOG-STRUCTURED is trivially honoured today (we don't log anything in library code), but Phase O.1 will introduce startup `println!` lines and the OpenTelemetryTracingBridge path. Need to re-verify during Phase O.1 review that no `format!`-style templates slip in.
