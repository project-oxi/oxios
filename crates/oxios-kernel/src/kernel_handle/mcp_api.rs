use crate::mcp::{self, McpBridge, McpServer, McpToolCallResult};
use crate::tools::tool_types::ToolDef;
use std::collections::HashMap;
use std::sync::Arc;

/// MCP system calls.
#[derive(Clone)]
pub struct McpApi {
    pub(crate) mcp_bridge: Arc<McpBridge>,
}

impl McpApi {
    /// Create a new McpApi.
    pub fn new(mcp_bridge: Arc<McpBridge>) -> Self {
        Self { mcp_bridge }
    }
    /// List registered MCP servers.
    pub fn list_servers(&self) -> Vec<String> {
        self.mcp_bridge.servers()
    }

    /// Get MCP server info.
    pub fn get_server(&self, name: &str) -> Option<McpServer> {
        self.mcp_bridge.get_server(name)
    }

    /// Register an MCP server.
    pub fn register_server(&self, server: McpServer) {
        self.mcp_bridge.register_server(server);
    }
    /// Overwrite a registered MCP server's configuration in place. The `name`
    /// is the key and cannot be changed. The old client is stopped before
    /// the config is mutated; callers (the route handler) are responsible
    /// for re-initializing the server afterwards so they can roll back on
    /// failure — same pattern as the register path.
    pub async fn update_server(
        &self,
        name: &str,
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
        enabled: bool,
    ) -> anyhow::Result<McpServer> {
        self.mcp_bridge
            .update_server(name, command, args, env, enabled)
            .await
    }

    /// Initialize a specific MCP server.
    pub async fn init_server(&self, name: &str) -> anyhow::Result<()> {
        self.mcp_bridge.initialize_server(name).await
    }

    /// Get MCP client status.
    pub async fn client_status(&self, name: &str) -> Option<bool> {
        if let Some(client) = self.mcp_bridge.client(name).await {
            Some(client.is_initialized().await)
        } else {
            None
        }
    }

    /// List all MCP tools as ToolDefs.
    pub async fn list_tools(&self) -> anyhow::Result<Vec<ToolDef>> {
        mcp::list_tool_defs(&self.mcp_bridge).await
    }

    /// Get cached tools for a server as ToolDefs.
    pub async fn cached_tools(&self, server_name: &str) -> Option<Vec<ToolDef>> {
        mcp::cached_tool_defs(&self.mcp_bridge, server_name).await
    }

    /// Call an MCP tool.
    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: serde_json::Value,
    ) -> anyhow::Result<McpToolCallResult> {
        self.mcp_bridge.call_tool(server, tool, arguments).await
    }

    /// MCP bridge reference.
    pub fn bridge(&self) -> &Arc<McpBridge> {
        &self.mcp_bridge
    }

    /// Number of configured MCP servers.
    pub fn server_count(&self) -> usize {
        self.mcp_bridge.servers().len()
    }

    /// Remove (disconnect and delete) an MCP server.
    pub async fn remove_server(&self, name: &str) -> anyhow::Result<()> {
        self.mcp_bridge.remove_server(name).await
    }

    /// Toggle MCP server enabled/disabled. Returns the new enabled state.
    pub async fn toggle_server(&self, name: &str) -> anyhow::Result<bool> {
        self.mcp_bridge.toggle_server(name).await
    }

    /// Shutdown all MCP servers.
    pub async fn shutdown_all(&self) -> anyhow::Result<()> {
        self.mcp_bridge.shutdown_all().await
    }
}
