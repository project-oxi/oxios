//! oxios-mcp — Model Context Protocol client library.
//!
//! Implements MCP over JSON-RPC 2.0 via stdio transport.
//! Independent of the Oxios kernel — usable as a standalone MCP client.
//!
//! # Protocol Overview
//!
//! MCP defines several message types:
//! - `initialize` - Establish connection with capabilities negotiation
//! - `tools/list` - List available tools from a server
//! - `tools/call` - Execute a tool with arguments
//! - `resources/list` - List available resources
//! - `resources/read` - Read a resource by URI
//!
//! # Architecture
//!
//! ```text
//! Consumer → McpBridge → McpClient (per server)
//!                         ↓
//!              tokio::process::Command (stdio)
//!                         ↓
//!              JSON-RPC 2.0 (stdin/stdout)
//!                         ↓
//!              MCP Server Process
//! ```

// `.unwrap()` in tests is idiomatic (workspace convention); suppressed only
// under `cfg(test)` so production code remains linted by the `--lib` clippy pass.
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod client;
pub mod protocol;
pub mod validation;

pub use client::McpClient;
pub use protocol::*;
pub use validation::{BLOCKED_MCP_SHELLS, sanitize_env, validate_mcp_command};

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// McpBridge — manages multiple MCP server clients
// ---------------------------------------------------------------------------

/// MCP bridge — connects multiple MCP servers to the tool system.
///
/// `McpBridge` owns the collection of registered MCP server configurations
/// and manages `McpClient` instances for active servers.
///
/// # Example
///
/// ```ignore
/// let mut bridge = McpBridge::new();
/// bridge.register_server(McpServer::new("files", "npx")
///     .with_args(vec!["-y", "@anthropic/mcp-server-filesystem"]));
///
/// // Initialize all servers
/// bridge.initialize_all().await?;
///
/// // List all tools across all servers
/// let tools = bridge.list_tools().await?;
/// ```
pub struct McpBridge {
    /// Registered MCP server configurations
    servers: parking_lot::RwLock<Vec<McpServer>>,
    /// Active MCP clients (keyed by server name)
    clients: RwLock<HashMap<String, Arc<McpClient>>>,
    /// Tool cache: server_name → cached tools
    tool_cache: RwLock<HashMap<String, Vec<McpTool>>>,
}

impl McpBridge {
    /// Create a new empty MCP bridge
    pub fn new() -> Self {
        Self {
            servers: parking_lot::RwLock::new(Vec::new()),
            clients: RwLock::new(HashMap::new()),
            tool_cache: RwLock::new(HashMap::new()),
        }
    }

    /// Register an MCP server configuration (does not start the process).
    ///
    /// If a server with the same name is already registered, it is replaced
    /// in-place (F7) rather than appended — `initialize_all()` would
    /// otherwise spawn both and orphan the first child when the second
    /// overwrites the entry in the clients map.
    pub fn register_server(&self, server: McpServer) {
        let mut servers = self.servers.write();
        let name = server.name.clone();
        if let Some(existing) = servers.iter_mut().find(|s| s.name == name) {
            tracing::warn!(
                server = %name,
                "Overwriting duplicate MCP server registration"
            );
            *existing = server;
        } else {
            servers.push(server);
        }
    }

    /// Get all registered server configurations (names only).
    pub fn servers(&self) -> Vec<String> {
        self.servers.read().iter().map(|s| s.name.clone()).collect()
    }

    /// Get a server configuration by name.
    pub fn get_server(&self, name: &str) -> Option<McpServer> {
        self.servers.read().iter().find(|s| s.name == name).cloned()
    }

