# Oxios Web IA Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:using-git-worktrees` before creating a worktree, then use `superpowers:subagent-driven-development` or `superpowers:executing-plans` to implement the assigned workstream task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the clean-major-version Oxios web information architecture:
one sidebar switcher for Operate, Knowledge, and Studio; project-optional
Studio; Automation as the scheduled-prompt execution domain; and truthful
cross-surface supervision and context handoffs.

**Architecture:** This is a destructive product/API cutover, not a redirect
layer. The work is split into four mergeable worktrees with explicit file
ownership and two integration waves. The automation and shared-shell branches
can start from the same clean baseline. After both merge, Studio and
Operate/Knowledge can proceed concurrently from that integration commit.

**Tech Stack:** Rust 2024, Tokio, Axum, Serde, SQLite; React, TypeScript 5,
TanStack Router/Query, Zustand, i18next, Tailwind, Vitest/MSW, Bun.

**Spec:**
[`2026-08-31-oxios-web-information-architecture-design.md`](2026-08-31-oxios-web-information-architecture-design.md)
is the authoritative product design. The mockups in its §3 are visual
references. `DESIGN.md` remains the design-token and visual-language authority.

## Global constraints

- This is a major-version break: do not introduce redirects, aliases, old
  local-storage translators, deprecated exports, or compatibility endpoints for
  Console/Brain/Chat/Task names.
- User-facing web UI remains bilingual Korean/English. Structural tool output,
  backend error source strings, and Rust comments remain English.
- The only top-level surfaces are `operate`, `knowledge`, and `studio`.
  `knowledge` is not a pseudo-mode; Brain is a named Knowledge subdomain.
- A Studio session may have no project. No-project is a valid state, not an
  error or a hidden default project. Project binding changes future turns only.
- A persona's visual presentation is server-derived. The client may never infer
  a lens, tool affordance, or permission from persona name, role, category, or
  display copy.
- Studio has exactly one persistent selector for session defaults:
  `project · persona · model · Brain` in `StudioContextBar`. The composer does
  not repeat those selectors. A one-turn override, if shipped, lives in a
  collapsed `Turn options` popover and resets after send.
- An Automation is a manual/cron/heartbeat saved instruction that launches an
  Automation Run. It is not a backlog item. Delete Task-only planning features
  (parent/assignee/priority/sort/dependencies/comments) instead of renaming
  them; Issues and Milestones remain the project planning domain.
- A surface, a presentation lens, a launch intent, and a project binding never
  grant filesystem, tool, model, or approval authority. Existing server gates
  remain the source of truth.
- Preserve one chat turn identity across streaming sink, turn registry,
  session, and `ExecEnv.session_id`. Do not add a Studio-specific message or
  event bus.
- Follow the existing structured tool-result bus and do not create a second
  event path for Automation or Operate panels.
- Do not delete arbitrary project roots, vault documents, or credentials. The
  major-release state reset may target only the explicitly named Oxios-managed
  state files.
- Run generated TanStack route-tree output using the repository's route
  generation command; never hand-edit `web/src/routeTree.gen.ts`.

## Delivery topology

```text
integration baseline
  ├─ WT-1 automation-domain ───┐
  └─ WT-2 surface-shell ───────┴─ merge + full CI = integration-1
                                      ├─ WT-3 studio ────────────────┐
                                      └─ WT-4 operate-knowledge ──────┴─ merge + full CI = major-release candidate
```

Each worker creates its worktree from the indicated base branch, owns only the
listed files, and rebases before handoff. A worker must not broaden its scope
to resolve another worker's conflict: report the exact conflicting commit/file
to the integrator instead.

| Worktree | Branch name | Starts from | Deliverable | Merge order |
|---|---|---|---|---|
| WT-1 | `codex/automation-domain` | integration baseline | Automation kernel/API/web domain with no Task public surface | 1 (parallel with WT-2) |
| WT-2 | `codex/surface-shell` | integration baseline | route tree, global shell, sidebar/bottom switcher, shared surface contracts | 1 (parallel with WT-1) |
| WT-3 | `codex/studio-surface` | `integration-1` | projectless/project-bound Studio and non-duplicated context controls | 2 (parallel with WT-4) |
| WT-4 | `codex/operate-knowledge` | `integration-1` | Operate/Knowledge route composition and truthful source-backed panels | 2 (parallel with WT-3) |

### Integration contracts

These contracts are frozen before any WT-3 or WT-4 coding. The integrator owns
their final reconciliation.

```ts
// web/src/types/surfaces.ts — owned by WT-2
export const SURFACES = ['operate', 'knowledge', 'studio'] as const
export type SurfaceId = (typeof SURFACES)[number]

export function deriveSurface(pathname: string): SurfaceId
```

