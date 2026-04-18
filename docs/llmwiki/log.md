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
