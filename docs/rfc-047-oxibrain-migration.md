# RFC-047: Oxibrain Migration — Retire `oxios-memory`, Route Through the Brain

> **Status**: Proposed (oxibrain-side M5 complete; oxios-side work remains)
> **Supersedes**: RFC-018 (oxios-memory extraction) — kept as historical record
> **Author**: oxibrain × oxios handoff, 2026-08-13
> **Depends on**: `oxibrain` crate (Consumption Contract 1.0, `doc/CONSUMPTION_CONTRACT.md`);
>                  `oxibrain` CLI ≥ `import-oxios` subcommand;
>                  `oxios-memory` importer (`crates/oxibrain-connectors/src/oxios.rs`)
> **Authority**: `oxibrain::doc::DESIGN.md` §16.3 is canonical for the migration
>                timeline; this RFC is canonical for the *oxios-side* work.

---

## 1. Summary

Retire `oxios-memory` and route all agent memory through the standalone
`oxibrain` crate. This is the oxios-side counterpart to oxibrain's M5
deliverables: the consumption contract (§16.4), the `import-oxios` importer
(§16.3), and the ADR-002 C1 fallback decision (§16.1).

The migration is gated by a single trigger from `oxibrain::doc::DESIGN.md`
§16.3: **the last `oxios_memory::` import removed from the oxios workspace.**
Until that line is gone, both substrates coexist and every kernel write to
`oxios-memory` is a future port. After it, the crate is removed from the
workspace in the same PR and marked deprecated on crates.io (not yanked).

---

## 2. Motivation

`oxios-memory` is 18,075 LOC of substrate that oxibrain has either
superseded, deferred, or explicitly declined to adopt (see `DESIGN.md` §16.2
for the full triage). Carrying it forward has three concrete costs:

1. **Two brains.** With `oxios-memory` live, agents write both to the
   `~/.oxios/workspace/memory.db` SQLite file (legacy) and to the
   `oxibrain`-managed table files (new). The split is invisible to the user
   and unrecoverable from the kernel because the two writers have independent
   in-memory indexes (`HnswMemoryIndex` in `oxios-memory`, HNSW-in-WAL
   `sqlite-vec` in `oxibrain-index`).
2. **Schema divergence.** `oxios-memory` ships weekly without a migration
   contract (DESIGN §16.2 / D2). Every extraction is keyed by content hash
   today; every rewrite of the cache layout is a corruption path for any
   downstream kernel that holds a `MemoryDatabase` connection.
3. **C1 contract drift.** ECOSYSTEM §C1 says the brain is additive, never
   load-bearing. With `oxios-memory` in the kernel, agents *do* have
   memory when the brain is down — but it is the wrong memory: not
   temporal, not bi-temporal, not assertion-shaped. Keeping it makes the
   "small local recall fallback" question (DESIGN §16.1, ADR-002) a
   permanent footnote instead of a resolved decision.

The deletion is the one thing that makes the rest of the ecosystem
decompositions (oximemo absorbing authoring, oxiline absorbing time
features) tractable. Until agents have a single memory owner, the
decomposition cannot converge.

---

## 3. Scope

### 3.1 In scope

- All `oxios_memory::` references in the `oxios` workspace.
- `oxios-kernel/Cargo.toml` dependency on `oxios-memory` (path dep,
  optional `sqlite-memory` feature).
- `Cargo.toml` workspace member list at the root.
- Surface parity between the **legacy** and **brain** APIs (binary
  crates, web routes, CLI panels that surface memory).
- One-time `oxibrain import-oxios` migration run against existing stores.
- `oxios-memory` 1.39.0 release on crates.io marked `deprecated`.

### 3.2 Out of scope (do not move)

- `oxios-memory::memory::sona` (trajectory learning) — explicit
  `oxios`-resident decision (`DESIGN.md` §16.2). The runtime port is the
  right home.
- `oxios-memory::memory::auto_bridge`, `auto_classify`, `auto_protect`
  — agent-runtime glue (`DESIGN.md` §16.2). They need a new home inside
  `oxios-kernel` (see §5.4 below).
- `oxios-memory::memory::hyperbolic`, `flash_attention`, `embedding_viz`
  — `DESIGN.md` §16.2 deferred; not v1 path. They remain in a
  long-lived `oxios-memory` 1.39.x branch only if needed for parity.
- `oxios-markdown` dissolution (ECOSYSTEM §3.3) — separate migration,
  not this RFC.
