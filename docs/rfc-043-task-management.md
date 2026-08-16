# RFC-043: Task Management System

> Ported from LobeHub's `builtin-tool-task` — full agent task lifecycle management.
> **Status**: Implemented (2026-08-16) — see docs/designs/2026-08-16-rfc-043-completion-design.md

## Overview

Oxios currently has **Cron Jobs** (scheduled message execution) but no **Task** concept.
This RFC adds a complete task management system:

- Task lifecycle: backlog → scheduled → running → completed/failed/paused
- Task hierarchy: parent-child subtasks with dependencies
- Scheduling: cron-based (schedule mode) + fixed-interval (heartbeat mode)
- Agent assignment: delegate tasks to specific agents
- Verify gate: delivery acceptance — a separate reviewer verifies completion
- Comments: threaded discussion per task
- Execution: `runTask` spawns a new agent conversation with the task instruction

## Data Model

### SQLite Schema

```sql
CREATE TABLE tasks (
    id TEXT PRIMARY KEY,                    -- UUID
    identifier TEXT UNIQUE NOT NULL,        -- short slug: "weekly-font-recs"
    name TEXT NOT NULL,                     -- "Weekly Font Recommendations"
    description TEXT,                       -- optional longer description
    instruction TEXT NOT NULL,              -- system prompt for the agent
    status TEXT NOT NULL DEFAULT 'backlog', -- backlog|scheduled|running|paused|completed|failed|canceled
    priority INTEGER DEFAULT 0,             -- 0=none, 1=urgent, 2=high, 3=medium, 4=low
    sort_order REAL,                        -- manual ordering

    -- Hierarchy
    parent_task_id TEXT REFERENCES tasks(id) ON DELETE CASCADE,
    -- Dependencies stored in task_dependencies table

    -- Assignment
    assignee_agent_id TEXT,                 -- which agent runs this task
    created_by_agent_id TEXT,               -- agent that created this task
    created_by_session_id TEXT,             -- session that created this task

    -- Scheduling
    automation_mode TEXT,                   -- 'schedule' | 'heartbeat' | NULL
    schedule_pattern TEXT,                  -- cron: "0 9 * * 1" (Mon 9am)
    schedule_timezone TEXT,                 -- IANA: "Asia/Seoul"
    heartbeat_interval_secs INTEGER,        -- heartbeat: 86400
    max_executions INTEGER,                 -- cap, NULL=unlimited
    execution_count INTEGER DEFAULT 0,      -- how many times it ran

    -- Verify gate
    verify_enabled INTEGER DEFAULT 0,
    verify_requirement TEXT,                -- "Output must include 3 font pairings"
    verify_max_iterations INTEGER DEFAULT 3,
    verify_verifier_agent_id TEXT,

    -- Lifecycle timestamps
    created_at TEXT NOT DEFAULT (datetime('now')),
    updated_at TEXT NOT DEFAULT (datetime('now')),
    started_at TEXT,                        -- first execution
    completed_at TEXT,                      -- final completion
    last_run_at TEXT,                       -- last execution attempt
    next_run_at TEXT,                       -- scheduled next tick

    -- Error tracking
    last_error TEXT,                        -- cleared on next success
    consecutive_failures INTEGER DEFAULT 0, -- fuse counter

    -- Context (JSON)
    context_json TEXT                       -- lifecycle audit, origin, scheduler state
);

CREATE TABLE task_dependencies (
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    depends_on_task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    PRIMARY KEY (task_id, depends_on_task_id)
);

CREATE TABLE task_comments (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    author_agent_id TEXT,                   -- NULL = user
    author_session_id TEXT,
    created_at TEXT NOT DEFAULT (datetime('now')),
    updated_at TEXT
);

CREATE TABLE task_runs (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    session_id TEXT,                        -- the conversation this run created
    trigger TEXT NOT NULL,                  -- 'manual' | 'schedule' | 'heartbeat'
    status TEXT NOT NULL DEFAULT 'running', -- running|completed|failed|verified
    summary TEXT,                           -- AI-synthesized result summary
    result_content TEXT,                    -- raw last assistant message
    started_at TEXT NOT DEFAULT (datetime('now')),
    completed_at TEXT,
    error TEXT,
    cost_usd REAL,
    tokens_used INTEGER
);

CREATE INDEX idx_tasks_status ON tasks(status);
CREATE INDEX idx_tasks_parent ON tasks(parent_task_id);
CREATE INDEX idx_tasks_assignee ON tasks(assignee_agent_id);
CREATE INDEX idx_tasks_next_run ON tasks(next_run_at) WHERE automation_mode IS NOT NULL;
CREATE INDEX idx_runs_task ON task_runs(task_id);
```

