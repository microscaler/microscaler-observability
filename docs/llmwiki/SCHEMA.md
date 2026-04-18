# Wiki schema and conventions

How this wiki is structured, what each page type is for, and the workflow an agent follows when ingesting new information, answering queries, or running a lint pass. If this file and any other page disagree, *this file wins* — fix the other page.

---

## Directory layout

```
docs/llmwiki/
├── README.md              # entry point — redirects to the four structural files below
├── SCHEMA.md              # this file
├── index.md               # content catalog, updated on every ingest
├── log.md                 # append-only chronological log
├── docs-catalog.md        # inventory of sibling `docs/` sources this wiki synthesises
├── topics/                # concept / subsystem explanations
├── entities/              # specific named things (types, env vars, files, services)
├── flows/                 # time-ordered process descriptions (init-flow, ingest-flow, …)
└── reference/             # condensed look-up pages (env vars, metric names, span names)
```

### `topics/`

Explanations of a *concept* or *subsystem*: "hexagonal architecture", "OTEL version pinning", "what happens when `init()` is called without an endpoint". Topics synthesise from code + the PRD + sibling-repo wikis. They answer "why" and "how".

Filename pattern: `topics/<kebab-case-slug>.md`.

### `entities/`

Pages about a *specific named thing*: a Rust type, a cargo feature flag, an env var, a deployment, a file. One entity = one page.

Filename pattern: `entities/entity-<kebab-case-slug>.md` (the `entity-` prefix is the organisation convention — consistent with Hauliage's `entities/entity-seed-order-txt.md` etc.).

### `flows/`

Time-ordered descriptions: the sequence of operations in `init()`, the sequence of operations on graceful shutdown, the sequence of operations when a span is emitted. Flows are step-by-step. They answer "what happens when".

Filename pattern: `flows/<kebab-case-slug>-flow.md`.

### `reference/`

Lookup tables: every env var the crate reads, every metric name it exposes, every span name it emits, every error variant. Dense, scannable, not prose.

Filename pattern: `reference/<kebab-case-slug>.md`.

---

## Page template

Every wiki page (except the four structural files) starts with a header block in this exact shape:

```markdown
# Page title

> **Status:** DRAFT | STABLE | OUTDATED
> **Last-synced:** YYYY-MM-DD — against source X (git SHA or PRD version).
> **Authority:** `path/to/primary/source.rs` + `docs/PRD.md §X` (or external link).
> **Related:** [`sibling-page-1`](./sibling-page-1.md), [`sibling-page-2`](./sibling-page-2.md).

## What this page covers

One paragraph. The scope. What's in and what's out.

## [content sections — free-form but consistent across siblings]

…

## Open questions

> **Open:** unresolved or contentious claim. Agents MUST flag new uncertainties here rather than silently dropping them.
```

`Status` values:

- **DRAFT** — new page, hasn't yet been validated against a second source. Prefer `DRAFT` for anything seeded from a single-session read.
- **STABLE** — synthesised from ≥ 2 sources OR validated against code, with a `Last-synced` date within 30 days.
- **OUTDATED** — known to contradict current code / PRD. Agents may still read but must not rely without re-checking.

`Authority` values:

- The single source-of-truth the page distils. If the authority changes, the page gets re-synced.

---

## log.md entry format

Strict — lets `grep "^## \[" log.md | tail -N` give a clean tail-of-history view:

```markdown
## [YYYY-MM-DD] <event-type> | <one-line summary>

Optional multi-line notes describing what was touched, what was synthesised, what questions emerged.
Touched pages go as a bulleted list:
- `topics/foo.md` — updated §openquestions
- `entities/entity-bar.md` — created
```

`<event-type>` is one of: **`ingest`** (new source integrated), **`query`** (question answered, possibly with a new page filed), **`lint`** (health-check pass — contradictions found, stale claims revised, orphans deleted), **`scaffold`** (structural change to the wiki itself), **`phase-O.x`** (wiki work tied to a specific PRD phase landing).

---

## Agent workflow

### Ingest

A new source arrives (a new piece of code, a new PRD section, a postmortem, a sibling-repo change). Steps:

1. Identify which existing wiki pages are affected. Open `index.md` to locate them.
2. Read the source in full. Extract the key claims.
3. For each affected page: decide update vs rewrite vs split. Updates preserve the `Last-synced` date only if the source matches the existing claims; rewrites bump it.
4. If no page fits: create a new `topics/` / `entities/` / `flows/` / `reference/` page using the template above. Status `DRAFT`.
5. Update `index.md` — add any new pages, bump `Last-synced` on anything you edited.
6. Append a `log.md` entry describing the ingest and the touched pages.
7. If you notice a contradiction between sources, add a `> **Open:**` note on the relevant page rather than silently picking one.

### Query

The user asks a question. Steps:

1. Read `index.md` to locate candidate pages.
2. Read the candidate pages — don't go directly to source unless the wiki is stale or silent.
3. If the wiki is silent on the exact question: drill into sources, then write the finding back as a new page (or a new section on an existing page).
4. Append a `log.md` entry with event-type `query` and a one-line summary.
5. If the answer revealed a gap or an inconsistency, flag it as `> **Open:**`.

### Lint

Once a week (or on explicit request):

1. Read every page. Look for: contradictions across pages, `Last-synced` dates older than 30 days, orphan pages with no inbound links, stale claims that newer phases have superseded.
2. Run `rg "Last-synced:" docs/llmwiki` and sort by date — oldest first.
3. Run `rg "> \*\*Open:\*\*" docs/llmwiki` to see accumulated questions.
4. Fix what you can; surface what you can't as a `log.md` entry describing remaining debt.

### Scaffold

Structural changes to the wiki itself (new directory, new page template, schema amendment). Always log this.

---

## Cross-repo wiki citation style

When a page in this wiki cites a page in a sibling repo's wiki:

```markdown
See [BRRTRouter `llmwiki/topics/router-match-radix.md`](../../../BRRTRouter/llmwiki/topics/router-match-radix.md).
See [Lifeguard `docs/llmwiki/topics/brrtrouter-integration-pitfalls.md`](../../../lifeguard/docs/llmwiki/topics/brrtrouter-integration-pitfalls.md).
```

Relative paths assume a standard `microscaler/` sibling checkout. If a wiki's location moves, cross-references get a `log.md` lint pass.

---

## Things this wiki is not

- **Not a source of truth for code.** Source is `src/*.rs`. If the wiki and the code disagree, the code wins — but the wiki gets updated in the same commit that changed the code.
- **Not a PRD.** The PRD (`../PRD.md`) is forward-looking; the wiki synthesises what exists today. Planned work belongs in the PRD; realised work belongs in the wiki.
- **Not a chat log.** Session transcripts are ephemeral; wiki pages are curated. If a session produces a useful insight, that insight goes on a wiki page, not just into `log.md`.
- **Not a documentation site.** Public rustdoc goes in `///` / `//!` comments, published via `cargo doc`. The wiki is the internal cross-cutting layer above the rustdoc.
