//! Memory tools for cross-session agent memory (RFC-047).
//!
//! Provides three tools backed by the oxibrain daemon:
//! - `memory_write` — remember content as a brain episode
//! - `memory_read` — recall assembled context, or an entity's beliefs by id
//! - `memory_search` — hybrid search over the brain
//!
//! All tools degrade gracefully: when the daemon is unavailable they return a
//! clear message and the agent turn continues.

use async_trait::async_trait;
use std::sync::Arc;

use oxicode_sdk::{AgentTool, AgentToolResult, ToolContext};
use serde_json::{Value, json};

use crate::brain::BrainConnection;

/// Tool for writing memory entries that persist across sessions.
pub struct MemoryWriteTool {
    brain: Arc<BrainConnection>,
}

impl MemoryWriteTool {
    /// Create a new MemoryWriteTool.
    pub fn new(brain: Arc<BrainConnection>) -> Self {
        Self { brain }
    }

    /// Create a `MemoryWriteTool` from a [`KernelHandle`].
    ///
    /// Extracts the brain connection from the kernel facade. Panics if the
    /// brain facade is not attached (it always is on the cached handle).
    pub fn from_kernel(kernel: &crate::kernel_handle::KernelHandle) -> Self {
        let brain = kernel
            .brain
            .as_ref()
            .expect("brain facade attached")
            .connection();
        Self::new(brain)
    }
}

impl std::fmt::Debug for MemoryWriteTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryWriteTool").finish()
    }
}

#[async_trait]

impl AgentTool for MemoryWriteTool {
    fn name(&self) -> &str {
        "memory_write"
    }

    fn label(&self) -> &str {
        "Memory Write"
    }

    fn description(&self) -> &str {
        "Store a recallable agent memory — facts about the user, behavioral patterns, session observations, preference corrections. Internal to the agent. Persisted across sessions via the oxibrain daemon."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The memory content to store"
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: Value,
        _signal: Option<tokio::sync::oneshot::Receiver<()>>,
        _ctx: &ToolContext,
    ) -> Result<AgentToolResult, oxicode_sdk::ToolError> {
        let content = params["content"].as_str().unwrap_or("").to_string();
        if content.is_empty() {
            return Ok(AgentToolResult::error("content is required"));
        }

        match self.brain.remember(&content, "agent").await {
            Some(id) => Ok(AgentToolResult::success(format!(
                "Memory stored (episode id: {id})",
            ))),
            None => Ok(AgentToolResult::error(
                "brain daemon unavailable — memory not stored",
            )),
        }
    }
}

/// Tool for reading memory: an entity's beliefs by id, or assembled recall
/// context when no id is given.
pub struct MemoryReadTool {
    brain: Arc<BrainConnection>,
}

impl MemoryReadTool {
    /// Create a new MemoryReadTool.
    pub fn new(brain: Arc<BrainConnection>) -> Self {
        Self { brain }
    }

    /// Create a `MemoryReadTool` from a [`KernelHandle`].
    pub fn from_kernel(kernel: &crate::kernel_handle::KernelHandle) -> Self {
        let brain = kernel
            .brain
            .as_ref()
            .expect("brain facade attached")
            .connection();
        Self::new(brain)
    }
}

impl std::fmt::Debug for MemoryReadTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryReadTool").finish()
    }
}

#[async_trait]

impl AgentTool for MemoryReadTool {
    fn name(&self) -> &str {
        "memory_read"
    }

    fn label(&self) -> &str {
        "Memory Read"
    }

    fn description(&self) -> &str {
        "Read memory. Provide 'id' to get an entity's current beliefs, or omit it to recall assembled context (pinned facts, high-salience beliefs, recent episodes) within a token budget."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Optional entity ID to read beliefs for."
                },
                "budget": {
                    "type": "integer",
                    "description": "Max tokens for recalled context (default 3000)"
                }
            }
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: Value,
        _signal: Option<tokio::sync::oneshot::Receiver<()>>,
        _ctx: &ToolContext,
    ) -> Result<AgentToolResult, oxicode_sdk::ToolError> {
        let budget = params["budget"].as_u64().unwrap_or(3000) as usize;

        if let Some(id) = params["id"].as_str() {
            return match self.brain.get_entity(id).await {
                Some(value) => {
                    let text =
                        serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
                    Ok(AgentToolResult::success(&text))
                }
                None => Ok(AgentToolResult::error(
                    "brain daemon unavailable — cannot read memory",
                )),
            };
        }

        match self
            .brain
            .recall("recent activities and relevant facts", budget)
            .await
        {
            Some(context) => Ok(AgentToolResult::success(&context)),
            None => Ok(AgentToolResult::error(
                "brain daemon unavailable — no memory to recall",
            )),
        }
    }
}

