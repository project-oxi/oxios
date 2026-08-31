//! Task tool — agent-facing automation management.
//!
//! Wraps [`AutomationStore`] (and the shared runner for `run`) behind the
//! [`AgentTool`] interface so agents can create, inspect, edit, schedule,
//! verify-gate, and trigger automations.
//!
//! Mechanical cutover note (web IA major version): the underlying domain is
//! the `automation` module; the tool surface keeps its `task` identity until
//! the API-rename task lands.

use async_trait::async_trait;

use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;

use oxicode_sdk::{AgentTool, AgentToolResult, ToolContext, ToolError};
use serde_json::{Value, json};

use crate::automation::{
    AutomationRunTrigger, AutomationStatus, AutomationStore, CreateAutomationParams,
    ListAutomationsParams, SetTriggerParams, SetVerifyParams, UpdateAutomationParams,
    execute_automation_run,
};
use crate::kernel_handle::KernelHandle;

/// Ceiling for an agent-triggered manual run (matches the design's 300 s
/// fire-and-forget budget; the HTTP manual endpoint keeps its own).
const AGENT_RUN_TIMEOUT_SECS: u64 = 300;

/// Agent tool for automation management.
///
/// Holds the kernel-owned [`AutomationStore`] and, when available, the
/// [`KernelHandle`] needed to spawn runs. The kernel slot is optional so
/// store-only actions stay unit-testable without assembling the 16-facade
/// handle (same degradation pattern as the memo/email optional slots); a
/// missing kernel only disables the `run` action.
///
/// ## Actions
///
/// | Action          | Description                    | Required params     | Optional params |
/// |-----------------|--------------------------------|---------------------|-----------------|
/// | `create`        | Create one automation          | `name`, `instruction` | `description`, `trigger`, `cron_pattern`, `timezone`, `heartbeat_interval_secs`, `max_executions`, `persona_id`, `project_id`, `brain_space` |
/// | `create_batch`  | Create several automations     | `tasks` (array)     | —               |
/// | `list`          | List automations               | —                   | `status`, `limit`, `offset` |
/// | `view`          | Automation detail + last run   | `id`                | —               |
/// | `edit`          | Partial update                 | `id`                | `name`, `description`, `instruction`, `persona_id`, `project_id`, `brain_space` |
/// | `update_status` | Change automation status       | `id`, `status`      | —               |
/// | `set_schedule`  | Set/replace the trigger        | `id`, `trigger`     | `cron_pattern`, `timezone`, `heartbeat_interval_secs`, `max_executions` |
/// | `set_verify`    | Configure the verify gate      | `id`                | `enabled`, `requirement`, `max_iterations` |
/// | `run`           | Trigger a background run       | `id`                | —               |
/// | `delete`        | Delete an automation           | `id`                | —               |
///
/// `id` is the automation UUID.
pub struct TaskTool {
    store: Arc<Mutex<AutomationStore>>,
    kernel: Option<Arc<KernelHandle>>,
}

impl TaskTool {
    /// Create a `TaskTool` from a [`KernelHandle`].
    ///
    /// Returns `None` when the assembler has not attached an automation
    /// store (automation management disabled) — registration then skips
    /// the tool.
    pub fn from_kernel(kernel: &Arc<KernelHandle>) -> Option<Self> {
        let store = kernel.automation_store.clone()?;
        Some(Self {
            store,
            kernel: Some(kernel.clone()),
        })
    }

    /// Store-only constructor (production registration path; `run` unavailable).
    /// Public so `register_tools_from_cspace_gated` can build a TaskTool from a
    /// raw store handle without an `&Arc<KernelHandle>` (the gated function only
    /// receives `&KernelHandle`, which is not Clone-able).
    pub fn for_store(store: Arc<Mutex<AutomationStore>>) -> Self {
        Self {
            store,
            kernel: None,
        }
    }

    /// Resolve an automation by id.
    async fn resolve(&self, key: &str) -> Result<crate::automation::Automation, String> {
        let store = self.store.lock().await;
        store
            .get_automation(key)
            .await
            .map_err(|e| format!("automation lookup failed: {e}"))
    }
}

impl std::fmt::Debug for TaskTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskTool").finish()
    }
}