```rust
// crates/oxios-kernel/src/persona/mod.rs — owned by WT-3
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PresentationLens {
    #[default]
    General,
    Code,
    Research,
    Operations,
    Writing,
}
```

```ts
// web/src/types/index.ts — owned by WT-3
export type PresentationLens = 'general' | 'code' | 'research' | 'operations' | 'writing'

export interface EffectiveProfile {
  persona_id: string
  tool_profile: ToolProfileName
  affordances: Affordance[]
  presentation_lens: PresentationLens
}

export interface StudioLaunchIntent {
  source: 'operate' | 'knowledge' | 'automation' | 'project'
  projectId?: string
  automationId?: string
  runId?: string
  sessionId?: string
  contextRefs?: StudioContextRef[]
  initialPrompt?: string
}
```

WT-1 owns the exact `Automation`, `AutomationRun`, trigger, status, verifier,
and immutable context-snapshot definitions listed in its data contract below.

WT-4 consumes Automation only through `/api/operate/*` projections. It must
not import `web/src/components/automation/*` or call store internals. WT-3
links to `/studio/automations/:automationId`, but does not own Automation CRUD.

## Repository map and ownership

| Area | Existing files to read first | Worktree owner | Result |
|---|---|---|---|
| Existing route generation | `web/src/routes/*`, `web/src/main.tsx`, `web/package.json` | WT-2 | canonical `/operate`, `/knowledge`, `/studio` hierarchy |
| Shared chrome | `web/src/components/layout/{app-layout,sidebar,mode-tabs,bottom-nav}.tsx`, `web/src/stores/sidebar.ts` | WT-2 | `SurfaceSwitcher` and `SurfaceSidebar` composition |
| Task execution | `crates/oxios-kernel/src/task/*`, `src/api/routes/task_routes.rs`, `src/api/plugin.rs`, `src/kernel.rs` | WT-1 | Automation storage, runner, routes, tool, scheduler |
| Task web UI | `web/src/routes/tasks.tsx`, `web/src/hooks/use-tasks.ts`, `web/src/types/task.ts`, `web/src/components/{task,cron}` | WT-1 | Automation Center, hooks/types/components |
| Conversation substrate | `web/src/routes/chat.tsx`, `web/src/stores/chat.ts`, `web/src/components/chat/*`, `web/src/components/workbench/*` | WT-3 | Studio route and stable conversation-first workbench |
| Persona projection | `crates/oxios-kernel/src/persona/mod.rs`, `src/api/routes/chat.rs`, `src/api/routes/events.rs`, `web/src/hooks/use-effective-profile.ts` | WT-3 | server-derived presentation lens |
| Current dashboard and agents | `web/src/routes/index.tsx`, `web/src/routes/agents/*`, `web/src/components/dashboard/*`, `web/src/hooks/use-approvals.ts` | WT-4 | Operate Attention and Run Center compositions |
| Current Brain/Knowledge | `web/src/routes/brain/*`, `web/src/routes/knowledge/*`, `web/src/components/{brain,knowledge}/*`, `web/src/hooks/use-brain.ts` | WT-4 | Knowledge memory/library workspace |

## WT-1 — Automation domain

**Objective:** Replace the Task runtime and API with the smaller, accurate
Automation domain. This worktree has no dependency on the new sidebar or
Studio route; it supplies the `/api/automations` contract consumed later.

**Files owned:**

- Rename directory: `crates/oxios-kernel/src/task/` →
  `crates/oxios-kernel/src/automation/`.
- Modify: `crates/oxios-kernel/src/lib.rs`,
  `crates/oxios-kernel/src/kernel_handle/mod.rs`,
  `crates/oxios-kernel/src/tools/{builtin/mod.rs,registration.rs}`, and
  `crates/oxios-kernel/src/capability/descriptor.rs`.
- Rename: `crates/oxios-kernel/src/tools/builtin/task_tool.rs` →
  `automation_tool.rs`.
- Rename: `src/api/routes/task_routes.rs` → `automation_routes.rs`; modify
  `src/api/routes/mod.rs`, `src/api/{server.rs,plugin.rs}`, and `src/kernel.rs`.
- Rename: `web/src/{types/task.ts,hooks/use-tasks.ts,routes/tasks.tsx}` to
  `automation.ts`, `use-automations.ts`, and `routes/studio/automations.tsx`.
- Rename owned components: `web/src/components/task/` →
  `web/src/components/automation/`; move only schedule/template components
  that are Automation-specific from `components/cron/` into
  `components/automation/`.
- Modify owned tests and MSW handlers that assert `/api/tasks`.

**Do not modify:** sidebar/layout files (WT-2), Studio conversation files
(WT-3), or `/api/operate/*` projections (WT-4).

**Data contract to implement:**

