# Daemonless Brain Integration — oxibrain 0.8 Cutover Design

**Date:** 2026-08-27
**Status:** Accepted (implemented 2026-08-28; supersedes the transport/lifecycle half of RFC-047 and all of RFC-049)
**Scope:** oxios ↔ oxibrain integration after the oxibrain v2.11 daemonless cutover

---

## 1. Summary

oxibrain 0.8.0 (2026-08-27, ARCHITECTURE v2.11) deleted the resident daemon, the
listening socket, the vault watcher, and the background extraction queue. oxios
currently consumes all of those. This design migrates oxios to the daemonless
consumption model with three lanes:

1. **Session child** — one lazily-spawned, kernel-owned `oxibrain serve --stdio`
   child for interactive surfaces (per-turn recall, web Brain tab, foundation
   handshake). The only long-lived brain process, owned by the oxios process,
   born on first use, dies with it.
2. **One-shot CLI** — admin verbs (`index --documents`, `extract --pending`,
   `--version` probe, install) run as short-lived `oxibrain` invocations.
3. **Skill + CLI for agents** — the `memory_{write,read,search}` kernel tools are
   removed; agents get a first-party `brain` skill and call the `oxibrain` CLI
   through the existing exec tool (structured mode, pre-allowed binary).

Vault ingestion stops being a registration RPC and becomes a declaration:
oxios idempotently seeds `~/.oxi/brain/documents.toml` with the vault root and
lets `index`/auto-materialize keep the document cache fresh.

`BrainConnection`, `BrainSupervisor`, `UnavailableHook`, `VaultRegisterPolicy`,
the launchd management, and the reconnect machinery are deleted and replaced by
a single `BrainSession` + a slim `BrainInstaller`.

## 2. Context

### 2.1 What oxibrain 0.8 shipped

- **Removed:** `oxibrain daemon` / `serve --daemon`, the default
  `~/.oxi/brain/oxibrain.sock`, PID file, vault watcher (`watch.rs`),
  `sync/run` RPC + `BrainClient::sync_run`, `episodes/for_locator`,
  `BrainClient::connect` / `connect_default` / `connect_endpoint` /
  `default_socket_path`, background `extract_pending` loop.
- **New transport:** `oxibrain-client 0.8` (crates.io) —
  `spawn_local(LocalProcessEndpoint { executable, dir })` spawns
  `oxibrain serve --stdio --dir <dir>` as a caller-owned child
  (`kill_on_drop`), JSON-RPC over stdin/stdout; `spawn_local_with_token` for
  scoped sessions; `from_io` for tests. `bring_up(token?, ClientHello)` →
  `BrainCapabilities` negotiates the session.
- **Two planes:** `brain.db` (durable memory ledger) + `documents.db`
  (disposable cache rebuilt from `documents.toml` `[[root]]` rows).
  `search` returns `{ memory, documents, freshness }`; `search_planes`
  restricts planes. `search`/`recall` auto-materialize unmaterialized roots.
- **Document history:** gix-backed `Brain::document_history` / native RPC
  `document_history` / `doc://` resource — replaces the occurrence chain.
- **Extraction:** `remember` extracts inline; failures park the episode in the
  `uncached_memory_episodes` backlog (`CapturedPending { pending_count }`).
  `oxibrain extract --pending` (CLI) / `extract_uncached` (RPC) drain it.
  No daemon means no background drain — **the consumer decides when**.
- **MCP surface (15 tools, cap unchanged):** `search`, `recall`, `brief`,
  `navigate`, `get_entity`, `traverse`, `timeline`, `why`, `contradictions`,
  `review_merges`, `stats`, `ingest`, `remember`, `declare`/`retract`,
  `merge_entities`, `redact`. Resources: `spaces://`, `space://`, `entity://`,
  `episode://`, `graph://`, `doc://`. Native RPCs: `handshake`, `reproject`,
  `spaces/list`, `document_history`, `pending_stats`, `extract_uncached`.
- **Ecosystem contracts:** C1 (brain additive, never load-bearing), C6
  (client dependency only, integration < 200 lines/app), C8 (caller-owned
  stdio, explicit `--dir`, no discovery). Default dir: `~/.oxi/brain`.

### 2.2 What breaks in oxios today

