//! Engine tool — wraps `EngineApi` behind the `AgentTool` interface.
//!
//! Provides agents with control over Oxios's model/engine settings:
//! current defaults, model catalog search, default model switch, and
//! copilot model switch.
//!
//! Actions: current, list_models, set_default_model, set_copilot_model.
//!
//! ## Example
//!
//! ```json
//! { "action": "current" }
//! { "action": "list_models", "provider": "anthropic", "query": "sonnet" }
//! { "action": "list_models", "query": "gpt-4o" }
//! { "action": "list_models" }
//! { "action": "set_default_model", "model": "anthropic/claude-sonnet-4-20250514" }
//! { "action": "set_copilot_model" }
//! { "action": "set_copilot_model", "model": "openai/gpt-4o-mini" }
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::kernel_handle::{EngineApi, KernelHandle};

/// Agent tool for inspecting and changing Oxios's model/engine settings.
///
/// Wraps the [`EngineApi`] domain of the [`KernelHandle`]. Allows agents to
/// read current defaults, search the model catalog, and switch the default
/// or copilot model.
///
/// ## Actions
///
/// | Action              | Description                                                  | Required params | Optional params        |
/// |---------------------|--------------------------------------------------------------|-----------------|------------------------|
/// | `current`           | Read the live engine config                                  | —               | —                      |
/// | `list_models`       | Browse / search the model catalog                            | —               | `provider`, `query`    |
/// | `set_default_model` | Persist + hot-swap a new default model                       | `model`         | —                      |
/// | `set_copilot_model` | Set or clear the QuickAsk (copilot) model override           | —               | `model` (omit to clear)|
pub struct EngineTool {
    /// Cheap clone of `KernelHandle::engine`. Deref coercion keeps every
    /// `tool.engine.x()` call site compiling identically to before the
    /// Arc-wrap migration.
    engine: Arc<EngineApi>,
}

impl EngineTool {
    /// Create a new `EngineTool` from a `KernelHandle`.
    pub fn from_kernel(kernel: &KernelHandle) -> Self {
        Self {
            engine: Arc::clone(&kernel.engine),
        }
    }

    /// Build an `EngineTool` backed by a default `EngineApi`, for unit tests
    /// that only exercise parameter-schema and unknown-action dispatch.
    ///
    /// Mirrors the cheapest constructible `EngineApi` pattern used in
    /// `engine_api.rs` / `supervisor.rs` / `tools/kernel_bridge.rs` —
    /// `OxiosEngine::new("anthropic/claude-sonnet-4-20250514")` plus a
    /// default config and a non-existent config path (the three tests
    /// never reach the underlying API). Production constructor semantics
    /// are unchanged.
    #[cfg(test)]
    fn for_tests() -> Self {
        use crate::config::OxiosConfig;
        use crate::engine::{EngineHandle, OxiosEngine};
        use crate::kernel_handle::RoutingStats;
        use parking_lot::RwLock;
        use std::path::PathBuf;

        let engine = Arc::new(OxiosEngine::new("anthropic/claude-sonnet-4-20250514"));
        let handle = Arc::new(EngineHandle::new(engine));
        let config = Arc::new(RwLock::new(OxiosConfig::default()));
        let api = EngineApi::new(
            config,
            PathBuf::from("/tmp/oxios-engine-tool-test-NONEXISTENT.toml"),
            Arc::new(RoutingStats::new()),
            handle,
        );
        Self {
            engine: Arc::new(api),
        }
    }
}

impl std::fmt::Debug for EngineTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineTool").finish()
    }
}

#[async_trait]
impl oxicode_sdk::AgentTool for EngineTool {
    fn name(&self) -> &str {
        "engine"
    }

    fn label(&self) -> &str {
        "Engine"
    }

    fn description(&self) -> &'static str {
        "Inspect and change Oxios's model/engine settings: current defaults, model catalog search, default model, copilot model."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["current", "list_models", "set_default_model", "set_copilot_model"],
                    "description": "Engine settings operation to perform"
                },
                "provider": {
                    "type": "string",
                    "description": "Provider id to browse (list_models). Optional; if absent, falls back to global search."
                },
                "query": {
                    "type": "string",
                    "description": "Substring filter for model id / name / provider (list_models)."
                },
                "model": {
                    "type": "string",
                    "description": "Model id to set. Required for set_default_model; omit on set_copilot_model to clear."
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
        _ctx: &oxicode_sdk::ToolContext,
    ) -> Result<oxicode_sdk::AgentToolResult, oxicode_sdk::ToolError> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter: action".to_string())?;

        match action {
            "current" => self.handle_current(),
            "list_models" => self.handle_list_models(&params),
            "set_default_model" => self.handle_set_default_model(&params).await,
            "set_copilot_model" => self.handle_set_copilot_model(&params).await,
            other => Err(format!(
                "Unknown engine action '{other}'. Valid: current, list_models, set_default_model, set_copilot_model"
            )),
        }
    }
}

