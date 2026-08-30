//! MCP manage tool — wraps `McpApi` behind the `AgentTool` interface.
//!
//! Provides agents with control over the Oxios MCP server registry: list,
//! add, update, remove, toggle enable/disable, and test (initialize) servers.
//!
//! Actions: list, add, update, remove, toggle, test.
//!
//! ## Example
//!
//! ```json
//! { "action": "list" }
//! { "action": "add", "name": "github", "command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"] }
//! { "action": "update", "name": "github", "command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"], "enabled": true }
//! { "action": "remove", "name": "github" }
//! { "action": "toggle", "name": "github" }
//! { "action": "test", "name": "github" }
//! ```

use std::collections::HashMap;

use async_trait::async_trait;
use oxicode_sdk::{AgentTool, AgentToolResult, ToolContext};
#[cfg(test)]
use oxios_mcp::McpBridge;
use oxios_mcp::McpServer;
use serde_json::{Value, json};

#[cfg(test)]
use std::sync::Arc;

use crate::kernel_handle::KernelHandle;
use crate::kernel_handle::McpApi;

/// Agent tool for MCP server registry management.
///
/// Wraps the `McpApi` domain of the `KernelHandle`. Allows agents to list,
/// add, update, remove, enable/disable, and test MCP servers.
///
/// ## Actions
///
/// | Action   | Description                                       | Required params              | Optional params |
/// |----------|---------------------------------------------------|------------------------------|-----------------|
/// | `list`   | List all configured MCP servers                   | —                            | —               |
/// | `add`    | Register and initialize a new MCP server          | `name`, `command`            | `args`, `env`   |
/// | `update` | Overwrite an existing server's configuration     | `name`, `command`, `enabled` | `args`, `env`   |
/// | `remove` | Disconnect and delete an MCP server               | `name`                       | —               |
/// | `toggle` | Flip the enabled/disabled state of a server       | `name`                       | —               |
/// | `test`   | (Re-)initialize a server and report client status | `name`                       | —               |
pub struct McpManageTool {
    api: McpApi,
}

impl McpManageTool {
    /// Create a new `McpManageTool` from a `KernelHandle`.
    pub fn from_kernel(kernel: &KernelHandle) -> Self {
        Self {
            api: kernel.mcp.clone(),
        }
    }

    /// Build a `McpManageTool` backed by an empty bridge, for unit tests that
    /// only exercise parameter-schema and unknown-action dispatch.
    #[cfg(test)]
    fn for_tests() -> Self {
        Self {
            api: McpApi::new(Arc::new(McpBridge::new())),
        }
    }
}

impl std::fmt::Debug for McpManageTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpManageTool").finish()
    }
}

#[async_trait]
impl AgentTool for McpManageTool {
    fn name(&self) -> &str {
        "mcp_manage"
    }

    fn label(&self) -> &str {
        "MCP Manage"
    }

    fn description(&self) -> &'static str {
        "Manage Oxios's MCP server registry: list, add, update, remove, enable/disable, and test servers."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "add", "update", "remove", "toggle", "test"],
                    "description": "MCP server registry operation to perform"
                },
                "name": {
                    "type": "string",
                    "description": "Server name (unique identifier). Required for add/update/remove/toggle/test."
                },
                "command": {
                    "type": "string",
                    "description": "Command to execute (required for add/update). Examples: 'npx', 'python', 'node'."
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Arguments passed to the command (add/update)."
                },
                "env": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Environment variables for the server process (add/update)."
                },
                "enabled": {
                    "type": "boolean",
                    "description": "Whether the server is enabled (required for update; default true on add)."
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
    ) -> Result<AgentToolResult, oxicode_sdk::ToolError> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter: action".to_string())?;

        match action {
            "list" => self.handle_list().await,
            "add" => self.handle_add(&params).await,
            "update" => self.handle_update(&params).await,
            "remove" => self.handle_remove(&params).await,
            "toggle" => self.handle_toggle(&params).await,
            "test" => self.handle_test(&params).await,
            other => Err(format!(
                "Unknown mcp_manage action '{other}'. Valid: list, add, update, remove, toggle, test"
            )),
        }
    }
}

// ── Action handlers ──────────────────────────────────────────────────────────