```rust
pub struct Automation {
    pub id: String,
    pub name: String,
    pub instruction: String,
    pub description: Option<String>,
    pub trigger: AutomationTrigger, // manual | cron | heartbeat
    pub cron_pattern: Option<String>,
    pub timezone: Option<String>,
    pub heartbeat_interval_secs: Option<u64>,
    pub max_executions: Option<u32>,
    pub execution_count: u32,
    pub persona_id: Option<String>,
    pub project_id: Option<String>,
    pub brain_space: Option<String>,
    pub verify: AutomationVerifyConfig,
    pub status: AutomationStatus,
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub struct AutomationRun {
    pub id: String,
    pub automation_id: String,
    pub session_id: Option<String>,
    pub trigger: AutomationRunTrigger, // manual | cron | heartbeat
    pub status: AutomationRunStatus,   // running | succeeded | failed | canceled
    pub context_snapshot: AutomationContextSnapshot,
    pub summary: Option<String>,
    pub result_content: Option<String>,
    pub error: Option<String>,
    pub cost_usd: Option<f64>,
    pub tokens_used: Option<u64>,
    pub started_at: String,
    pub completed_at: Option<String>,
}
```

`AutomationContextSnapshot` contains only the selected IDs/values at run
start: persona ID, project ID, Brain space, trigger, verification setting, and
instruction. The runner must use that snapshot for the whole execution. It may
not reload a changed Automation midway through a run.

### Task 1.1: Establish the Automation model and SQLite store

- [ ] Move the task module to `automation`, rename public types and methods
  (`TaskStore` → `AutomationStore`, `create_task` → `create_automation`, etc.),
  and update all Rust imports in the owned file list.
- [ ] Delete model fields and tables exclusively supporting Task planning:
  `identifier`, `priority`, `sort_order`, `parent_task_id`, assignee/creator
  task fields, dependencies, and comments. Do not leave serialised ignored
  fields or old `serde(alias = ...)` names.
- [ ] Add `persona_id`, `project_id`, and `brain_space` to the Automation
  definition. They are nullable; `None` means projectless/default-bound, never
  a string sentinel.
- [ ] Replace the `tasks`/`task_runs` schema with `automations`/
  `automation_runs`. The assembler opens exactly
  `"{workspace}/automations.db"`, not `tasks.db`. The major-version release
  notes must identify `tasks.db` as an Oxios-managed retired state file; no
  runtime code deletes it.
- [ ] Store an `AutomationContextSnapshot` JSON value in the run row before
  execution begins. Add a store method whose transaction both creates this run
  and marks the definition execution state, preventing an in-flight execution
  from observing later edits.
- [ ] Write unit tests in `automation/model.rs` and `automation/store.rs` for:
  manual/cron/heartbeat validation; absent project ID; schedule next-run
  calculation; paused/exhausted exclusion; crash recovery; and immutable
  snapshot behavior after an Automation edit.
- [ ] Run:

  ```bash
  cargo test -p oxios-kernel automation::
  cargo fmt --all -- --check
  ```

### Task 1.2: Rename the execution tool, lifecycle wiring, and HTTP API

- [ ] Rename `TaskTool` to `AutomationTool`; its only operations are
  `create`, `list`, `get`, `update`, `pause`, `resume`, `run`, `runs`, and
  `delete`. Remove comments/dependency/backlog actions from schema, help text,
  dispatch, capability descriptor, and tests.
- [ ] Update both tool registration paths and the gate exemption lists together
  if the Automation tool is always-on. Preserve the existing registration
  invariant test; add Automation to it rather than bypassing a gate.
- [ ] Rename the runner to `execute_automation_run`. Its `run_goal` invocation
  must receive the immutable run snapshot's instruction/context and preserve
  the existing timeout + verify/repair deadline semantics.
- [ ] Rename `AppState.task_store`, `KernelHandle::with_task_store`, server
  construction, and the 60-second auto-run loop to Automation names. Remove
  dependency deferral logic entirely. Retain stranded-run recovery, cron
  normalisation, heartbeat, max-executions, and bounded scheduling.
- [ ] Replace `src/api/routes/task_routes.rs` with
  `automation_routes.rs`; expose only:

  ```text
  GET/POST   /api/automations
  GET/PUT/DELETE /api/automations/:id
  PUT        /api/automations/:id/status
  PUT        /api/automations/:id/trigger
  PUT        /api/automations/:id/verify
  POST       /api/automations/:id/run
  GET        /api/automations/:id/runs
  ```

  Return `404`/router fallback for every `/api/tasks*` path; do not register a
  compatibility route.
- [ ] Update API route tests to create an Automation, schedule a cron run,
  start a manual run, assert that the stored run has its context snapshot, and
  assert `GET /api/tasks` does not resolve.