// ── Action handlers ─────────────────────────────────────────────────────────

impl EngineTool {
    /// Return the current engine config (defaults + credential status + routing).
    fn handle_current(&self) -> Result<oxicode_sdk::AgentToolResult, oxicode_sdk::ToolError> {
        Ok(oxicode_sdk::AgentToolResult::success(
            serde_json::to_string_pretty(&self.engine.config())
                .unwrap_or_else(|_| "{}".to_string()),
        ))
    }

    /// List providers (no filter), or models per provider with optional query,
    /// or cross-provider search when only `query` is given.
    fn handle_list_models(
        &self,
        params: &Value,
    ) -> Result<oxicode_sdk::AgentToolResult, oxicode_sdk::ToolError> {
        let provider = params.get("provider").and_then(|v| v.as_str());
        let query = params.get("query").and_then(|v| v.as_str());

        let payload = if let Some(provider) = provider {
            let models = self.engine.models(provider, query);
            json!({
                "provider": provider,
                "query": query,
                "models": models,
                "count": models.len(),
            })
        } else if let Some(query) = query.filter(|q| !q.is_empty()) {
            let models = self.engine.search_models(query);
            json!({
                "query": query,
                "models": models,
                "count": models.len(),
            })
        } else {
            let providers = self.engine.providers();
            json!({
                "providers": providers,
                "count": providers.len(),
            })
        };

        Ok(oxicode_sdk::AgentToolResult::success(
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
        ))
    }

    /// Validate `model` upfront, then delegate to `EngineApi::set_model`
    /// which validates + persists + hot-swaps the runtime engine.
    async fn handle_set_default_model(
        &self,
        params: &Value,
    ) -> Result<oxicode_sdk::AgentToolResult, oxicode_sdk::ToolError> {
        let model = params
            .get("model")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "set_default_model requires 'model' parameter".to_string())?;

        match self.engine.set_model(model) {
            Ok(()) => Ok(oxicode_sdk::AgentToolResult::success(format!(
                "Default model set to `{model}`."
            ))),
            Err(e) => Ok(oxicode_sdk::AgentToolResult::error(format!(
                "Failed to set default model '{model}': {e}"
            ))),
        }
    }

    /// Optional `model` — omitting it clears the copilot override and the
    /// copilot path then inherits the default model.
    async fn handle_set_copilot_model(
        &self,
        params: &Value,
    ) -> Result<oxicode_sdk::AgentToolResult, oxicode_sdk::ToolError> {
        let model = params.get("model").and_then(|v| v.as_str());

        match self.engine.set_quick_ask_model(model) {
            Ok(()) => match model {
                Some(m) => Ok(oxicode_sdk::AgentToolResult::success(format!(
                    "Copilot model set to `{m}`."
                ))),
                None => Ok(oxicode_sdk::AgentToolResult::success(
                    "Copilot model cleared (inherits default).",
                )),
            },
            Err(e) => Ok(oxicode_sdk::AgentToolResult::error(format!(
                "Failed to set copilot model: {e}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxicode_sdk::AgentTool;
    use serde_json::json;

    #[test]
    fn tool_name_is_engine() {
        let tool = EngineTool::for_tests();
        assert_eq!(tool.name(), "engine");
        let schema = tool.parameters_schema();
        let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
        for a in [
            "current",
            "list_models",
            "set_default_model",
            "set_copilot_model",
        ] {
            assert!(actions.iter().any(|x| x == a), "schema missing action {a}");
        }
    }

    #[tokio::test]
    async fn unknown_action_is_tool_error() {
        let err = EngineTool::for_tests()
            .execute(
                "c-1",
                json!({"action": "nope"}),
                None,
                &oxicode_sdk::ToolContext::default(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Unknown engine action"));
    }

    #[tokio::test]
    async fn set_default_model_requires_model() {
        let err = EngineTool::for_tests()
            .execute(
                "c-1",
                json!({"action": "set_default_model"}),
                None,
                &oxicode_sdk::ToolContext::default(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("requires 'model'"));
    }
}