- `oxios-mcp` — outbound MCP client; oxios keeps it. oxibrain ships its
  own server (`oxibrain-mcp`) — that is a *new* crate, not a port.

### 3.3 Retirement trigger

> **The last `oxios_memory::` import removed from the oxios workspace.**
> Not a date. (DESIGN §16.3)

The PR that removes that import also:

- Removes `crates/oxios-memory` from the root `Cargo.toml` workspace list.
- Removes `oxios-memory` from `oxios-kernel/Cargo.toml`.
- Removes `oxios-memory` from `[dependencies]` in any other consumer
  (none today, but verify).
- Adds a `## [Unreleased]` CHANGELOG entry marking the removal.
- Bumps oxios to `1.40.0` (semver-major because the public API changes
  — kernel re-exports disappear).

A separate GitHub PR, performed by the release engineer, marks
`oxios-memory` 1.39.0 on crates.io as deprecated. **Never yanked.** The
crates.io entry stays until the next major release that drops the
`oxios` 1.39.x compatibility shim, then ages out via the usual
six-month crates.io retirement policy.

---

## 4. What oxibrain already provides (the surface oxios consumes)

The stable facade is pinned in `oxibrain::doc::CONSUMPTION_CONTRACT.md`
1.0. The relevant methods for oxios:

| oxibrain API | Replaces |
|---|---|
| `Brain::open(config)` / `Brain::with_llm(config, clock, llm)` | `oxios_memory::MemoryManager::new` + `SqliteMemoryStore` open |
| `Brain::ingest(space, content, source, trust, extractor_id)` | `MemoryStorage::store` for agent traces |
| `Brain::query(query)` (hybrid / lexical / semantic) | `MemoryStorage::search` / `SqliteMemoryStore::hybrid_query` |
| `Brain::assemble_context(space, query, budget)` | `ProactiveRecall::assemble` (the per-turn call) |
| `Brain::traverse(space, spec)` | `MemoryGraph::neighbors` (subgraph neighbor lookup) |
| `Brain::beliefs(space, entity_id)` | `MemoryEntry::beliefs` (current-slice beliefs) |
| `Brain::reproject()` | `oxios_memory::dream::DreamProcess` (full rewrite) |
| `Brain::consolidate(space, config)` | `DreamProcess::consolidate_step` (incremental) |
| `Brain::apply_decay(space)` | `DecayEngine::decay` |
| `Brain::compact(space)` | `CompactionTree::compact` |
| `Brain::timeline(space, entity_id, from, to)` | (new — no legacy equivalent) |
| `Brain::diff(space, entity_id, at_a, at_b)` | (new — audit primitive) |
| `Brain::why(space, statement_id)` | `MemoryStorage::explain` |
| `Brain::issue_token / verify_token / revoke_token` | (Capability-gated MCP access) |
| `Brain::export_jsonl` / `Brain::import_jsonl` | (backup + migration) |

Six of these are *new* capabilities oxios never had — timeline, diff,
taxonomy, communities, RR answer provenance, episodic search. They
arrive for free because the predicate-registry model and the
bi-temporal fold were never features of `oxios-memory`.

---

## 5. The migration plan

### 5.1 Add `oxibrain` to `oxios-kernel`

```toml
# crates/oxios-kernel/Cargo.toml
[dependencies]
oxibrain = { version = "0.1", default-features = false, features = ["http-llm"] }
```

`oxibrain` does not require any other oxi crate (`DESIGN.md` §16.4
standalone guarantee). The default-disabled feature list keeps the
default `cargo build -p oxios-kernel` standalone-tolerant.

The `oxios-memory` dependency stays in `Cargo.toml` until the last
kernel import is removed (§3.3). New code writes only against
`oxibrain::*`.

### 5.2 Subsystem-by-subsystem port

The mapping is straight-line from `DESIGN.md` §16.2:

| `oxios-memory` module | New home | Notes |
|---|---|---|
| `embedding{,_cache}` + `hnsw{,_memory_index}` + `chunking` + `normalizer` | `oxibrain-index` (already shipped) | behind `EmbeddingPort` |
| `sqlite/{database, store, search/*}` | `oxibrain-store` (new schema, new migrations) | D2 chose isolation |
| `decay` | `Brain::apply_decay` | salience signal only |
| `compaction` | `Brain::compact` | deterministic |
| `dream` | `Brain::consolidate` + `Brain::summarize_communities` | D5: derived episodes |
| `proactive` | `Brain::assemble_context` + `RecallHints` (L sub-project) | per-turn call |
| `graph` (PageRank) | internal salience signal | not a knowledge graph |
| `root_index` | `Brain::snapshot_indexes` | |
| `quota` | inference budget enforcement on `LlmPort` | not a memory budget |
| `types` (`MemoryEntry`, tiers) | replaced by `Episode` + `Belief` | vocabulary wins |
| `MemoryStorage` trait + `StateStore` impl | replaced by `oxibrain_client::Brain` direct | trait is gone |
| `memory::storage::MemoryGit` | not adopted — `GitLayer` stays in `oxios-kernel` for the workspace journal | |
| `sona` | **stays in oxios** (see §3.2) | runtime concern |
| `auto_bridge`, `auto_classify`, `auto_protect` | **stays in oxios**, moves under `oxios-kernel::memory_agent::` (§5.4) | runtime glue |
| `hyperbolic`, `flash_attention`, `embedding_viz` | not adopted (`DESIGN.md` §16.2) | removed from runtime callsites |

### 5.3 Kernel code changes

The `oxios-kernel` crate has 12 files that touch `oxios_memory::*` today
(grep at time of writing):

```
crates/oxios-kernel/src/agent_runtime.rs     — `oxios_memory::memory::sona`
crates/oxios-kernel/src/embedding.rs         — `oxios_memory::memory::embedding::*`
crates/oxios-kernel/src/lib.rs               — large re-export block
crates/oxios-kernel/src/memory/mod.rs        — trait bridges + re-exports
crates/oxios-kernel/src/memory/markdown_bridge.rs — `MarkdownSource` impl
crates/oxios-kernel/src/memory/auto_memory_bridge.rs — `auto_bridge` re-export
crates/oxios-kernel/src/metrics.rs           — uses `MemoryEntry`
crates/oxios-kernel/src/mount/manager.rs     — `MemoryDatabase`
crates/oxios-kernel/src/orchestrator.rs      — `MemoryDatabase`
crates/oxios-kernel/src/persistence_hook.rs  — `TrajectoryStep`
crates/oxios-kernel/src/project/manager.rs   — `MemoryDatabase`
```

The porting order:

1. **`lib.rs` re-exports** — keep the old `pub use oxios_memory::*` block
   intact; do not yank it. The kernel can hold both surfaces for the
   duration of the migration (`oxios_memory::*` for legacy callers,
   `oxibrain::*` for new code). This is the explicit "cutover, not
   fork" rule (`DESIGN.md` §16.3).
2. **`embedding.rs`** — replace the `EmbeddingEngine` direct embedding
   with `oxibrain::EmbeddingPort` adapter. The adapter is feature-gated
   behind `oxibrain/embedding-gguf` (or whatever the upstream feature
   name is when 0.1 publishes).
3. **`memory/mod.rs`** — the `MemoryStorage-for-StateStore` trait impl
   stays because it is the *kernel*-side storage layer that oxibrain
   reads through. The `DreamConfig<F: From>` bridge becomes a
   deref/drop — `Brain::consolidate` takes its own config struct.
4. **`memory/markdown_bridge.rs`** — drop. `oxibrain` ships its own
   markdown vault connector (`oxibrain-connectors/src/markdown.rs`)
   that reads through `MarkdownSource` (already implemented in
   `oxios-kernel`). The connector is the substitute; the bridge is the
   glue oxibrain no longer needs.
5. **`memory/auto_memory_bridge.rs`** — keep as a thin re-export of
   `auto_bridge`, but the underlying module moves to
   `oxios-kernel::memory_agent::auto_bridge` (see §5.4).
6. **`agent_runtime.rs`** — the `TrajectoryStep` and `Trajectory` types
   stay because `sona` stays. No change to the storage path; only the
   *memory* half of the trajectory (the `MemoryEstimate` field) goes
   away — agents record trajectories without episodic memory, the
   brain reads them as `Conversation` episodes if they hit the brain.
7. **`mount/manager.rs`, `orchestrator.rs`, `project/manager.rs`** —
   `MemoryDatabase` references are index lookups for the kernel-side
   HNSW bootstrap. Once `embedding.rs` is converted, the
   `MemoryDatabase::open` calls are replaced by the `Brain::open`
   path; the migration goes through `oxibrain-index` exclusively.
8. **`persistence_hook.rs`** — `TrajectoryStep` stays; no storage change.
9. **`metrics.rs`** — `MemoryEntry` references are for the legacy
   event payload. Replace with `oxibrain::Episode` (or drop the metric
   if it is purely a tally of legacy activity).