| Site | File(s) | Breakage |
|---|---|---|
| Socket connect + lazy reconnect | `crates/oxios-kernel/src/brain/mod.rs` | `BrainClient::connect` deleted |
| Daemon supervisor (launchd, respawn) | `crates/oxios-kernel/src/brain/supervisor.rs` | nothing to supervise |
| Boot vault ingestion (`sync_run` retry loop) | `src/kernel.rs`, `brain/mod.rs` (`VaultRegisterPolicy`) | RPC deleted |
| `[brain] socket_path` config | `crates/oxios-kernel/src/config.rs`, `share/default-config.toml` | socket gone |
| Foundation socket handshake (proto 1–2) | `crates/oxios-kernel/src/foundation/bootstrap.rs` | connect path deleted |
| Web `/api/brain/*` + Brain tab | `src/api/routes/workspace.rs`, `web/src/types/brain.ts` | transport + search envelope |
| `oxios brain {status,ingest,ask,install,start,stop,uninstall}` | `src/cli.rs`, `src/main.rs` | transport; start/stop meaningless |
| memory tools | `crates/oxios-kernel/src/tools/memory_tools.rs` | transport (kept only if we keep the tools — we don't) |

## 3. Goals / Non-goals

**Goals**

- Consume oxibrain 0.8 exactly the way its ecosystem contracts prescribe
  (C1/C6/C8): client dependency, caller-owned child, explicit dir.
- Zero resident brain state that outlives the oxios process. No launchd, no
  PID files, no reconnect policy zoo.
- Agents get the full fifteen-verb surface via skill + CLI instead of three
  proxied tools.
- Degradation contract unchanged: brain absent → `None`/empty everywhere,
  turns complete, boot never blocks (C1).
- Net deletion of kernel code (supervisor, reconnect, retry policy, memory
  tools) larger than the new `BrainSession`.

**Non-goals**

- Embedding the `oxibrain` engine crate (violates C6; pulls GGUF deps into
  the kernel).
- Scoped-token sessions for agents (single-user personal OS; the session child
  runs unscoped, as the socket daemon did).
- Proxying oxibrain's HTTP operations console (`serve --http` ships its own).
- Two-plane search rendering inside `KnowledgeLens` (the lens already searches
  the vault markdown directly; the documents plane would double-cover it).
- Any oxibrain-side change.

## 4. Architecture

```
                        oxios process
┌──────────────────────────────────────────────────────────────────┐
│  kernel                                                          │
│  ┌────────────┐   lazy spawn / respawn    ┌───────────────────┐  │
│  │BrainSession│──────────────────────────▶│ oxibrain serve    │  │
│  │ (1 client) │  JSON-RPC over stdio      │  --stdio --dir    │  │
│  └─────┬──────┘                           └───────────────────┘  │
│        │ consumers                                               │
│   agent_runtime recall/remember · /api/brain/* · KnowledgeLens   │
│   foundation handshake                                           │
│                                                                  │
│  ┌──────────────┐  one-shot tokio::process::Command              │
│  │BrainInstaller│──▶ oxibrain index --documents [--embed]        │
│  │ (GitHub DL)  │──▶ oxibrain extract --pending                  │
│  └──────────────┘──▶ oxibrain --version / spaces  (probes)       │
└──────────────────────────────────────────────────────────────────┘
          ▲ skill teaches verbs; exec structured mode, binary pre-allowed
     agents (oxicode-sdk loop)
```

Three lanes, one binary, one data dir. Interactive surfaces share one session
child; everything else is a fresh process that exits.

## 5. Component design

### 5.1 `BrainSession` (replaces `BrainConnection`)

New file `crates/oxios-kernel/src/brain/session.rs`; `brain/mod.rs` rewritten
around it; `brain/config.rs` rewritten (see §5.3).

```rust
pub struct BrainSession {
    client: tokio::sync::Mutex<Option<BrainClient>>,
    endpoint: LocalProcessEndpoint,     // { executable, dir }
    space: String,
    available: AtomicBool,
    spawn_gate: tokio::sync::Mutex<()>, // serializes (re)spawns
    backoff: SpawnBackoff,              // capped: [1s, 5s, 30s] then stay degraded
    spawner: Arc<dyn Spawn>,            // injectable for tests (from_io)
}

#[async_trait]
pub trait Spawn: Send + Sync {
    async fn spawn(&self, ep: &LocalProcessEndpoint) -> Result<BrainClient>;
}
```

Lifecycle rules:

- **Lazy:** nothing spawns at boot. The first call that needs the brain spawns
  the child (`BrainClient::spawn_local`) and runs `bring_up` once (no token).
- **Call path:** `client == None` → attempt spawn (subject to backoff) → on
  failure degrade (`None`) and log once at `warn`, then at `debug` until a
  spawn succeeds. Call error → drop the client (`kill_on_drop` reaps the
  child), mark unavailable; the next call respawns. No hooks, no supervisor
  callback — a failed spawn *is* the install trigger when `auto_install` (the
  spawn path first asks `BrainInstaller::ensure_binary()` once per process,
  rate-limited 30 s, mirroring today's lazy-reconnect respawn cadence).

Method surface (callers keep their names; types shift only where oxibrain's
wire changed):

| Method | Notes |
|---|---|
| `recall(query, budget)` | MCP `recall`; context-text assembly unchanged |
| `remember(content, source)` | typed client `ingest` — returns the episode id. Extraction is **always deferred**: the MCP `remember` tool's synchronous extraction requires a client-sampling session, which an unscoped stdio session never has. Episodes land in the backlog; the drain timer (§5.5) extracts them. `Captured/CapturedPending` outcomes belong to the facade/CLI path, not this one |
| `search(query, mode, limit, planes)` | two-plane `SearchResponseDto`; `planes=None` → both |
| `get_entity(id)` | typed client method (replaces the `entity://` resource read) |
| `timeline(id, from, to)` | typed client method (replaces `timeline://` resource read) |
| `why`, `contradictions`, `stats` | unchanged |
| `brief`, `traverse`, `review_merges(section)` | `call_tool_json` passthrough, unchanged |
| `spaces()` | native `spaces/list`, unchanged |
| `space_overview()` | `resources/read space://{space}` via `call_rpc_json` |
| `document_history(space, alias, locator, limit)` | new — native RPC |
| `pending_stats()` | new — native RPC, feeds the drain timer (§5.5) |

Plumbing: the shared `call()` helper from `BrainConnection` survives with the
spawn step replacing the reconnect step.

### 5.2 `BrainInstaller` (the surviving half of the supervisor)

`brain/supervisor.rs` is reduced to an installer module
(`brain/installer.rs`):

- **Kept as-is (already pure/tested):** `asset_urls`, `verify_sha256`,
  `extract_single_binary`, `GithubInstaller` (`Installer` trait), install root
  `~/.oxi/bin`, `RELEASES_LATEST_URL`, `ASSET_TAR`.
- **Binary resolution order** (unchanged from RFC-049):
  `[brain].binary_path` → managed `~/.oxi/bin/oxibrain` → `which("oxibrain")`.
- `ensure_binary() -> Option<PathBuf>`: resolve; if none and `auto_install`,
  download + verify (state logged); returns the executable for
  `LocalProcessEndpoint`.
- **Deleted:** `LAUNCHD_LABEL` plist machinery (`build_plist`, bootout/load),
  `spawn_detached` + PID file, `daemon.log`, `SupervisorState` lifecycle
  (`Disabled → NotInstalled → Installing → Starting → Online → Failed`),
  `ManagedBy`, `respawn_if_needed`, keepalive status → the *state* of the
  world is now just: binary present? child spawned?
- **One-time legacy cleanup** (boot, best-effort, log-only): if
  `~/Library/LaunchAgents/com.oxi.oxibrain.plist` exists → `launchctl bootout`
  + remove the file; remove stale `~/.oxi/brain/oxibrain.sock` and
  `.oxibrain.pid`. Idempotent.

### 5.3 Config `[brain]`

```toml
[brain]
enabled = true            # keep
dir = ""                  # NEW — "" → ~/.oxi/brain ($OXI_BRAIN_DIR honored)
space = "personal"        # keep (fallback under resolve_space precedence)
binary_path = ""          # keep
auto_install = true       # RENAMED from auto_manage
```

- `socket_path` and `auto_manage` are **removed**; parsing them is a
  warn-and-ignore (`serde` unknown-field warning via a pre-parse check) so old
  configs don't hard-fail. `share/default-config.toml` updated; config tests
  updated.
- Space resolution precedence is unchanged (`~/.oxi/config.toml [vault].space`
  wins; `resolve_space` survives verbatim).
- `FoundationConfig.brain_socket` → `brain_dir` (same defaulting).
- `Cargo.toml`: `oxibrain-client = { workspace = true }` with
  `oxibrain-client = "0.8"` from **crates.io** (0.8.0 is published; the git
  tag pin is dropped) in `[workspace.dependencies]` and the kernel crate.

### 5.4 Boot wiring & document-plane declaration

`src/kernel.rs` boot block (lines ~1234–1295 today) is replaced:

1. Resolve `dir`, `space`, `binary` (no spawn, no install at boot).
2. Build `Arc<BrainSession>`; attach via `KernelHandle::with_brain(BrainApi)`
   — `BrainApi` now wraps `BrainSession` + `BrainInstaller` (the facade
   shrinks accordingly).
3. Detached boot task (only when `enabled`): **`ensure_documents_root()`**
   - Read `<dir>/documents.toml`; if no `[[root]]` row with
     `alias = "vault"`, append one:
     `alias = "vault"`, `path = <resolved knowledge root>`
     (`config.kernel.resolved_knowledge_root()`:
     `kernel.knowledge_root` → `[vault].path` →
     `~/.oxi/vault`), `space = <resolved space>`. User-authored rows are
     preserved as data (the file is re-serialized by the `toml` crate, so
     formatting may normalize); a mismatched existing `vault` row is left
     alone and logged.
   - Then one-shot `oxibrain index --documents --embed` (§5.2 command lane).
     Failure logs and degrades — first `search`/`recall` auto-materializes
     anyway (oxibrain does this by design); `--embed` at boot merely warms the
     vector chunks.
4. `VaultRegisterPolicy`, `register_vault_source`, and the
   `with_on_unavailable` hook are deleted.

### 5.5 Extraction backlog drain
No daemon drains the extraction backlog anymore; oxios owns the cadence.
Every kernel-ingested episode lands in the backlog (the MCP surface cannot
extract inline without a sampling session — §5.1 `remember`), so the timer is
the *primary* extraction path, not an error handler:

- Timer (every 10 min **while a session child is alive**): call
  `pending_stats()`; if `count > 0`, spawn one-shot
  `oxibrain extract --pending` (background, detached). Local GGUF extraction
  is ~13 s/episode, so the drain is always async and never blocks a turn.
- The timer lives in the kernel's existing background-task pattern (same place
  the cron auto-start loop lives); it does nothing when the brain is disabled
  or degraded — a backlog waiting while oxios is not running is the
  daemonless contract, not a bug.

### 5.6 `agent_runtime`

- Compaction summary → `remember`: unchanged (episode id logged); extraction
  catches up via the drain timer.


### 5.7 Agent surface — skill + CLI replaces memory tools

**Deletions**

- `crates/oxios-kernel/src/tools/memory_tools.rs` (all three tools).
- Registration: `tools/builtin/mod.rs` (unconditional block),
  `tools/registration.rs` (`"memory"` domain arm),
  `tools/kernel_bridge.rs` (name list + count comment),
  `tools/registry.rs` (`ToolMeta` entries).
- `retrieval.rs`: the `"memory"` domain manifest entry is reworded to describe
  the brain skill (the domain stays so persona gating semantics survive).

**Additions**

- First-party skill `share/default-skills/brain/SKILL.md` (RFC-009 format,
  English): when to `remember` (durable facts, preferences, corrections —
  one fact per call, `--space` from context), when to `ask` (retrieval at
  task start or when the user references past events/people), navigation
  (`page`, `entity show`, `timeline`, `why`, `document-history` for vault
  provenance), and what **not** to put in the brain (user notes → `knowledge`
  tool; secrets never). All examples use exec structured mode
  (`binary: "oxibrain", args: [...]`).
- **Exec seeding:** when the brain is enabled, the kernel appends the resolved
  binary name (`oxibrain`) to `exec.allowed_commands` at boot (idempotent,
  user entries preserved). Effect: agents calling exec in structured mode
  with `oxibrain` hit `ExecPolicyResolver → ToolPolicy::Auto` — zero approval
  friction, exactly like today's memory tools. Shell-mode invocations of
  `oxibrain` still go through normal shell approvals.
- Persona manifests referencing `memory_{read,write,search}` are updated to
  the `memory` domain + brain skill wording (the domain gate remains the
  authorization boundary; a persona without `memory` cannot run the CLI
  path either — exec structured mode is additionally gated by the
  `exec` permission as today).

Tradeoff accepted: agents restricted from `exec` entirely lose direct brain
access; their turns still receive kernel recall injection (§5.6), so memory
*reading* degrades gracefully, memory *writing* is unavailable. Documented in
the skill.

### 5.8 `KnowledgeLens`

- `brain: Option<Arc<BrainConnection>>` → `Option<Arc<BrainSession>>`;
  `recall_for_context` and the `@`-mention `search` keep their shapes.
- Lens searches stay **memory-plane** (`search_planes(…, ["memory"])`) — the
  lens already searches vault markdown through `KnowledgeBase`; the documents
  plane would duplicate those hits.

### 5.9 Web API & Brain tab

- All 13 `/api/brain/*` routes keep their paths and handlers; they delegate to
  `BrainApi` → `BrainSession`.
- `GET /api/brain/search` gains an optional `planes` query param and returns
  the two-plane envelope. `web/src/types/brain.ts` mirrors
  `SearchResponseDto { memory, documents, freshness }`; the Brain tab gains a
  documents-hits section (freshness badge per root) alongside memory hits.
- `GET /api/brain/status` becomes:
  ```json
  { "available": bool, "space": str, "episodes": int|null,
    "binary": { "installed": bool, "path": str|null, "version": str|null },
    "pending_extraction": int|null }
  ```
  The `supervisor` field is removed. The web status banner states map to:
  `not installed` / `installing` / `ready` / `degraded` (call-failure path).
- `system.rs` health + `/api/doctor`: `brain.healthy = session available OR
  binary present` (a not-yet-spawned session with an installed binary is
  healthy — spawn is lazy). Doctor message text updated
  ("Brain: ready (session child)" / "Brain: binary missing — install with
  `oxios brain install`").
- New route `GET /api/brain/document-history?space&alias&locator&limit`
  exposing `document_history` (the Brain tab entity page links revisions).

### 5.10 CLI `oxios brain`

Shrinks to management verbs; query/ingest verbs belong to the `oxibrain` CLI:

| Command | Fate |
|---|---|
| `install` | keep — GitHub Releases install (force reinstall) |
| `uninstall` | keep — remove managed binary + legacy launchd cleanup |
| `status` | keep — binary path/version (`--version` probe), dir, space, stats via one-shot `oxibrain stats`, pending count |
| `reindex` | keep — runs the boot `ensure_documents_root()` + `index --documents --embed` task on demand |
| `extract` | keep — runs `oxibrain extract --pending` on demand |
| `ingest`, `ask` | remove — use `oxibrain ingest` / `oxibrain ask` (oxios no longer proxies query verbs) |
| `consolidate` | remove — no oxibrain 0.8 equivalent verb (facade op); users run `oxibrain reextract` / `regenerate-summaries` |
| `export` stub | remove |
| `curate` / `dream` | remove — the subcommands are guidance stubs today (no logic); KnowledgeBase curation lives in the `knowledge_curation` tool |

`main.rs::cmd_brain` rewritten against one-shot CLI + installer; the socket
helper is deleted.

### 5.11 Foundation bootstrap (RFC-048)

`foundation/bootstrap.rs`:

- `handshake_brain(socket, may_start)` → `handshake_brain(endpoint,
  may_install)`: spawn session child (or reuse the kernel's session when
  embedded in boot), `bring_up(ClientHello)` → `BrainCapabilities`; classify
  `Compatible` / `Unavailable` (spawn failed) / `Incompatible`
  (protocol/schema out of range — the typed handshake error carries it).
- The `Starter` hook becomes an installer hook
  (`ensure_binary`, rate-limited).
- `MIN/MAX_BRAIN_PROTOCOL_VERSION` stay (1–2); `ServerInfo.schema_version`
  feeds the compatibility verdict.

### 5.12 Metrics

- `oxibrain_available` gauge: now means "session child spawned and last call
  OK" (set on spawn success / call failure, as today).
- `oxibrain_recall_total`: unchanged.
- No new metrics (pending count is already visible via status/doctor).

## 6. Error handling & degradation

| Failure | Behavior |
|---|---|
| Binary missing, `auto_install=false` | spawn fails once per process (warn), every op `None`; status/doctor say "binary missing" |
| Binary missing, `auto_install=true` | installer runs once per 30 s max; success → spawn proceeds |
| Child dies mid-session | call errors → client dropped → next call respawns |
| Spawn storm prevention | capped backoff [1s, 5s, 30s]; after the third consecutive failure retries at most every 30 s |
| `Locked` writer contention (CLI index/extract concurrent with child) | surfaces as a call error → degrade for that op; op-scoped locks on oxibrain's side bound the window |
| `index`/`extract` one-shot failure | logged, degraded; retried by next timer tick / boot |
| Brain disabled | zero spawns, zero CLI invocations, config seeds nothing; all ops `None` (today's contract) |

Boot never blocks on the brain anywhere (detached tasks only).

## 7. Security

- The session child runs unscoped (full capability) exactly like the socket
  daemon did — same trust boundary, single-user host.
- Agent CLI access is gated twice: `exec` permission (persona) + structured
  binary allowlist (`oxibrain` seeded). Metacharacter blocking on structured
  args keeps the surface tame; the skill teaches structured mode.
- Kernel-constructed one-shot commands (index/extract/probe) use the
  `host_tools/provisioner.rs` pattern (direct `tokio::process::Command`,
  kernel-owned argv) — not the agent sandbox path.
- No secrets in any new surface; `documents.toml` contains paths only.

## 8. Migration & cleanup

1. **Release N (this change):**
   - Dep config: `socket_path`/`auto_manage` warn-and-ignore.
   - Legacy launchd bootout + stale socket/PID removal (§5.2).
   - `documents.toml` seeded on first boot.
   - brain.db data itself is untouched — oxibrain 0.8 owns its schema
     migrations (v11); retired pull-source rows become provenance-only, per
     its changelog.
2. **Docs:** `AGENTS.md` (brain bullet + dependency line),
   `docs/ARCHITECTURE.md` (§ memory: session child, three lanes),
   `docs/USER-GUIDE.md` (brain commands), `CHANGELOG.md`. RFC-047/049 headers
   gain a "superseded-by" note pointing here. RFC-050's
   `register_vault_source` runbook section is marked historical.
3. **Crates.io:** `oxibrain-client 0.8` replaces the git-tag pin.

## 9. Testing

- **Unit (pure):** config parse/defaults/renames; `documents.toml` merge
  logic (append-only, user rows preserved, existing vault row untouched);
  spawn backoff policy; binary resolution order; status-shape builders.
- **Session with fake transport:** `Spawn` trait injects
  `BrainClient::from_io` over an in-memory duplex; scripted JSON-RPC
  responder exercises recall/search/remember/degradation/respawn without a
  real binary (mirrors oxibrain's own client tests).
- **E2E smoke (`#[ignore]`, needs installed binary):** spawn → `remember` →
  `ask`/`search` two-plane → `document_history` → drop; degradation timing
  when the binary path is bogus.
- **C1 gate:** existing CI pattern — main flow with no brain binary present
  (boot completes, turns complete, `/api/brain/*` degrade).
- **Web:** route tests updated to the two-plane envelope + status shape.
- **Skill:** content smoke (frontmatter parses through SkillManager; exec
  examples name the allowed binary).

## 10. Risks & accepted tradeoffs

- **Per-turn spawn after child death** costs one cold start (~tens of ms
  process + SQLite open; oxibrain budgets `Brain::open < 2 s` but measures
  far lower). Acceptable on a failure path, never on the happy path.
- **Exec-only agent access** (§5.7): a persona with `memory` but without
  `exec` can't write memories. Explicit, documented, persona-fixable — and
  the common personas carry both.
- **Backlog waits when oxios is off** — inherent to daemonless; the CLI
  (`oxios brain extract` / `oxibrain extract --pending`) covers power users.
- **documents.toml shared ownership**: oximemo also seeds it (per ECOSYSTEM);
  append-only alias-keyed merge keeps both apps from clobbering each other;
  a divergent vault row is logged, never rewritten.

## 11. Out of scope

Embedded engine (C6), agent tokens/scoped sessions, HTTP console proxying,
KnowledgeLens two-plane rendering, `oxibrain`-side changes, persona editor UI
for the skill.