- [ ] Run:

  ```bash
  cargo test -p oxios -- api::routes
  cargo test -p oxios-kernel automation:: tools::builtin::automation_tool
  cargo clippy --workspace --all-features -- -D warnings
  ```

### Task 1.3: Build the Automation web domain without assuming Studio chrome

- [ ] Replace `Task*` TypeScript types with `Automation*` equivalents. Do not
  preserve TypeScript aliases. Model `projectId?: string | null`,
  `personaId?: string | null`, `brainSpace?: string | null`, and the immutable
  run `contextSnapshot` explicitly.
- [ ] Replace `use-tasks.ts` with `use-automations.ts`, use query keys rooted
  at `['automations']`, and call only `/api/automations`. Invalidate the
  definition and per-definition run-history keys after mutation.
- [ ] Refactor `tasks.tsx` into a route component exported from
  `routes/studio/automations.tsx`; it may render standalone until WT-3 embeds
  it in the Studio shell. Rename visible copy to Automation and remove Gantt,
  parent/dependency/comment controls.
- [ ] Keep and rename the useful editor components: trigger editor,
  verification editor, template gallery, run-history list, list/detail form.
  Add explicit selectors for project (including `No project`), persona, and
  Brain; selection is persisted on the definition, not inferred from the
  browser's current Studio conversation.
- [ ] Write component tests with MSW for: projectless create; cron create;
  pause/resume; test run; immutable run-context display; no run history; and
  no calls to `/api/tasks`.
- [ ] Run:

  ```bash
  cd web
  bun run typecheck
  bun run test -- use-automations automation
  bun run lint
  ```

- [ ] Commit only owned files:

  ```bash
  git add crates/oxios-kernel/src/automation crates/oxios-kernel/src/tools \
    src/api src/kernel.rs web/src/types/automation.ts web/src/hooks/use-automations.ts \
    web/src/components/automation web/src/routes/studio/automations.tsx web/src/__tests__
  git commit -m "feat(automation): replace scheduled tasks with automations"
  ```

## WT-2 — Surface shell and destructive route topology

**Objective:** Replace legacy mode names with the shared Operate/Knowledge/
Studio shell. This worktree establishes navigation and URL ownership, but does
not move conversation, automation, dashboard, or knowledge contents.

**Files owned:**

- Rename: `web/src/components/layout/mode-tabs.tsx` → `surface-switcher.tsx`.
- Modify: `web/src/components/layout/{app-layout,sidebar,bottom-nav}.tsx`,
  `web/src/stores/sidebar.ts`, `web/src/components/layout/command-palette/control.tsx`,
  i18n primary navigation keys, and all tests for these units.
- Create: `web/src/types/surfaces.ts`,
  `web/src/components/layout/{surface-sidebar,operate-sidebar,knowledge-sidebar,studio-sidebar}.tsx`.
- Create route layout/placeholder files for `/operate`, `/knowledge`, and
  `/studio`, without claiming the leaf routes owned by WT-1/3/4.
- Do not delete legacy content route files in this worktree. The owner that
  creates each replacement deletes its own legacy file in the second wave;
  regenerate `routeTree.gen.ts` locally through its script but leave its final
  committed form to the integration branch.

**Do not modify:** Automation domain types/routes (WT-1), chat store and
message/composer/workbench components (WT-3), or dashboard/Brain/Knowledge
content components (WT-4).

### Task 2.1: Define surfaces and route ownership

- [ ] Add `web/src/types/surfaces.ts` with `SURFACES`, `SurfaceId`, and
  `deriveSurface(pathname)`. It must map `/operate` to `operate`, every
  `/knowledge` route to `knowledge`, every `/studio` route to `studio`, and
  return `operate` for unknown paths during router error rendering.
- [ ] Add unit tests covering exact root, nested path, trailing slash, and
  unknown path values. The test must prove that no result is `console`,
  `brain`, or `chat`.
- [ ] Create parent route files so the final tree has the route ownership
  specified in the product design §10. WT-2 owns only parent/index shell
  files; reserve `studio/automations*` for WT-1, `studio/sessions/*` for WT-3,
  and substantive `operate/*` and `knowledge/*` leaves for WT-4.
- [ ] Create only the `/operate`, `/knowledge`, and `/studio` parent routes and
  their shell-safe empty states. WT-1 deletes `/tasks`; WT-3 deletes `/chat`
  and `/sessions/*`; WT-4 deletes `/brain/*`, legacy `/knowledge/*`, and the
  root dashboard. None of those workers may add TanStack `redirect` or
  `beforeLoad` compatibility behavior. `/` is a normal not-found after the
  cutover, with `/operate` as the documented replacement.
- [ ] Regenerate the route tree using the project script and run:

  ```bash
  cd web
  bun run typecheck
  bun run test -- sidebar route
  ```

### Task 2.2: Replace ModeTabs with the one shared SurfaceSwitcher