New file: `crates/oxios-kernel/src/memory_agent/mod.rs` (see §5.4).

### 5.4 New submodule: `oxios-kernel::memory_agent`

Holds the `oxios`-resident pieces per §3.2:

```
crates/oxios-kernel/src/memory_agent/
├── mod.rs              # module structure + re-exports
├── auto_bridge.rs      # moved from oxios_memory::memory::auto_bridge
├── auto_classify.rs    # moved from oxios_memory::memory::auto_classify
├── auto_protect.rs     # moved from oxios_memory::memory::auto_protect
└── sona.rs             # moved from oxios_memory::memory::sona
```

These move verbatim. They do not depend on `oxios_memory::*` themselves
once moved; they were *consumers* of memory, not the memory substrate.
The kernel gains ~3,000 LOC; the workspace loses 18,075 LOC; net
contraction is ~15,000 LOC.

### 5.5 One-time data migration

For existing users with `~/.oxios/workspace/memory.db` populated:

```bash
# 1. Stop oxios daemon.
oxios service stop

# 2. Confirm the source file exists.
ls -lh ~/.oxios/workspace/memory.db

# 3. Import into the oxibrain store. Idempotent.
oxibrain init --at ~/.oxi/brain
oxibrain import-oxios \
    --source ~/.oxios/workspace/memory.db \
    --space personal \

# 4. Re-extract the imported episodes (the importer writes raw
#    episodes, not derived beliefs).
oxibrain reextract --space personal

# 5. Smoke test — a query that previously hit the legacy store
#    should now return from the brain.
oxibrain ask "what did I work on last week?" --space personal

# 6. Start the new daemon.
oxibrain serve --daemon --http 127.0.0.1:18080
oxios service start
```

The `oxibrain import-oxios` subcommand is a *forward* migration: it
preserves the original `memory.db` and never writes back to it. Users
inspect the brain, retire the legacy file on their own schedule, and
delete the SQLite file once the new daemon has been running for a
billing cycle with no missing-data reports.

Inside the importer (`oxibrain-connectors/src/oxios.rs`):

- Each `MemoryEntry` becomes an `Episode` with `kind = Primary`,
  `source = SourceRef::AgentTrace`, `trust = SemiTrusted` (per
  `DESIGN.md` §16.3).
- Original creation date is prepended to the content so temporal
  extraction has the timestamp on the page.
- Entries already in `Conversation` episodes (i.e. ingested via the
  current `oxios-memory` bridge) are deduplicated by content hash and
  ingested exactly once.

### 5.6 CLI surface changes

The `oxios` CLI gains one new command and keeps the legacy memory
subcommand under a deprecation banner for one release.

```bash
# New (oxios ≥ 1.40):
oxios brain status                    # offline | degraded | online
oxios brain ingest <path|->           # writes an episode via Brain::ingest
oxios brain ask "<query>"             # delegates to Brain::assemble_context
oxios brain watch                     # opens the brain daemon if down
```

```bash
# Deprecated (oxios ≤ 1.39, removed in 1.40):
oxios memory recall   →  oxios brain ask
oxios memory dream    →  oxios brain consolidate (called by scheduler, not user)
oxios memory stats    →  oxios brain status
```

The deprecation banner is a one-line stderr warning, not a hard error,
until the 1.40 release removes the old subcommands.

### 5.7 Web surface

The web panel `MemoryMap` (which renders `MemoryMapEntry` from
`oxios-memory::embedding_viz`) is replaced by a redirect to the
`oxibrain` desktop UI's Graph Explorer (`apps/brain-ui/` in the
oxibrain repo). The `OxiosEngine` reports the brain URL once it is
running; the panel shows a "view in brain" link. The deprecated
`MemoryMap` panel ships behind the `legacy-memory` feature flag for
one release, then is removed.

### 5.8 Tokens, scopes, and the C1 contract

Once the kernel talks to oxibrain for memory, the kernel acts as an
oxibrain *consumer*. It needs:

- An oxibrain token issued with `Scope { spaces: ["personal"], caps: [Read, Write, Ingest] }` (no `Sample` — the kernel has its own LLM port via `oxicode-sdk`).
- Token issued at first boot, stored in `~/.oxios/workspace/oxibrain-token`.
- Rotation follows the `oxibrain::token::revoke` + `issue` cycle; the
  kernel re-issues on startup if the token is missing.

