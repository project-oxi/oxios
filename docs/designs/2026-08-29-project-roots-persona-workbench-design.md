# Project Roots and Persona-Adaptive Workbench

> **Date:** 2026-08-29
>
> **Status:** Approved design — implementation intentionally deferred
>
> **Supersedes:** The mount-facing parts of RFC-025 and the mount-dependent
> portions of `2026-06-29-mount-as-referenceable-design.md`; the separate
> `/code`-mode assumption in `2026-08-04-code-workspace-conversation-first-redesign.md`
>
> **User decision:** This is a clean break. Existing `~/.oxios` persisted
> application data may be discarded; no legacy-data migration is required.

## 1. Decision

Oxios has one user-facing filesystem context: a **Project**. A Project owns
zero or more local folder roots. It is selected explicitly for a chat session,
and its roots are the sole source of the agent's current working directory and
filesystem scope.

**Mounts are removed entirely.** They are not renamed, hidden behind a
compatibility layer, or retained as an internal abstraction. The mount database,
manager, API, tool, auto-detection, `@` attachment, metadata fields, UI, and
configuration are deleted.

Oxios has no Chat/Codex mode switch. A session's effective **persona** selects
a bounded tool profile and UI affordances. Coding personas use a
conversation-first workbench; other personas retain a focused chat surface.
Persona selection never grants filesystem authority: the selected Project does.

## 2. Goals

1. Project creation feels like Codex: enter a name, select any number of
   folders, or skip folder selection.
2. A project is the complete, visible explanation for filesystem access.
3. Folderless projects support general chat, writing, research, planning, and
   any work that does not need local files.
4. Coding personas make file work, terminal activity, diffs, and previews easy
   to inspect without turning the product into a permanently visible IDE.
5. The chat transcript reaches a terminal-agent quality bar: truthful live
   activity, durable action provenance, useful tool output, review in context,
   robust cancellation/reconnect behavior, and keyboard-first operation.
6. The clean break leaves no mount terminology or dead fallback paths in the
   runtime or product surface.

## 3. Non-goals

- No automatic discovery or registration of arbitrary local folders from chat
  text. Users explicitly choose project folders.
- No project-level tool profile. Personas request tools; the trusted runtime
  resolves authority from the caller, approval policy, and selected project.
- No separate coding route, separate session type, or duplicated chat
  implementation.
- No automatic migration of existing mounts, projects, persona defaults, or
  session bindings.
- No permanent three-pane IDE shell.

## 4. Project and filesystem model

### 4.1 Data model

The persisted project record is reduced to the fields with durable product
meaning:

```rust
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    /// Canonical, de-duplicated, existing directories. May be empty.
    pub root_paths: Vec<PathBuf>,
    /// Project-specific instructions injected only while this project is active.
    pub instructions: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
}
```

The following legacy fields are removed: `description`, `paths`, `tags`,
`emoji`, `source`, `memory_visible`, and `mount_ids`. The UI derives a stable
project icon from the project name and root metadata when needed; it does not
persist presentation-only project fields.

`root_paths` has the following invariants:

- Every path is absolute, canonicalized, an existing directory, and appears at
  most once.
- Order is intentional. `root_paths[0]` is the process CWD. All roots are
  available to filesystem tools and shown in the workspace context.
- An empty list is valid. It means that this session has no project filesystem
  scope, not that it falls back to an implicit global workspace.
- Removing a root affects subsequent turns only. An already running turn keeps
  its immutable execution snapshot.

### 4.2 Creation and editing flow

The create dialog contains only:

1. **Project name** (required).
2. **Folders** (optional, repeatable): “Choose folder…” opens the native
   macOS directory picker; each chosen directory is shown as a removable row.
3. **Instructions** (optional).

The picker is host-local and is exposed through an authenticated localhost API
that returns only user-confirmed directories. The web client does not attempt
to use a browser file-input value as a filesystem path. The target is macOS
ARM64, so the first implementation uses the native macOS open-directory panel.

Editing uses the same root-list component. It includes an explicit “No folders
yet” state and keeps project deletion separate from folder removal.

### 4.3 Session and runtime data flow

```mermaid
sequenceDiagram
    participant U as User
    participant W as Web chat
    participant K as Kernel
    participant R as Agent runtime
    U->>W: Select Project
    W->>K: turn { project_id, persona_id? }
    K->>K: Load project and snapshot root_paths
    K->>R: ExecEnv { project, allowed_paths, cwd, persona profile }
    R->>R: cwd = root_paths.first(); tools constrained to all roots
```

- `project_id` is the only persisted and transmitted workspace binding.
- A session may be unfiled (`project_id = None`). Selecting a project binds
  the session and starts future turns in that context.
- The project instruction block and root list appear in the agent prompt only
  when a project is active. A folderless project contributes instructions but
  no filesystem paths.
- Runtime path checks intersect the selected project roots with the caller's
  existing authority and approval policy. Projects describe requested context;
  they do not bypass RBAC or path sandboxing.
