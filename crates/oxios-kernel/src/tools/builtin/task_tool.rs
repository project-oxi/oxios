//! Task tool — agent-facing task management (RFC-043 Phase 2).
//!
//! Wraps [`TaskStore`] (and the shared runner for `run`) behind the
//! [`AgentTool`] interface so agents can create, inspect, edit, schedule,
//! verify-gate, comment on, and trigger tasks.
//!
//! ## Example
//!
//! ```json
//! { "action": "create", "name": "Weekly digest", "instruction": "Summarize the week" }
//! { "action": "set_verify", "id": "weekly-digest", "enabled": true, "requirement": "Must cite sources" }
//! { "action": "run", "id": "weekly-digest" }
//! { "action": "list", "status": "scheduled" }
//! ```

use async_trait::async_trait;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;

use oxicode_sdk::{AgentTool, AgentToolResult, ToolContext, ToolError};
use serde_json::{Value, json};

use crate::kernel_handle::KernelHandle;
use crate::task::store::TaskStore;
use crate::task::{
    CreateTaskParams, ListTasksParams, SetScheduleParams, SetVerifyParams, TaskRunTrigger,
    TaskStatus, UpdateTaskParams, execute_task_run,
};

/// Ceiling for an agent-triggered manual run (matches the design's 300 s
/// fire-and-forget budget; the HTTP manual endpoint keeps its own).
const AGENT_RUN_TIMEOUT_SECS: u64 = 300;

/// Agent tool for task management (RFC-043).
///
/// Holds the kernel-owned [`TaskStore`] and, when available, the
/// [`KernelHandle`] needed to spawn runs. The kernel slot is optional so
/// store-only actions stay unit-testable without assembling the 16-facade
/// handle (same degradation pattern as the memo/email optional slots); a
/// missing kernel only disables the `run` action.
///
/// ## Actions
///
/// | Action          | Description                    | Required params     | Optional params |
/// |-----------------|--------------------------------|---------------------|-----------------|
/// | `create`        | Create one task                | `name`, `instruction` | `identifier`, `description`, `priority`, `parent_task_id`, `assignee_agent_id`, `sort_order` |
/// | `create_batch`  | Create several tasks           | `tasks` (array)     | —               |
/// | `list`          | List tasks                     | —                   | `status`, `assignee_agent_id`, `parent_task_id`, `limit`, `offset` |
/// | `view`          | Task detail + last run         | `id`                | —               |
/// | `edit`          | Partial update                 | `id`                | `name`, `description`, `instruction`, `priority`, `sort_order`, `parent_task_id`, `assignee_agent_id` |
/// | `update_status` | Change task status             | `id`, `status`      | —               |
/// | `set_schedule`  | Set/clear automation           | `id`                | `automation_mode`, `schedule_pattern`, `schedule_timezone`, `heartbeat_interval_secs`, `max_executions` |
/// | `set_verify`    | Configure the verify gate      | `id`                | `enabled`, `requirement`, `max_iterations`, `verifier_agent_id` |
/// | `run`           | Trigger a background run       | `id`                | —               |
/// | `add_comment`   | Comment on a task              | `id`, `content`     | —               |
/// | `delete`        | Delete a task                  | `id`                | —               |
///
/// `id` accepts either the task UUID or its identifier.
pub struct TaskTool {
    store: Arc<Mutex<TaskStore>>,
    kernel: Option<Arc<KernelHandle>>,
    agent_id: Option<String>,
}

impl TaskTool {
    /// Create a `TaskTool` from a [`KernelHandle`].
    ///
    /// Returns `None` when the assembler has not attached a task store
    /// (task management disabled) — registration then skips the tool.
    pub fn from_kernel(kernel: &Arc<KernelHandle>, agent_id: &str) -> Option<Self> {
        let store = kernel.task_store.clone()?;
        Some(Self {
            store,
            kernel: Some(kernel.clone()),
            agent_id: Some(agent_id.to_string()),
        })
    }

    /// Store-only constructor (unit tests; `run` unavailable).
    #[cfg(test)]
    fn for_store(store: Arc<Mutex<TaskStore>>, agent_id: Option<String>) -> Self {
        Self {
            store,
            kernel: None,
            agent_id,
        }
    }

