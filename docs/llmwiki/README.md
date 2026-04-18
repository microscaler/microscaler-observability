# microscaler-observability wiki

Living LLM-maintained knowledge base for the `microscaler-observability` crate. Follows the pattern from [Karpathy's LLM Wiki gist](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f).

## Start here (always)

1. [`SCHEMA.md`](./SCHEMA.md) — directory conventions, page templates, agent workflow.
2. [`index.md`](./index.md) — full content catalog.
3. [`log.md`](./log.md) — chronological history of wiki edits and session activity.
4. [`docs-catalog.md`](./docs-catalog.md) — inventory of every `docs/*.md` source this wiki cross-references.

## Why this wiki exists

Each session's knowledge is otherwise lost when the agent context ends. The wiki accumulates synthesized, cross-linked summaries of what earlier sessions learned — so the next agent doesn't rediscover the same facts from source every time. Raw sources (Rust source, `docs/PRD.md`, sibling-repo code) stay as-is; the wiki is the *compiled-and-kept-current* layer that sits between them and whoever's reading.

## Repo status check

This crate is **v0.0.1 scaffold**. `init()` deliberately panics; real implementation lands with Phase O.1 of [`../PRD.md`](../PRD.md). The wiki is seeded with the concepts that are *already true today* (hexagonal role, version pinning, what `ShutdownGuard` will be) plus placeholders for concepts that phases will add (OTLP exporter flow, subscriber composition, Pyroscope wiring).

Cross-repo reminders:

- BRRTRouter wiki: [`../../BRRTRouter/llmwiki/`](../../../BRRTRouter/llmwiki/).
- Lifeguard wiki: [`../../lifeguard/docs/llmwiki/`](../../../lifeguard/docs/llmwiki/).
- Hauliage wiki: [`../../hauliage/docs/llmwiki/`](../../../hauliage/docs/llmwiki/).

See [`topics/sibling-repos-and-wikis.md`](./topics/sibling-repos-and-wikis.md) for the responsibility split.