- [ ] Replace `SidebarMode` and its persisted storage key with `SurfaceId`.
  Delete, rather than translate, existing sidebar mode persistence; preserve
  only the generic sidebar collapsed/mobile-open state if it has no legacy
  mode value.
- [ ] Implement `SurfaceSwitcher` immediately beneath the Oxios identity in
  `Sidebar`. Expanded mode is the horizontal three-label tab control;
  collapsed mode is the existing vertical icon treatment. Its links are
  `/operate`, `/knowledge`, and `/studio`.
- [ ] Implement `SurfaceSidebar`, selected strictly from `deriveSurface`.
  `OperateSidebar`, `KnowledgeSidebar`, and `StudioSidebar` own only their
  local navigation groups; no sidebar renders global quick links from a legacy
  mode.
- [ ] Move the existing ChatSessionNav grouping into `StudioSidebar` only as a
  temporary data consumer. Rename its visible `Unfiled` section to `No
  project`. Do not change session behavior in this worktree.
- [ ] Update `BottomNav` to render the same `SURFACES` constant. The mobile
  drawer contains only the active surface's local tree; it has no second
  top-level switcher.
- [ ] Write component tests for desktop/collapsed/mobile active state,
  accessible labels, hrefs, `No project` presence, and one switcher only.

### Task 2.3: Make AppLayout surface-aware without special legacy branches

- [ ] Remove `isChat`, `isKnowledge`, and `/brain/knowledge` branches from
  `AppLayout`. Introduce a route-level layout contract: Studio occupies a
  full-height outlet; Knowledge may opt into its reader/inspector layout;
  Operate is scrollable page content.
- [ ] Keep global SSE, notifications, command palette, shortcuts, and the
  shared sidebar mounted exactly once. Do not remount global providers when a
  surface changes.
- [ ] Rename command palette groups and primary verbs to Operate/Knowledge/
  Studio. Preserve actual commands but remove display copy that calls a
  surface Console, Brain, Chat, or Task.
- [ ] Add/adjust i18n keys in both locales together. Verify primary navigation
  has no fallback English key in Korean mode.
- [ ] Run:

  ```bash
  cd web
  bun run typecheck
  bun run test -- app-layout surface-switcher bottom-nav
  bun run lint
  bun run build
  ```

- [ ] Commit only owned files:

  ```bash
  git add web/src/components/layout web/src/stores/sidebar.ts web/src/types/surfaces.ts \
    web/src/routes/operate web/src/routes/knowledge web/src/routes/studio \
    web/src/i18n web/src/__tests__
  git commit -m "feat(web): replace legacy modes with operate knowledge studio"
  ```

## Integration gate 1 — merge WT-1 and WT-2

The integrator must create `integration-1` only after both branch test suites
pass. Resolve only these expected joins:

- WT-1's `routes/studio/automations.tsx` must sit under WT-2's `/studio`
  parent route; do not move the file back to `/tasks`.
- WT-2 owns the command-palette control entry and changes its route/API copy to
  Automation while preserving its existing control behavior.
- All references to `task_store`, `/api/tasks`, `/tasks`, `TaskTool`, and
  `TaskStatus` must be absent except historic RFC prose.
- All references to `SidebarMode`, `ModeTabs`, and `SIDEBAR_MODES` must be
  absent from shipped code.

Run from the merged branch:

```bash
rg -n --glob '!docs/**' '(/api/tasks|/tasks|TaskTool|TaskStore|SidebarMode|ModeTabs|SIDEBAR_MODES)' .
cargo fmt --all -- --check
cargo check --workspace --all-features
cargo test --workspace
cd web && bun run typecheck && bun run test && bun run lint && bun run build
```

The `rg` command must return no matches. If an intentional non-user-facing
historical comment remains, remove it rather than adding a suppression.

## WT-3 — Studio conversation surface

**Objective:** Make Studio the conversation-first home for both projectless
and project-bound work. It consumes the merged surface shell and Automation
route but owns neither the Automation editor nor navigation architecture.

**Files owned:**

- Rename/recompose: `web/src/routes/chat.tsx` → `web/src/routes/studio/index.tsx`;
  move session route behavior into `web/src/routes/studio/sessions/$sessionId.tsx`.
- Create: `web/src/components/studio/{studio-shell,studio-context-bar,studio-composer,studio-launch-sheet,studio-inspector,turn-options-popover}.tsx` and
  focused tests.
- Modify: `web/src/components/chat/chat-input.tsx`,
  `web/src/components/workbench/WorkbenchShell.tsx`, effective-profile TypeScript
  types/hooks/tests, chat route/store tests, and persona/chat SSE response
  construction in the files below.