- A selected project with no roots never enables file, terminal, or directory
  UI on its own. The coding persona can explain how to add folders.

### 4.4 API surface

`ProjectInfo`, create/update routes, tool parameters, session state, and web
types use `root_paths` and `instructions`. Project list/search operates on
name and instructions only.

New local API:

```text
POST /api/system/pick-folders
→ { paths: string[] } | user-cancelled response
```

It is available only to the local authenticated desktop host. The server
validates and canonicalizes selected paths before returning them; the project
write endpoint validates them again. Remote clients receive an explicit
unavailable result instead of a fake file picker.

## 5. Mount removal

Delete all mount-specific code and contracts:

- `mount/` module, `MountManager`, `MountApi`, database schema, migration,
  path promotion, detection, enrichment, and mount tool.
- Kernel/gateway request metadata (`mount_ids`, `mount_tag`), orchestration
  resolution, prompt rendering, and runtime fallback logic.
- Persona `default_mount_ids` and its picker/editing behavior.
- Mount HTTP routes, hooks, components, navigation entry, route, i18n keys,
  `@` mention result type, chips, detection badge, and drag/drop support.
- Project compatibility columns and migration functions.

The path resolution responsibilities move directly to `ProjectManager` and the
execution-environment builder. Project-aware planning and worktree operations
read the selected project's ordered roots directly.

The release reset procedure deletes the Oxios application state at
`~/.oxios/` after the binary and web bundle are updated. It does not delete
`~/.oxicode/`, credentials owned by other products, or arbitrary user project
folders. First boot recreates normal defaults and an empty project store.

## 6. Persona-adaptive shared chat

### 6.1 Resolution model

The session's effective persona is resolved in this order:

```text
turn override → persisted session persona → global default persona
```

The kernel resolves one immutable execution profile for the turn. That profile
provides tool exposure, tool-availability diagnostics, and UI affordances. It
does not mint authority. The project snapshot and approval/RBAC policy remain
separate inputs to the effective capabilities.

The old string capability checks are replaced over time by typed affordances
from the resolved profile. Until that resolver lands, the UI consumes the same
profile-derived capability snapshot emitted for the turn; it must not infer
coding state from persona names or categories alone.

### 6.2 UI families

| Effective affordances | UI family | Default surface |
|---|---|---|
| No filesystem/terminal/review affordances | Focused chat | Transcript and composer |
| File inspection or editing affordance | Conversation-first workbench | Transcript, compact project bar, activity rail |
| Review affordance | Conversation-first workbench | Transcript; diff stage only while changes need review |
| Terminal affordance | Conversation-first workbench | Transcript; terminal drawer only on demand or for a long-lived command |

The category `coding` is a useful preset and picker label, not a second app
mode. A custom persona with equivalent profile affordances receives the same
workbench; a coding-labeled persona without them does not show false controls.

## 7. Conversation-first coding workbench

### 7.1 Shell

The current `/chat` route remains the single chat substrate. When the resolved
profile includes coding affordances, it renders `WorkbenchShell` around the
same conversation component:

```text
Project bar: project name · roots summary · branch/status · model · ⌘K
Activity rail: Conversation | Files | Terminal | Changes | Preview
Main canvas: shared transcript + composer
Conditional stage: Diff | Preview | Terminal (only when relevant)
```

There is no fixed file explorer or editor column. The conversation is always
the widest surface and remains accessible when the stage is hidden.

### 7.2 Emergent surfaces

| Event or action | Result |
|---|---|
| User selects Files or presses `⌘P` | File slide-over or quick-open; selecting a file opens an editor modal |
| Agent edits a file | Inline edit artifact; Diff stage opens if it is focal or changes await review |
| Agent emits a renderable artifact | Inline artifact; Preview stage opens |
| User selects Changes | Diff stage opens and pins while review remains unresolved |
| Long-lived/interactive command or user selects Terminal | Terminal drawer/stage opens |
| Short command, file read, search, or reasoning | Inline transcript artifact only; no stage |

The stage auto-dismisses after a completed non-review artifact unless the user
pinned it. Pending changes pin the diff stage until reviewed or reverted. The
same artifact always remains reachable from the transcript after dismissal.

### 7.3 Project affordances

The project selector is placed in the chat header rather than being a mode
switch. It supports create, switch, edit folders, detach the session, and a
plain “No project” state. The chat sidebar continues to group sessions under
projects, with unfiled sessions visible separately. Selecting a session
rehydrates its project and persona before a turn begins.

For a folderless coding-project session, the shell stays conversational but
shows a compact “Add folders to enable local tools” action in the project bar.
It does not show a misleading file rail, terminal, or directory tree.

## 8. Chat experience quality bar

The workbench reuses and strengthens the primary chat experience rather than
introducing a second renderer. The following are baseline requirements for all
personas unless their effective profile intentionally removes an affordance.

### 8.1 Truthful live work

- Stream sequential reasoning, tool, and response blocks without flattening
  their ordering.
