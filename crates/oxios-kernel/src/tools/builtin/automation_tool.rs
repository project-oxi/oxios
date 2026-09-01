//! Automation tool — agent-facing automation management.
//!
//! Wraps [`AutomationStore`] (and the shared runner for `run`) behind the
//! [`AgentTool`] interface so agents can manage automations: create, list,
//! inspect, update, pause/resume, trigger runs, read run history, delete.
//!
//! Scheduling (cron / heartbeat) and the verify gate are NOT exposed as
//! separate tool ops in this major-version cutover; the HTTP layer keeps
//! `PUT /api/automations/:id/{trigger,verify}` for those concerns. Agents
//! that need to set or change the trigger pass it via the `create` op
//! (and re-`create` if mutating).

use async_trait::async_trait;

use std::sync::Arc;
use tokio::sync::Mutex;

use oxicode_sdk::{AgentTool, AgentToolResult, ToolContext, ToolError};
use serde_json::{Value, json};

use crate::automation::{
    AutomationRunTrigger, AutomationStatus, AutomationStore, CreateAutomationParams,
    ListAutomationsParams, UpdateAutomationParams, execute_automation_run,
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
/// missing kernel only disables the `run` op.
///
/// ## Actions
///
/// | Action   | Description                    | Required params     | Optional params |
/// |----------|--------------------------------|---------------------|-----------------|
/// | `create` | Create one automation          | `name`, `instruction` | `description`, `trigger`, `cron_pattern`, `timezone`, `heartbeat_interval_secs`, `max_executions`, `persona_id`, `project_id`, `brain_space` |
/// | `list`   | List automations               | —                   | `status`, `limit`, `offset` |
/// | `get`    | Automation detail              | `id`                | —               |
/// | `update` | Partial update (absent key = keep, null = clear) | `id` | `name`, `description`, `instruction`, `persona_id`, `project_id`, `brain_space` |
/// | `pause`  | Set status to `paused`         | `id`                | —               |
/// | `resume` | Set status to `active`         | `id`                | —               |
/// | `run`    | Trigger a background run       | `id`                | —               |
/// | `runs`   | Run history (newest first)     | `id`                | —               |
/// | `delete` | Delete an automation           | `id`                | —               |
///
/// `id` is the automation UUID.
pub struct AutomationTool {
    store: Arc<Mutex<AutomationStore>>,
    kernel: Option<Arc<KernelHandle>>,
}

impl AutomationTool {
    /// Create an `AutomationTool` from a [`KernelHandle`].
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
    /// Public so `register_tools_from_cspace_gated` can build an
    /// `AutomationTool` from a raw store handle without an `&Arc<KernelHandle>`
    /// (the gated function only receives `&KernelHandle`, which is not
    /// Clone-able).
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

impl std::fmt::Debug for AutomationTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutomationTool").finish()
    }
}

/// Compact automation row for `list` output (full detail via `get`).
fn automation_summary(automation: &crate::automation::Automation) -> Value {
    json!({
        "id": automation.id,
        "name": automation.name,
        "status": automation.status.to_string(),
        "trigger": automation.trigger.to_string(),
        "next_run_at": automation.next_run_at,
    })
}

#[async_trait]
impl AgentTool for AutomationTool {
    fn name(&self) -> &str {
        "automation"
    }

    fn label(&self) -> &str {
        "Automations"
    }