### Rust Structs

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub identifier: String,
    pub name: String,
    pub description: Option<String>,
    pub instruction: String,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub sort_order: Option<f64>,
    pub parent_task_id: Option<String>,
    pub assignee_agent_id: Option<String>,
    pub created_by_agent_id: Option<String>,
    pub created_by_session_id: Option<String>,
    pub automation_mode: Option<TaskAutomationMode>,
    pub schedule_pattern: Option<String>,
    pub schedule_timezone: Option<String>,
    pub heartbeat_interval_secs: Option<u64>,
    pub max_executions: Option<u32>,
    pub execution_count: u32,
    pub verify_enabled: bool,
    pub verify_requirement: Option<String>,
    pub verify_max_iterations: u32,
    pub verify_verifier_agent_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub last_run_at: Option<String>,
    pub next_run_at: Option<String>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskStatus {
    Backlog,
    Scheduled,
    Running,
    Paused,
    Completed,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskAutomationMode {
    Schedule,    // cron-based
    Heartbeat,   // fixed interval
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskPriority {
    None = 0,
    Urgent = 1,
    High = 2,
    Medium = 3,
    Low = 4,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskComment {
    pub id: String,
    pub task_id: String,
    pub content: String,
    pub author_agent_id: Option<String>,
    pub created_at: String,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRun {
    pub id: String,
    pub task_id: String,
    pub session_id: Option<String>,
    pub trigger: TaskRunTrigger,
    pub status: TaskRunStatus,
    pub summary: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub error: Option<String>,
    pub cost_usd: Option<f64>,
    pub tokens_used: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskRunTrigger {
    Manual,
    Schedule,
    Heartbeat,
}
```

## API Endpoints

```
GET    /api/tasks                       — list tasks (filter by status, assignee, parent)
POST   /api/tasks                       — create task
POST   /api/tasks/batch                 — create multiple tasks
GET    /api/tasks/:id                   — view task detail
PUT    /api/tasks/:id                   — edit task (name, desc, instruction, priority, parent, deps)
DELETE /api/tasks/:id                   — delete task
PUT    /api/tasks/:id/status            — update status
PUT    /api/tasks/:id/schedule          — set/clear schedule (cron + heartbeat)
PUT    /api/tasks/:id/verify            — set/clear verify gate
POST   /api/tasks/:id/run               — trigger manual run
POST   /api/tasks/:id/runs              — list runs for a task

POST   /api/tasks/:id/comments          — add comment
PUT    /api/tasks/:id/comments/:cid     — update comment
DELETE /api/tasks/:id/comments/:cid     — delete comment

GET    /api/tasks/:id/dependencies      — list dependencies
POST   /api/tasks/:id/dependencies      — add dependency
DELETE /api/tasks/:id/dependencies/:dep — remove dependency
```

## Scheduler

### Schedule Mode (cron)
- Uses the existing `cron-parser` crate (already in web dependencies)
- On each tick: check `next_run_at <= now AND status = 'scheduled'`
- Spawn a new agent session with the task instruction
- Record the run in `task_runs`
- Increment `execution_count`, check `max_executions`

### Heartbeat Mode (fixed interval)
- Background tokio task per heartbeat-enabled task
- `tokio::time::interval(Duration::from_secs(heartbeat_interval_secs))`
- Same execution flow as schedule mode

### Failure Fuse
- `consecutive_failures` counter increments on each failed automation tick
- When it hits 3: auto-pause the task, notify user
- Manual run failures do NOT touch this counter

## Agent Tool

New Rust tool `TaskTool` implementing `AgentTool`:

```rust
pub struct TaskTool {
    handle: Arc<KernelHandle>,
}

impl AgentTool for TaskTool {
    fn name(&self) -> &str { "task" }
    fn description(&self) -> &str { "Manage tasks: create, list, edit, run, schedule, verify" }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "create_batch", "list", "view", "edit",
                             "update_status", "set_schedule", "set_verify",
                             "run", "add_comment", "delete"]
                },
                // ... action-specific params
            }
        })
    }
}
```

Actions map 1:1 to the API endpoints above.

## Frontend

### Pages

```
/tasks                    — Task list (kanban or list view)
/tasks/:id                — Task detail (instruction, runs, comments, schedule)
/tasks/templates          — Task template gallery (already built)
```

### Components

| Component | Description |
|-----------|-------------|
| `TaskList` | List/kanban of tasks, filterable by status |
| `TaskCard` | Compact card: name, status badge, assignee, schedule |
| `TaskDetail` | Full view: instruction, dependencies, runs timeline, comments |
| `TaskCreateDialog` | Form: name, instruction, assignee, schedule, priority |
| `TaskScheduleEditor` | Cron editor + heartbeat interval selector |
| `TaskVerifyConfig` | Verify gate: requirement, max iterations, verifier agent |
| `TaskRunTimeline` | Timeline of past runs with results |
| `TaskStatusBadge` | Colored badge per status |
| `TaskDependencyGraph` | Visual graph of task dependencies (d3-force, already in deps) |

### Sidebar Addition

Add "Tasks" to the sidebar under the Agents category:

```
Agents
├── Agents
├── Agent Groups
├── Personas
├── Skills
└── Tasks          ← NEW
```

## Verify Gate

When a task completes and `verify_enabled = true`:

1. The completing agent marks status = `completed`
2. The verify system spawns a **separate agent** (verifier) with:
   - The task's instruction
   - The run's result content
   - The verify requirement
3. The verifier checks if the result meets the requirement
4. If pass: task stays `completed`
5. If fail: task goes back to `running` with repair instructions
6. After `max_iterations` failures: task → `failed` with error

## Integration with Existing Oxios

### Cron → Task Migration
Existing cron jobs can be migrated to tasks:
```
cron_job { name, schedule, goal } → task { name, instruction=goal, automation_mode='schedule', schedule_pattern=schedule }
```

The existing `/cron-jobs` page becomes a simplified view of tasks with `automation_mode = 'schedule'`.

### A2A Integration
Tasks with `assignee_agent_id` use the existing A2A delegation system:
```rust
kernel_handle.a2a.delegate_task(from, to, task_spec).await
```

### Session Integration
Each task run creates a new session:
```rust
let session = kernel_handle.agent.create_session(
    agent_id, Some(task.instruction)
).await?;
```

## Implementation Phases

### Phase 1: Backend Core (Rust)
1. SQLite schema migration (`oxios-kernel/src/task/mod.rs`)
2. `TaskModel` — CRUD operations
3. `TaskScheduler` — cron + heartbeat background loop
4. `TaskRunner` — spawn agent sessions for task execution
5. API routes (`src/api/routes/task_routes.rs`)

### Phase 2: Agent Tool
6. `TaskTool` — register in `tools/kernel_bridge.rs`
7. Task creation from natural language ("set up a weekly font recommendation task")
8. Task listing and management from chat

### Phase 3: Frontend
9. `/tasks` route with TaskList + TaskCard
10. `/tasks/:id` route with TaskDetail
11. TaskCreateDialog with schedule editor
12. TaskScheduleEditor (cron + heartbeat)
13. TaskStatusBadge
14. Sidebar "Tasks" entry

### Phase 4: Verify Gate
15. `TaskVerifier` — separate agent execution for acceptance testing
16. TaskVerifyConfig UI
17. Run timeline with verify results

### Phase 5: Migration + Polish
18. Migrate existing cron jobs to tasks
19. TaskDependencyGraph (d3-force visualization)
20. TaskRunTimeline with cost/token tracking
21. Failure fuse notifications

## File Map

```
crates/oxios-kernel/src/
├── task/
│   ├── mod.rs              — public API
│   ├── model.rs            — Task struct + SQLite operations
│   ├── scheduler.rs        — cron + heartbeat scheduler loop
│   ├── runner.rs           — spawn agent sessions for task execution
│   ├── verifier.rs         — verify gate execution
│   └── migrate.rs          — cron → task migration

