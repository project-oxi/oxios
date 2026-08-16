# RFC-043 Completion — Design

> **Date**: 2026-08-16
> **Status**: Approved (implementing)
> **Scope**: `crates/oxios-kernel/src/task/`, `crates/oxios-kernel/src/tools/builtin/task_tool.rs`, `src/api/routes/task_routes.rs`, `src/api/plugin.rs`, `src/kernel.rs`, `web/src/routes/tasks.tsx`, `web/src/components/task/`, `web/src/routes/cron-jobs.tsx`

## Problem

RFC-043 shipped Phases 1 and 3 only: task CRUD, scheduling, run history, and the
web list/detail page. Missing: the verify gate (the `PUT /:id/verify` endpoint
echoes its input and persists nothing), the `task` agent tool, comments,
dependencies, edit/batch endpoints, cron→task migration, the failure fuse, and
the dependency graph. This design closes all of it.

## Inventory of gaps → decisions

### D1 — TaskStore moves onto KernelHandle

Today `plugin.rs` opens `tasks.db` and `AppState` owns the store, so kernel-side
code (the new `TaskTool`) cannot reach it. The assembler (`src/kernel.rs`)
already follows attach patterns (`with_orchestrator`). Change:

- `src/kernel.rs` opens `TaskStore` at boot (path unchanged:
  `<workspace>/tasks.db`) and attaches it: `KernelHandle.task_store:
  Arc<tokio::sync::Mutex<TaskStore>>`.
- `AppState.task_store` is removed; routes and `spawn_task_auto_run` use
  `state.kernel.task_store.clone()`.

### D2 — Verify gate (RFC §Verify Gate)

- `TaskStore::set_verify(id, SetVerifyParams)` persists `verify_enabled`,
  `verify_requirement`, `verify_max_iterations`, `verify_verifier_agent_id`.
- `execute_task_run` moves from `task_routes.rs` into
  `crates/oxios-kernel/src/task/runner.rs` (kernel crate) so the tool, the
  manual-run endpoint, and the auto-run tick share one path. It loads the task
  itself (needs verify config + instruction).
- Flow, with a **single overall deadline of `timeout_secs`** shared by execution,
  verification, and repair re-runs:
  1. `mark_running(trigger)`.
  2. Run the instruction via `KernelHandle::run_goal` (bounded by remaining
     budget).
  3. If the run succeeded and `verify_enabled`: loop up to
     `verify_max_iterations` times:
     - Ask a **separate verifier conversation** (`run_goal`, fresh session) with
       the instruction, the acceptance criterion
       (`verify_requirement` falling back to the instruction), and the current
       result. The prompt demands a first line of exactly `PASS` or `FAIL`.
     - `PASS` → run finalized as status **`verified`**; task terminal status
       `completed`. Done.
     - `FAIL` → if attempts remain, re-run the goal with the verifier feedback
       appended as repair instructions, then verify again.
     - Exhausted → run finalized `failed` with
       `Verification failed after N attempt(s): <last feedback>`.
  4. Without verify: identical to today.
- Verdict parsing (`parse_verdict`): first non-empty line; leading markdown
  markers/whitespace tolerated; `PASS` (case-insensitive) → pass, `FAIL` → fail;
  anything else → fail with "verifier produced no parseable verdict" plus the
  raw output. Conservative default because an unreadable verdict must not
  silently accept work.
- `mark_finished` gains a `verified: bool` parameter; run status becomes
  `verified` when true (schema already allows it), task status stays
  `completed`.

### D3 — Dependencies

- New table `task_dependencies(task_id, depends_on_task_id)` (both FKs
  `ON DELETE CASCADE`, PK the pair) + `idx_task_deps`.
- Store API: `add_dependency` (rejects self-edge, duplicate, and cycles via
  recursive walk), `remove_dependency`, `dependency_ids(task_id)`.
- `get_task_by_id` / `list_tasks` populate `Task.dependencies` (one extra query
  per task; fine at this scale, keeps the web contract honest — the field
  exists today but is always empty).
- **Auto-run gating**: in the tick loop, a due task whose dependencies are not
  all `completed` is deferred — `next_run_at` pushed to the next cron fire
  (schedule mode) or `now + interval` (heartbeat), and the skip is logged.
  Manual runs are NOT gated (explicit user intent).

### D4 — Comments

- New table `task_comments` per RFC schema. Store CRUD:
  `add_comment` / `update_comment` / `delete_comment` / `list_comments`.
- Routes: `GET`/`POST /api/tasks/:id/comments`,
  `PUT`/`DELETE /api/tasks/:id/comments/:cid`.