- Show a quiet transcript-tail working state between model/tool phases.
- Correlate every live lifecycle event to the same turn key used for tokens,
  cancellation, persistence, and reconnect replay.
- Stop cancels the server-side turn and its agent, retains partial output, and
  marks it as interrupted rather than successful.
- Socket close flushes received output; reconnect resolves to one complete
  terminal message instead of duplicating or losing blocks.

### 8.2 Action provenance and inspection

- Every significant action produces a concise inline artifact card with type,
  target, status, elapsed time, and a useful one-line result.
- Tool cards distinguish success, pending approval, denial, cancellation, and
  failure; permission reasons remain visible in the expanded view.
- Code edits include a compact diff teaser and open the same change in review.
- Search keeps citations; rendered artifacts and file previews are linked to
  their producing turn.

### 8.3 Conversation ergonomics

- The composer remains rich but does not expose controls that the effective
  persona cannot use.
- `⌘K`, `⌘P`, slash commands, copy, retry, stop, and review are keyboard-first
  and have accessible focus/label semantics.
- Markdown stays readable during streaming, preserves correct code copy
  payloads, and renders inline code as inline content.
- Session loading uses a skeleton/error/retry state rather than stale or blank
  transcript content.

## 9. Error handling and security

- Folder picker cancellation changes nothing; unavailable native picking has a
  clear local-only explanation.
- Invalid, missing, non-directory, duplicate, or unauthorized roots are
  rejected before project persistence with English API error output and
  localized UI messages.
- Removing/deleting a project cannot silently broaden a session's scope. A
  deleted active project detaches the session and subsequent turns run without
  project filesystem context.
- A stale project root is surfaced in project editing; it is never silently
  replaced with an ambient workspace.
- Persona/profile resolution failure is fail-closed for affected tools and UI
  affordances while preserving basic chat.
- Existing tool approvals, RBAC, path sandboxing, audit events, and immutable
  turn snapshots remain mandatory.

## 10. Implementation sequence

1. Add focused contract tests for project roots and session/execution scope.
2. Simplify the project schema, API, and kernel facade; add native local folder
   selection; delete mount persistence and all mount contracts in the same
   change.
3. Rewire orchestration, agent runtime, planner, access checks, chat transport,
   command palette, and persona records from mount bindings to project roots.
4. Replace mount UI/routes/navigation/i18n with the project-root create/edit
   flow and header project selector.
5. Make persona execution-profile affordances the one source for conditional
   chat/workbench controls.
6. Implement `WorkbenchShell`, activity rail, slide-over/quick-open, modal
   editor, terminal drawer, artifact cards, and emergent stage on `/chat`.
7. Close the chat quality gaps in §8 and add integration coverage for streaming,
   cancellation, reconnection, path permissions, and review.
8. Build the new binary/web assets; reset `~/.oxios/`; perform a clean-first-
   boot smoke test.

Every sequence step keeps tool output in English and web UI strings bilingual.
No implementation step may preserve a mount fallback.

## 11. Verification

### Kernel and API

- Project root validation: zero roots, one root, multiple ordered roots,
  duplicate roots, files instead of directories, deleted roots, and invalid
  paths.
- Execution environment: no project/no filesystem scope; folderless project/
  instructions only; multiple roots/CWD is first root; all tool paths are
  constrained to roots and caller authority.
- Project selection persists per session and restores before the next turn.
- The Rust workspace contains no reachable `mount_ids`, `MountManager`, mount
  API route, mount tool, or mount database invocation.
- Folder-picker endpoints are authenticated, cancel safely, and reject remote
  access.

### Web

- Create/edit project flow supports multiple selected folders and an intentional
  folderless project.
- Persona/profile changes update affordances without changing the route or
  losing transcript state.
- Coding affordances show the conversation-first workbench; non-coding
  affordances do not show coding controls.
- Stage emergence, pin/dismiss behavior, editor modal, quick-open, terminal
  drawer, diff review, and artifact-card reopen are unit- and browser-tested.
- English/Korean i18n coverage, TypeScript typecheck, Biome, tests, and build
  are clean.

### Release gates

Run the repository gates mandated by `AGENTS.md`: formatting, all-feature
check/clippy, nextest, doc tests, frozen Bun install, web typecheck/tests/
lint/build, `cargo audit`, and `cargo deny check`. Smoke test a clean
`~/.oxios/` boot with a folderless project and a multi-root coding project.

## 12. Acceptance criteria

1. A new user can create a named project with zero, one, or multiple folders
   using the native folder picker.
2. The selected project's roots are the only project-derived filesystem context
   for an agent; a folderless project grants none.
3. There is no Mount concept in user UI, API, runtime contracts, tools, or
   persistent schema.
4. Changing personas adapts one chat surface; it does not navigate to or
   activate a separate Chat/Codex mode.
5. Coding personas get a conversation-first workbench with only relevant,
   profile-authorized controls and emergent visual surfaces.
6. The transcript remains the durable record of work and meets the quality
   requirements in §8 for both coding and non-coding personas.
