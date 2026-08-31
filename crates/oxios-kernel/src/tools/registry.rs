//! Tool metadata registry — known tools catalog for the frontend.
//!
//! This module provides a static catalog of all Oxios kernel tools
//! and their metadata (name, description, category). The frontend
//! settings UI uses this via `GET /api/tools/registry` to render
//! the `allowed_tools` multi-select widget.
//!
//! The catalog is a superset of the tools reachable via the agent runtime's
//! live path: the turn CSpace walk in
//! [`super::registration::register_tools_from_cspace_gated`]. It includes all
//! always-on tools and every CSpace-driven tool a profile could possibly
//! activate. `register_from_resolved_profile` is exported for the
//! profile-driven path but has no production caller yet.
//!
//! MCP tools are dynamically registered per-server and are NOT
//! included here. Users can type MCP tool names manually when
//! customising the `allowed_tools` list.

use serde::Serialize;

/// Phase D: per-tool human intervention requirement (LobeHub-aligned).
/// Drives the frontend 4-tier tool render registry's `interventions` slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub enum HumanIntervention {
    /// Tool is safe to run without user confirmation.
    #[default]
    None,
    /// Approval required only when args match certain criteria (path outside
    /// sandbox, etc.). The existing AccessGate path-based check remains the
    /// primary gate; this level is informational for the UI.
    Conditional,
    /// Approval always required before execution.
    Required,
}

/// Metadata for a single tool in the registry.
#[derive(Debug, Clone, Serialize)]
pub struct ToolMeta {
    /// Tool identifier (matches `AgentTool::name()`).
    pub name: &'static str,
    /// Human-readable description key (frontend translates via i18n).
    pub description_key: &'static str,
    /// Category slug for UI grouping.
    pub category: &'static str,
    /// Phase D: whether this tool requires human approval before execution.
    #[serde(default)]
    pub human_intervention: HumanIntervention,
}

impl ToolMeta {
    pub const fn new(
        name: &'static str,
        description_key: &'static str,
        category: &'static str,
    ) -> Self {
        Self {
            name,
            description_key,
            category,
            human_intervention: HumanIntervention::None,
        }
    }

    /// Phase D: builder method to override the default intervention level.
    pub const fn with_intervention(mut self, level: HumanIntervention) -> Self {
        self.human_intervention = level;
        self
    }
}

/// Return the full static tool catalog.
///
/// This is the single source of truth for "which tools exist" shown
/// in the frontend settings. The list mirrors
/// [`super::registration`] — always-on tools + CSpace-driven tools.
pub fn known_tools() -> &'static [ToolMeta] {
    TOOL_CATALOG
}

const TOOL_CATALOG: &[ToolMeta] = &[
    // ── Always-on tools (registered for every agent) ──────────────
    ToolMeta::new("read", "tools.read", "fs"),
    ToolMeta::new("write", "tools.write", "fs").with_intervention(HumanIntervention::Conditional),
    ToolMeta::new("edit", "tools.edit", "fs").with_intervention(HumanIntervention::Conditional),
    ToolMeta::new("grep", "tools.grep", "fs"),
    ToolMeta::new("find", "tools.find", "fs"),
    ToolMeta::new("ls", "tools.ls", "fs"),
    ToolMeta::new("web_search", "tools.webSearch", "comms"),
    ToolMeta::new("get_search_results", "tools.getSearchResults", "comms"),
    ToolMeta::new("todo", "tools.todo", "system"),
    ToolMeta::new("issue", "tools.issue", "system"),
    // ── Kernel domain tools (CSpace-driven) ───────────────────────
    ToolMeta::new("exec", "tools.exec", "exec").with_intervention(HumanIntervention::Required),
    ToolMeta::new("browse", "tools.browse", "comms"),
    ToolMeta::new("project", "tools.project", "system"),
    ToolMeta::new("kernel_agent", "tools.kernelAgent", "system"),
    ToolMeta::new("a2a_delegate", "tools.a2aDelegate", "a2a"),
    ToolMeta::new("a2a_send", "tools.a2aSend", "a2a"),
    ToolMeta::new("a2a_query", "tools.a2aQuery", "a2a"),
    ToolMeta::new("persona", "tools.persona", "system"),
    ToolMeta::new("cron", "tools.cron", "system"),
    ToolMeta::new("security", "tools.security", "system"),
    ToolMeta::new("budget", "tools.budget", "system"),
    ToolMeta::new("resource", "tools.resource", "system"),
    ToolMeta::new("knowledge", "tools.knowledge", "system"),
    ToolMeta::new("calendar", "tools.calendar", "system"),
    ToolMeta::new("send_email", "tools.sendEmail", "comms"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_populated() {
        let tools = known_tools();
        assert!(!tools.is_empty(), "tool catalog should not be empty");
        assert!(
            tools.iter().any(|t| t.name == "read"),
            "read tool should be in catalog"
        );
        assert!(
            tools.iter().any(|t| t.name == "exec"),
            "exec tool should be in catalog"
        );
        assert!(
            !tools.iter().any(|t| t.name == "memory_read"),
            "memory tools removed with the brain skill migration"
        );
    }

    #[test]
    fn all_tools_have_required_fields() {
        for tool in known_tools() {
            assert!(!tool.name.is_empty(), "tool name should not be empty");
            assert!(
                !tool.description_key.is_empty(),
                "description_key should not be empty"
            );
            assert!(!tool.category.is_empty(), "category should not be empty");
        }
    }

    #[test]
    fn no_duplicate_names() {
        let names: Vec<&str> = known_tools().iter().map(|t| t.name).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(names.len(), sorted.len(), "duplicate tool names found");
    }
}