impl McpManageTool {
    async fn handle_list(&self) -> Result<AgentToolResult, oxicode_sdk::ToolError> {
        let names = self.api.list_servers();
        if names.is_empty() {
            return Ok(AgentToolResult::success("No MCP servers configured."));
        }

        let mut entries: Vec<Value> = Vec::with_capacity(names.len());
        for name in &names {
            let entry = match self.api.get_server(name) {
                Some(server) => {
                    let connected = self.api.client_status(name).await.unwrap_or(false);
                    json!({
                        "name": server.name,
                        "command": server.command,
                        "args": server.args,
                        "env": server.env,
                        "enabled": server.enabled,
                        "connected": connected,
                    })
                }
                None => json!({"name": name, "missing": true}),
            };
            entries.push(entry);
        }

        Ok(AgentToolResult::success(
            serde_json::to_string_pretty(&json!({
                "servers": entries,
                "count": entries.len(),
            }))
            .unwrap_or_else(|_| "[]".to_string()),
        ))
    }

    async fn handle_add(&self, params: &Value) -> Result<AgentToolResult, oxicode_sdk::ToolError> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "add requires 'name' parameter".to_string())?
            .to_string();
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "add requires 'command' parameter".to_string())?
            .to_string();
        let args = parse_string_array(params.get("args"));
        let env = parse_env_map(params.get("env"));
        let enabled = params
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let mut server = McpServer::new(&name, &command);
        server.args = args;
        server.env = env;
        server.enabled = enabled;

        self.api.register_server(server);

        // Initialize, mirroring the register route handler pattern. If init
        // fails, roll back the registration so the registry doesn't keep a
        // phantom entry for a server that never came up.
        if let Err(e) = self.api.init_server(&name).await {
            tracing::error!(
                server = %name,
                error = %e,
                "MCP server init failed after registration; rolling back"
            );
            if let Err(rb) = self.api.remove_server(&name).await {
                tracing::warn!(
                    server = %name,
                    error = %rb,
                    "Failed to roll back MCP server registration"
                );
            }
            return Ok(AgentToolResult::error(format!(
                "Failed to initialize MCP server '{name}': {e}. Registration rolled back."
            )));
        }

        let connected = self.api.client_status(&name).await.unwrap_or(true);
        Ok(AgentToolResult::success(
            serde_json::to_string_pretty(&json!({
                "status": "registered",
                "name": name,
                "command": command,
                "connected": connected,
            }))
            .unwrap_or_else(|_| "{}".to_string()),
        ))
    }

    async fn handle_update(
        &self,
        params: &Value,
    ) -> Result<AgentToolResult, oxicode_sdk::ToolError> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "update requires 'name' parameter".to_string())?
            .to_string();
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "update requires 'command' parameter".to_string())?
            .to_string();
        let enabled = params
            .get("enabled")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| "update requires 'enabled' parameter".to_string())?;
        let args = parse_string_array(params.get("args"));
        let env = parse_env_map(params.get("env"));

        // PUT must replace, not create: reject unknown servers (the bridge's
        // update_server would otherwise register a phantom entry).
        let Some(previous) = self.api.get_server(&name) else {
            return Ok(AgentToolResult::error(format!(
                "MCP server '{name}' not found"
            )));
        };

        if let Err(e) = self
            .api
            .update_server(&name, command, args, env, enabled)
            .await
        {
            return Ok(AgentToolResult::error(format!(
                "Failed to update MCP server '{name}': {e}"
            )));
        }

        // Re-initialize when the server should run; roll back to the previous
        // configuration when the new one fails to come up (REST contract).
        if enabled && let Err(e) = self.api.init_server(&name).await {
            tracing::error!(
                server = %name,
                error = %e,
                "MCP server init failed after update; rolling back"
            );
            let _ = self
                .api
                .update_server(
                    &name,
                    previous.command.clone(),
                    previous.args.clone(),
                    previous.env.clone(),
                    previous.enabled,
                )
                .await;
            if previous.enabled {
                let _ = self.api.init_server(&name).await;
            }
            return Ok(AgentToolResult::error(format!(
                "Failed to initialize MCP server '{name}' after update: {e}. Update rolled back."
            )));
        }

        Ok(AgentToolResult::success(
            serde_json::to_string_pretty(&json!({
                "status": "updated",
                "name": name,
                "enabled": enabled,
            }))
            .unwrap_or_else(|_| "{}".to_string()),
        ))
    }

    async fn handle_remove(
        &self,
        params: &Value,
    ) -> Result<AgentToolResult, oxicode_sdk::ToolError> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "remove requires 'name' parameter".to_string())?
            .to_string();

        match self.api.remove_server(&name).await {
            Ok(()) => Ok(AgentToolResult::success(
                serde_json::to_string_pretty(&json!({
                    "status": "removed",
                    "name": name,
                }))
                .unwrap_or_else(|_| "{}".to_string()),
            )),
            Err(e) => Ok(AgentToolResult::error(format!(
                "Failed to remove MCP server '{name}': {e}"
            ))),
        }
    }

    async fn handle_toggle(
        &self,
        params: &Value,
    ) -> Result<AgentToolResult, oxicode_sdk::ToolError> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "toggle requires 'name' parameter".to_string())?
            .to_string();

        match self.api.toggle_server(&name).await {
            Ok(enabled) => Ok(AgentToolResult::success(
                serde_json::to_string_pretty(&json!({
                    "name": name,
                    "enabled": enabled,
                }))
                .unwrap_or_else(|_| "{}".to_string()),
            )),
            Err(e) => Ok(AgentToolResult::error(format!(
                "Failed to toggle MCP server '{name}': {e}"
            ))),
        }
    }

    async fn handle_test(&self, params: &Value) -> Result<AgentToolResult, oxicode_sdk::ToolError> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "test requires 'name' parameter".to_string())?
            .to_string();

        let init_result = self.api.init_server(&name).await;
        let connected = self.api.client_status(&name).await.unwrap_or(false);
        let tool_count = self
            .api
            .cached_tools(&name)
            .await
            .map(|t| t.len())
            .unwrap_or(0);

        match init_result {
            Ok(()) => Ok(AgentToolResult::success(
                serde_json::to_string_pretty(&json!({
                    "status": "ok",
                    "name": name,
                    "connected": connected,
                    "tools": tool_count,
                }))
                .unwrap_or_else(|_| "{}".to_string()),
            )),
            Err(e) => Ok(AgentToolResult::error(format!(
                "MCP server '{name}' init failed: {e} (connected={connected}, tools={tool_count})"
            ))),
        }
    }
}