    /// Resolve `key` as task id first, then identifier.
    async fn resolve(&self, key: &str) -> Result<crate::task::Task, String> {
        let store = self.store.lock().await;
        if let Ok(task) = store.get_task_by_id(key).await {
            return Ok(task);
        }
        store
            .get_task_by_identifier(key)
            .await
            .map_err(|e| format!("task lookup failed: {e}"))?
            .ok_or_else(|| format!("task '{key}' not found"))
    }
}

impl std::fmt::Debug for TaskTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskTool").finish()
    }
}

/// Compact task row for `list` output (full detail via `view`).
fn task_summary(task: &crate::task::Task) -> Value {
    json!({
        "id": task.id,
        "identifier": task.identifier,
        "name": task.name,
        "status": task.status.to_string(),
        "priority": task.priority,
        "automation_mode": task.automation_mode.as_ref().map(|m| m.to_string()),
        "next_run_at": task.next_run_at,
    })
}

#[async_trait]
impl AgentTool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }

    fn label(&self) -> &str {
        "Tasks"
    }

    fn description(&self) -> &'static str {
        "Manage long-lived tasks — create, schedule, verify-gate, run, and track work items. \
         Actions: create, create_batch, list, view, edit, update_status, set_schedule, \
         set_verify, run, add_comment, delete."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "create", "create_batch", "list", "view", "edit", "update_status",
                        "set_schedule", "set_verify", "run", "add_comment", "delete"
                    ],
                    "description": "Task operation to perform"
                },
                "id": {
                    "type": "string",
                    "description": "Task UUID or identifier (required for view/edit/update_status/set_schedule/set_verify/run/add_comment/delete)"
                },
                "name": {
                    "type": "string",
                    "description": "Task name (create)"
                },
                "instruction": {
                    "type": "string",
                    "description": "Goal instruction executed on each run (create, edit)"
                },
                "identifier": {
                    "type": "string",
                    "description": "Stable slug identifier; generated from the name when omitted (create)"
                },
                "description": {
                    "type": "string",
                    "description": "Human-readable description (create, edit)"
                },
                "priority": {
                    "type": "integer",
                    "description": "Priority 0-255, higher wins (create, edit)"
                },
                "parent_task_id": {
                    "type": "string",
                    "description": "Parent task id for nesting (create, edit)"
                },
                "assignee_agent_id": {
                    "type": "string",
                    "description": "Agent assigned to this task (create, edit)"
                },
                "sort_order": {
                    "type": "number",
                    "description": "Manual sort position (create, edit)"
                },
                "tasks": {
                    "type": "array",
                    "description": "Array of task objects for create_batch"
                },
                "status": {
                    "type": "string",
                    "enum": ["backlog", "scheduled", "running", "paused", "completed", "failed", "canceled"],
                    "description": "New status (update_status)"
                },
                "automation_mode": {
                    "type": "string",
                    "enum": ["schedule", "heartbeat", null],
                    "description": "Automation kind; null clears scheduling (set_schedule)"
                },
                "schedule_pattern": {
                    "type": "string",
                    "description": "Cron expression, e.g. '0 */6 * * *' (set_schedule)"
                },
                "schedule_timezone": {
                    "type": "string",
                    "description": "IANA timezone for the cron schedule (set_schedule)"
                },
                "heartbeat_interval_secs": {
                    "type": "integer",
                    "description": "Repeat interval in seconds for heartbeat mode (set_schedule)"
                },
                "max_executions": {
                    "type": "integer",
                    "description": "Stop after N executions (set_schedule)"
                },
                "enabled": {
                    "type": "boolean",
                    "description": "Arm/disarm the verify gate (set_verify)"
                },
                "requirement": {
                    "type": "string",
                    "description": "Acceptance criterion checked by a separate verifier conversation (set_verify)"
                },
                "max_iterations": {
                    "type": "integer",
                    "description": "Max verify/repair attempts, default 3 (set_verify)"
                },
                "verifier_agent_id": {
                    "type": "string",
                    "description": "Optional dedicated verifier agent (set_verify)"
                },
                "content": {
                    "type": "string",
                    "description": "Comment text (add_comment)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: Value,
        _signal: Option<tokio::sync::oneshot::Receiver<()>>,
        ctx: &ToolContext,
    ) -> Result<AgentToolResult, ToolError> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter: action".to_string())?;

        let id_of = || {
            params
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        };

        match action {
            "create" => {
                let mut p: CreateTaskParams = serde_json::from_value(params.clone())
                    .map_err(|e| format!("create: invalid parameters: {e}"))?;
                // Agent-authored creations are stamped (schema column already
                // existed; RFC-043 Phase 2).
                if p.created_by_agent_id.is_none() {
                    p.created_by_agent_id = self.agent_id.clone();
                }
                if p.created_by_session_id.is_none() {
                    p.created_by_session_id = ctx.session_id.clone();
                }
                let store = self.store.lock().await;
                match store.create_task(p).await {
                    Ok(task) => Ok(AgentToolResult::success(
                        serde_json::to_string_pretty(&task).unwrap_or_default(),
                    )),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to create task: {e}"
                    ))),
                }
            }

            "create_batch" => {
                let raw = params
                    .get("tasks")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| "create_batch requires 'tasks' array".to_string())?;
                let mut created = Vec::new();
                let mut errors = Vec::new();
                {
                    let store = self.store.lock().await;
                    for (i, item) in raw.iter().enumerate() {
                        let mut p: CreateTaskParams = serde_json::from_value(item.clone())
                            .map_err(|e| format!("create_batch: tasks[{i}] invalid: {e}"))?;
                        if p.created_by_agent_id.is_none() {
                            p.created_by_agent_id = self.agent_id.clone();
                        }
                        if p.created_by_session_id.is_none() {
                            p.created_by_session_id = ctx.session_id.clone();
                        }
                        match store.create_task(p).await {
                            Ok(task) => created.push(json!({
                                "id": task.id, "identifier": task.identifier
                            })),
                            Err(e) => errors.push(format!("tasks[{i}]: {e}")),
                        }
                    }
                }
                Ok(AgentToolResult::success(
                    serde_json::to_string_pretty(&json!({
                        "created": created,
                        "created_count": created.len(),
                        "errors": errors,
                    }))
                    .unwrap_or_default(),
                ))
            }

            "list" => {
                let p: ListTasksParams = serde_json::from_value(params.clone())
                    .map_err(|e| format!("list: invalid parameters: {e}"))?;
                // `status` (singular) is friendlier for agents; map it onto
                // the store's `statuses` filter.
                let p = match params.get("status").and_then(|v| v.as_str()) {
                    Some(s) if p.statuses.is_none() => ListTasksParams {
                        statuses: Some(vec![s.to_string()]),
                        ..p
                    },
                    _ => p,
                };
                let store = self.store.lock().await;
                match store.list_tasks(p).await {
                    Ok(tasks) if tasks.is_empty() => {
                        Ok(AgentToolResult::success("No tasks found."))
                    }
                    Ok(tasks) => Ok(AgentToolResult::success(
                        serde_json::to_string_pretty(&json!({
                            "tasks": tasks.iter().map(task_summary).collect::<Vec<_>>(),
                            "count": tasks.len(),
                        }))
                        .unwrap_or_default(),
                    )),
                    Err(e) => Ok(AgentToolResult::error(format!("Failed to list tasks: {e}"))),
                }
            }

            "view" => {
                let key = id_of().ok_or_else(|| "view requires 'id' parameter".to_string())?;
                let task = match self.resolve(&key).await {
                    Ok(t) => t,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                let last_run = {
                    let store = self.store.lock().await;
                    store.latest_run(&task.id).await.ok().flatten()
                };
                Ok(AgentToolResult::success(
                    serde_json::to_string_pretty(&json!({
                        "task": task,
                        "last_run": last_run,
                    }))
                    .unwrap_or_default(),
                ))
            }

            "edit" => {
                let key = id_of().ok_or_else(|| "edit requires 'id' parameter".to_string())?;
                let task = match self.resolve(&key).await {
                    Ok(t) => t,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                let p: UpdateTaskParams = serde_json::from_value(params.clone())
                    .map_err(|e| format!("edit: invalid parameters: {e}"))?;
                let store = self.store.lock().await;
                if let Err(e) = store.update_task(&task.id, p).await {
                    return Ok(AgentToolResult::error(format!(
                        "Failed to update task: {e}"
                    )));
                }
                match store.get_task_by_id(&task.id).await {
                    Ok(updated) => Ok(AgentToolResult::success(
                        serde_json::to_string_pretty(&updated).unwrap_or_default(),
                    )),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Updated but failed to reload: {e}"
                    ))),
                }
            }

            "update_status" => {
                let key =
                    id_of().ok_or_else(|| "update_status requires 'id' parameter".to_string())?;
                let status_str = params
                    .get("status")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "update_status requires 'status' parameter".to_string())?;
                let status =
                    TaskStatus::from_str(status_str).map_err(|e| format!("update_status: {e}"))?;
                let task = match self.resolve(&key).await {
                    Ok(t) => t,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                let store = self.store.lock().await;
                match store.update_status(&task.id, &status).await {
                    Ok(()) => Ok(AgentToolResult::success(format!(
                        "Task '{}' status set to {status}.",
                        task.identifier
                    ))),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to update status: {e}"
                    ))),
                }
            }

            "set_schedule" => {
                let key =
                    id_of().ok_or_else(|| "set_schedule requires 'id' parameter".to_string())?;
                let task = match self.resolve(&key).await {
                    Ok(t) => t,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                let p: SetScheduleParams = serde_json::from_value(params.clone())
                    .map_err(|e| format!("set_schedule: invalid parameters: {e}"))?;
                let store = self.store.lock().await;
                if let Err(e) = store.set_automation(&task.id, p).await {
                    return Ok(AgentToolResult::error(format!(
                        "Failed to set schedule: {e}"
                    )));
                }
                match store.get_task_by_id(&task.id).await {
                    Ok(updated) => Ok(AgentToolResult::success(
                        serde_json::to_string_pretty(&json!({
                            "automation_mode": updated.automation_mode.as_ref().map(|m| m.to_string()),
                            "schedule_pattern": updated.schedule_pattern,
                            "next_run_at": updated.next_run_at,
                            "status": updated.status.to_string(),
                        }))
                        .unwrap_or_default(),
                    )),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Scheduled but failed to reload: {e}"
                    ))),
                }
            }

            "set_verify" => {
                let key =
                    id_of().ok_or_else(|| "set_verify requires 'id' parameter".to_string())?;
                let task = match self.resolve(&key).await {
                    Ok(t) => t,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                let p: SetVerifyParams = serde_json::from_value(params.clone())
                    .map_err(|e| format!("set_verify: invalid parameters: {e}"))?;
                let store = self.store.lock().await;
                if let Err(e) = store.set_verify(&task.id, p).await {
                    return Ok(AgentToolResult::error(format!(
                        "Failed to set verify config: {e}"
                    )));
                }
                match store.get_task_by_id(&task.id).await {
                    Ok(updated) => Ok(AgentToolResult::success(
                        serde_json::to_string_pretty(&json!({
                            "verify_enabled": updated.verify_enabled,
                            "verify_requirement": updated.verify_requirement,
                            "verify_max_iterations": updated.verify_max_iterations,
                        }))
                        .unwrap_or_default(),
                    )),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Saved but failed to reload: {e}"
                    ))),
                }
            }

            "run" => {
                let key = id_of().ok_or_else(|| "run requires 'id' parameter".to_string())?;
                let task = match self.resolve(&key).await {
                    Ok(t) => t,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                let kernel = match &self.kernel {
                    Some(k) => k.clone(),
                    None => {
                        return Ok(AgentToolResult::error(
                            "Task runner unavailable (no kernel handle).".to_string(),
                        ));
                    }
                };
                // Fire-and-forget: the agent's loop must not block for the
                // length of a run. Capture the pre-spawn latest run so the
                // bounded poll below can identify the NEW run row that
                // `execute_task_run`'s `mark_running` opens within
                // milliseconds.
                let prev_run_id = self
                    .store
                    .lock()
                    .await
                    .latest_run(&task.id)
                    .await
                    .ok()
                    .flatten()
                    .map(|r| r.id);
                let store = self.store.clone();
                let task_id = task.id.clone();
                tokio::spawn(async move {
                    let (_, success, _) = execute_task_run(
                        store,
                        kernel,
                        &task_id,
                        TaskRunTrigger::Manual,
                        AGENT_RUN_TIMEOUT_SECS,
                    )
                    .await;
                    tracing::info!(%task_id, success, "agent-triggered task run finished");
                });
                let mut run_id = None;
                for _ in 0..25 {
                    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                    if let Ok(Some(run)) = self.store.lock().await.latest_run(&task.id).await
                        && Some(&run.id) != prev_run_id.as_ref()
                    {
                        run_id = Some(run.id);
                        break;
                    }
                }
                Ok(AgentToolResult::success(
                    serde_json::to_string(&json!({
                        "task_id": task.id,
                        "run_id": run_id,
                        "note": "Run started in the background; poll the view action for the outcome.",
                    }))
                    .unwrap_or_default(),
                ))
            }

            "add_comment" => {
                let key =
                    id_of().ok_or_else(|| "add_comment requires 'id' parameter".to_string())?;
                let content = params
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "add_comment requires 'content' parameter".to_string())?;
                let task = match self.resolve(&key).await {
                    Ok(t) => t,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                let store = self.store.lock().await;
                match store
                    .add_comment(
                        &task.id,
                        content,
                        self.agent_id.as_deref(),
                        ctx.session_id.as_deref(),
                    )
                    .await
                {
                    Ok(comment) => Ok(AgentToolResult::success(
                        serde_json::to_string_pretty(&comment).unwrap_or_default(),
                    )),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to add comment: {e}"
                    ))),
                }
            }

            "delete" => {
                let key = id_of().ok_or_else(|| "delete requires 'id' parameter".to_string())?;
                let task = match self.resolve(&key).await {
                    Ok(t) => t,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                let store = self.store.lock().await;
                match store.delete_task(&task.id).await {
                    Ok(()) => Ok(AgentToolResult::success(format!(
                        "Task '{}' ({}) deleted.",
                        task.identifier, task.id
                    ))),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to delete task: {e}"
                    ))),
                }
            }

            other => Err(format!(
                "Unknown task action '{other}'. Valid: create, create_batch, list, view, edit, \
                 update_status, set_schedule, set_verify, run, add_comment, delete"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::TaskStore;

    fn tool() -> TaskTool {
        TaskTool::for_store(
            Arc::new(Mutex::new(TaskStore::in_memory().unwrap())),
            Some("agent-1".to_string()),
        )
    }

    async fn run_action(tool: &TaskTool, params: Value) -> Result<AgentToolResult, ToolError> {
        tool.execute("call-1", params, None, &ToolContext::default())
            .await
    }

    #[test]
    fn test_schema_structure() {
        let schema = tool().parameters_schema();
        let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
        for expected in [
            "create",
            "create_batch",
            "list",
            "view",
            "edit",
            "update_status",
            "set_schedule",
            "set_verify",
            "run",
            "add_comment",
            "delete",
        ] {
            assert!(actions.iter().any(|a| a == expected), "missing {expected}");
        }
        assert_eq!(schema["required"][0], "action");
    }

    #[tokio::test]
    async fn run_without_id_errors() {
        let err = run_action(&tool(), json!({"action": "run"}))
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("id"), "error should mention id: {msg}");
    }

    #[tokio::test]
    async fn list_on_empty_store_succeeds() {
        let res = run_action(&tool(), json!({"action": "list"}))
            .await
            .unwrap();
        assert!(res.success);
        assert_eq!(res.output, "No tasks found.");
    }

    #[tokio::test]
    async fn create_stores_task_with_agent_stamp() {
        let tool = tool();
        let res = run_action(
            &tool,
            json!({"action": "create", "name": "Fonts", "instruction": "Recommend fonts"}),
        )
        .await
        .unwrap();
        assert!(res.success, "create failed: {}", res.output);
        // Assert via the store directly, using the identifier the tool
        // returned (slug_from_name appends a random suffix).
        let created: serde_json::Value = serde_json::from_str(&res.output).unwrap();
        let identifier = created["identifier"].as_str().unwrap().to_string();
        let store = tool.store.lock().await;
        let task = store
            .get_task_by_identifier(&identifier)
            .await
            .unwrap()
            .expect("task stored");
        assert_eq!(task.instruction, "Recommend fonts");
        assert_eq!(task.created_by_agent_id.as_deref(), Some("agent-1"));
    }

    #[tokio::test]
    async fn set_verify_action_persists() {
        let tool = tool();
        run_action(
            &tool,
            json!({"action": "create", "name": "V", "instruction": "x"}),
        )
        .await
        .unwrap();
        let created = {
            let store = tool.store.lock().await;
            store.list_tasks(ListTasksParams::default()).await.unwrap()
        };
        let id = created[0].id.clone();
        let res = run_action(
            &tool,
            json!({"action": "set_verify", "id": id, "enabled": true,
                   "requirement": "must include BANANA", "max_iterations": 5}),
        )
        .await
        .unwrap();
        assert!(res.success, "set_verify failed: {}", res.output);
        let task = tool.store.lock().await.get_task_by_id(&id).await.unwrap();
        assert!(task.verify_enabled);
        assert_eq!(
            task.verify_requirement.as_deref(),
            Some("must include BANANA")
        );
        assert_eq!(task.verify_max_iterations, 5);
    }
}
