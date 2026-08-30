# Project Issues, Milestones, and Session Todo

> **Date:** 2026-08-29
>
> **Status:** Approved design — implementation in progress
>
> **Depends on:** `oxicode-sdk` 0.77.0 (issue store + tool, todo tool + state
> provider, all ungated re-exports)
>
> **Independent of:** the deferred `root_paths` / mount-removal work in
> `2026-08-29-project-roots-persona-workbench-design.md`. Nothing here reads a
> project's filesystem roots.

## 1. Decision

Oxios gains two features from the engine's 0.77.0 surface:

1. **Issues and milestones, scoped to a Project.** GitHub-shaped: an issue has
   a number, title, status, priority, labels, and a markdown body; a milestone
   groups issues and tracks progress. Both belong to a Project and to nothing
   else.
2. **Todo, scoped to a chat session.** A phased task plan the agent maintains
   across the turns of one conversation, rendered live in the transcript the
   way Claude Code Desktop renders it.

**Oxios issues are Oxios's own.** They are not shared with, discovered by, or
synchronised to `oxicode`'s per-folder `.oxicode/issues/` store. The two
products track different scopes — a terminal agent tracks the folder it was
launched in; Oxios tracks a Project, which may span zero or many folders. An
Oxios project's issues live in the project's own directory and are reached only
through the Oxios `issue` tool and the web UI.

We reuse the engine's *implementation* without sharing its *data*:
`FileIssueStore`, `cas_retry`, `liveness`, and the issue/todo types all come
from `oxicode-sdk`. Only the storage location, the tool shell, and the
milestone layer are ours. This follows the standing rule in `AGENTS.md`
("reuse oxicode-sdk, never reimplement") while keeping the product boundary
sharp: if the two issue systems converged, `oxicode` would become nothing but
a TUI skin over Oxios.

## 2. Goals

1. A Project has an issue tracker with project-level numbering (`#12` is unique
   within the project) and milestones that group its issues.
2. The rule for where issues live has no special cases: a project with zero
   folders and a project with five folders behave identically.
3. The agent can only mutate issues through the `issue` tool, never by editing
   the files — so CAS and assignment locking cannot be bypassed.
4. Concurrent work on one issue is detected and survives crashes without
   heartbeats or timeouts.
5. A todo plan survives across the turns of a chat session and is visible in
   the transcript and above the composer while it is live.
6. No change to the file-access security model is required to land this.

## 3. Non-goals

- No sharing, symlinking, or discovery bridge with `oxicode`'s
  `.oxicode/issues/` store.
- No change to `issues_dir()` or any other engine behaviour. `oxicode-sdk`
  stays a crates.io dependency consumed as published.
- No GitHub sync. `IssueMeta.github` stays unpopulated (the engine reserves it
  for its own future work).
- No migration of the rest of the Oxios home. See §11.
- No cross-project issue views, boards, or dependencies between issues.
- No todo persistence beyond the daemon's lifetime.

## 4. Storage

```text
~/.oxi/oxios/projects/<project-uuid>/
  issues/
    0001-fix-login.md          # YAML frontmatter + markdown body
    0002-add-milestones.md
    .alive/<ownership-id>      # flock files (SDK-managed)
  milestones.yaml
```

One directory per project, always. That single fact gives project-level issue
numbering for free — `FileIssueStore::next_id()` scans one directory for the
maximum id, so one directory is one id space.

`~/.oxi/oxios/` is the declared future Oxios application home: `~/.oxi/` is the
per-user ecosystem umbrella, and app-owned state lives in a namespaced
subdirectory beside the shared `vault/`, `brain/`, and `foundation/` trees.
Project data is placed at its final address now; migrating the rest of
`~/.oxios/` is a separate change (§11).

### 4.1 Why this address needs no security work

`gate.rs::OXI_HOME_DENY_ROOTS` is `[".oxi", ".oxicode"]` — a **whole-root deny**
applied at `AccessGate` construction. Project data placed under `~/.oxi/` is
therefore already unreachable by the file tools (`read`/`write`/`edit`/`grep`/
`find`/`ls`) with no new deny entry, no parity-test change, and no weakening of
the fail-closed posture.

This is not a happy accident but the property we want: **the agent reaches
issues through the `issue` tool or not at all.** `FileIssueStore` calls
`std::fs` directly and does not pass through `AccessGate`, so the tool works
while raw file access is denied. An agent that could `edit` an issue file
directly would silently defeat the CAS check and the assignment lock.

### 4.2 Path resolution

New module `crates/oxios-kernel/src/project/paths.rs`:

```rust
/// Oxios application data home. `~/.oxi/oxios`, overridable by `OXIOS_DATA_HOME`.
pub fn data_home() -> PathBuf;
/// `<data_home>/projects/<id>` — created lazily on first write.
pub fn project_dir(id: ProjectId) -> PathBuf;
/// `<project_dir>/issues`
pub fn project_issues_dir(id: ProjectId) -> PathBuf;
/// `<project_dir>/milestones.yaml`
pub fn project_milestones_path(id: ProjectId) -> PathBuf;
```

`OXIOS_DATA_HOME` exists so tests are hermetic; it is not a documented user
setting.

Removing a project deletes its directory, after the same confirmation the
existing delete flow already requires.

## 5. Issues

### 5.1 Store

`oxicode_sdk::FileIssueStore`, opened per project against
`project_issues_dir(id)` and cached in an `Arc` map on `IssueApi`. The store
already provides everything we need:

- markdown + YAML frontmatter documents, atomic temp+rename writes
- content-hash CAS (`Conflict` when the file changed since the read)
- in-process same-file write serialisation
- process-liveness assignment via `flock` on `.alive/<id>`
- lazy orphan reaping on open

Mutations from the agent tool go through `oxicode_sdk::cas_retry`, matching the
engine's contract: the store stays strict and never retries itself; only the
tool layer reconciles. Direct API (web UI) mutations pass the hash the client
last read and surface `Conflict` to the user rather than silently retrying —
a human editing an issue should be told someone else changed it.

### 5.2 Ownership identity