The `Sample` capability is **not** granted to the kernel by default.
`C1` (`ECOSYSTEM.md` §2) holds because the daemon can be down and the
kernel still functions — it just runs without episodic memory, exactly
as the oxios design intent (ADR-002).

---

## 6. Verification

The migration is not "done" when the last import is gone. It is done
when the following hold:

1. **`cargo test -p oxios --workspace`** passes with `oxios-memory`
   removed from the workspace (no path dep, no member entry).
2. **`cargo test -p oxios --workspace --all-features`** passes.
3. **Workspace size** shrinks by ≥ 14,000 LOC (the 18,075 minus the
   ~3,000 that moves to `memory_agent`).
4. **Migration round-trip**: a seeded `~/.oxios/workspace/memory.db`
   fixture imports into oxibrain, `reextract` produces a non-empty
   graph, and `oxibrain ask` returns the same primary entities that
   the legacy `MemoryStorage::search` would have.
5. **Degradation test**: with the oxibrain daemon stopped, `oxios
   service status` reports `degraded`, agent turns still complete
   (no hang), and the failure appears in the audit log with a
   `BrainError::Storage` marker.
6. **C1 smoke test**: with the daemon stopped, the `oxios web` UI
   shows the memory panel as unavailable, not as a spinner.
7. **Benchmark parity**: `Brain::assemble_context` p95 on a 1K-episode
   fixture is within 10% of the legacy `ProactiveRecall::assemble`
   timing on the same fixture. (Acceptance: not slower than the old
   code by more than 10%. The new code is expected to be faster on
   larger corpora because the ledger/projection split is cheaper than
   the legacy tier walk.)
8. **Eval parity**: the `oxibrain eval --suite fast` benchmark suite
   (LongMemEval, LoCoMo) runs against the imported corpus and reports
   within the §14.2 regression gate of the prior measurement.

---

## 7. Risks and mitigations

| Risk | Mitigation |
|---|---|
| `MemoryDatabase` opens in 4 kernel files have hidden dependencies | Phase 1: write a libtest that opens a fixture `memory.db` and snapshots every `MemoryDatabase::xxx` access; port one file at a time. |
| `DreamProcess` semantics diverge from `Brain::consolidate` | The `MRI(↓)` statement (§5.1) keeps the old bridge alive for one release; users running `oxios memory dream` continue to see the old output while the new path is shadowed. |
| Token secret leaks to disk | `~/.oxios/workspace/oxibrain-token` mode 0600, generated by `getrandom` (the same fix applied to oxibrain tokens in commit `db78ad9`). |
| Sona movement costs more than 3,000 LOC | If the cutover runs long, ship `oxios-memory` 1.39.x with the `auto_bridge`/`auto_classify`/`auto_protect` modules deleted but `sona` intact, then delete the whole crate in a later release. |
| Crates.io deprecation PR is gated by an unrelated owner | Mark `oxios-memory` as `unmaintained` instead of `deprecated` if the crate owner is no longer reachable; the effect on Cargo is the same. |
| The kernel becomes silently slower | §6.7 benchmark gate. If the new path is more than 10% slower, abort the migration and revisit the `DECAY`/`HNSW` choices in oxibrain-index. |

---

## 8. Sequencing

The migration is one PR per slice, not one mega-PR. Each slice
compiles, tests, and ships independently:

1. **Slice 0 — dependencies.** Add `oxibrain` to `oxios-kernel/Cargo.toml`.
   Add `oxibrain` workspace dep. No code changes. Verify `cargo
   build -p oxios-kernel`.
2. **Slice 1 — `memory_agent` module.** Create `oxios-kernel::memory_agent`
   and move `auto_bridge`, `auto_classify`, `auto_protect`, `sona`
   verbatim. Update `oxios-kernel::memory::mod.rs` to re-export from
   the new location. `oxios-memory` is still in the workspace.
3. **Slice 2 — `embedding.rs`.** Replace `EmbeddingEngine` with
   `oxibrain::EmbeddingPort`. Verify `MemoryMapEntry` consumers
   compile.
4. **Slice 3 — `mount`, `orchestrator`, `project` managers.** Replace
   `MemoryDatabase::open` with `Brain::open`. Verify boot path.
5. **Slice 4 — `metrics.rs` and `persistence_hook.rs`.** Drop
   `MemoryEntry` references; replace with `Episode` where the data
   still exists, drop where it does not.
6. **Slice 5 — `markdown_bridge.rs`.** Delete. `oxibrain-connectors`
   takes over.
