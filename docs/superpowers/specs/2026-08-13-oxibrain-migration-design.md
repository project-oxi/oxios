# Oxibrain Migration Design — oxios-side Big-Bang Cutover

> **Date**: 2026-08-13
> **RFC**: RFC-047 (canonical for scope/timeline; this spec is canonical for implementation)
> **Approach**: Big-bang — single PR removes `oxios-memory` and wires `oxibrain` daemon
> **Version**: oxios 1.39.0 -> 1.40.0 (semver-major)

---

## 1. Goal

Retire `oxios-memory` (~13,600 LOC) in a single PR. Replace all agent memory with the standalone `oxibrain` daemon accessed via Unix-domain socket. The brain is a shared system service (oxios, oximemo, oxiline all consume it), managed independently — oxios connects at startup and degrades gracefully when unavailable.

## 2. RFC-047 Discrepancies (codebase-verified)

The RFC was written against assumptions that do not match the actual codebase. Six corrections:

| # | RFC claim | Actual state | Impact on this design |
|---|---|---|---|
| 1 | oxios-memory = 18,075 LOC | ~13,600 LOC (measured) | LOC reduction ~12.6K net |
| 2 | CLI has `oxios memory recall/dream/stats` to deprecate | No memory CLI commands exist | CLI is purely additive |
| 3 | Web = MemoryMap panel only | 6 tabs, 10 hooks, 12 route handlers (~2000 LOC) | Full web rebuild |
| 4 | MemoryApi facade consumed | Dead code — web routes bypass it, call AgentApi directly | Delete facade |
| 5 | oxibrain on crates.io v0.1 | Not published (path deps only) | Prerequisite blocker |
| 6 | Transport = HTTP | `oxibrain-client` uses Unix-domain socket JSON-RPC | Transport is Unix socket |
| 7 | memory_agent modules move "verbatim" | All 4 have hard `oxios_memory::*` type dependencies | Refactor/drop, not verbatim |

## 3. Architecture

```
oxios daemon (single process)
  Kernel
    brain/              [NEW] daemon connection + degradation
      mod.rs            BrainConnection (connect, reconnect, health)
      config.rs         BrainConfig (socket_path, space)
    memory_agent/       [NEW] runtime glue
      sona.rs           (~630 LOC, EmbeddingProvider trait rehomed)
    kernel_db.rs        [NEW] SQLite connection for mount/project tables
    embedding.rs        [MODIFIED] TF-IDF provider + traits moved from oxios-memory
    (memory/ deleted entirely)
  Binary (src/)
    kernel.rs           [MODIFIED] boot path rewired to BrainConnection
    api/routes/         [MODIFIED] /api/brain/* replaces /api/memory/*
    cli.rs              [MODIFIED] brain subcommand added
  Web (web/)
    brain panel         [REBUILT] brain-native data model

         | Unix-domain socket (JSON-RPC)
         v
  oxibrain daemon (independent process, shared service)
```

### Integration topology: Daemon (BrainClient)

- oxibrain runs as a separate daemon process (shared by oxios, oximemo, oxiline).
- oxios connects via Unix-domain socket at startup.
- Connection failure -> degraded mode (agent turns complete normally, memory unavailable).
- No token auth initially (Unix socket = local-only, filesystem permissions suffice).

## 4. Design decisions

### D1 — memory_agent = sona only

RFC section 5.4 claims `auto_bridge`, `auto_classify`, `auto_protect`, `sona` move verbatim. Verified false:

| Module | LOC | Depends on | Disposition |
|---|---|---|---|
| **sona** | 630 | `EmbeddingProvider`, `EmbeddingVector` traits only | **Move** + rehome traits to `embedding.rs` (~30 LOC change) |
| **auto_bridge** | 1003 | `MemoryManager`, `MemoryEntry`, `MemoryType`, `MarkdownSource` | **Drop** as stateful struct. Decompose into: `BrainConnection::ingest_note()` for import, CLI `brain export` for export |
| **auto_classify** | 341 | `MemoryType` enum (deleted in same PR) | **Drop** — brain extraction pipeline replaces it |
| **auto_protect** | 384 | `MemoryEntry`, `ProtectionLevel` (deleted) | **Drop** — brain salience/decay replaces it |

Rationale: the brain already performs extraction, classification, and salience-based decay internally. Keeping kernel-side duplicates would violate single-owner principle. sona is the only module the brain does not replace (trajectory pattern learning is a runtime concern).

### D2 — KernelDatabase extracted from MemoryDatabase

`MemoryDatabase` is the SQLite backend for mount and project tables, not just memory. It cannot be deleted wholesale. Extract:

```
KernelDatabase (~60 LOC):
  - open(path) / open_in_memory()
  - WAL mode, foreign keys
  - conn() accessor (MutexGuard<Connection>)
  - NO memory schema, NO sqlite-vec
```

Changes: `sqlite-vec` removed from kernel. `rusqlite` becomes non-optional. `sqlite-memory` feature deleted. MountManager and ProjectManager switch from `Arc<MemoryDatabase>` to `Arc<KernelDatabase>`.

### D3 — Big-bang cutover

Single PR. No coexistence period, no adapter shim, no dual-surface. Justified by:
- RFC D3 forbids re-export shims.
- The web rebuild is large enough that a half-migrated state is worse than a clean break.
- `cargo test --workspace` gates the merge.

### D4 — BrainConnection degradation contract

All brain operations return `None` or empty when the daemon is unavailable. No error propagation to callers. Agent turns complete normally. The kernel logs a warning at degraded entry and at each reconnection attempt.

### D5 — BrainApi replaces AgentApi memory methods

AgentApi loses all memory methods (`memory_stats`, `list_all_memories`, `get_memory`, `search_memory`, `semantic_search_memory`, `remember`, `forget_memory`, `set_memory_pinned`, `rebuild_hnsw_index`). These are replaced by `KernelHandle::brain: BrainApi`.

The existing `MemoryApi` facade (dead code — never consumed) is deleted entirely.

## 5. BrainConnection API

```rust
pub struct BrainConnection {
    client: tokio::sync::Mutex<Option<BrainClient>>,  // &mut self methods require exclusive access
    config: BrainConfig,
}

pub struct BrainConfig {
    socket_path: PathBuf,
    space: String,
}
```

Agent-runtime methods (typed Rust returns):
- `recall(query, budget) -> Option<String>` — assembled context text
- `remember(content, source) -> Option<String>` — episode id

Web API methods (JSON passthrough):
- `search(query, mode, limit) -> Option<Value>`
- `get_entity(entity_id) -> Option<Value>`
- `timeline(entity_id, from, to) -> Option<Value>`
- `why(statement_id) -> Option<Value>`
- `contradictions() -> Option<Value>`
- `stats() -> Option<Value>`

Status:
- `is_available() -> bool`
- `reconnect() -> bool`

Brain MCP tool mapping:
- `recall` -> MCP `recall` tool
- `remember` -> MCP `remember` tool (ingest + synchronous extraction)
- `search` -> MCP `search` tool (mode: hybrid/lexical/semantic/graph/community)
- `get_entity` -> MCP `get_entity` tool
- `timeline` -> MCP `timeline` tool
- `why` -> MCP `why` tool
- `contradictions` -> MCP `contradictions` tool

## 6. Kernel file-by-file changes

| File | Action |
|---|---|
| `lib.rs` | Delete `pub use oxios_memory::*` block (~35 lines). Add `pub mod brain`, `pub mod memory_agent`, `pub mod kernel_db`. Re-export BrainApi, BrainConnection, BrainConfig, sona types |
| `embedding.rs` | Move TF-IDF provider + `EmbeddingProvider`/`EmbeddingVector` traits from oxios-memory (~250 LOC). Define locally |
| `memory/mod.rs` | **Delete entirely.** MemoryStorage-for-StateStore, MemoryGit-for-GitLayer, DreamConfig From, all re-exports |
| `memory/markdown_bridge.rs` | **Delete** |
| `memory/auto_memory_bridge.rs` | **Delete** |
| `agent_runtime.rs` | `oxios_memory::memory::sona::*` -> `crate::memory_agent::sona::*` (4 sites) |
| `persistence_hook.rs` | Same import path change (1 site) |
| `mount/manager.rs` | `MemoryDatabase` -> `KernelDatabase` (import, field, test) |
| `project/manager.rs` | Same |
| `orchestrator.rs` | Test only: `MemoryDatabase::open_in_memory(64)` -> `KernelDatabase::open_in_memory()` |
| `metrics.rs` | `oxios_memory_entries_total` -> `oxibrain_available` (gauge). `oxios_memory_recall_total` -> `oxibrain_recall_total` |

New files:
- `crates/oxios-kernel/src/brain/mod.rs` — BrainConnection
- `crates/oxios-kernel/src/brain/config.rs` — BrainConfig
- `crates/oxios-kernel/src/memory_agent/mod.rs` — module declaration
- `crates/oxios-kernel/src/memory_agent/sona.rs` — moved from oxios-memory
- `crates/oxios-kernel/src/kernel_db.rs` — KernelDatabase

## 7. Binary crate changes

### Boot path (`src/kernel.rs`)

Delete:
- `MemoryManager::new(state_store)` + `set_git_layer`
- SQLite memory init block (~100 lines)
- `DreamProcess::new` + `spawn_dream_task`
- AgentApi memory_manager/hnsw_index params