// ── Param helpers ────────────────────────────────────────────────────────────

fn parse_string_array(value: Option<&Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    let Some(arr) = value.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect()
}

fn parse_env_map(value: Option<&Value>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(obj) = value.and_then(|v| v.as_object()) else {
        return out;
    };
    for (k, v) in obj {
        if let Some(s) = v.as_str() {
            out.insert(k.clone(), s.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_name_is_mcp_manage() {
        let schema = McpManageTool::for_tests().parameters_schema();
        assert_eq!(McpManageTool::for_tests().name(), "mcp_manage");
        let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
        for a in ["list", "add", "update", "remove", "toggle", "test"] {
            assert!(actions.iter().any(|x| x == a), "schema missing action {a}");
        }
    }

    #[tokio::test]
    async fn unknown_action_is_tool_error() {
        let tool = McpManageTool::for_tests();
        let err = tool
            .execute(
                "c-1",
                json!({"action": "nope"}),
                None,
                &ToolContext::default(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Unknown mcp_manage action"));
    }

    #[tokio::test]
    async fn missing_required_param_is_tool_error() {
        let tool = McpManageTool::for_tests();
        let err = tool
            .execute(
                "c-1",
                json!({"action": "add"}),
                None,
                &ToolContext::default(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("requires"));
    }

    #[tokio::test]
    async fn update_unknown_name_returns_not_found_without_mutation() {
        // Param validation order: validate → pre-check → mutate. An unknown
        // name must short-circuit before any bridge mutation, matching the
        // REST PUT contract (no phantom registration).
        let tool = McpManageTool::for_tests();
        let result = tool
            .execute(
                "c-1",
                json!({
                    "action": "update",
                    "name": "phantom",
                    "command": "npx",
                    "enabled": true,
                }),
                None,
                &ToolContext::default(),
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert!(
            result.output.contains("MCP server 'phantom' not found"),
            "expected not-found error, got: {}",
            result.output
        );
        // Pre-check is the only mutation gate: bridge must remain empty.
        assert!(tool.api.list_servers().is_empty());
    }
}