    fn description(&self) -> &'static str {
        "Manage long-lived automations — create, inspect, update, pause/resume, \
         run, and track recurring or on-demand work items. Actions: create, list, \
         get, update, pause, resume, run, runs, delete."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "create", "list", "get", "update", "pause", "resume",
                        "run", "runs", "delete"
                    ],
                    "description": "Automation operation to perform"
                },
                "id": {
                    "type": "string",
                    "description": "Automation UUID (required for get/update/pause/resume/run/runs/delete)"
                },
                "name": {
                    "type": "string",
                    "description": "Automation name (create)"
                },
                "instruction": {
                    "type": "string",
                    "description": "Goal instruction executed on each run (create, update)"
                },
                "description": {
                    "type": ["string", "null"],
                    "description": "Human-readable description (create, update); omit to keep, null to clear"
                },
                "trigger": {
                    "type": "string",
                    "enum": ["manual", "cron", "heartbeat"],
                    "description": "What starts the automation (create)"
                },
                "cron_pattern": {
                    "type": "string",
                    "description": "Cron expression, e.g. '0 */6 * * *' (required for the cron trigger)"
                },
                "timezone": {
                    "type": "string",
                    "description": "IANA timezone qualifying the cron schedule (create)"
                },
                "heartbeat_interval_secs": {
                    "type": "integer",
                    "description": "Repeat interval in seconds (required for the heartbeat trigger)"
                },
                "max_executions": {
                    "type": "integer",
                    "description": "Stop after N executions (create)"
                },
                "persona_id": {
                    "type": ["string", "null"],
                    "description": "Persona bound to the automation (create, update); omit to keep, null to unbind"
                },
                "project_id": {
                    "type": ["string", "null"],
                    "description": "Project bound to the automation (create, update); omit to keep, null for projectless"
                },
                "brain_space": {
                    "type": ["string", "null"],
                    "description": "Brain space bound to the automation (create, update); omit to keep, null to unbind"
                },
                "status": {
                    "type": "string",
                    "enum": ["active", "paused", "exhausted", "failed"],
                    "description": "Status filter (list)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max rows to return (list)"
                },
                "offset": {
                    "type": "integer",
                    "description": "Row offset for pagination (list)"
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

            "list" => {
                let p: ListAutomationsParams = serde_json::from_value(params.clone())
                    .map_err(|e| format!("list: invalid parameters: {e}"))?;
                let store = self.store.lock().await;
                match store.list_automations(p).await {
                    Ok(list) if list.is_empty() => {
                        Ok(AgentToolResult::success("No automations found."))
                    }
                    Ok(list) => Ok(AgentToolResult::success(
                        serde_json::to_string_pretty(&json!({
                            "automations": list.iter().map(automation_summary).collect::<Vec<_>>(),
                            "count": list.len(),
                        }))
                        .unwrap_or_default(),
                    )),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to list automations: {e}"
                    ))),
                }
            }

            "get" => {
                let key = id_of().ok_or_else(|| "get requires 'id' parameter".to_string())?;
                let automation = match self.resolve(&key).await {
                    Ok(a) => a,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                Ok(AgentToolResult::success(
                    serde_json::to_string_pretty(&json!({
                        "automation": automation,
                    }))
                    .unwrap_or_default(),
                ))
            }

            "update" => {
                let key = id_of().ok_or_else(|| "update requires 'id' parameter".to_string())?;
                let automation = match self.resolve(&key).await {
                    Ok(a) => a,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                let p: UpdateAutomationParams = serde_json::from_value(params.clone())
                    .map_err(|e| format!("update: invalid parameters: {e}"))?;
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

            "pause" => {
                let key = id_of().ok_or_else(|| "pause requires 'id' parameter".to_string())?;
                self.set_status(&key, AutomationStatus::Paused).await
            }

            "resume" => {
                let key = id_of().ok_or_else(|| "resume requires 'id' parameter".to_string())?;
                self.set_status(&key, AutomationStatus::Active).await
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
                        "automation_id": automation.id,
                        "run_id": run_id,
                        "note": "Run started in the background; poll the runs action for the outcome.",
                    }))
                    .unwrap_or_default(),
                ))
            }

            "runs" => {
                let key = id_of().ok_or_else(|| "runs requires 'id' parameter".to_string())?;
                let automation = match self.resolve(&key).await {
                    Ok(a) => a,
                    Err(e) => return Ok(AgentToolResult::error(e)),
                };
                let runs = {
                    let store = self.store.lock().await;
                    store.list_runs(&automation.id).await
                };
                match runs {
                    Ok(runs) => Ok(AgentToolResult::success(
                        serde_json::to_string_pretty(&json!({
                            "automation_id": automation.id,
                            "runs": runs,
                            "count": runs.len(),
                        }))
                        .unwrap_or_default(),
                    )),
                    Err(e) => Ok(AgentToolResult::error(format!("Failed to list runs: {e}"))),
                }
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
                "Unknown automation action '{other}'. Valid: create, list, get, \
                 update, pause, resume, run, runs, delete"
            )),
        }
    }
}