The assignment lock only protects anything if the caller identity is non-empty
and matches a `flock` the process actually holds. The engine documents this at
length (`oxicode` AGENTS.md, defect #13) because getting it wrong is silent.

Oxios identity: `oxios-<pid>-<session-id>`, where `<session-id>` is the chat
session id from `MsgCtx.session_id`.

That id is stable across the turns of a session:
`Orchestrator::handle_unified` computes `session_id.unwrap_or(request_id)`, and
the first turn's value is returned to the client and echoed back on every
later turn — the single turn key that `AGENTS.md` requires. A test pins this;
if it ever stops holding, issue ownership breaks in a way that is invisible
without one.

`IssueOwnership` (new, on `IssueApi`) holds one `liveness::AliveGuard` per
`(project, session)`, acquired on that session's first issue mutation and held
for the session's lifetime. Guards release when the session is deleted, and
otherwise when the daemon exits — at which point the OS drops every lock and
all assignments become reclaimable, which is exactly the intended behaviour.

`AgentConfig.session_id` is currently `None` at
`agent_runtime.rs:1059`. It is set to the ownership id, which flows through the
SDK into `ToolContext.session_id`. Two existing consumers improve as a side
effect: `hook_runner.rs` (currently receives `""`) and `task_tool.rs`
(currently records `created_by_session_id: None`).

### 5.3 Agent tool

An Oxios `IssueTool` (tool name `issue`), not the SDK's. Three reasons:

1. The SDK tool's description hardcodes "stored as markdown files in
   `.oxicode/issues/`" — false for Oxios and actively misleading to the agent.
2. It opens one store fixed at construction; ours resolves the store from the
   turn's project.
3. Milestone actions have to live somewhere.

Actions: `list`, `read`, `create`, `update`, `reopen`, `start`, `release`,
`close`, plus `milestone_list`, `milestone_create`, `milestone_set`. Parameter
schema and semantics otherwise mirror the SDK tool, including `IssuePatch`
keep-vs-replace (`labels: []` clears, omitted keeps) and advisory
`content_hash`.

With no active project the tool returns a plain error naming the cause
("no project is selected for this session; issues are per-project"), not a
silent no-op.

The tool resolves its project from `ExecEnv.project_id`, which **already exists
and is already populated** by `Orchestrator::resolve_exec_env` from explicit
`project_ids` or project auto-detection. It is currently dropped before
reaching tool registration (`execute_inner` takes `_session_ctx` unused); this
design threads it through `execute_inner` → `run_agent` →
`register_tools_from_cspace_gated`.

## 6. Milestones

The SDK owns `IssueMeta` and drops unknown frontmatter keys on write, so a
milestone field cannot be added to the issue file without forking the type.
Membership therefore rides a reserved label and the milestone's own record
lives beside the issues:

- **Membership:** label `milestone:<slug>` on the issue. Round-trips safely
  through the SDK, appears in every existing filter and list view, and is
  visible in the raw file.
- **Definition:** `milestones.yaml` in the project directory.

```yaml
milestones:
  - slug: v0-4
    title: v0.4 release
    description: Issue tracker and todo UI
    due: 2026-09-30        # optional
    status: open           # open | closed
    created_at: 2026-08-29T10:00:00Z
    updated_at: 2026-08-29T10:00:00Z
```

Written atomically (temp+rename), same as issues. An issue carries at most one
`milestone:` label; setting a new one replaces the old.

Progress (`closed / total`) is derived by listing issues with the label — never
stored, so it cannot go stale.

Deleting a milestone strips its label from every member issue in one pass;
partial failure leaves the label in place and reports which issues were not
updated, rather than orphaning members silently.

## 7. Todo

### 7.1 State

`OxiosTodoState` implements `oxicode_sdk::TodoStateProvider` over
`Arc<RwLock<Vec<TodoPhase>>>`, delegating to `oxicode_agent::tools::todo::apply_ops`
for the op semantics (three-state normalisation, single in-progress task per
phase, auto-promotion on completion). The kernel already depends on
`oxicode-agent` directly, so the helpers the SDK does not re-export
(`apply_ops`, `format_summary`) are available without a new dependency.

`TodoRegistry` on `KernelHandle` maps session id → `Arc<OxiosTodoState>`, so a
plan survives across the turns of a session. `AgentConfig.todo` is set from the
registry per run; `todo_reminders_enabled` stays on so the SDK's stop-time
incomplete-todo reminder works, and `todo_eager_mode` stays `Off` (we do not
force a plan on every turn).

### 7.2 Tool

An Oxios `TodoTool` wrapping the SDK contract, for one reason: the SDK tool
ignores its `tool_call_id`, so it cannot publish a structured snapshot for the
UI. Ours applies the ops, stashes the resulting phases into the structured
result bus keyed by `tool_call_id`, and returns the same text summary to the
model. Name, schema, and model-facing output are unchanged — this mirrors
`KernelWebSearchTool`, which wraps the SDK search tool for exactly the same
reason.

## 8. Structured tool results

`SearchResultBus` (added with the web_search work) is already a generic
`HashMap<String, Value>` keyed by `tool_call_id`; only its name and module are
search-specific. Rather than add a second and third bus for todo and issue, it
moves to `tools/structured_results.rs` as `StructuredResultBus`, and
`KernelHandle.search_results` becomes `KernelHandle.tool_results`. Six call
sites; no behaviour change.

The whole delivery path then already exists and needs no new plumbing:

```text
tool stashes payload  →  StructuredResultBus[tool_call_id]
  →  agent-runtime completion callback takes it
  →  KernelEvent::ToolExecutionFinished.results
  →  WS tool_end.results
  →  web stream adapter  →  ToolRenderProps.result
```

No new kernel event is introduced. The sticky todo bar derives its state from
the most recent `todo` tool result in the session, which the web store already
receives.

## 9. API

```text
GET    /api/projects/:id/issues?status=&priority=&label=&milestone=&text=
POST   /api/projects/:id/issues
GET    /api/projects/:id/issues/:num
PATCH  /api/projects/:id/issues/:num          # IssuePatch semantics + content_hash
POST   /api/projects/:id/issues/:num/close
POST   /api/projects/:id/issues/:num/reopen
POST   /api/projects/:id/issues/:num/release  # force-release a stale assignment

GET    /api/projects/:id/milestones
POST   /api/projects/:id/milestones
PATCH  /api/projects/:id/milestones/:slug
DELETE /api/projects/:id/milestones/:slug
```

Issue responses carry `content_hash` so the client can send it back on `PATCH`
and get a real conflict instead of a lost update. `409` on `Conflict`, `423` on
`Assigned` (with the owning session and acquisition time), `404` on `NotFound`.

## 10. Web UI

### 10.1 Todo in chat

- `chat/tool-renders/Todo.tsx`, registered for `todo`. Phase-grouped checklist
  using the SDK's status glyphs (`☐ ▶ ☑ ✗ ⏸`), completed items struck through,
  the in-progress item emphasised, blocked items showing their reason.
  Collapses past a threshold with a "N more" affordance. Updates in place as
  successive `todo` calls land in the same turn.
- `chat/todo-progress-bar.tsx`, sticky above the composer while the session's
  latest snapshot has open tasks: `3/7 · <current task>`. Clicking scrolls to
  the newest todo card. Hidden when every task is closed.
- Chat store gains `todoBySession`, updated from any `tool_end` whose
  `tool_name` is `todo`.

### 10.2 Issues in the project page

`routes/projects/$projectId.tsx` gains two tabs beside the existing detail
content:

- **Issues** — list with status/priority/label/milestone filters and a text
  search, matching the filter vocabulary the engine's TUI panel uses. Rows show
  number, title, priority, labels, milestone, and an assignment badge when a
  live session holds the issue. Detail view renders the markdown body and
  allows edit, close, and reopen.
- **Milestones** — cards with title, due date, `closed / total` progress, and
  the member issue list. Create, edit, close, delete.

`chat/tool-renders/Issue.tsx` renders `issue` tool calls in the transcript:
action, affected issue, and resulting state.

All new strings land in both `en.json` and `ko.json` (the web UI is bilingual
per `AGENTS.md`).

## 11. Home migration (separate track)

`~/.oxi/oxios/` is where the whole Oxios home belongs, but moving it is a
separate change: 216 hardcoded `.oxios` sites across ~30 files, a retarget of
`OXICODE_HOME` (which `credential.rs` uses to place the SDK auth store), and a
migration for existing installs.

That change is worth doing on its own merits, because it *simplifies* the
security model rather than complicating it. `~/.oxios` needs a 15-entry
`OXIOS_HOME_DENY_SUBPATHS` list plus a parity test for one reason: the agent
workspace lives inside it and cannot be denied. Moving the workspace out to a
visible `~/oxios/` — where user-facing working files belong anyway — lets
`~/.oxi` stay a whole-root deny and deletes the sub-path list entirely.

Nothing in this design blocks on that, and nothing here has to move when it
happens.

## 12. Implementation sequence

1. **SDK 0.77.0 bump.** `oxicode-sdk`, `oxicode-agent`, `oxicode-ai`
   0.73.0 → 0.77.0. *(done — zero breakages; `cargo check --workspace
   --all-features` clean)*
2. **Structured result bus generalisation.** Rename and relocate; no behaviour
   change.
3. **Project data paths.** `project/paths.rs`, project-delete cleanup.
4. **Milestone store.** Types, atomic YAML read/write, label helpers.
5. **`IssueApi`.** Per-project store cache, ownership registry, issue and
   milestone operations.
6. **Ownership wiring.** Thread `ExecEnv.project_id` through `execute_inner` →
   `run_agent` → registration; set `AgentConfig.session_id`.
7. **Agent tools.** Oxios `IssueTool` and `TodoTool`; `TodoRegistry`; register
   in `kernel_bridge.rs`; CSpace capability entries.
8. **HTTP routes.** Issue and milestone endpoints; error mapping.
9. **Web — todo.** Render, sticky bar, store, i18n.
10. **Web — issues.** Project tabs, issue tool render, i18n.
11. **Docs.** `AGENTS.md` gotchas, `ARCHITECTURE.md` subsystem entry,
    `CHANGELOG.md`.

## 13. Verification

**Kernel**
- Issue numbering is monotonic per project and independent across projects.
- CAS: a stale hash from the agent tool auto-reconciles; a stale hash on the
  HTTP `PATCH` returns 409.
- Ownership: two distinct live sessions collide on `start`; a session whose
  guard is dropped is reclaimed; an empty ownership id never reaches the store.
- The ownership id is identical on turn 1 and turn 2 of a session.
- Milestones: label round-trips through the SDK write path; progress is derived
  correctly; deletion strips labels from all members.
- Todo: phases survive across two turns of one session; two sessions do not
  share a plan.

**Security**
- Existing `gate.rs` parity tests still pass unchanged.
- A file tool targeting `~/.oxi/oxios/projects/**` is denied.
- The `issue` tool succeeds against the same path.

**Web**
- Todo card renders every status; the sticky bar appears and disappears on the
  right transitions.
- Issue list filters compose; conflict and assignment errors surface as
  messages, not silent failures.

**Gates**
`cargo fmt --check` · `clippy --workspace --all-features -D warnings` ·
`cargo nextest run --workspace` · `cargo test --workspace --doc` · web
typecheck, tests, Biome, build.

## 14. Acceptance criteria

1. Every project has an issue tracker; a project with no folders behaves
   exactly like one with folders.
2. Issue numbers are project-scoped and stable.
3. The agent creates, claims, updates, and closes issues through the `issue`
   tool, and cannot reach the files through any file tool.
4. Two concurrent sessions cannot both hold one issue; a crash releases the
   claim without a timeout.
5. Milestones group issues, show derived progress, and survive an SDK write.
6. A todo plan created in one turn is still there in the next turn of the same
   session, renders as a live checklist in the transcript, and shows progress
   above the composer while tasks remain open.
7. `~/.oxios/` gains nothing new; no security constant changes.