/// Tool for searching the brain (hybrid retrieval).
pub struct MemorySearchTool {
    brain: Arc<BrainConnection>,
}

impl MemorySearchTool {
    /// Create a new MemorySearchTool.
    pub fn new(brain: Arc<BrainConnection>) -> Self {
        Self { brain }
    }

    /// Create a `MemorySearchTool` from a [`KernelHandle`].
    pub fn from_kernel(kernel: &crate::kernel_handle::KernelHandle) -> Self {
        let brain = kernel
            .brain
            .as_ref()
            .expect("brain facade attached")
            .connection();
        Self::new(brain)
    }
}

impl std::fmt::Debug for MemorySearchTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemorySearchTool").finish()
    }
}

#[async_trait]

impl AgentTool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn label(&self) -> &str {
        "Memory Search"
    }

    fn description(&self) -> &str {
        "Search the brain with hybrid retrieval (lexical + semantic). Returns ranked targets (episodes, statements, entities) with scores; use memory_read with an id for content."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Text to search for"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results (default 10)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: Value,
        _signal: Option<tokio::sync::oneshot::Receiver<()>>,
        _ctx: &ToolContext,
    ) -> Result<AgentToolResult, oxicode_sdk::ToolError> {
        let query = params["query"].as_str().unwrap_or("");
        if query.is_empty() {
            return Ok(AgentToolResult::error("query is required"));
        }

        let limit = params["limit"].as_u64().unwrap_or(10) as usize;

        match self.brain.search(query, "hybrid", limit).await {
            Some(value) => {
                let items = value
                    .get("items")
                    .and_then(|i| i.as_array())
                    .cloned()
                    .unwrap_or_default();
                if items.is_empty() {
                    return Ok(AgentToolResult::success("No matching memories found."));
                }
                let mut output = format!("Found {} matching memories:\n\n", items.len());
                for item in items.iter().take(limit) {
                    let kind = item
                        .pointer("/target/kind")
                        .and_then(|k| k.as_str())
                        .unwrap_or("memory");
                    let id = item
                        .pointer("/target/id")
                        .and_then(|i| i.as_str())
                        .unwrap_or("?");
                    let score = item
                        .get("fused_score")
                        .and_then(|s| s.as_f64())
                        .unwrap_or(0.0);
                    output.push_str(&format!(
                        "- {kind} {id} (score: {score:.3})\n",
                        kind = kind,
                        id = id,
                        score = score,
                    ));
                }
                output.push_str("\nUse memory_read with an id to get content.");
                Ok(AgentToolResult::success(&output))
            }
            None => Ok(AgentToolResult::error(
                "brain daemon unavailable — cannot search memory",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn make_degraded_brain() -> Arc<BrainConnection> {
        // A connection to a nonexistent socket is always degraded.
        let dir = tempfile::TempDir::new().unwrap();
        let config = crate::brain::BrainConfig::new(dir.path().join("missing.sock"), "personal");
        Arc::new(crate::brain::BrainConnection::connect(config).await)
    }

    #[tokio::test]
    async fn test_memory_write_tool_schema_and_degraded_execute() {
        let tool = MemoryWriteTool::new(make_degraded_brain().await);
        assert_eq!(tool.name(), "memory_write");
        let schema = tool.parameters_schema();
        assert!(schema["required"].is_array());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "content")
        );

        // Degraded daemon → clear error, no panic.
        let result = tool
            .execute(
                "t1",
                json!({"content": "hello"}),
                None,
                &ToolContext::default(),
            )
            .await
            .unwrap();
        assert!(!result.success, "degraded write must error");
    }

    #[tokio::test]
    async fn test_memory_read_tool_schema_and_degraded_execute() {
        let tool = MemoryReadTool::new(make_degraded_brain().await);
        assert_eq!(tool.name(), "memory_read");
        let result = tool
            .execute("t1", json!({}), None, &ToolContext::default())
            .await
            .unwrap();
        assert!(!result.success, "degraded read must error");
    }

    #[tokio::test]
    async fn test_memory_search_tool_schema_and_degraded_execute() {
        let tool = MemorySearchTool::new(make_degraded_brain().await);
        assert_eq!(tool.name(), "memory_search");
        let schema = tool.parameters_schema();
        assert!(schema["required"].is_array());
        let result = tool
            .execute(
                "t1",
                json!({"query": "rust"}),
                None,
                &ToolContext::default(),
            )
            .await
            .unwrap();
        assert!(!result.success, "degraded search must error");
    }
}