- Modify: `crates/oxios-kernel/src/persona/{mod.rs,persistence.rs,manager.rs}`;
  `src/api/{persona_routes.rs,routes/chat.rs,routes/events.rs}`; relevant web
  persona editor/types.

**Do not modify:** `SurfaceSwitcher`/surface store and route parents (WT-2),
Automation CRUD/components (WT-1), or Operate/Knowledge route/components
(WT-4).

### Task 3.1: Add an explicit presentation-lens contract

- [ ] Add `PresentationLens` to the kernel persona model with a serde default
  of `General`. Add the same field to persisted persona snapshots, persona
  create/update request types, persona summaries, and web persona types.
- [ ] Update default personas with explicit lens values in their definitions;
  do not derive a lens from `Persona.name` or `Persona.role`. The persona editor
  offers the five fixed values and describes it as presentation only.
- [ ] Add `presentation_lens` to kernel `EffectiveProfile`; return it from
  `resolve_effective_profile`; include it in session load, chat completion, and
  event payloads wherever `effective_profile` already travels.
- [ ] Update `web/src/types/index.ts`, `use-effective-profile.ts`, store
  hydration, and test fixtures. A missing field from an interrupted rolling
  deploy maps to `general` at the deserialize boundary only; it is not inferred
  elsewhere.
- [ ] Add Rust tests showing a persona with `Research` returns a Research lens
  whether project roots are absent or present, while only affordances vary by
  roots. Add web tests for default fallback and live SSE refresh.
- [ ] Run:

  ```bash
  cargo test -p oxios-kernel persona::
  cargo test -p oxios -- api::routes::chat
  cd web && bun run test -- use-effective-profile affordances
  ```

### Task 3.2: Move the shared conversation into Studio without remounting it

- [ ] Implement `StudioShell` as the full-height descendant of the Studio
  route. It contains `StudioContextBar`, the existing stable transcript DOM,
  an on-demand inspector, and `StudioComposer` in that order.
- [ ] Port the current Chat route's streaming, cancellation, scroll anchoring,
  loading, error/retry, compression, approval, path-access, and block ordering
  unchanged. `WorkbenchShell` stays the profile-gated stage host and must not
  replace or re-key the transcript when persona/lens/project context changes.
- [ ] Implement reading/work width lanes at block level: prose uses a
  `max-w-[760px]` reading lane; code, tables, tool output, diffs, images, and
  artifacts may use a bounded `max-w-[1240px]` work lane. Do not apply one
  persona-wide max width to every block.
- [ ] Drive inspector tabs from present data only: Context/Sources, Activity,
  Outline, and Review. The default is closed for General/Writing/Research;
  it may open during a real Code/Operations live activity event but always has
  explicit close and focus return.
- [ ] Update existing workbench and affordance tests to prove that a Builder
  session is still a transcript-first session and switching lenses preserves
  the transcript node and scroll state.

### Task 3.3: Make context selection single-source and projectless first-class

- [ ] Implement `StudioContextBar` as the sole persistent default selector for
  project, persona, model, and Brain. Each control opens a local menu and
  visibly states whether its change applies from the next turn. Use the current
  session's stored project/persona/model/Brain bindings; do not store a second
  Studio-only copy.
- [ ] Remove persistent project/model/persona controls from `chat-input.tsx`.
  Keep attachments, source mentions, slash commands, tool affordance controls,
  editor, cancellation, and send. Add optional `Turn options` only if it
  implements one-turn overrides that are serialized with one send then cleared.
- [ ] For a fresh `/studio` visit, create or show a first-class projectless
  session with `No project · General · configured model · Not connected/Brain`
  in the context bar. It must not create a project or display a setup warning.
- [ ] Reuse `useChatStore.setActiveProject` and the existing singular
  `project_id` request field. When the user attaches/replaces a project, update
  the session binding through the canonical session API before the next send.
  Existing messages retain their original recorded metadata; do not rewrite
  history or re-run prior turns.
- [ ] Build `StudioLaunchSheet` around `StudioLaunchIntent`: validate source
  references through canonical APIs; offer projectless/current/choose-project;
  render stale or inaccessible references as removable unavailable chips;
  never fall back to a similarly named project/source.
- [ ] Add component/MSW tests for: no-project happy path; attach project then
  assert only next payload includes `project_id`; context-bar change is not
  duplicated in composer; one-turn override clears; unbound Brain stays
  neutral; and a Code persona with zero roots shows no filesystem controls.
- [ ] Run:

  ```bash
  cd web
  bun run typecheck
  bun run test -- studio chat-input workbench-shell affordance-gating
  bun run lint
  bun run build
  ```