crates/oxios-kernel/src/tools/builtin/
└── task_tool.rs            — AgentTool for task management

src/api/routes/
└── task_routes.rs          — REST API for tasks

web/src/
├── routes/
│   ├── tasks.tsx           — task list page
│   └── tasks/$taskId.tsx   — task detail page
├── components/task/
│   ├── task-list.tsx
│   ├── task-card.tsx
│   ├── task-detail.tsx
│   ├── task-create-dialog.tsx
│   ├── task-schedule-editor.tsx
│   ├── task-verify-config.tsx
│   ├── task-run-timeline.tsx
│   ├── task-status-badge.tsx
│   └── task-dependency-graph.tsx
├── hooks/
│   └── use-tasks.ts        — React Query hooks for task API
└── types/
    └── task.ts             — TypeScript types
```

## Implementation notes (2026-08-16 completion)

- **Verify loop semantics** — execution, verification, and repair re-runs
  share ONE deadline (`timeout_secs`); a verify-enabled run can never exceed
  the caller's ceiling. An unparseable verifier verdict is a FAIL (an
  unreadable verdict must never accept work).
- **A2A delegation is a non-goal for task execution** —
  `A2AProtocol::delegate_task` is fire-and-forget messaging with no result
  return; wiring `assignee_agent_id` through it would produce runs that never
  record outcomes. Assignee stays metadata + list filter.
- **Run cost/token columns stay NULL** — `OrchestrationResult` carries no
  usage data; `task_runs.cost_usd`/`tokens_used` await SDK-side per-run
  usage. The schema supports backfill.
- **Cron migration is copy-based** — jobs are never deleted or disabled by
  migration; retiring a cron job stays an explicit user decision.
- **Wire format** — task DTOs serialize camelCase (matching the shipped web
  types); the `task` agent tool keeps its snake_case schema via serde
  aliases.