Add:
```rust
let brain = Arc::new(BrainConnection::connect(brain_config).await);
let kernel_db = Arc::new(KernelDatabase::open(&db_path)?);
```

### API routes

Delete from `workspace.rs`: all `handle_memory_*` handlers (12), `MemoryMapCache`, `compute_memory_map_entries`, `memory_entry_to_detail`, `memory_map_content_signature`.

Delete from `project_routes.rs`: `MemoryEntry` usage.

Add: `handle_brain_search`, `handle_brain_recall`, `handle_brain_entity`, `handle_brain_contradictions`, `handle_brain_timeline`, `handle_brain_why`, `handle_brain_status`. Routes under `/api/brain/*`.

Remove `memory_map_cache` from `AppState`.

### CLI (`src/cli.rs` + `src/main.rs`)

```bash
oxios brain status                          # daemon online/offline + episode count
oxios brain ingest <path|->                 # file or stdin -> brain episode
oxios brain ask "<query>"                   # recall test
oxios brain export --format memory-md       # brain -> MEMORY.md projection
              --output <path>
```

## 8. Web surface rebuild

Delete: `types/memory.ts`, `hooks/use-memory.ts`, `routes/memory.tsx`, `components/memory/*`

Create brain-native panel:

```
web/src/
  types/brain.ts              Episode, Entity, Belief, Statement, SearchHit, TimelineEntry
  hooks/use-brain.ts          useBrainSearch, useBrainEntity, useBrainStatus, useBrainContradictions, useBrainTimeline
  routes/brain.tsx            /brain route
  components/brain/
    overview.tsx              entity/episode/contradiction counts
    search.tsx                hybrid search (lexical/semantic/graph)
    entity-detail.tsx         beliefs, timeline, why (provenance)
    contradictions.tsx        contradiction inbox
```

Tabs: overview | search | entity | contradictions

The old tier/type/protection model is gone. The brain-native model (episodes, entities, beliefs, statements, contradictions, timeline) replaces it.

## 9. Data migration (one-time)

```bash
oxios service stop
oxibrain import-oxios --source ~/.oxios/workspace/memory.db --space personal
oxibrain reextract --space personal
oxibrain serve --daemon
oxios service start
oxios brain status
oxios brain ask "what did I work on last week?"
```

Forward-only: `memory.db` preserved. Users retire it on their own schedule.

## 10. Cargo.toml changes

### oxios-kernel/Cargo.toml

```toml
# Delete
# oxios-memory = { version = "1.39.0", path = "../oxios-memory" }
# sqlite-vec = { version = "0.1", optional = true }
# sqlite-memory = ["dep:rusqlite", "dep:sqlite-vec", "oxios-memory/sqlite-memory"]

# Add
oxibrain-client = "0.1"

# Change: rusqlite non-optional
rusqlite = { version = "0.40", features = ["bundled"] }
```

### root Cargo.toml

```toml
[workspace]
members = [
    # "crates/oxios-memory",  # DELETED
    ...
]

[features]
default = ["web", "cli", "browser", "screenshot"]
# sqlite-memory deleted from default and features
```

## 11. Prerequisites

1. **oxibrain + oxibrain-client published to crates.io as v0.1.** Currently not published (path deps only). This is an oxibrain-side deliverable that blocks the oxios PR.
2. **brain daemon running** and accessible via Unix socket for integration testing.

## 12. Verification

| # | Criterion | Method |
|---|---|---|
| 1 | `cargo build -p oxios-kernel` compiles without oxios-memory | Build check |
| 2 | `cargo test --workspace` passes | Full test suite |
| 3 | `cargo clippy --workspace -- -D warnings` clean | Lint |
| 4 | Degradation: brain daemon stopped, agent turn completes | Manual test |
| 5 | Migration round-trip: seeded memory.db imports, recall returns same entities | `oxibrain import-oxios` + `oxios brain ask` |
| 6 | Web: `/brain` panel renders with daemon running | Browser check |
| 7 | LOC reduction >= 10,000 | `tokei` / `wc -l` |

## 13. Post-merge actions

1. **CHANGELOG**: add `## [Unreleased]` entry marking oxios-memory removal, brain daemon integration.
2. **Version bump**: oxios 1.39.0 -> 1.40.0 in all crate Cargo.toml files.
3. **crates.io deprecation**: separate GitHub PR by release engineer marks oxios-memory 1.39.0 as deprecated (not yanked). The crates.io entry stays until the next major release, then ages out.
4. **oxios-memory crate deletion**: the `crates/oxios-memory/` directory is removed from the workspace in this PR. The crate continues to exist on crates.io as deprecated.