7. **Slice 6 — `oxios` binary CLI.** Add `oxios brain *` subcommands.
   Mark `oxios memory *` as deprecated.
8. **Slice 7 — `web` panel.** Replace `MemoryMap` with a redirect to
   the brain UI. Remove panel under `legacy-memory` feature.
9. **Slice 8 — `lib.rs` re-exports.** Delete the `pub use oxios_memory::*`
   block. Verify `cargo build -p oxios` succeeds.
10. **Slice 9 — `Cargo.toml` workspace.** Remove
    `crates/oxios-memory` from members. Remove `oxios-memory` from
    `oxios-kernel/Cargo.toml`. **This is the trigger PR.**
11. **Slice 10 — release.** `oxios` 1.40.0. Crates.io PR to mark
    `oxios-memory` 1.39.0 deprecated.

Slices 0–9 land on `main` as individual PRs. Slice 10 is the release
engineer's job and is gated by all prior slices passing CI.

---

## 9. Open questions

1. **Sona home.** `oxios-kernel::memory_agent::sona` is the natural
   home, but `sona` is a runtime pattern-learning engine with a
   different test surface from the memory manager. Should it move
   under `oxios-kernel::runtime::sona` instead? Default: leave the
   move under `memory_agent` for cohesion, revisit in a later RFC.
2. **Auto-protect semantics.** `auto_protect` decides which memories
   survive eviction. Once eviction is a salience signal in oxibrain,
   does `auto_protect` still have a job? Default: yes — it decides
   *which* signal, not whether one exists. The kernel still owns the
   policy; the brain owns the storage.
3. **Workspace journal.** `~/.oxios/workspace/journal.md` is a
   markdown file the kernel writes, not the brain. After this RFC,
   the kernel should *tell* the brain about journal edits via
   `Brain::ingest_note`. The journal itself stays in
   `oxios-kernel::journal` (the user-facing file); the
   `MemoryGit-for-GitLayer` impl can be deleted.

---

## 10. Decision log

**D1 — `oxibrain` is a standalone dependency, not a path dep.** Per
`DESIGN.md` §16.4 and the oxibrain `AGENTS.md` standalone guarantee,
the `oxibrain` crate ships with no `oxios-*` dependency. Path-depending
it would create a circular import (oxios importing oxibrain
importing oxios-*). The crates.io version pins v0.1 with the
`http-llm` feature for default builds.

**D2 — Coexistence, not yank.** `oxios-memory` 1.39.0 stays on
crates.io as deprecated. Yanking would break anyone who has pinned
1.39.x directly. Deprecation is the polite exit.

**D3 — No re-export shim.** `DESIGN.md` §16.3 forbids "no shims, no
dual maintenance, no renaming a published crate into a facade over
another repo." The kernel's `pub use oxios_memory::*` block in
`lib.rs` is removed in Slice 9, not aliased. Breaking change, semver
1.39 → 1.40.

**D4 — One token per kernel process.** The kernel issues one
oxibrain token at startup and reuses it. Per-episode tokens would
inflate the audit log and add roundtrip latency per ingestion.

**D5 — `Sample` capability is opt-in, not default.** The kernel has
its own LLM access via `oxicode-sdk`. Granting `Sample` would route
episode content through the user's MCP client model, which is a
privacy surprise (§12.3 of DESIGN.md). The kernel does not need
sampling; it does its own extraction.

---

## 11. References

- `oxibrain::doc::DESIGN.md` §16 — Relationship to the oxi ecosystem
- `oxibrain::doc::DESIGN.md` §16.3 — Migration (the upstream plan)
- `oxibrain::doc::DESIGN.md` §16.2 — Substrate triage (the per-module disposition)
- `oxibrain::doc::CONSUMPTION_CONTRACT.md` 1.0 — Stable public surface
- `oxibrain::doc::adr/ADR-002-c1-fallback-decision.md` — C1 fallback decision
- `oxibrain::docs/superpowers/handoffs/2026-08-12-m5-oxios-migration.md` —
  oxibrain-side M5 deliverables (already shipped)
- `oxios::docs/rfc-018-memory-consolidation.md` — predecessor (kept for history)
- `oxios::docs/ARCHITECTURE.md` §3.4 — AgentRuntime and the memory call site
- `oxios::AGENTS.md` — oxios conventions

---

End of RFC. Decision required: §3.3 retirement trigger wording (already
canonical via oxibrain DESIGN §16.3), §5.4 placement of `memory_agent`,
§9 open questions.