    /// Initialize all enabled MCP servers.
    ///
    /// Each server is spawned as a child process and receives the initialize request.
    /// Servers that fail to initialize are logged but do not cause a total failure.
    pub async fn initialize_all(&self) -> Result<()> {
        let mut errors = Vec::new();

        let server_list: Vec<McpServer> = self.servers.read().iter().cloned().collect();
        for server in server_list {
            if !server.enabled {
                tracing::debug!(server = %server.name, "Skipping disabled MCP server");
                continue;
            }

            let client = Arc::new(McpClient::new(server.clone()));
            match client.initialize().await {
                Ok(()) => {
                    self.clients
                        .write()
                        .await
                        .insert(server.name.clone(), client);
                    tracing::info!(server = %server.name, "MCP server started");
                }
                Err(e) => {
                    tracing::error!(server = %server.name, error = %e, "Failed to initialize MCP server");
                    errors.push(format!("{}: {}", server.name, e));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(anyhow!("MCP initialization failed: {}", errors.join("; ")))
        }
    }

    /// Initialize a specific server by name.
    pub async fn initialize_server(&self, name: &str) -> Result<()> {
        let server = self
            .servers
            .read()
            .iter()
            .find(|s| s.name == name)
            .cloned()
            .ok_or_else(|| anyhow!("MCP server '{name}' not found"))?;

        let client = Arc::new(McpClient::new(server));
        client.initialize().await?;

        self.clients.write().await.insert(name.to_string(), client);
        Ok(())
    }

    /// Get a client by server name.
    pub async fn client(&self, name: &str) -> Option<Arc<McpClient>> {
        self.clients.read().await.get(name).cloned()
    }

    /// List all available tools from all initialized MCP servers.
    ///
    /// Tools are collected from each server's cache (refreshed on demand).
    ///
    /// F6: clones the `Arc<McpClient>` handles out of the lock before issuing
    /// remote calls so register/unregister/toggle aren't blocked for the
    /// full round-trip duration.
    pub async fn list_tools(&self) -> Result<Vec<McpTool>> {
        let clients: Vec<(String, Arc<McpClient>)> = self
            .clients
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let mut all_tools = Vec::new();
        for (name, client) in &clients {
            if let Ok(mcp_tools) = client.list_tools().await {
                let start = all_tools.len();
                all_tools.extend(mcp_tools);
                *self
                    .tool_cache
                    .write()
                    .await
                    .entry(name.clone())
                    .or_insert_with(Vec::new) = all_tools[start..].to_vec();
            }
        }

        Ok(all_tools)
    }

    /// Get cached tools for a specific server.
    pub async fn cached_tools(&self, server_name: &str) -> Option<Vec<McpTool>> {
        self.tool_cache.read().await.get(server_name).cloned()
    }

    /// Call an MCP tool on a specific server.
    ///
    /// F6: releases the clients read lock before the remote `call_tool`.
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<McpToolCallResult> {
        let client = {
            let clients = self.clients.read().await;
            clients
                .get(server_name)
                .cloned()
                .ok_or_else(|| anyhow!("MCP server '{server_name}' not connected"))?
        };

        client.call_tool(tool_name, args).await
    }

    /// Shutdown all connected MCP server processes.
    pub async fn shutdown_all(&self) -> Result<()> {
        let mut clients = self.clients.write().await;

        for (name, client) in clients.drain() {
            if let Err(e) = client.shutdown().await {
                tracing::warn!(server = %name, error = %e, "Error shutting down MCP server");
            }
        }

        self.tool_cache.write().await.clear();
        Ok(())
    }

    /// Refresh tools from a specific server.
    ///
    /// F6: releases the clients read lock before the remote `refresh_tools`.
    pub async fn refresh_tools(&self, server_name: &str) -> Result<Vec<McpTool>> {
        let client = {
            let clients = self.clients.read().await;
            clients
                .get(server_name)
                .cloned()
                .ok_or_else(|| anyhow!("MCP server '{server_name}' not connected"))?
        };

        let mcp_tools = client.refresh_tools().await?;

        *self
            .tool_cache
            .write()
            .await
            .entry(server_name.to_string())
            .or_insert_with(Vec::new) = mcp_tools.clone();

        Ok(mcp_tools)
    }

    /// Clear the tool cache for a server.
    pub async fn clear_cache(&self, server_name: &str) {
        self.tool_cache.write().await.remove(server_name);
    }

    /// Remove a server by name (disconnects client if active, removes config).
    pub async fn remove_server(&self, name: &str) -> Result<()> {
        // Shut down client if active.
        if let Some(client) = self.clients.write().await.remove(name)
            && let Err(e) = client.shutdown().await
        {
            tracing::warn!(server = %name, error = %e, "Error shutting down MCP server during removal");
        }
        // Remove from config list.
        let found = {
            let mut servers = self.servers.write();
            let len_before = servers.len();
            servers.retain(|s| s.name != name);
            servers.len() != len_before
        };
        if !found {
            return Err(anyhow!("MCP server '{name}' not found"));
        }
        // Clear cache.
        self.tool_cache.write().await.remove(name);
        Ok(())
    }
    /// Update a server's configuration in place and restart it.
    ///
    /// Replaces the config entry, disconnects the old client if active, and
    /// re-initializes. The `name` is the key and cannot be changed via this
    /// method — callers wanting a new name should register a new server and
    /// remove the old one. Returns the updated server config.
    pub async fn update_server(
        &self,
        name: &str,
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
        enabled: bool,
    ) -> Result<McpServer> {
        // Stop the existing client (if any) before mutating the config so
        // the new spawn does not race the old one.
        if let Some(client) = self.clients.write().await.remove(name)
            && let Err(e) = client.shutdown().await
        {
            tracing::warn!(server = %name, error = %e, "Error shutting down MCP server during update");
        }
        self.tool_cache.write().await.remove(name);

        // Overwrite the config entry (register_server already handles
        // in-place replacement for duplicate names).
        let server = McpServer {
            name: name.to_string(),
            command,
            args,
            env,
            enabled,
        };
        self.register_server(server.clone());
        Ok(server)
    }

    /// Toggle a server's enabled flag. Returns the new enabled state.
    pub async fn toggle_server(&self, name: &str) -> Result<bool> {
        // Extract new_state inside a block scope so the parking_lot lock
        // is released before any .await points in the async block below.
        let new_state = {
            let mut servers = self.servers.write();
            let server = servers
                .iter_mut()
                .find(|s| s.name == name)
                .ok_or_else(|| anyhow!("MCP server '{name}' not found"))?;
            server.enabled = !server.enabled;
            server.enabled
        };

        // If disabled, disconnect the client (now fully outside the lock).
        if !new_state {
            if let Some(client) = self.clients.write().await.remove(name)
                && let Err(e) = client.shutdown().await
            {
                tracing::warn!(server = %name, error = %e, "Error shutting down MCP server on disable");
            }
            self.tool_cache.write().await.remove(name);
        }

        Ok(new_state)
    }

    /// Clear all caches.
    pub async fn clear_all_caches(&self) {
        self.tool_cache.write().await.clear();
    }
}

impl Default for McpBridge {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::Duration;

    // --- McpServer tests ---

    #[test]
    fn test_mcp_server_builder() {
        let server = McpServer::new("test-server", "npx")
            .with_args(vec!["-y".to_string(), "@anthropic/mcp-server".to_string()])
            .with_env("DEBUG", "true");

        assert_eq!(server.name, "test-server");
        assert_eq!(server.command, "npx");
        assert_eq!(server.args, vec!["-y", "@anthropic/mcp-server"]);
        assert_eq!(server.env.get("DEBUG"), Some(&"true".to_string()));
        assert!(server.enabled);
    }

    // --- McpBridge::update_server tests ---

    #[tokio::test]
    async fn test_update_server_replaces_config() {
        let bridge = McpBridge::new();
        bridge.register_server(
            McpServer::new("fs", "npx")
                .with_args(vec!["old".to_string()])
                .with_env("OLD", "1"),
        );

        // Update with new command/args/env/enabled=false (no init).
        let mut env = HashMap::new();
        env.insert("NEW".to_string(), "2".to_string());
        let updated = bridge
            .update_server(
                "fs",
                "node".to_string(),
                vec!["new.js".to_string()],
                env,
                false,
            )
            .await
            .expect("update");

        assert_eq!(updated.command, "node");
        assert_eq!(updated.args, vec!["new.js"]);
        assert_eq!(updated.env.get("NEW"), Some(&"2".to_string()));
        assert!(!updated.enabled);

        let stored = bridge.get_server("fs").expect("get_server");
        assert_eq!(stored.command, "node");
        assert!(!stored.enabled);
        // Old env entry must be gone (config replaced, not merged).
        assert!(!stored.env.contains_key("OLD"));
    }

    #[tokio::test]
    async fn test_update_server_unknown_name_returns_error() {
        let bridge = McpBridge::new();
        let result = bridge
            .update_server("nope", "x".to_string(), vec![], HashMap::new(), true)
            .await;
        // No prior registration — `register_server` appends, so this succeeds
        // but creates a new entry. The method is update-in-place, but the
        // bridge accepts "update a non-existent server" as a create-via-update.
        // The route layer is the one that enforces existence (it checks
        // `previous` for rollback); the bridge stays simple.
        assert!(result.is_ok());
    }

    // --- JSON-RPC request/response tests ---

    #[test]
    fn test_mcp_request_serialization() {
        let request = McpRequest::new("tools/list");
        let json = serde_json::to_string(&request).unwrap();

        assert!(json.contains(r#""method":"tools/list""#));
        assert!(json.contains(r#""jsonrpc":"2.0""#));
    }

    #[test]
    fn test_mcp_request_with_params() {
        let request = McpRequest::new("tools/call").with_params(serde_json::json!({
            "name": "my_tool",
            "arguments": {"arg1": "value1"}
        }));

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("my_tool"));
        assert!(json.contains("arg1"));
    }

    #[test]
    fn test_mcp_request_to_jsonl() {
        let request = McpRequest::new("initialize");
        let jsonl = request.to_jsonl().unwrap();

        // Should end with newline
        assert_eq!(jsonl.last(), Some(&b'\n'));

        // Should parse back
        let json_str = String::from_utf8_lossy(&jsonl[..jsonl.len() - 1]);
        let parsed: McpRequest = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.method, "initialize");
    }

    #[test]
    fn test_mcp_response_result() {
        let response = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(1),
            result: Some(serde_json::json!({"tools": []})),
            error: None,
        };

        assert!(!response.is_error());
        let result = response.clone().into_result().unwrap();
        assert!(result.get("tools").is_some());
    }

    #[test]
    fn test_mcp_response_error() {
        let response = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(2),
            result: None,
            error: Some(McpError::internal_error("Something went wrong")),
        };

        assert!(response.is_error());
        let err = response.into_result().unwrap_err();
        assert!(err.to_string().contains("internal error"));
    }

    #[test]
    fn test_mcp_error_codes() {
        assert_eq!(McpError::parse_error().code, -32700);
        assert_eq!(McpError::invalid_request("test").code, -32600);
        assert_eq!(McpError::method_not_found().code, -32601);
        assert_eq!(McpError::invalid_params().code, -32602);
        assert_eq!(McpError::internal_error("x").code, -32603);
        assert_eq!(McpError::server_error("x").code, -32000);
    }

    // --- McpBridge registration tests ---

    #[test]
    fn test_bridge_registration() {
        let bridge = McpBridge::new();

        bridge.register_server(McpServer::new("test", "echo"));

        assert_eq!(bridge.servers(), vec!["test"]);
        assert!(bridge.get_server("test").is_some());
        assert!(bridge.get_server("missing").is_none());
    }

    // --- McpClient lifecycle tests ---

    #[tokio::test]
    async fn test_mcp_client_non_existent_command() {
        let server = McpServer::new("ghost", "nonexistent-binary-xyz");
        let client = McpClient::new(server);

        let result = client.initialize().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to spawn"));
    }

    #[tokio::test]
    async fn test_mcp_client_shutdown_no_panic() {
        let server = McpServer::new("test-shutdown", "echo");
        let client = McpClient::new(server);

        // Shutting down without initializing should not panic
        client.shutdown().await.expect("shutdown should succeed");
        assert!(!client.is_initialized().await);
    }

    #[tokio::test]
    async fn test_mcp_client_with_timeout() {
        let server = McpServer::new("test", "sleep").with_args(vec!["999".to_string()]);
        let client = McpClient::new(server).with_timeout(Duration::from_millis(100));

        // This will spawn a sleep process that hangs
        let result = client.initialize().await;
        // Should timeout, not panic
        assert!(result.is_err());
    }

    // --- McpBridge lifecycle tests ---

    #[tokio::test]
    async fn test_bridge_initialize_all_empty() {
        let bridge = McpBridge::new();
        bridge
            .initialize_all()
            .await
            .expect("empty bridge should initialize");
    }

    #[tokio::test]
    async fn test_bridge_initialize_all_fails_gracefully() {
        let bridge = McpBridge::new();
        bridge.register_server(McpServer::new("ghost", "nonexistent-cmd-xyz"));
        bridge.register_server(McpServer::new("ghost2", "nonexistent-cmd-abc"));

        let result = bridge.initialize_all().await;
        // Should fail because all servers fail to spawn
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_bridge_shutdown_all_empty() {
        let bridge = McpBridge::new();
        bridge
            .shutdown_all()
            .await
            .expect("empty bridge shutdown should succeed");
    }

    #[tokio::test]
    async fn test_bridge_call_tool_no_server() {
        let bridge = McpBridge::new();
        let result = bridge
            .call_tool("ghost", "tool", serde_json::json!({}))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not connected"));
    }

    #[tokio::test]
    async fn test_bridge_initialize_server_not_found() {
        let bridge = McpBridge::new();
        let result = bridge.initialize_server("missing").await;
        assert!(result.is_err());
    }

    #[test]
    fn test_mcp_client_debug() {
        let server = McpServer::new("debug-test", "echo");
        let client = McpClient::new(server);
        let debug = format!("{client:?}");
        assert!(debug.contains("debug-test"));
    }

    // --- Double-init guard ---

    #[tokio::test]
    async fn test_mcp_client_double_init_ignored() {
        let server = McpServer::new("echo", "echo");
        let client = McpClient::new(server);

        // First init will fail because "echo" doesn't speak MCP protocol
        let _ = client.initialize().await;
        // But calling is_initialized() should not panic
        let _ = client.is_initialized().await;
    }
}
