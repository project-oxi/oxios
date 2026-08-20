//! Memo tool — wraps [`MemoApi`] behind the `AgentTool` interface (oximemo).
//!
//! Only compiled with the `memo` cargo feature (the module declaration in
//! `builtin/mod.rs` is feature-gated). Registered conditionally by the
//! `kernel.memo.manage` arm in
//! [`register_from_resolved_profile`](crate::tools::registration) when
//! `[memo].enabled`.
//!
//! Delegates to [`MemoApi`] (shared via `Arc`), so agent mutations publish
//! `KernelEvent::MemoCreated`/`MemoDeleted` like any kernel domain op.

use std::sync::Arc;

use async_trait::async_trait;
use oxicode_sdk::{AgentTool, AgentToolResult, ToolContext};
use serde_json::{Value, json};

use crate::kernel_handle::{KernelHandle, MemoApi};

/// Agent tool for oximemo memo management.
///
/// oxios reads/writes the user's oximemo vault directly — the same store the
/// oximemo app uses. Memos are quick-capture notes; oxios is a co-client.
///
/// ## Operations
///
/// | Op       | Description            | Required params | Optional params |
/// |----------|------------------------|-----------------|-----------------|
/// | `create` | Create a memo          | `body`          | `category`      |
/// | `get`    | Get a single memo      | `id`            | —               |
/// | `list`   | List recent memos      | —               | `limit`         |
/// | `search` | Full-text search memos | `query`         | `limit`         |
/// | `delete` | Soft-delete a memo     | `id`            | —               |
pub struct MemoTool {
    /// Live slot — re-read each call so a web-UI enable/disable takes effect
    /// without re-registering the tool.
    slot: Arc<parking_lot::RwLock<Option<Arc<MemoApi>>>>,
}

impl MemoTool {
    /// Build from a kernel handle. Always registered when the `memo` feature
    /// is compiled in; reads the live slot on each call and errors cleanly when
    /// oximemo is disabled or not yet connected (mirrors the email tool).
    pub fn try_from_kernel(kernel: &KernelHandle) -> Option<Self> {
        Some(Self {
            slot: kernel.memo.clone(),
        })
    }

    /// Snapshot the live facade, or error if oximemo is not connected.
    fn api(&self) -> Result<Arc<MemoApi>, String> {
        self.slot
            .read()
            .clone()
            .ok_or_else(|| "oximemo is not connected. Enable it in Settings → Memo.".to_string())
    }
}

impl std::fmt::Debug for MemoTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoTool").finish()
    }
}

#[async_trait]
impl AgentTool for MemoTool {
    fn name(&self) -> &str {
        "memo"
    }

    fn label(&self) -> &str {
        "Memo"
    }

    fn description(&self) -> &'static str {
        "Manage oximemo memos — create, get, list, search, delete. \
         Memos are the user's quick-capture notes in the oximemo app; oxios \
         reads and writes the same vault the app uses."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "op": {
                    "type": "string",
                    "enum": ["create", "get", "list", "search", "delete"],
                    "description": "Memo operation to perform"
                },
                "body": {
                    "type": "string",
                    "description": "Memo body (create). Markdown; #tags are auto-extracted."
                },
                "id": {
                    "type": "string",
                    "description": "Memo id (UUIDv7, hyphenated). Required for get/delete."
                },
                "query": {
                    "type": "string",
                    "description": "Full-text search query (search)."
                },
                "category": {
                    "type": "string",
                    "description": "Category id, e.g. \"inbox\", \"todo\" (create, optional)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results for list/search (default 20).",
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
            "create" => self.exec_create(&params).await,
            "get" => self.exec_get(&params).await,
            "list" => self.exec_list(&params).await,
            "search" => self.exec_search(&params).await,
            "delete" => self.exec_delete(&params).await,
            other => Err(format!(
                "Unknown memo op '{other}'. Valid: create, get, list, search, delete"
            )),
        }
    }
}

impl MemoTool {
    async fn exec_create(&self, params: &Value) -> Result<AgentToolResult, String> {
        let body = params
            .get("body")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "create requires 'body' parameter".to_string())?;
        let category = params
            .get("category")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        match self.api()?.create_memo(body.to_string(), category).await {
            Ok(memo) => Ok(AgentToolResult::success(
                serde_json::to_string_pretty(&json!({ "status": "created", "memo": memo }))
                    .unwrap_or_default(),
            )),
            Err(e) => Ok(AgentToolResult::error(format!(
                "Failed to create memo: {e}"
            ))),
        }
    }

    async fn exec_get(&self, params: &Value) -> Result<AgentToolResult, String> {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "get requires 'id' parameter".to_string())?;
        match self.api()?.get_memo(id).await {
            Ok(memo) => Ok(AgentToolResult::success(
                serde_json::to_string_pretty(&memo).unwrap_or_default(),
            )),
            Err(e) => Ok(AgentToolResult::error(format!("Failed to get memo: {e}"))),
        }
    }

    async fn exec_list(&self, params: &Value) -> Result<AgentToolResult, String> {
        let limit = param_limit(params);
        match self.api()?.list(limit).await {
            Ok(items) => Ok(AgentToolResult::success(
                serde_json::to_string_pretty(&json!({ "memos": items, "count": items.len() }))
                    .unwrap_or_default(),
            )),
            Err(e) => Ok(AgentToolResult::error(format!("Failed to list memos: {e}"))),
        }
    }

    async fn exec_search(&self, params: &Value) -> Result<AgentToolResult, String> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "search requires 'query' parameter".to_string())?;
        let limit = param_limit(params);
        match self.api()?.search(query, limit).await {
            Ok(items) => Ok(AgentToolResult::success(
                serde_json::to_string_pretty(&json!({
                    "query": query,
                    "results": items,
                    "count": items.len()
                }))
                .unwrap_or_default(),
            )),
            Err(e) => Ok(AgentToolResult::error(format!(
                "Failed to search memos: {e}"
            ))),
        }
    }

    async fn exec_delete(&self, params: &Value) -> Result<AgentToolResult, String> {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "delete requires 'id' parameter".to_string())?;
        match self.api()?.delete_memo(id).await {
            Ok(()) => Ok(AgentToolResult::success(format!("Memo '{id}' deleted."))),
            Err(e) => Ok(AgentToolResult::error(format!(
                "Failed to delete memo: {e}"
            ))),
        }
    }
}

/// Parse the optional `limit` param, clamped to `[1, 100]`, default 20.
fn param_limit(params: &Value) -> u32 {
    params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .clamp(1, 100) as u32
}