impl AutomationTool {
    /// Shared helper for the `pause` / `resume` ops.
    async fn set_status(
        &self,
        key: &str,
        status: AutomationStatus,
    ) -> Result<AgentToolResult, ToolError> {
        let automation = match self.resolve(key).await {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool() -> AutomationTool {
        AutomationTool::for_store(Arc::new(Mutex::new(AutomationStore::in_memory().unwrap())))
    }

    async fn run_action(
        tool: &AutomationTool,
        params: Value,
    ) -> Result<AgentToolResult, ToolError> {
        tool.execute("call-1", params, None, &ToolContext::default())
            .await
    }

    #[test]
    fn name_and_label_are_automation() {
        let t = tool();
        assert_eq!(t.name(), "automation");
        assert_eq!(t.label(), "Automations");
    }

    #[test]
    fn schema_lists_only_the_nine_actions() {
        let schema = tool().parameters_schema();
        let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
        let action_strs: Vec<&str> = actions.iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(
            action_strs,
            vec![
                "create", "list", "get", "update", "pause", "resume", "run", "runs", "delete",
            ]
        );
        assert_eq!(schema["required"][0], "action");
    }

    #[test]
    fn description_omits_removed_actions() {
        // No mention of view, edit, update_status, set_schedule, set_verify,
        // create_batch, add_comment, dependencies, backlog.
        for forbidden in [
            "create_batch",
            "view",
            "edit",
            "update_status",
            "set_schedule",
            "set_verify",
            "comment",
            "dependency",
            "backlog",
        ] {
            assert!(
                !tool().description().contains(forbidden),
                "description must not mention `{forbidden}`"
            );
        }
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
        assert_eq!(res.output, "No automations found.");
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
    async fn pause_and_resume_round_trip_status() {
        let tool = tool();
        let created = run_action(
            &tool,
            json!({"action": "create", "name": "P", "instruction": "x"}),
        )
        .await
        .unwrap();
        let id: String = serde_json::from_str::<serde_json::Value>(&created.output).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();

        let pause = run_action(&tool, json!({"action": "pause", "id": id}))
            .await
            .unwrap();
        assert!(pause.success, "pause failed: {}", pause.output);
        let store = tool.store.lock().await;
        assert_eq!(
            store.get_automation(&id).await.unwrap().status,
            AutomationStatus::Paused,
        );
        drop(store);

        let resume = run_action(&tool, json!({"action": "resume", "id": id}))
            .await
            .unwrap();
        assert!(resume.success, "resume failed: {}", resume.output);
        let store = tool.store.lock().await;
        assert_eq!(
            store.get_automation(&id).await.unwrap().status,
            AutomationStatus::Active,
        );
    }

    #[tokio::test]
    async fn runs_returns_run_history() {
        let tool = tool();
        // Pre-create one automation. The `run` action needs a kernel handle
        // (absent in the store-only constructor), so we can't trigger a run
        // here. We still exercise `runs` against the in-memory store: the
        // store starts with no runs, which is the empty-path the agent will
        // hit when listing history before any run.
        let created = run_action(
            &tool,
            json!({"action": "create", "name": "H", "instruction": "y"}),
        )
        .await
        .unwrap();
        let id: String = serde_json::from_str::<serde_json::Value>(&created.output).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();

        let res = run_action(&tool, json!({"action": "runs", "id": id}))
            .await
            .unwrap();
        assert!(res.success, "runs failed: {}", res.output);
        let body: serde_json::Value = serde_json::from_str(&res.output).unwrap();
        assert_eq!(body["count"], 0);
        assert!(body["runs"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn unknown_action_is_rejected() {
        let err = run_action(&tool(), json!({"action": "bogus"}))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("Unknown automation action"),
            "got: {err}"
        );
    }
}