/// Compact automation row for `list` output (full detail via `view`).
fn task_summary(automation: &crate::automation::Automation) -> Value {
    json!({
        "id": automation.id,
        "name": automation.name,
        "status": automation.status.to_string(),
        "trigger": automation.trigger.to_string(),
        "next_run_at": automation.next_run_at,
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
        "Manage long-lived automations — create, schedule, verify-gate, run, and track \
         recurring or on-demand work items. Actions: create, create_batch, list, view, \
         edit, update_status, set_schedule, set_verify, run, delete."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "create", "create_batch", "list", "view", "edit", "update_status",
                        "set_schedule", "set_verify", "run", "delete"
                    ],
                    "description": "Automation operation to perform"
                },
                "id": {
                    "type": "string",
                    "description": "Automation UUID (required for view/edit/update_status/set_schedule/set_verify/run/delete)"
                },
                "name": {
                    "type": "string",
                    "description": "Automation name (create)"
                },
                "instruction": {
                    "type": "string",
                    "description": "Goal instruction executed on each run (create, edit)"
                },
                "description": {
                    "type": "string",
                    "description": "Human-readable description (create, edit)"
                },
                "trigger": {
                    "type": "string",
                    "enum": ["manual", "cron", "heartbeat"],
                    "description": "What starts the automation (create, set_schedule)"
                },
                "cron_pattern": {
                    "type": "string",
                    "description": "Cron expression, e.g. '0 */6 * * *' (required for the cron trigger)"
                },
                "timezone": {
                    "type": "string",
                    "description": "IANA timezone qualifying the cron schedule (create, set_schedule)"
                },
                "heartbeat_interval_secs": {
                    "type": "integer",
                    "description": "Repeat interval in seconds (required for the heartbeat trigger)"
                },
                "max_executions": {
                    "type": "integer",
                    "description": "Stop after N executions (create, set_schedule)"
                },
                "persona_id": {
                    "type": "string",
                    "description": "Persona bound to the automation (create, edit); omit for default"
                },
                "project_id": {
                    "type": "string",
                    "description": "Project bound to the automation (create, edit); omit for projectless"
                },
                "brain_space": {
                    "type": "string",
                    "description": "Brain space bound to the automation (create, edit)"
                },
                "tasks": {
                    "type": "array",
                    "description": "Array of automation objects for create_batch"
                },
                "status": {
                    "type": "string",
                    "enum": ["active", "paused", "exhausted", "failed"],
                    "description": "New status (update_status)"
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
        _ctx: &ToolContext,
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
                let p: CreateAutomationParams = serde_json::from_value(params.clone())
                    .map_err(|e| format!("create: invalid parameters: {e}"))?;
                let store = self.store.lock().await;
                match store.create_automation(p).await {
                    Ok(automation) => Ok(AgentToolResult::success(
                        serde_json::to_string_pretty(&automation).unwrap_or_default(),
                    )),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to create automation: {e}"
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
                        let p: CreateAutomationParams = serde_json::from_value(item.clone())
                            .map_err(|e| format!("create_batch: tasks[{i}] invalid: {e}"))?;
                        match store.create_automation(p).await {
                            Ok(automation) => created.push(json!({ "id": automation.id })),
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
                let p: ListAutomationsParams = serde_json::from_value(params.clone())
                    .map_err(|e| format!("list: invalid parameters: {e}"))?;
                let store = self.store.lock().await;
                match store.list_automations(p).await {
                    Ok(list) if list.is_empty() => Ok(AgentToolResult::success("No tasks found.")),
                    Ok(list) => Ok(AgentToolResult::success(
                        serde_json::to_string_pretty(&json!({
                            "tasks": list.iter().map(task_summary).collect::<Vec<_>>(),
                            "count": list.len(),
                        }))
                        .unwrap_or_default(),
                    )),
                    Err(e) => Ok(AgentToolResult::error(format!("Failed to list tasks: {e}"))),
                }
            }

            "view" => {
                let key = id_of().ok_or_else(|| "view requires 'id' parameter".to_string())?;
                let automation = match self.resolve(&key).await {
                    Ok(a) => a,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                let last_run = {
                    let store = self.store.lock().await;
                    store.latest_run(&automation.id).await.ok().flatten()
                };
                Ok(AgentToolResult::success(
                    serde_json::to_string_pretty(&json!({
                        "task": automation,
                        "last_run": last_run,
                    }))
                    .unwrap_or_default(),
                ))
            }

            "edit" => {
                let key = id_of().ok_or_else(|| "edit requires 'id' parameter".to_string())?;
                let automation = match self.resolve(&key).await {
                    Ok(a) => a,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                let p: UpdateAutomationParams = serde_json::from_value(params.clone())
                    .map_err(|e| format!("edit: invalid parameters: {e}"))?;
                let store = self.store.lock().await;
                if let Err(e) = store.update_automation(&automation.id, p).await {
                    return Ok(AgentToolResult::error(format!(
                        "Failed to update automation: {e}"
                    )));
                }
                match store.get_automation(&automation.id).await {
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
                let status = AutomationStatus::from_str(status_str)
                    .map_err(|e| format!("update_status: {e}"))?;
                let automation = match self.resolve(&key).await {
                    Ok(a) => a,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                let store = self.store.lock().await;
                match store.update_status(&automation.id, &status).await {
                    Ok(()) => Ok(AgentToolResult::success(format!(
                        "Automation '{}' status set to {status}.",
                        automation.id
                    ))),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to update status: {e}"
                    ))),
                }
            }

            "set_schedule" => {
                let key =
                    id_of().ok_or_else(|| "set_schedule requires 'id' parameter".to_string())?;
                let automation = match self.resolve(&key).await {
                    Ok(a) => a,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                let p: SetTriggerParams = serde_json::from_value(params.clone())
                    .map_err(|e| format!("set_schedule: invalid parameters: {e}"))?;
                let store = self.store.lock().await;
                if let Err(e) = store.set_trigger(&automation.id, p).await {
                    return Ok(AgentToolResult::error(format!(
                        "Failed to set schedule: {e}"
                    )));
                }
                match store.get_automation(&automation.id).await {
                    Ok(updated) => Ok(AgentToolResult::success(
                        serde_json::to_string_pretty(&json!({
                            "trigger": updated.trigger.to_string(),
                            "cron_pattern": updated.cron_pattern,
                            "timezone": updated.timezone,
                            "heartbeat_interval_secs": updated.heartbeat_interval_secs,
                            "max_executions": updated.max_executions,
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
                let automation = match self.resolve(&key).await {
                    Ok(a) => a,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                let p: SetVerifyParams = serde_json::from_value(params.clone())
                    .map_err(|e| format!("set_verify: invalid parameters: {e}"))?;
                let store = self.store.lock().await;
                if let Err(e) = store.set_verify(&automation.id, p).await {
                    return Ok(AgentToolResult::error(format!(
                        "Failed to set verify config: {e}"
                    )));
                }
                match store.get_automation(&automation.id).await {
                    Ok(updated) => Ok(AgentToolResult::success(
                        serde_json::to_string_pretty(&json!({
                            "verify_enabled": updated.verify.enabled,
                            "verify_requirement": updated.verify.requirement,
                            "verify_max_iterations": updated.verify.max_iterations,
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
                let automation = match self.resolve(&key).await {
                    Ok(a) => a,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                let kernel = match &self.kernel {
                    Some(k) => k.clone(),
                    None => {
                        return Ok(AgentToolResult::error(
                            "Automation runner unavailable (no kernel handle).".to_string(),
                        ));
                    }
                };
                // Fire-and-forget: the agent's loop must not block for the
                // length of a run. Capture the pre-spawn latest run so the
                // bounded poll below can identify the NEW run row that
                // `execute_automation_run`'s `begin_run` opens within
                // milliseconds.
                let prev_run_id = self
                    .store
                    .lock()
                    .await
                    .latest_run(&automation.id)
                    .await
                    .ok()
                    .flatten()
                    .map(|r| r.id);
                let store = self.store.clone();
                let automation_id = automation.id.clone();
                tokio::spawn(async move {
                    let (_, success, _) = execute_automation_run(
                        store,
                        kernel,
                        &automation_id,
                        AutomationRunTrigger::Manual,
                        AGENT_RUN_TIMEOUT_SECS,
                    )
                    .await;
                    tracing::info!(%automation_id, success, "agent-triggered automation run finished");
                });
                let mut run_id = None;
                for _ in 0..25 {
                    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                    if let Ok(Some(run)) = self.store.lock().await.latest_run(&automation.id).await
                        && Some(&run.id) != prev_run_id.as_ref()
                    {
                        run_id = Some(run.id);
                        break;
                    }
                }
                Ok(AgentToolResult::success(
                    serde_json::to_string(&json!({
                        "task_id": automation.id,
                        "run_id": run_id,
                        "note": "Run started in the background; poll the view action for the outcome.",
                    }))
                    .unwrap_or_default(),
                ))
            }

            "delete" => {
                let key = id_of().ok_or_else(|| "delete requires 'id' parameter".to_string())?;
                let automation = match self.resolve(&key).await {
                    Ok(a) => a,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                let store = self.store.lock().await;
                match store.delete_automation(&automation.id).await {
                    Ok(()) => Ok(AgentToolResult::success(format!(
                        "Automation '{}' deleted.",
                        automation.id
                    ))),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to delete automation: {e}"
                    ))),
                }
            }

            other => Err(format!(
                "Unknown task action '{other}'. Valid: create, create_batch, list, view, edit, \
                 update_status, set_schedule, set_verify, run, delete"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::AutomationStore;

    fn tool() -> TaskTool {
        TaskTool::for_store(Arc::new(Mutex::new(AutomationStore::in_memory().unwrap())))
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
    async fn create_stores_automation() {
        let tool = tool();
        let res = run_action(
            &tool,
            json!({"action": "create", "name": "Fonts", "instruction": "Recommend fonts"}),
        )
        .await
        .unwrap();
        assert!(res.success, "create failed: {}", res.output);
        let created: serde_json::Value = serde_json::from_str(&res.output).unwrap();
        let id = created["id"].as_str().unwrap().to_string();
        let store = tool.store.lock().await;
        let automation = store.get_automation(&id).await.unwrap();
        assert_eq!(automation.instruction, "Recommend fonts");
        // Projectless by default — None, never a string sentinel.
        assert_eq!(automation.project_id, None);
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
            store
                .list_automations(ListAutomationsParams::default())
                .await
                .unwrap()
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
        let automation = tool.store.lock().await.get_automation(&id).await.unwrap();
        assert!(automation.verify.enabled);
        assert_eq!(
            automation.verify.requirement.as_deref(),
            Some("must include BANANA")
        );
        assert_eq!(automation.verify.max_iterations, 5);
    }
}
