//! Project tool — wraps `ProjectManager` behind the `AgentTool` interface.
//!
//! Provides agents with Project query capabilities through an action-based
//! parameter schema. Actions: list, get.
//!
//! Agents can query projects; creating, updating, and removing projects is
//! user-level only.

use async_trait::async_trait;
use std::sync::Arc;

use oxicode_sdk::{AgentTool, AgentToolResult, ToolContext};
use serde_json::{Value, json};

use crate::kernel_handle::KernelHandle;
use crate::project::ProjectManager;

/// Agent tool for Project queries.
///
/// Wraps the `ProjectManager` behind a single `AgentTool` implementation.
/// The tool uses an `action` parameter to dispatch operations.
///
/// ## Actions
///
/// | Action | Description         | Required params |
/// |--------|---------------------|-----------------|
/// | `list` | List all Projects   | —               |
/// | `get`  | Get Project details | `id` or `name`  |
pub struct ProjectTool {
    project_manager: Option<Arc<ProjectManager>>,
}

impl ProjectTool {
    /// Create a new `ProjectTool` from a `KernelHandle`.
    pub fn from_kernel(kernel: &KernelHandle) -> Self {
        Self {
            project_manager: kernel.projects.as_ref().map(|p| p.project_manager.clone()),
        }
    }
}

impl std::fmt::Debug for ProjectTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectTool").finish()
    }
}

#[async_trait]
impl AgentTool for ProjectTool {
    fn name(&self) -> &str {
        "project"
    }

    fn label(&self) -> &str {
        "Project"
    }

    fn description(&self) -> &'static str {
        "Query registered Projects — work contexts with filesystem roots and \
         instructions. Actions: list, get."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "get"],
                    "description": "Project operation to perform"
                },
                "id": {
                    "type": "string",
                    "description": "Project UUID"
                },
                "name": {
                    "type": "string",
                    "description": "Project name (alternative to id for 'get')"
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

        let pm = self
            .project_manager
            .as_ref()
            .ok_or_else(|| "Project system not available (SQLite not enabled)".to_string())?;

        match action {
            "list" => {
                let projects = pm.list_projects();
                if projects.is_empty() {
                    return Ok(AgentToolResult::success("No Projects registered."));
                }
                let mut output = format!("Found {} Project(s):\n\n", projects.len());
                for p in &projects {
                    let roots_str = if p.root_paths.is_empty() {
                        "(no roots)".to_string()
                    } else {
                        p.root_paths
                            .iter()
                            .map(|p| p.to_string_lossy().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    output.push_str(&format!(
                        "- {} ({}) roots={}\n",
                        p.name,
                        &p.id.to_string()[..8.min(p.id.to_string().len())],
                        roots_str,
                    ));
                }
                Ok(AgentToolResult::success(output))
            }

            "get" => {
                let project = if let Some(id_str) = params.get("id").and_then(|v| v.as_str()) {
                    let id = uuid::Uuid::parse_str(id_str)
                        .map_err(|e| format!("Invalid project ID: {e}"))?;
                    pm.get_project(id)
                } else if let Some(name) = params.get("name").and_then(|v| v.as_str()) {
                    pm.get_project_by_name(name)
                } else {
                    return Err("'get' requires 'id' or 'name' parameter".to_string());
                };

                match project {
                    Some(p) => Ok(AgentToolResult::success(
                        serde_json::to_string_pretty(&json!({
                            "id": p.id.to_string(),
                            "name": p.name,
                            "root_paths": p.root_paths.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>(),
                            "instructions": p.instructions,
                            "last_active": p.last_active_at.to_rfc3339(),
                        }))
                        .unwrap_or_default(),
                    )),
                    None => Ok(AgentToolResult::error("Project not found")),
                }
            }

            other => Err(format!(
                "Unknown project action '{other}'. Valid: list, get"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_actions() {
        let schema = ProjectTool {
            project_manager: None,
        }
        .parameters_schema();
        let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
        assert_eq!(actions.len(), 2);
        assert!(actions.iter().any(|a| a == "list"));
        assert!(actions.iter().any(|a| a == "get"));
    }
}
