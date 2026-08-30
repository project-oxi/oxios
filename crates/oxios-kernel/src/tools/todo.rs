//! Session-scoped todo: state provider, registry, and the `todo` agent tool.
//!
//! The SDK supplies the op semantics (`apply_ops`: three-state normalisation,
//! at most one in-progress task per phase, auto-promotion of the next task on
//! completion) and the model-facing summary (`format_summary`). This module
//! supplies the two things the SDK deliberately leaves to the host:
//!
//! - **Where the state lives.** [`TodoRegistry`] keys one [`OxiosTodoState`]
//!   per chat session, so a plan created on one turn is still there on the
//!   next. A per-run provider would reset the plan every message.
//! - **How the UI sees it.** The SDK's `TodoTool` ignores its `tool_call_id`,
//!   so it cannot publish a structured snapshot. [`TodoTool`] here applies the
//!   same ops and additionally stashes the resulting phases into the
//!   [`StructuredResultBus`], which carries them to `tool_end.results` and the
//!   Web UI's live checklist. Name, schema, and model-facing output are
//!   unchanged — the same wrapping `KernelWebSearchTool` does, for the same
//!   reason.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use oxicode_agent::tools::todo::{apply_ops, format_summary};
use oxicode_sdk::{
    AgentTool, AgentToolResult, TodoOp, TodoPhase, TodoStateProvider, TodoUpdateResult, ToolContext,
};
use parking_lot::{Mutex, RwLock};
use serde_json::{Value, json};

use super::structured_results::StructuredResultBus;

/// Todo state for one chat session.
///
/// Mirrors the SDK's `InMemoryTodoState` reference implementation; kept as our
/// own type so the registry can hand out `Arc`s and so the phases can be read
/// for the UI without going through the tool.
#[derive(Debug, Clone, Default)]
pub struct OxiosTodoState {
    phases: Arc<RwLock<Vec<TodoPhase>>>,
}

impl OxiosTodoState {
    /// Create empty state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the current phases.
    pub fn phases(&self) -> Vec<TodoPhase> {
        self.phases.read().clone()
    }
}

impl TodoStateProvider for OxiosTodoState {
    fn get_phases(&self) -> Vec<TodoPhase> {
        self.phases.read().clone()
    }

    fn set_phases_sync(&self, phases: Vec<TodoPhase>) {
        *self.phases.write() = phases;
    }

    fn apply_ops<'a>(
        &'a self,
        ops: Vec<TodoOp>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<TodoUpdateResult, String>> + Send + 'a>>
    {
        Box::pin(async move {
            // Take and release the write lock synchronously: holding a
            // parking_lot guard across an `.await` would make the future
            // `!Send` and break `tokio::spawn` upstream.
            let result = {
                let mut phases = self.phases.write();
                apply_ops(&mut phases, &ops)
            };
            Ok(result)
        })
    }
}

/// Per-session todo state, so a plan survives across a session's turns.
#[derive(Debug, Default)]
pub struct TodoRegistry {
    sessions: Mutex<HashMap<String, Arc<OxiosTodoState>>>,
}

impl TodoRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// State for a session, created on first use.
    pub fn for_session(&self, session: &str) -> Arc<OxiosTodoState> {
        self.sessions
            .lock()
            .entry(session.to_string())
            .or_insert_with(|| Arc::new(OxiosTodoState::new()))
            .clone()
    }

    /// Current phases for a session without creating state for it.
    ///
    /// Used by read-only surfaces (the API's session view) that must not
    /// mint state for a session that never made a plan.
    pub fn peek(&self, session: &str) -> Option<Vec<TodoPhase>> {
        self.sessions.lock().get(session).map(|s| s.phases())
    }

    /// Drop a session's plan. Called when the session is deleted.
    pub fn forget(&self, session: &str) {
        self.sessions.lock().remove(session);
    }
}