- [ ] Commit only owned files:

  ```bash
  git add crates/oxios-kernel/src/persona src/api/persona_routes.rs src/api/routes/chat.rs \
    src/api/routes/events.rs web/src/components/studio web/src/components/chat \
    web/src/components/workbench web/src/routes/studio/index.tsx \
    'web/src/routes/studio/sessions/$sessionId.tsx' web/src/stores/chat.ts \
    web/src/hooks/use-effective-profile.ts web/src/types/index.ts web/src/__tests__
  git commit -m "feat(studio): make conversation context-first and project-optional"
  ```

## WT-4 — Operate and Knowledge surfaces

**Objective:** Compose the remaining two surfaces from truthful existing data
and narrowly scoped projections. This worktree uses the WT-2 sidebar boundary
and the WT-1 Automation API; it does not redefine Automation storage or Studio
conversation mechanics.

**Files owned:**

- Create: `web/src/routes/operate/{index,runs,projects,index.tsx}.tsx` and
  focused `operate/system/*` routes; `web/src/components/operate/*`; matching
  `web/src/hooks/use-operate.ts` and tests.
- Recompose/rename: `web/src/routes/brain/*` and `web/src/routes/knowledge/*`
  under `web/src/routes/knowledge/{memory,library}/*`; create
  `web/src/components/knowledge/knowledge-workspace.tsx` and
  `studio-handoff.tsx`.
- Modify: dashboard/agent/Brain/Knowledge components only as needed to reuse
  them as presentational children.
- Create or modify: `src/api/routes/operate_routes.rs`, source projections in
  existing Brain/Knowledge API modules, and registrations in
  `src/api/routes/mod.rs`.

**Do not modify:** sidebar and parent route shell (WT-2), Automation storage
or endpoints (WT-1), Studio shell/context bar/chat store/persona lens (WT-3).

### Task 4.1: Build source-backed Operate projections and Attention

- [ ] Add `GET /api/operate/attention`, `/api/operate/runs`,
  `/api/operate/projects/:id/context`, and `/api/operate/capabilities` to an
  `operate_routes.rs` module. Each payload contains source IDs, timestamps,
  exact state/reason, available safe actions, and canonical deep route data.
- [ ] Build projections from `KernelHandle` APIs/stores only. Do not add a
  scheduler, lifecycle manager, event bus, or client-side fan-out to infer a
  relationship. Redact tool arguments, credentials, and inaccessible paths.
- [ ] Attention includes only pending approval, explicit failed run, waiting
  input, quota/budget block, and backend-defined long-running state. A healthy
  run is not presented as attention and an absent owner/project is omitted.
- [ ] Implement `OperateAttention`, `RunCenter`, and a bounded `OperateInspector`.
  The inspector order is action/state → recorded origin → exact scope →
  rationale/impact → safe action → canonical detail → typed Studio launch.
- [ ] Implement `ProjectControlRoom` from roots/Git summary, project issue and
  milestone APIs, active runs, and recorded deliverables. Omit a row if no
  source-backed relation exists.
- [ ] Write Axum tests for no attention, one approval, failed Automation Run,
  redacted capability records, missing project relationship, and invalid
  project ID. Write component/MSW tests for panel-local API failure and
  `Investigate in Studio` launch-intent payload.

### Task 4.2: Build the explicit Knowledge memory/library workspace

- [ ] Move Brain routes to `/knowledge/memory/{index,search,entities,
  contradictions,timeline}` and Library routes to `/knowledge/library/{index,
  journal,graph,assets}`. Remove old `/brain/*` and flat `/knowledge/*` files;
  do not register route redirects.
- [ ] Implement `KnowledgeWorkspace` with Context list, reader/synthesis
  canvas, and source inspector. Each row displays a source-kind badge:
  `Memory / Brain` or `Library / Knowledge`. Memory and Library queries remain
  distinct; a combined list may interleave results only after each item carries
  its source type and backing ID/path.
- [ ] Add `GET /api/knowledge/context` only if existing APIs cannot provide
  typed source records in one request. Its entries must have `kind`, title or
  path, date when supplied, excerpt, backing identity, and relation metadata.
  Do not manufacture confidence, coverage, or provenance values.
- [ ] Implement `Add to Studio` through the `StudioLaunchIntent` contract:
  projectless, current Studio session when browser state contains one, and
  explicit project/session picker. It must not write source content into
  memory/library merely to make the handoff work.
- [ ] Test source type labeling, unbound Brain neutral state, missing source
  metadata, projectless handoff, explicit chosen-project handoff, stale source
  chip, keyboard focus return, and panel-local failure.

### Task 4.3: Integrate local navigation and release independent views

- [ ] Populate WT-2's `OperateSidebar` groups: Now, Projects, Runs, System;
  and `KnowledgeSidebar` groups: Memory / Brain and Library / Knowledge. Do
  not change the surface switcher itself.
- [ ] Replace legacy dashboard usage with `/operate` Attention as home content;
  migrate dashboard metrics only where they are a source-backed answer to an
  Operate question. Do not preserve a passive card grid simply to fill space.
