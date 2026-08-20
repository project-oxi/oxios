//! Timeline tool — wraps [`TimelineApi`] behind the `AgentTool` interface (oxiline).
//!
//! Only compiled with the `timeline` cargo feature (the module declaration in
//! `builtin/mod.rs` is feature-gated). Registered conditionally by the
//! `kernel.timeline.manage` arm in
//! [`register_from_resolved_profile`](crate::tools::registration) when
//! `[timeline].enabled`. **Read-only (context-in)**: agents observe the user's
//! time-tracking data; they do not mutate it.

use std::sync::Arc;

use async_trait::async_trait;
use oxicode_sdk::{AgentTool, AgentToolResult, ToolContext};
use serde_json::{Value, json};

use crate::kernel_handle::{KernelHandle, TimelineApi};

/// Agent tool for reading the user's oxiline time-tracking data.
///
/// oxios reads the same SQLite database the oxiline app uses. v1 is read-only.
///
/// ## Operations
///
/// | Op           | Description                | Optional params          |
/// |--------------|----------------------------|--------------------------|
/// | `now`        | Current activity + today   | —                        |
/// | `activities` | List activities            | `active_only` (bool)     |
/// | `timeline`   | Recent records (newest 1st) | `days` (u32), `limit`    |
pub struct TimelineTool {
    /// Live slot — re-read each call so a web-UI enable/disable takes effect
    /// without re-registering the tool.
    slot: Arc<parking_lot::RwLock<Option<Arc<TimelineApi>>>>,
}

impl TimelineTool {
    /// Build from a kernel handle. Always registered when the `timeline`
    /// feature is compiled in; reads the live slot on each call and errors
    /// cleanly when oxiline is disabled or not yet connected.
    pub fn try_from_kernel(kernel: &KernelHandle) -> Option<Self> {
        Some(Self {
            slot: kernel.timeline.clone(),
        })
    }

    /// Snapshot the live facade, or error if oxiline is not connected.
    fn api(&self) -> Result<Arc<TimelineApi>, String> {
        self.slot.read().clone().ok_or_else(|| {
            "oxiline is not connected. Enable it in Settings → Timeline.".to_string()
        })
    }
}

impl std::fmt::Debug for TimelineTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimelineTool").finish()
    }
}

#[async_trait]
impl AgentTool for TimelineTool {
    fn name(&self) -> &str {
        "timeline"
    }

    fn label(&self) -> &str {
        "Timeline"
    }

    fn description(&self) -> &'static str {
        "Read the user's oxiline time-tracking data — current activity, today's \
         plan compliance, and recent records. Read-only; oxios shares the same \
         store the oxiline app uses."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "op": {
                    "type": "string",
                    "enum": ["now", "activities", "timeline"],
                    "description": "Timeline operation to perform"
                },
                "active_only": {
                    "type": "boolean",
                    "description": "activities: omit soft-deleted activities (default true)."
                },
                "days": {
                    "type": "integer",
                    "description": "timeline: how many days back to include (default 1).",
                    "default": 1
                },
                "limit": {
                    "type": "integer",
                    "description": "timeline: max records (default 20).",
                    "default": 20
                }
            },
            "required": ["op"]
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: Value,
        _signal: Option<tokio::sync::oneshot::Receiver<()>>,
        _ctx: &ToolContext,
    ) -> Result<AgentToolResult, oxicode_sdk::ToolError> {
        let op = params
            .get("op")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter: op".to_string())?;

        match op {
            "now" => self.exec_now(&params).await,
            "activities" => self.exec_activities(&params).await,
            "timeline" => self.exec_timeline(&params).await,
            other => Err(format!(
                "Unknown timeline op '{other}'. Valid: now, activities, timeline"
            )),
        }
        .map(Ok)
        .unwrap_or_else(|e| Ok(AgentToolResult::error(e)))
    }
}

impl TimelineTool {
    async fn exec_now(&self, _params: &Value) -> Result<AgentToolResult, String> {
        match self.api()?.now().await {
            Ok(state) => Ok(AgentToolResult::success(
                serde_json::to_string_pretty(&state).unwrap_or_default(),
            )),
            Err(e) => Ok(AgentToolResult::error(format!(
                "Failed to read current timeline state: {e}"
            ))),
        }
    }

    async fn exec_activities(&self, params: &Value) -> Result<AgentToolResult, String> {
        let active_only = params
            .get("active_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        match self.api()?.activities(active_only).await {
            Ok(items) => Ok(AgentToolResult::success(
                serde_json::to_string_pretty(&json!({
                    "activities": items,
                    "count": items.len()
                }))
                .unwrap_or_default(),
            )),
            Err(e) => Ok(AgentToolResult::error(format!(
                "Failed to list activities: {e}"
            ))),
        }
    }

    async fn exec_timeline(&self, params: &Value) -> Result<AgentToolResult, String> {
        let days = param_u32(params, "days", 1);
        let limit = param_u32(params, "limit", 20).clamp(1, 100);
        match self.api()?.timeline(days, limit).await {
            Ok(items) => Ok(AgentToolResult::success(
                serde_json::to_string_pretty(&json!({
                    "records": items,
                    "count": items.len()
                }))
                .unwrap_or_default(),
            )),
            Err(e) => Ok(AgentToolResult::error(format!(
                "Failed to read timeline: {e}"
            ))),
        }
    }
}

/// Parse an optional `u32` param, clamped to `[0, 365]`, default `def`.
fn param_u32(params: &Value, key: &str, def: u32) -> u32 {
    params
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|n| (n as u32).min(365))
        .unwrap_or(def)
}
