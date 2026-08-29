# RFC-025: Mount + Project System

> **Status:** SUPERSEDED — the mount-facing sections below are historical
> record. The mount subsystem was removed on 2026-08-29; project roots are
> the sole filesystem scope. See
> `docs/designs/2026-08-29-project-roots-persona-workbench-design.md`.
> **Date:** 2026-06-15
> **Replaces:** RFC-011 (project-system) — evolves the single `Project` concept into two distinct concepts (`Mount` + `Project`)
> **Related:** RFC-008 (memory consolidation — Dream integration), RFC-011 Phase 1–4

## Summary

Evolve the Project system (RFC-011) into two distinct concepts:

- **Mount** — a path alias: a name bound to one or more filesystem paths (`oxios` → `/Volumes/MERCURY/PROJECTS/oxios`). Lightweight and independent. The **agent explores** the path with tools (`ls`/`read`/`grep`) and accumulates a description and metadata automatically over time.
- **Project** — an optional memory/instruction context bundle that **references** one or more Mounts. Holds custom instructions and pre-written memories. This is the Claude/ChatGPT "Project" concept.

Sessions **bind** Mounts (`mount_ids: Vec`, 1:N — multi-path injection) and may **belong to** one Project (`project_id: Option` — sidebar grouping). The system injects a **`## Workspace Context`** block into the agent's system prompt — the largest missing piece in RFC-011's implementation.

## Motivation

### Problems with the current Project system (RFC-011)

1. **Concept conflation.** The current `Project` is simultaneously a path alias AND a memory partition — two different concerns. Users familiar with Claude/ChatGPT "Projects" (pre-written memory + instructions) expect that semantics, while the path-alias role needs a distinct name. The name "Project" also collides with the LLM-app meaning; "environment variable" (VS Code-style alias) collides with the host's real env vars.
2. **System prompt injection is missing.** `build_system_prompt(seed, persona_prompt, capabilities_xml, kernel_manifest)` carries **no project context**. CWD is set, but the agent never sees the project's name, description, paths, or structure in its prompt. The core promise — *"say 'oxios' and the context flows in"* — is unimplemented.
3. **1:N session–project mapping is incomplete.** `SessionContext` has `secondary_project_ids`, but the orchestrator takes only the first token (`ids_str.split(',').next()`), and the web store tracks a single `activeProjectId`. Multi-project work is first-class in the data model but 1:1 in practice. (This RFC resolves the original concern at the Mount level — see Concept Model.)
4. **Excessive user input.** Create/edit dialogs require name, icon, description, tags, paths, memory_visible — **all manual**. No auto-generation. `AiSuggested` source was removed in RFC-011 Phase 1 and never re-added.
5. **Manual tags are questionable.** Tags exist only for detection layer 3, but the Mount name already drives detection (layer 1). Auto-detected metadata (tech stack, languages) can serve the same purpose with zero user effort.

### What Mount + Project solves