- [ ] Add `Open in Operate` links for exceptional Automation runs using the
  Automation ID/run ID. This link must not expose or edit Automation trigger
  data in Operate.
- [ ] Update i18n in both locales and add no-data/error/loading behavior for
  zero roots, no sources, unbound Brain, no attention, and partial projection
  failure.
- [ ] Run:

  ```bash
  cargo test -p oxios -- api::routes::operate
  cd web
  bun run typecheck
  bun run test -- operate knowledge
  bun run lint
  bun run build
  ```

- [ ] Commit only owned files:

  ```bash
  git add src/api/routes web/src/routes/operate web/src/routes/knowledge \
    web/src/components/operate web/src/components/knowledge web/src/hooks/use-operate.ts \
    web/src/i18n web/src/__tests__
  git commit -m "feat(web): add operate and knowledge surfaces"
  ```

## Final integration and release plan

### Task 5.1: Merge the second wave

- [ ] Rebase WT-3 and WT-4 onto the same `integration-1` commit before review.
- [ ] Merge WT-3 first, then WT-4. Resolve only typed launch-intent and route
  references in the integration branch; do not duplicate a Studio context bar
  inside Knowledge or Operate.
- [ ] Confirm Studio automation links target WT-1's exact
  `/studio/automations/:automationId` route and Operate deep links preserve
  Automation/run IDs without passing prompt text.
- [ ] Confirm no legacy route file is still imported. Regenerate and commit the
  single final `web/src/routeTree.gen.ts` after all file-route changes; never
  resolve generator output by hand.

### Task 5.2: Conduct destructive-cutover audit

- [ ] Search shipped source, generated route tree, i18n, and web tests for
  old public terminology and paths:

  ```bash
  rg -n --glob '!docs/**' --glob '!CHANGELOG.md' \
    '(Console|Chat|Task|/chat|/tasks|/api/tasks|SidebarMode|ModeTabs)' \
    crates src web
  ```

  Every result must be either a non-product protocol term outside this change
  (for example Tokio `task`) or be renamed. Do not suppress web/product
  matches.
- [ ] Verify the only persistent reset targets are the old browser storage keys
  and exact Oxios-managed task database path. Publish release notes that tell
  the user the precise state path and do not run deletion automatically.
- [ ] Verify Agent lifecycle remains owned by `AgentLifecycleManager` and
  `Supervisor`, not Operate; the kernel remains monolithic and
  `oxicode-sdk` remains a crates.io dependency.

### Task 5.3: Run acceptance smoke tests and full gates

- [ ] Browser-test the following in light/dark, desktop/tablet/mobile,
  keyboard-only, and reduced-motion modes:

  1. Start a projectless Studio conversation and send a General turn.
  2. Attach a project and prove only the subsequent request contains the
     project ID; detach to return to `No project`.
  3. Switch to a Builder persona and prove the transcript survives while the
     stage appears only for server-derived affordances.
  4. Create a projectless Research Automation, test-run, schedule, pause,
     inspect its immutable run context, and open an exceptional run in Operate.
  5. Add a Library source to a projectless Studio session, then repeat through
     explicit project selection; verify source IDs/paths in the context strip.
  6. Resolve or deny an approval in Operate and launch Studio investigation
     without a fabricated project association.
  7. Exercise no roots, no Brain, no attention, no source, stale source,
     SSE disconnect, cancellation, retry, and partial projection failures.

- [ ] Run the complete repository gates:

  ```bash
  cargo fmt --all -- --check
  cargo clippy --workspace --all-features -- -D warnings
  cargo check --workspace --all-features
  cargo nextest run --workspace --no-fail-fast
  cargo test --workspace --doc
  cargo audit
  cargo deny check

  cd web
  bun install --frozen-lockfile
  bun run typecheck
  bun run test
  bun run lint
  bun run build
  ```

- [ ] Tag only after every gate is green and release notes include the major
  break: three new surfaces, Studio's projectless default, Automation replacing
  Task, removed routes/API aliases, and explicit safe state-reset guidance.

## Implementation completion criteria

1. Sidebar and mobile navigation show only Operate, Knowledge, and Studio;
   exactly one surface switcher exists per viewport.
2. Studio is fully usable without a project and has one persistent context
   selector location; composer duplicate controls do not exist.
3. `presentation_lens` is server-derived and cannot alter access permissions.
4. Automations execute through one runner path, preserve immutable run context,
   and expose no Task planning/comment/dependency APIs.
5. Knowledge distinguishes Brain memory from Library documents while providing
   a validated, typed handoff to Studio.
6. Operate is exception/decision first; it never invents ownership, health,
   source relations, or authority.
7. No old route, public endpoint, client persistence translator, or
   user-facing Task terminology remains in the new release.