/// Serialize phases for the Web UI's live checklist.
///
/// Shape is deliberately flat and stable: the renderer keys off `status`
/// strings, not enum ordinals, so adding a status upstream degrades to an
/// unknown-status row instead of breaking the card.
pub fn phases_to_json(phases: &[TodoPhase]) -> Value {
    json!({
        "phases": phases
            .iter()
            .map(|p| json!({
                "name": p.name,
                "tasks": p.tasks.iter().map(|t| json!({
                    "content": t.content,
                    "status": t.status.as_str(),
                    "icon": t.status.icon(),
                    "blockReason": t.block_reason,
                })).collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
        "total": phases.iter().map(|p| p.tasks.len()).sum::<usize>(),
        "completed": phases
            .iter()
            .flat_map(|p| p.tasks.iter())
            .filter(|t| matches!(
                t.status,
                oxicode_sdk::TodoStatus::Completed | oxicode_sdk::TodoStatus::Abandoned
            ))
            .count(),
        "inProgress": phases
            .iter()
            .flat_map(|p| p.tasks.iter())
            .find(|t| t.status == oxicode_sdk::TodoStatus::InProgress)
            .map(|t| t.content.clone()),
    })
}

/// The `todo` tool. Same contract as the SDK's, plus a UI snapshot.
#[derive(Debug)]
pub struct TodoTool {
    bus: Arc<StructuredResultBus>,
}

impl TodoTool {
    /// Create the tool bound to the structured-result bus.
    pub fn new(bus: Arc<StructuredResultBus>) -> Self {
        Self { bus }
    }
}

#[async_trait]
impl AgentTool for TodoTool {
    fn name(&self) -> &str {
        "todo"
    }

    fn label(&self) -> &str {
        "Todo"
    }

    fn essential(&self) -> bool {
        false
    }

    fn description(&self) -> &str {
        "Phased todo list manager for the current conversation. Use init to \
         create a plan, start/done/drop to transition tasks, block/unblock to \
         gate a task on external input, append to add, rm to remove, view to \
         read. On each completion the earliest still-open task auto-promotes \
         to in_progress. Tasks should be 5-10 words describing WHAT not HOW. \
         The plan persists across the turns of this conversation."
    }

    fn parameters_schema(&self) -> Value {
        // Mirrors the SDK tool's schema exactly — the model must see one
        // contract regardless of which layer implements it.
        json!({
            "type": "object",
            "properties": {
                "ops": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "properties": {
                            "op": {
                                "type": "string",
                                "enum": ["init", "start", "done", "drop", "block", "unblock", "rm", "append", "view"]
                            },
                            "task": {"type": "string", "description": "Task content (verbatim)"},
                            "phase": {"type": "string", "description": "Phase name"},
                            "reason": {"type": "string", "description": "Why the task is blocked (block op only)"},
                            "items": {"type": "array", "items": {"type": "string"}},
                            "list": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "phase": {"type": "string"},
                                        "items": {"type": "array", "items": {"type": "string"}}
                                    }
                                }
                            }
                        },
                        "required": ["op"]
                    }
                }
            },
            "required": ["ops"]
        })
    }

    async fn execute(
        &self,
        tool_call_id: &str,
        params: Value,
        _signal: Option<tokio::sync::oneshot::Receiver<()>>,
        ctx: &ToolContext,
    ) -> Result<AgentToolResult, String> {
        let Some(provider) = ctx.todo.as_ref() else {
            return Ok(AgentToolResult::error(
                "Todo state is not configured for this run",
            ));
        };

        let Some(ops_value) = params.get("ops").cloned() else {
            return Ok(AgentToolResult::error("Missing required parameter: ops"));
        };
        let ops: Vec<TodoOp> = match serde_json::from_value(ops_value) {
            Ok(ops) => ops,
            Err(e) => return Ok(AgentToolResult::error(format!("Invalid ops format: {e}"))),
        };

        let result = match provider.apply_ops(ops).await {
            Ok(result) => result,
            Err(e) => return Ok(AgentToolResult::error(e)),
        };

        // The UI snapshot rides the bus; the model gets the text summary.
        self.bus
            .insert(tool_call_id, phases_to_json(&result.phases));

        Ok(AgentToolResult::success(format_summary(
            &result.phases,
            &result.errors,
            false,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxicode_sdk::TodoStatus;

    fn init_ops(items: &[&str]) -> Vec<TodoOp> {
        vec![TodoOp::Init {
            list: None,
            items: Some(items.iter().map(|s| s.to_string()).collect()),
        }]
    }

    #[tokio::test]
    async fn a_session_keeps_its_plan_across_turns() {
        let registry = TodoRegistry::new();

        // Turn 1 creates the plan.
        let turn1 = registry.for_session("sess-1");
        TodoStateProvider::apply_ops(turn1.as_ref(), init_ops(&["write code", "run tests"]))
            .await
            .unwrap();

        // Turn 2 asks the registry again — same state, plan intact.
        let turn2 = registry.for_session("sess-1");
        let phases = turn2.phases();
        assert_eq!(phases.iter().map(|p| p.tasks.len()).sum::<usize>(), 2);
    }

    #[tokio::test]
    async fn sessions_do_not_share_a_plan() {
        let registry = TodoRegistry::new();
        let a = registry.for_session("sess-a");
        TodoStateProvider::apply_ops(a.as_ref(), init_ops(&["only mine"]))
            .await
            .unwrap();

        assert!(registry.for_session("sess-b").phases().is_empty());
    }

    #[test]
    fn peek_does_not_mint_state() {
        let registry = TodoRegistry::new();
        assert!(registry.peek("never-seen").is_none());
        assert!(
            registry.peek("never-seen").is_none(),
            "peek must stay non-creating"
        );
    }

    #[tokio::test]
    async fn forget_drops_the_plan() {
        let registry = TodoRegistry::new();
        let s = registry.for_session("sess-1");
        TodoStateProvider::apply_ops(s.as_ref(), init_ops(&["task"]))
            .await
            .unwrap();
        registry.forget("sess-1");
        assert!(registry.peek("sess-1").is_none());
    }

    #[test]
    fn forget_is_idempotent_for_unknown_sessions() {
        let registry = TodoRegistry::new();
        // Session-delete cleanup runs for every deleted session; most never
        // made a plan. Forgetting twice must stay a quiet no-op.
        registry.forget("never-seen");
        registry.forget("never-seen");
        assert!(registry.peek("never-seen").is_none());
    }

    #[tokio::test]
    async fn snapshot_reports_progress_and_current_task() {
        let state = OxiosTodoState::new();
        TodoStateProvider::apply_ops(&state, init_ops(&["first", "second", "third"]))
            .await
            .unwrap();
        TodoStateProvider::apply_ops(
            &state,
            vec![TodoOp::Done {
                task: Some("first".into()),
                phase: None,
            }],
        )
        .await
        .unwrap();

        let snap = phases_to_json(&state.phases());
        assert_eq!(snap["total"], 3);
        assert_eq!(snap["completed"], 1);
        // Completing a task auto-promotes the next one, so the UI always has
        // something to show as "current".
        assert_eq!(snap["inProgress"], "second");
    }

    #[tokio::test]
    async fn snapshot_carries_status_strings_and_block_reasons() {
        let state = OxiosTodoState::new();
        TodoStateProvider::apply_ops(&state, init_ops(&["waiting"]))
            .await
            .unwrap();
        TodoStateProvider::apply_ops(
            &state,
            vec![TodoOp::Block {
                task: Some("waiting".into()),
                phase: None,
                reason: Some("needs a decision".into()),
            }],
        )
        .await
        .unwrap();

        let snap = phases_to_json(&state.phases());
        let task = &snap["phases"][0]["tasks"][0];
        assert_eq!(task["status"], TodoStatus::Blocked.as_str());
        assert_eq!(task["blockReason"], "needs a decision");
    }

    #[tokio::test]
    async fn set_phases_sync_replaces_state() {
        let state = OxiosTodoState::new();
        TodoStateProvider::apply_ops(&state, init_ops(&["old"]))
            .await
            .unwrap();
        state.set_phases_sync(Vec::new());
        assert!(state.phases().is_empty());
    }
}
