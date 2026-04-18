# JSF AV rules — how we adapted them for this crate

> **Status:** DRAFT
> **Last-synced:** 2026-04-18 — against `../references/jsf-writeup.md` (imported from `../../../BRRTRouter/docs/JSF/JSF_WRITEUP.md`), `../references/jsf-audit-opinion.md`, and `../references/jsf-compliance.md`.
> **Authority:** `../../../clippy.toml` + `../../../Cargo.toml` `[lints]` table. The references are the *why*; the lint files are the *what*.
> **Related:** [`pragmatic-rust-guidelines.md`](./pragmatic-rust-guidelines.md), [`hexagonal-architecture.md`](./hexagonal-architecture.md), [`../flows/init-flow.md`](../flows/init-flow.md).

## What this page covers

The [Joint Strike Fighter Air Vehicle C++ Coding Standards](https://www.stroustrup.com/JSF-AV-rules.pdf) were written for safety-critical avionics software — predictable performance, no runtime failures, bounded everything. BRRTRouter distilled a subset of those rules into a "BRRTRouter-SAFE" profile for its hot path (see `../references/jsf-writeup.md` + `../references/jsf-compliance.md`). This crate follows the same distillation, adapted for its role as the OTEL output adapter.

The full writeup is `../references/jsf-writeup.md` (43 KB); the audit opinion is `../references/jsf-audit-opinion.md` (6.6 KB); the compliance summary is `../references/jsf-compliance.md` (3.9 KB). Those files are the **raw sources**; this page is the synthesised "what it means for this crate" layer.

## The six JSF principles we inherit

### 1. Bounded complexity (JSF AV Rule 1, 3)

- **JSF intent:** Functions ≤ 200 logical SLOC; cyclomatic complexity ≤ 20. The rationale is that a reviewer must be able to reason about the function in one pass.
- **What it means here:** `clippy.toml` sets `too-many-lines-threshold = 200` and `cognitive-complexity-threshold = 30`. Because the `pedantic` + `nursery` lint groups are denied in `Cargo.toml`, any function exceeding either threshold is a compile error, not a warning.
- **Specifically for this crate:** `init()` is the only function anywhere near a "hot path" boundary — but it's called once per process at startup, not per request. The bounded-complexity rule is still a useful hygiene rule for future maintainers, and it ties the workspace consistently across this crate and BRRTRouter.

### 2. Allocation discipline (JSF AV Rule 206)

- **JSF intent:** *"Allocation/deallocation from/to the free store (heap) shall not occur after initialization."* The rationale is fragmentation and non-deterministic latency.
- **What it means here:** This crate's hot path is *inside the OTEL exporter* (Phase O.1) — which already does batched allocation via `BatchSpanProcessor` / `BatchLogRecordProcessor`. We can't and shouldn't go zero-allocation. What we *can* do: keep the public API surface (e.g. `ObservabilityConfig`) allocation-free for the common case. Builder methods that take `impl Into<String>` are already allocation-aware.
- **Specifically for this crate:** The strict zero-alloc-hot-path rule applies to BRRTRouter's dispatch loop, not to us. But we inherit the `stack-size-threshold = 512000` cap on stack allocations and the `enum-variant-size-threshold = 256` — both defensive rather than active constraints today.

### 3. No exceptions (JSF AV Rule 208)

- **JSF intent:** No `throw` / `catch`. Rationale: predictable control flow, no surprise stack-unwinding.
- **What it means here:** Already our **G3** golden rule (see `../../../AGENTS.md`). `unwrap_used`, `expect_used`, `panic`, `unreachable`, `todo`, `unimplemented` all at `deny` in the `[lints.clippy]` table. The only exemptions are the two deliberate scaffold stubs (`init()` + `from_env()`) and test modules.
- **JSF added insight we already honour:** *"All failure modes are explicit and surfaced as an error-type, never as process aborts."* That's exactly what [`ObservabilityError`](../entities/entity-shutdown-guard.md) does — it names every failure mode (`AlreadyInitialized`, `InvalidEndpoint { value, reason }`, `ExporterConstruction(...)`, `SubscriberAlreadyInstalled`, `Shutdown(...)`) so callers can pattern-match rather than grep panic strings.

### 4. Data & type rules (JSF AV Rule 148, 209, 215)

- **JSF intent:** Enums over integer codes for finite sets; newtypes for IDs; avoid hidden polymorphism.
- **What it means here:** `OtlpProtocol` is an enum (`Grpc` / `HttpProto` / `HttpJson`), not a string. `Sampler` is an enum (`ParentBasedAlwaysOn` / `ParentBasedTraceIdRatio(f64)` / `AlwaysOff`). No integer codes. No magic strings.
- **Remaining gap:** Phase O.1's env-var parsing will turn strings from `OTEL_EXPORTER_OTLP_PROTOCOL` into `OtlpProtocol`. The public `ObservabilityConfig.endpoint: Option<String>` is a `String` rather than a newtype like `OtlpEndpoint` — defensible because we pass it straight to `opentelemetry-otlp` which wants a `String`. If URL validation ever gets lifted into this crate, a newtype is right.

### 5. Flow control (JSF AV Rule 119)

- **JSF intent:** No recursion; structured branching only.
- **What it means here:** Trivially honoured — this crate has no recursive algorithms, no path-matching, no tree walks. The rule matters more for BRRTRouter's radix tree. We inherit it as a forward-looking invariant.

### 6. Testing discipline (JSF AV Rule 219-221)

- **JSF intent:** All dispatch paths (base tests applied to derived types; structural coverage of polymorphic resolutions) tested.
- **What it means here:** Already our **G1** golden rule — 19 tests at v0.0.1 scaffold baseline, every public item has a unit test, every bug fix will land with a regression test. Phase O.1 adds an integration test (`tests/otlp_roundtrip.rs`) that exercises the complete init → emit → flush cycle against an in-process OTLP gRPC receiver.

## What this crate does NOT inherit from JSF

Three JSF rules are either handled by Rust (no need to enforce) or don't apply (C++ specific):

- **Width-explicit integer typedefs (AV Rule 209 literal interpretation).** Rust's `u16`/`u32`/`usize` are already explicit. We don't need `Uint16_t` aliases.
- **No `goto` / restricted `break` / `continue` (AV Rule 190-196).** `goto` doesn't exist in Rust. `break 'label` with labelled loops is idiomatic and stays allowed.
- **Template / generics discipline (AV Rule 101-106).** Rust monomorphisation is well-understood; we don't need JSF's rules about avoiding template meta-programming.

The full BRRTRouter audit opinion (`../references/jsf-audit-opinion.md`) covers these distinctions in more depth.

## Lints that enforce the above at compile time

| JSF principle | Enforcement mechanism |
|---|---|
| Bounded complexity (AV 1, 3) | `clippy.toml` → `cognitive-complexity-threshold = 30`, `too-many-lines-threshold = 200`, `too-many-arguments-threshold = 8` |
| Stack discipline (AV 206 adaptation) | `clippy.toml` → `stack-size-threshold = 512000` |
| No panics (AV 208) | `Cargo.toml` → `[lints.clippy]` `unwrap_used` / `expect_used` / `panic` / `unreachable` / `todo` / `unimplemented` = `deny` |
| Strong types (AV 148, 209) | `OtlpProtocol` + `Sampler` enums in `src/config.rs`; `ObservabilityError` typed variants in `src/error.rs` |
| No recursion (AV 119) | Trivially honoured by the crate's design |
| Test coverage (AV 219-221) | **G1 golden rule** + 19 tests at scaffold + Phase O.1 integration test plan |

## How this ties into the workspace standard

- BRRTRouter landed `clippy.toml` with the same thresholds (see `../../../BRRTRouter/clippy.toml`) and distilled the rationale across three docs: `JSF_WRITEUP.md` / `JSF_AUDIT_OPINION.md` / `JSF_COMPLIANCE.md`. Those are now imported as `../references/jsf-*.md` in this repo.
- **Lifeguard** has its own coding rules in `../../../lifeguard/AGENT.md` but hasn't formally adopted JSF. When Lifeguard's next major cleanup happens, adopting the same `clippy.toml` thresholds would close the loop.
- **Hauliage** consumes both and therefore inherits the rules transitively. Hauliage's own `AGENTS.md` enumerates its service-level rules but doesn't re-state JSF — the transitive-inheritance path is implicit.

If BRRTRouter's thresholds change, this crate's `clippy.toml` should update in the same PR. The thresholds are numbers, not principles — they can drift independently, but they shouldn't drift *silently*.

## Open questions

> **Open:** BRRTRouter uses `#![deny(clippy::panic, clippy::unwrap_used, clippy::expect_used)]` at the module level in `src/router/core.rs` and `src/dispatcher/core.rs` (the hot-path modules). We use *crate-wide* `deny` instead. Trade-off: crate-wide is stricter (good for our scope — tiny surface) but module-level lets BRRTRouter keep non-hot-path modules looser. If this crate grows to have dev/test helpers that reasonably want `unwrap`, revisit.

> **Open:** BRRTRouter's `JSF_COMPLIANCE.md` cites a grep-based heuristic for catching `format!` / `to_string` / `String::from` in the hot path. We don't have a hot path in that sense, but the mindset is worth adopting for any future inner loop (e.g. a Phase O.11 spanmetrics poller).
