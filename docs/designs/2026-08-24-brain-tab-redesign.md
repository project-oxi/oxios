# Brain Tab Redesign — show what the daemon actually provides

**Date:** 2026-08-24
**Status:** implemented
**Scope:** oxios kernel `brain` module, `/api/brain/*` routes, web Brain tab

## Problem

The web Brain tab fails to show the information the oxibrain daemon actually
provides. Root causes, verified against the live daemon on this machine:

1. **Broken calls (P0).** The daemon removed the `get_entity` and `timeline`
   MCP tools (they are now MCP *resources*: `entity://`, `timeline://`).
   `BrainConnection::get_entity`/`timeline` still call the removed tools, so
   every entity/timeline lookup returns `unknown tool`. Worse,
   `BrainConnection::call` drops the whole client on any tool error, so one
   entity lookup degrades the entire brain connection.
2. **Wrong frontend types (P1).** `BrainSearchHit` models an old
   `RankingResult` shape (`{target: {kind, id}, fused_score, rank, salience}`).
   The daemon returns `[{entity_id, entity_surface, entity_type, score,
   snippet}]` (bare array). The search page reads `data.items` (always
   `undefined`) and the chat @-mention flow maps `hit.target.id` — both render
   nothing.
3. **Unexposed surfaces (P1).** The daemon provides `brief` (entity/space/topic
   markdown pages), `traverse` (belief-filtered subgraph), `review_merges`
   (merges / extraction failures / sources console data), `spaces/list`, and
   the `space://` overview (counts + recent entities). Oxios exposes none of
   them. Real extraction failures exist on this machine and are invisible.
4. **Raw JSON rendering (P1).** Entity / timeline / why / contradictions render
   as pretty-printed JSON blobs (`JsonBlock`).

## Verified daemon contracts (v0.6.x, live probe 2026-08-24)

MCP tools: `search, recall, brief, navigate, ingest, declare, why,
contradictions, stats, traverse, review_merges, remember, retract,
merge_entities, redact`.

Resources (`resources/read`, JSON in `contents[0].text`):

- `entity://{id}?space=` → `Belief[]` — `{statement, valid_from, valid_to,
  support: {affirm_count, deny_count, distinct_episodes, trust_weights},
  confidence, status}`
- `timeline://{id}?space=&from=ms&to=ms` → `TimelineEntry[]` —
  `{statement_id, predicate, object_repr, object_entity, valid_from,
  valid_to, status, recorded_at}`
- `space://{name}` → `{space, space_id, entity_count, episode_count,
  contradiction_count, recent_entities: [{id, surface, type}]}`
- `graph://{id}?depth=&space=` → `TraversalResult` (same as `traverse` tool)

Tools returning JSON text: `search` → hit array; `why` → ExplainBlock;
`contradictions` → ContradictionDetail[]; `traverse` →
`{nodes, edges, truncated}`; `review_merges {section}` → MergeRecord[] |
ExtractionFailure[] | SourceRow[]; `spaces/list` (native RPC) → SpaceSummary[].

Valid-time sentinels: `±(2^63-1)`-scale values mean "always"; frontend renders
them as `—`.

## Design

### Kernel (`crates/oxios-kernel/src/brain/mod.rs`)

- Add a private `read_resource(uri)` helper: `resources/read` via
  `BrainClient::call_rpc_json`, parse `contents[0].text` as JSON. No new
  dependency — `call_rpc_json` exists in oxibrain-client 0.6.0.
- `get_entity` / `timeline` switch to the `entity://` / `timeline://`
  resources (fixes the P0).
- New passthroughs following the existing degradation contract
  (`None`/empty when unavailable):
  `brief(target_kind, entity_id, topic) -> Option<String>` (markdown),
  `traverse(start, depth, max_nodes, direction) -> Option<Value>`,
  `review_merges(section) -> Option<Value>`,
  `spaces() -> Option<Value>`,
  `space_overview() -> Option<Value>`.
- URI building lives in a pure function (`resource_uri`) so it is
  unit-testable without a daemon.

### API (`src/api/routes/workspace.rs`)

New routes mirroring the daemon surface (all degrade to `null`/`[]`):

- `GET /api/brain/brief?target_kind=&entity_id=&topic=` → `{markdown}`
- `GET /api/brain/graph?start=<csv>&depth=&max_nodes=&direction=` →
  TraversalResult
- `GET /api/brain/operations/{section}` (merges|failures|sources) → array
- `GET /api/brain/spaces` → SpaceSummary[]
- `GET /api/brain/space` → configured space overview

Existing routes keep their paths; `/api/brain/search` keeps passing the bare
array through (contract-faithful).

### Web

- `types/brain.ts` rewritten to the daemon contracts above.
- Entity page: brief markdown as the readable page (custom link renderer:
  `entity://` links load that entity in place), beliefs table (statement,
  confidence, support, status + per-row why), timeline table (predicate,
  object, validity, recorded). Entity id arrives via `?id=` query param,
  search/overview/graph links, or manual input.
- Search page: real hit rows (surface, type badge, snippet, score) linking to
  the entity page; @-mention flow maps the same shape.
- Contradictions page: structured `ContradictionDetail` cards with subject →
  predicate → object, affirm/deny episode counts, and a why drill-down.
- Operations page (new): merges / extraction failures / sources sections.
- Overview: keeps stat cards, adds recent-entity cards (from `space://`) and
  the spaces table.
- Sidebar gains an Operations item; i18n keys added (ko/en); MSW handlers
  match the real shapes.

### Not in scope

Write/curation actions in the web UI (retract/merge/redact), space switching,
episode browsing. The daemon console (`oxibrain serve` UI) remains the curation
surface; this redesign makes Oxios a faithful *read* window.

## Verification

- Kernel unit tests for URI building + resource text parsing; existing
  degradation tests unchanged.
- `cargo fmt/clippy/test` on touched crates; web typecheck/Biome/test/build.
- Browser walk-through against the live daemon (screenshot evidence).