### D5 — Edit + batch

- `PUT /api/tasks/:id` — partial update (`name`, `description`, `instruction`,
  `priority`, `sort_order`, `parent_task_id`, `assignee_agent_id`); SQL SET
  clause built from present fields only.
- `POST /api/tasks/batch` — `{ "tasks": [CreateTaskParams, …] }`.

### D6 — `task` agent tool (RFC Phase 2)

`crates/oxios-kernel/src/tools/builtin/task_tool.rs`, following `CronTool`
(holds `Arc<Mutex<TaskStore>>`, action-dispatch schema). Actions: `create`,
`create_batch`, `list`, `view`, `edit`, `update_status`, `set_schedule`,
`set_verify`, `run`, `add_comment`, `delete`. `run` spawns
`execute_task_run` fire-and-forget and returns the run id immediately (the
agent polls `view`); agent-authored creations stamp
`created_by_agent_id` from the tool context. Registered in
`register_all_kernel_tools`.

### D7 — Cron → task migration (RFC Phase 5 #18)

- `task/migrate.rs`: `migrate_cron_to_tasks(&CronScheduler, &TaskStore,
  dry_run) -> MigrationReport` — one task per cron job
  (`instruction = goal`, `automation_mode = schedule`,
  `schedule_pattern = job.schedule`), skipping names that already exist as task
  identifiers. **Copies, never deletes** the cron job (explicit user opt-in to
  retire a job afterwards).
- `POST /api/tasks/migrate-cron { "dry_run"?: bool }`.
- Web: "Migrate to tasks" action on `/cron-jobs`.

### D8 — Failure fuse (RFC Phase 5 #21)

`mark_finished` reads the run's `trigger`: only `schedule`/`heartbeat`
failures increment `consecutive_failures` (RFC: "Manual run failures do NOT
touch this counter"). At ≥ 3 the task is set to **`paused`** (not rescheduled),
`last_error` prefixed `Paused after 3 consecutive failures: …`, and a
`tracing::warn!` fires. The paused state + error are visible in the web detail.

### D9 — Web UI

- `TaskVerifyConfig` section in the detail dialog (enabled toggle, requirement
  textarea, max-iterations input) wired to the existing `useSetTaskVerify`.
- Run history: `verified` status renders with the success dot + a distinct
  "verified" badge.
- Comments thread in the detail dialog.
- Dependencies section: list with status chips, add (pick from existing tasks),
  remove — plus `TaskDependencyGraph` (d3-force, already a dependency) drawing
  the task and its dependency closure.
- Edit dialog (name/description/instruction/priority/assignee).
- `/cron-jobs` page: migrate-to-tasks action.
- All strings en/ko in `web/src/i18n/locales/`.

### D10 — Minor TODOs (from the completeness audit)

1. `src/kernel.rs` `cleanup()` now calls `shutdown_promotion_scanner()` first;
   `#[allow(dead_code)]` dropped (Promo-6 wiring).
2. `src/api/routes/chat.rs` `tool_calls` TODO: the WS terminal-message metadata
   already carries `tool_calls` when the orchestrator populates it (same key
   the non-streaming path parses at chat.rs:188); verify at implementation and
   remove the stale TODO, or document precisely what's missing.
3. `oxios-markdown` `should_split_checklist` has zero callers (only its own
   test) — delete method + test rather than inventing a heuristic nobody
   asked for.

### D11 — Docs + release

RFC-047/044/039 and the context-compression design get truthful status
headers; the eleven `docs/superpowers/plans/*.md` files get a one-line shipped
banner (no retroactive checkbox ticking — plans are historical records);
RFC-043 gets an Implemented header plus the decisions below. CHANGELOG gains
`[1.41.0]`; release flow: push → tag `v1.41.0` → CI (native binary +
crates.io).

## Non-goals (explicit)

- **A2A delegation as task execution.** `A2AProtocol::delegate_task` is
  fire-and-forget messaging over the MessageBus — no result returns to the
  caller, so wiring `assignee_agent_id` through it would produce runs that
  never record outcomes. Assignee stays metadata + list filter until the A2A
  protocol grows a request/response delegation.
- **Run cost/token auto-population.** `OrchestrationResult` carries no usage
  data; `task_runs.cost_usd`/`tokens_used` stay NULL until the SDK exposes
  per-run usage. Schema already supports backfill.
- **Replacing `/cron-jobs` with a task view.** Migration is copy-based (D7);
  the cron page stays until users retire it themselves.