- **Clear naming.** `Mount` = path binding (Unix `mount` metaphor, fits Oxios's "Agent OS" identity; no collision with host env vars). `Project` = memory/instruction bundle (matches user expectations from other LLM apps). Since the path-alias role moves out of `Project`, the name `Project` now cleanly means what users expect.
- **Real prompt injection.** A `## Workspace Context` section is added to the system prompt with active Mounts, project instructions, and relevant memories.
- **Multi-path via Mounts, grouping via Project.** Sessions hold `mount_ids: Vec` (the real 1:N — multi-path work, each granting access). Sessions belong to at most one Project (`project_id`) which groups them in the sidebar and contributes its Mounts/instructions/memories.
- **Minimal user input.** The user provides **name + path only**. The agent explores and writes `auto_description` / `auto_meta` over time, including during Dream consolidation. Mounts are living objects — project contents change, so their descriptions do too.
- **No manual tags.** Detection uses Mount name, path, and auto-detected metadata.

## Design

### Concept Model

```
┌──────────┐  project.mount_ids (N)   ┌──────────┐
│  Mount   │ ◄────────────────────────│ Project  │── memories (junction, N:M)
│ path     │                          │ instr.   │ ◄── owns many sessions (1:N)
│ alias    │                          │ bundle   │
└────┬─────┘                          └────┬─────┘
     │                                     │ session.project_id (single)
     │ session.mount_ids (1:N)             ▼
     ▼                                 ┌──────────────────────────┐
┌──────────────────────────┐          │        Session           │
│        Session           │ ◄────────┤  project_id: Option      │ (grouping)
│  mount_ids: Vec          │          │  mount_ids: Vec          │ (injection, N)
└──────────────────────────┘          └──────────────────────────┘
```

- A **Mount** can exist alone (quick work: *"review oxios"* needs only the Mount).
- A **Project** is optional. It references Mounts, carries custom instructions + memories, **and groups sessions** (1:N — a Project owns many sessions; a session belongs to at most one Project, or none).
- A session **binds** Mounts directly for path access (`mount_ids`, 1:N) **and may belong to** one Project (`project_id`, grouping). Belonging to a Project auto-activates that Project's referenced Mounts.

**The key split:** "inject multiple paths" (Mount-level, `mount_ids: Vec`) is distinct from "belong to a group" (Project-level, `project_id: Option`). The original request to "inject multiple projects" is satisfied by multiple Mounts; Project ownership is singular, matching Claude/ChatGPT's folder model.

### Data Model

```rust
/// A path alias. A name bound to one or more filesystem path(s).
pub struct Mount {
    pub id: MountId,                  // Uuid
    pub name: String,                 // unique, e.g. "oxios"
    pub paths: Vec<PathBuf>,          // ≥1 path; paths[0] is CWD when primary
    pub auto_description: String,     // agent-explored; updated over time
    pub auto_meta: MountMeta,         // auto-detected stack/languages/structure
    pub source: MountSource,          // Manual | AutoDetected
    // ── enrichment state (see "Enrichment Triggers") ──
    pub last_marker_snapshot: HashMap<PathBuf, SystemTime>,  // marker mtime at last enrich
    pub enrichment_pending: bool,     // drift detected; agent nudged to refresh
    pub last_enriched_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
}

/// Auto-detected metadata, written by the agent as it explores.
pub struct MountMeta {
    pub languages: Vec<String>,       // ["rust", "typescript"]
    pub stack: Vec<String>,           // ["tokio", "axum", "react"]
    pub markers: Vec<String>,         // detected files: ["Cargo.toml", "AGENTS.md"]
    pub summary: String,              // one-line derived summary
}

/// Optional memory/instruction bundle. May reference Mounts, or stand alone
/// for non-code contexts (travel planning, writing) — `mount_ids` is empty then.
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub mount_ids: Vec<MountId>,      // Mounts this project covers; may be empty (non-code)
    pub instructions: String,         // custom system-prompt instructions
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // memories via the existing project_memory junction (N:M, unchanged)
}
```

**Removed** from RFC-011's `Project`: `description` (→ `Mount.auto_description`), `tags` (→ `Mount.auto_meta`), `emoji` (UI-only; derive from name/auto_meta), `memory_visible` (per-project gating replaced by explicit project binding), `paths` (→ `Mount`).

### Session Binding

**Key distinction: "injection" (context) and "membership" (grouping) are separate concerns.**

```rust
pub struct SessionContext {
    pub recall_timing: Option<RecallTiming>,

    // ── Membership (grouping) — one Project owns this session ──
    pub project_id: Option<ProjectId>,   // the Project this session belongs to (sidebar grouping)

    // ── Injection (context) — N Mounts bound for path access ──
    pub mount_ids: Vec<MountId>,         // primary first, then secondary
}
```

- **`project_id`** (singular): the Project this session *belongs to*. Drives sidebar grouping (Project → its sessions). Optional — a session can be project-less ("general chat"). This is the 1:N grouping (one Project owns many sessions).
- **`mount_ids`** (plural): the Mounts *bound* for this session's context (path access + CWD). This is the 1:N injection (one session binds many Mounts) — how multi-path work is expressed.

**Why split it this way:** the original request ("inject multiple projects per session") was really about working across *multiple paths* at once. That concern lives at the Mount level (`mount_ids: Vec`). Project, by contrast, is the grouping/ownership unit — and grouping is naturally 1:N (a session sits in one folder, like Claude/ChatGPT). A Project's referenced Mounts are auto-activated when the session belongs to that Project (see below).

### What a Project gives a session that belongs to it

When `session.project_id = Some(P)`:
1. `P.mount_ids` are **auto-added** to the session's active Mounts (unless the user explicitly removed one).
2. `P.instructions` + `P`'s linked memories are injected into `## Workspace Context`.
3. The session appears under `P` in the sidebar.

So Project = a saved *preset* (Mounts + instructions + memories) that a session joins. The user can still add extra Mounts to the session beyond what the Project brings (cross-project path work).

### System Prompt Injection (NEW — core feature)

`build_system_prompt` gains a `workspace_context: Option<&str>` parameter, injected **right after the Goal section**:

```
## Workspace Context

### Active Mounts
- **oxios** → /Volumes/MERCURY/PROJECTS/oxios
  Oxios Agent Operating System. Rust, tokio async, oxi-sdk. Monolithic kernel crate.
- **oxi-sdk** → /Users/.../oxi
  Crates.io SDK for Oxios agents.

### Project Instructions: oxios-dev
<custom instructions text>

### Relevant Project Memories
- <memory content>
- <memory content>
```

- Mounts inject: name, primary path, `auto_description`, `auto_meta.summary`.
- Projects inject: custom `instructions` and their linked memories (via `project_memory`).
- Location: after Goal/Constraints, before Persona — the agent must see its workspace before it acts.

### Auto-Detection + Approval Badge

On message arrival, `detect_mounts(message)`:

1. Name match ("oxios" → Mount named "oxios")
2. Path extraction + prefix match
3. `auto_meta` keyword match (replaces manual tags)

A detected Mount is **auto-injected** into `session.mount_ids` (as primary if none is set), CWD applied, and the system prompt updated. The UI shows a dismissible badge (`[🔧 oxios] 적용됨`). The user can dismiss or switch. Secondary Mounts can be added explicitly via UI multi-select.

### Agent-Driven Enrichment + Dream Integration

Mounts are living objects: their `auto_description` / `auto_meta` evolve as the underlying project changes. The agent explores the path with tools (`ls`/`read`/`grep`), then writes its findings via the `mount.update` action. This happens on three concrete triggers and a rate-limit/cost guard — **see the "Enrichment Triggers" section for the full spec.** A manual "re-scan" is also available any time from the UI/CLI.

Enrichment is **agent-driven (tools)**, never a raw LLM call divorced from tool use — the agent's description is grounded in the real files it read (it can cite `AGENTS.md`, the dependency list, etc.), consistent with Oxios's "agent OS" model.

## Detection & Binding Precedence

Sessions hold a persistent `mount_ids` binding, **and** per-message detection can also fire. These two must have a clear precedence to avoid the agent's context thrashing between turns.

### Model: sticky-primary, detection fills secondary

```
on message arrival:
  detected = detect_mounts(message)
  session_mounts = session.mount_ids (from explicit binding or prior turn)

  if session_mounts is empty:
      # No binding yet → detection seeds the primary slot.
      if detected: session.mount_ids = [detected]
      badge: "[🔧 oxios] 적용됨"

  else if detected is in session_mounts:
      # Already bound → nothing changes (no badge spam).
      pass

  else if detected is Some:
      # Already bound to something else → add as SECONDARY, never replace.
      session.mount_ids.push(detected)
      badge: "[+ oxi] 보조로 추가됨" (dismissible)

  # Primary changes ONLY by explicit user action (UI promote/demote, CLI --mount).
```

**Why this model:**
- Empty-slot seeding honors the core promise: *"say 'oxios' and it flows in."*
- Non-empty stickiness keeps context stable across turns — the agent doesn't lose its CWD because the user mentioned another repo in passing.
- Explicit promotion is the user's escape hatch when detection got the primary wrong.

Detection considers **only Mounts**, never Projects. Projects are always explicit (they carry user-written instructions and shouldn't be guessed).

### Detection ambiguity

When multiple Mounts match (e.g. message contains paths under both `oxios` and `oxi`):
- Prefer the **most specific path** (longest matching prefix).
- If still tied, prefer the Mount whose name appears in the message.
- If still tied, do not auto-inject — show all candidates as a hint and let the user pick.

## Enrichment Triggers

Agent-driven enrichment (writing `auto_description` / `auto_meta`) needs concrete triggers, not a vague "debounced."

### Three triggers

1. **Explicit `mount.rescan`** — user or agent calls it deliberately (UI button, CLI `mount rescan`, agent `mount` tool action). Always allowed; writes immediately.

2. **Drift detection (cheap, passive)** — when a session binds a Mount, snapshot the `mtime` of its **marker files** (`Cargo.toml`, `package.json`, `go.mod`, `AGENTS.md`, `README.md`, …) into `mount.last_marker_snapshot`. On the next session that binds the same Mount, compare current mtime vs snapshot. If any marker drifted → set `mount.enrichment_pending = true`. The agent is nudged (system-prompt line) to refresh, then clears the flag.

   Cost: a handful of `stat()` calls per session — negligible.

3. **Dream-time refresh (RFC-008 Phase 2)** — during Dream's *Gather Signal* phase, re-snapshot markers for all Mounts with `last_active_at` within the dream window. Mounts whose content drifted since the last Dream get refreshed. This catches changes that happened in other sessions the current agent never saw.

### The write path

Enrichment is **agent-driven** — the agent explores with `ls`/`read`/`grep`, then calls `mount.update { id, auto_description?, auto_meta? }`. The system validates and writes. We never make a raw LLM summarization call divorced from tool use; the agent's enrichment is grounded in the real files it read (so it can cite `AGENTS.md`, the dependency list, etc.).

### Cost guard

- `mount.update` is rate-limited per Mount: at most once per `drift event` + once per explicit `rescan`. No tight loops.
- Dream-time refresh is capped at N Mounts per Dream cycle.
- `auto_description` is bounded (e.g. ≤ 500 chars); longer exploration goes into memories, not the Mount record.

## Path Access & Security

**Today's bug:** `agent_runtime.rs` adds only `project_paths[0]` to `allowed_paths`. Secondary project paths are silently inaccessible to the agent. Mount fixes this.

### Rule: every bound Mount grants path access

When building permissions for a session:
```rust
for mount_id in session.mount_ids {          // primary + secondary, all of them
    for path in mount.paths {
        let pattern = format!("{}/​**", path.trim_end_matches('/'));
        perms.allow_path(&pattern);           // dedup: skip if already covered
    }
}
// CWD = primary mount paths[0] (unchanged fallback if empty)
```

- Overlapping/parent paths are deduplicated (a parent pattern already covers children).
- Projects do **not** add paths — they only reference Mounts. Path access comes from Mounts.
- This replaces the single-`project_paths[0]` logic and makes multi-mount work real.

### Permissions are per-session, derived

Access is recomputed each session from the bound Mounts — no stale permission grants persist beyond the binding. Revoking a Mount from a session revokes its paths.

## Prompt Budget

`## Workspace Context` is bounded so it can't crowd out the Seed and tools.

- **Soft budget:** ~1500 tokens for the whole section.
- **Fill order** (stop when budget hit):
  1. Primary Mount — full (`auto_description` + `auto_meta.summary` + path).
  2. Active Project `instructions` — full.
  3. Secondary Mounts — name + path + one-line summary only.
  4. Project memories — most-recent / highest-importance first.
- **Over budget:** truncate `auto_description` to one line; drop secondary details; cap memories at top-K.
- When there are zero Mounts and zero Projects, the section is omitted entirely (no empty headers).

## Auto-Meta Detection Rules

`MountMeta` replaces manual tags. Detected by cheap heuristics on marker files, then refined by the agent.

| Marker | Inferred meta |
|--------|---------------|
| `Cargo.toml` | language `rust`; stack from `[dependencies]` (tokio, serde, axum, reqwest, …) |
| `package.json` | language `typescript`/`javascript`; stack from `dependencies` + `devDependencies` |
| `go.mod` | language `go`; stack from `require` blocks |
| `pyproject.toml` / `requirements.txt` | language `python`; stack from dependencies |
| `AGENTS.md` / `CLAUDE.md` / `.cursorrules` | marker; first paragraph seeds `summary` |
| `README.md` | first paragraph seeds `summary` (lower priority than AGENTS.md) |
| `crates/` dir | structure hint `cargo-workspace` |
| `packages/` or `apps/` dir | structure hint `monorepo` |

- Heuristics run at **drift detection** time (cheap `stat` + tiny reads), producing a draft `MountMeta`.
- The agent refines it during enrichment (e.g. extracts the real dependency shortlist, adds architecture notes from `AGENTS.md`).
- Detection layer 3 matches against `auto_meta.languages` + `auto_meta.stack` + `auto_meta.summary` keywords — replacing manual tags with no user effort.

## API

### Orchestrator

```rust
pub async fn handle_message(
    &self,
    user_id: &str,
    user_message: &str,
    session_id: Option<&str>,
    mount_ids: Option<&str>,     // "uuid1,uuid2,..." — primary first; multi-path injection
    project_id: Option<&str>,    // optional single Project membership
    request_id: &str,
) -> Result<OrchestrationResult>
```

`OrchestrationResult` exposes `active_mount_ids: Vec<Uuid>`, `active_project_id: Option<Uuid>`, and `mount_tag: Option<String>` (e.g. `[🔧 oxios + oxi-sdk]`).

When `project_id` is set, its `mount_ids` are merged into the active Mounts (union, preserving the session's explicit primary).

### CLI

```bash
oxios mount list
oxios mount add oxios --path /Volumes/MERCURY/PROJECTS/oxios   # name + path only
oxios mount rescan oxios                                       # force agent enrichment
oxios mount remove oxios

oxios project list
oxios project add oxios-dev --mount oxios --mount oxi-sdk      # references mounts
oxios project set-instructions oxios-dev --file ./instructions.md

oxios run --mount oxios --also oxi-sdk "review the API"
oxios run --project oxios-dev "continue the refactor"
```

### Web API

```
GET    /api/mounts
POST   /api/mounts                 { name, paths }              # minimal input
GET    /api/mounts/:id
PUT    /api/mounts/:id
DELETE /api/mounts/:id
POST   /api/mounts/:id/rescan                                   # trigger enrichment

GET    /api/projects
POST   /api/projects               { name, mount_ids, instructions }
GET    /api/projects/:id
PUT    /api/projects/:id
DELETE /api/projects/:id
GET    /api/projects/:id/memories
POST   /api/projects/:id/memories
DELETE /api/projects/:id/memories/:memoryId

# Session binding
POST   /api/sessions/:id/mounts    { mount_ids }                # primary first (injection)
POST   /api/sessions/:id/project  { project_id }              # single grouping parent (optional)
DELETE /api/sessions/:id/project                               # unparent (becomes project-less)
```

### Agent Tools

`mount` tool (new):
- `list`, `get`, `update` — refine `auto_description` / `auto_meta` (agent-driven enrichment)

`project` tool (slimmed from RFC-011):
- `list`, `get`, `link_memory`, `unlink_memory`

Agents still **cannot** create or remove Mounts/Projects — those remain user-level operations (CLI/Web).

## Migration

### Data migration

1. Add `mounts` table. Add `mount_ids` JSON column to `projects`. `project_memory` junction unchanged.
2. For each existing `Project` row, decide Mount-only vs Mount+Project-bundle by a **concrete rule** (no fuzzy matching):
   - **Has ≥1 path:** create a `Mount` (`name`, `paths`, `auto_description = old description`, `auto_meta` seeded from `tags`, `source`).
     - **Has ≥1 linked memory in `project_memory`** → ALSO create a `Project` bundle with `mount_ids = [that mount]`, `instructions = old description`, and copy its `project_memory` rows. (Memories must keep their parent.)
     - **No linked memories** → Mount alone is sufficient.
   - **Has no paths (non-code, e.g. "travel planning"):** do NOT create a Mount (Mount = path alias). Create a `Project` bundle with `mount_ids = vec![]`, `instructions = old description`, and copy its `project_memory` rows. This preserves the non-code use case.
   - Either way, **no data is discarded.**
3. Sessions: `primary_project_id` + `secondary_project_ids` → `mount_ids` (primary project's mount first). `project_id = None` (grouping) — set only if the user explicitly creates/assigns a Project bundle.
4. Deprecate, then drop, legacy `projects.description` / `tags` / `emoji` / `memory_visible` columns.

### Code migration

- `ProjectManager` → `MountManager` (CRUD, detection) + `ProjectManager` (bundle CRUD, memory linking, session-grouping).
- `SessionContext` fields: `mount_ids: Vec` (was `primary_project_id` + `secondary_project_ids`) and `project_id: Option` (single grouping parent).
- `build_system_prompt` gains workspace-context injection.
- Orchestrator parses the full `mount_ids` list (not just the first token); merges Project's `mount_ids` when `project_id` is set.
- Web: `chat-session-nav.tsx` becomes a Project-tree (Project nodes own their sessions) with inline create; `stores/chat.ts` `activeProjectId` → `activeProjectId` (grouping) + `activeMountIds` (injection); detection badge drives `activeMountIds`.

## Touch-Point Inventory

**Kernel:** `project/mod.rs`, `manager.rs`, `detection.rs`, `project_db.rs` → split into `mount/` + `project/` (bundle). `session_context.rs`; `orchestrator.rs` (handle_message signature + prompt injection); `agent_runtime.rs` (`build_system_prompt`, CWD from primary mount); `kernel_handle/project_api.rs` → `mount_api.rs` + `project_api.rs`; `tools/builtin/project_tool.rs` → add `mount_tool.rs`.

**Web backend:** `routes/project_routes.rs` → `mount_routes.rs` + `project_routes.rs`; `routes/chat.rs` (`mount_ids` / `project_id` in request/response/metadata); `chat-session-nav.tsx` → Project-tree.

**Web frontend:** `stores/chat.ts` (`activeMountIds` + `activeProjectIds`, activate detection badge); `components/project/*` → `mount/` + `project/`; `routes/projects/*`; `types/`.

**CLI:** `cmd_run.rs` output fields; new `mount` / `project` subcommands.

## Phased Plan

**Phase 1 — Mount core + prompt injection** ✅ *(the biggest user-visible win)*
- [x] Mount data model + manager + DB schema
- [x] `## Workspace Context` injection in `build_system_prompt`
- [x] Orchestrator parses the full `mount_ids` list (`resolve_mount_workspace`)
- [x] Path-access fix: every bound Mount grants path access (was `project_paths[0]`-only)
- [x] Web `/api/mounts` CRUD routes + `mount_ids`/`mount_tag` in the chat flow
- [x] Frontend: Mount UI (types, hooks, dialog, management page, detection badge)

**Phase 2 — Project bundle layer + sidebar grouping** ✅
- [x] Project struct gains `mount_ids: Vec` + `instructions: String` (additive migration)
- [x] Orchestrator injects Project instructions + auto-activates referenced Mounts
- [x] `PUT /api/projects/:id` accepts `mount_ids` + `instructions`
- [x] EditProjectDialog: Mount multi-select chips + instructions textarea
- [x] Sidebar reorganized as Project-tree (Project folders → sessions, unfiled group)
- [x] `session.project_id` top-level field (replaces buggy `metadata["project_ids"]`)

**Phase 3 — Agent-driven enrichment** ✅
- [x] `mount` tool (list/get/update): agent explores and writes auto_description/auto_meta
- [x] Auto-meta detection (meta_detection.rs): Cargo.toml→rust, package.json→ts, etc.
- [x] Drift detection: marker mtime snapshot comparison
- [x] `check_all_drift()` for Dream-time refresh hook

**Phase 4 — UX polish** ✅
- [x] Detection-badge full flow (dismiss / accept / bind)
- [x] Mount re-scan trigger (`POST /api/mounts/:id/rescan`)
- [x] Drag-to-reparent sessions between Projects (HTML5 drag-and-drop)
- [x] One-time data migration: legacy Project `paths` → Mounts (idempotent, runs at startup)

**Phase 5 — Frequent-path auto-promotion** ✅
- [x] `mount::path_promotion` — extract paths from session trajectories (tool
  calls) + user messages, normalize to project roots via marker walk-up
  (`Cargo.toml`, `package.json`, `.git`, …), tally within a sliding window
- [x] `MountManager::promote_frequent_paths()` — promotes roots crossing the
  threshold into Mounts (`MountSource::AutoPromoted`), seeds `auto_meta`, skips
  already-covered roots and name collisions
- [x] Background scanner (Dream-cadence) wired in the kernel assembler; reads
  `load_all_sessions()` so it sees full trajectories
- [x] `MountsConfig` (`[mounts]` in config.toml): `auto_promote_enabled`,
  `auto_promote_threshold` (default 3), `auto_promote_window_days` (14),
  `auto_promote_interval_secs` (3600)
- [x] Frontend: ✨ badge + violet chip on auto-promoted Mount cards
- [x] 7 new tests (extraction, windowing, root collapse, threshold, coverage-skip, e2e)

## Sidebar UX: Project-Centric Chat Navigation

The Chat sidebar is reorganized around **Projects as folders that own their sessions**. This is the primary place to manage Projects — no separate management page required for day-to-day use.

### Sidebar structure (Chat mode)

```
┌─ Chat Sidebar ──────────────────────────────┐
│ [+ 새 세션]                                  │  ← floating, prompt for Project (optional)
│                                             │
│ 📁 oxios-dev                          ▾ [⚙] │  ← Project node (expand/collapse, ⚙ = edit)
│ ├─ 🔧 oxios  🔧 oxi-sdk                     │  ← Mounts this Project brings (read-only chips)
│ ├─ 💬 PR 리뷰 #142                       ●  │  ← sessions belonging to this Project
│ ├─ 💬 리팩토링: agent_runtime              │
│ └─ 💬 API 설계                              │
│                                             │
│ 📁 my-blog                            ▸ [⚙] │  ← collapsed Project
│                                             │
│ ── 일반 세션 ──                             │  ← project-less sessions (project_id = None)
│ 💬 점심 추천                                │
│ 💬 여행 계획                                │
│                                             │
│ [+ 새 Project]                              │
└─────────────────────────────────────────────┘
```

### Interactions

- **Click a Project node** → expands/collapses its sessions (and sets `activeProjectId` for context — instructions/memories apply to the next session).
- **`⚙` on a Project** → inline edit drawer: Mounts (multi-select chips), instructions (editor), linked memories. Saves without leaving the sidebar.
- **Click a session** → loads it; its `project_id` determines grouping, its `mount_ids` determine active paths.
- **`+ 새 세션`** → asks "어떤 Project에 만들까요?" (multi-select includes "없음 / 일반 세션"). Creates the session under the chosen Project (or project-less).
- **`+ 새 Project`** → inline create: name + pick Mounts + write instructions. Stays in the sidebar.
- **Drag a session between Projects** → re-parents `session.project_id` (optional Phase 4 polish).

### Mount detection still works on top

Regardless of which Project a session belongs to, the **Mount detection badge** operates on `mount_ids`. Mentioning "oxi-sdk" in an `oxios-dev` session adds `oxi-sdk` as a secondary Mount (sticky-primary) without changing the Project membership. So a session can belong to `oxios-dev` *and* pull in extra Mounts as the conversation evolves.

### Where Mounts are managed

Mounts are lighter-weight and rarely hand-edited (name + path, then auto-enriched). They're managed from:
- The Project edit drawer (the common case — pick which Mounts a Project brings).
- A dedicated `/mounts` page (for power users: list, add path-only, rescan, delete).

Most users never visit `/mounts` — they manage Mounts through Projects.

## Deletion & Cascade Semantics

- **Delete a Mount:** removes the `mounts` row. References in `project.mount_ids` and `session.mount_ids` are pruned (these are JSON arrays — re-write without the id). The Mount's `auto_description`/`auto_meta` are lost; users who care should convert notable facts into a memory first. The underlying filesystem path is **never** touched.
- **Delete a Project:** removes the `projects` row. Its `project_memory` junction rows are deleted (cascade), but the **memories themselves remain** — they exist independently (RFC-011 N:M decision). Sessions belonging to it become project-less (`session.project_id = None`) but are otherwise unaffected — they keep their `mount_ids`. Mounts the Project referenced are unaffected.
- **Delete a session:** does not touch Mounts, Projects, or memories — only the session record.

## Open Questions

| Question | Resolution |
|----------|------------|
| One Mount per path, or can a Mount hold multiple paths? | Multiple — `paths: Vec` (e.g. a repo + a config dir). `paths[0]` is CWD. |
| Can a Mount belong to multiple Projects? | Yes — `project.mount_ids` is a reference list; many Projects can reference one Mount. |
| Keep emoji as a field or derive it? | Derive from `auto_meta`/name in the UI; not stored as a first-class field. |
| How aggressive is auto-injection on detection? | Inject by default with a dismissible badge (auto-badge policy). |
| Enrichment frequency / cost guard? | Rate-limit `mount.update` to once-per-drift + once-per-explicit-rescan; cap Dream-time refresh per cycle; bounded `auto_description` (≤ 500 chars). |
| Non-code contexts (no paths)? | Become path-less `Project` bundles (`mount_ids = []`); never become Mounts. |
| Memories when a Project is deleted? | Survive — only the junction rows are removed (N:M, memories are independent). |
