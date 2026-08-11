//! CSpace → Tool Registry mapping.
//!
//! This module bridges the capability system and the agent's tool registry.
//! Given an agent's [`CSpace`], it walks the capabilities and registers
//! exactly the set of tools the agent is authorised to use.
//!
//! # Registration tiers
//!
//! | Tier | Tools | Condition |
//! |------|-------|-----------|
//! | Always-on | `ReadTool`, `WriteTool`, `EditTool`, `GrepTool`, `FindTool`, `LsTool`, `WebSearchTool`, `GetSearchResultsTool` | Every agent gets these |
//! | CSpace-driven | `ExecTool`, `BrowserTool`, kernel domain tools, MCP, A2A, etc. | Only if a matching capability with sufficient rights exists |
//!
//! # Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use oxicode_sdk::ToolRegistry;
//! use oxios_kernel::capability::template::CapabilityTemplate;
//!
//! let registry = ToolRegistry::new();
//! let cspace = CapabilityTemplate::standard().build();
//! let cache = Arc::new(oxicode_sdk::SearchCache::new());
//! // register_tools_from_cspace(&registry, &kernel, &cspace, cache, agent_id);
//! ```

use std::sync::Arc;

use oxicode_sdk::{
    EditTool, FindTool, GetSearchResultsTool, GrepTool, LsTool, ReadTool, SearchCache,
    ToolRegistry, WebSearchTool, WriteTool,
};

use crate::KernelHandle;
use crate::access_manager::{AccessGate, AgentContext};
use crate::capability::{CSpace, ResourceRef, Rights};
use crate::tools::builtin::*;
use crate::tools::gated_tool::GatedTool;
use crate::tools::{A2aDelegateTool, A2aQueryTool, A2aSendTool, ExecTool, KnowledgeTool};
use crate::types::AgentId;

/// Register the always-on tool set into a [`ToolRegistry`].
///
/// Every agent receives these tools regardless of its capability space.
/// This consists of file-system tools (read, write, edit, grep, find, ls)
/// and web search tools.
///
/// This helper is also useful for unit tests that need a basic tool set
/// without constructing a full CSpace.
pub fn register_always_on(registry: &ToolRegistry, search_cache: Arc<SearchCache>) {
    registry.register(ReadTool::new());
    registry.register(WriteTool::new());
    registry.register(EditTool::new());
    registry.register(GrepTool::new());
    registry.register(FindTool::new());
    registry.register(LsTool::new());
    registry.register(WebSearchTool::new(search_cache.clone()));
    registry.register(GetSearchResultsTool::new(search_cache));
}

/// Register the headless-browser browse tools when the engine is available.
///
/// Uses oxios-owned tools (RFC-046) backed by `oxibrowser-core` 0.20 directly.
/// Only compiled with the `browser` feature; without it this is a no-op stub.
#[cfg(feature = "browser")]
fn register_browser_tools(kernel: &KernelHandle, registry: &ToolRegistry) {
    use crate::tools::browse::{BrowseTool, BrowseExtractTool, BrowseSessionTool, BrowseScriptTool};
    if let Some(browser) = &kernel.browser
        && let Some(engine) = browser.try_engine()
    {
        registry.register(BrowseTool::new(engine.clone()));
        registry.register(BrowseExtractTool::new(engine.clone()));
        registry.register(BrowseSessionTool::new(engine.clone()));
        registry.register(BrowseScriptTool::new(engine));
    }
}

/// No-op stub when the `browser` feature is disabled.
#[cfg(not(feature = "browser"))]
fn register_browser_tools(_kernel: &KernelHandle, _registry: &ToolRegistry) {}

/// Register always-on tools with access gate and (RFC-035) approval wrapping.
///
/// Same as [`register_always_on`] but wraps each tool in [`GatedTool`] so that
/// all file operations pass through the access gate and approval gate.
///
/// `approval_gate`, `event_bus`, and `pending_approvals` may all be `None`
/// for headless / test paths; the gated tool still honors the access gate
/// but skips the approval step.
#[allow(clippy::too_many_arguments)]
pub fn register_always_on_gated(
    registry: &ToolRegistry,
    search_cache: Arc<SearchCache>,
    gate: Arc<AccessGate>,
    context: AgentContext,
    approval_gate: Option<Arc<crate::approval::ApprovalGate>>,
    event_bus: Option<crate::event_bus::EventBus>,
    pending_approvals: Option<Arc<crate::tools::PendingToolApprovals>>,
    pending_path_access: Option<Arc<crate::tools::PendingPathAccess>>,
) {
    registry.register(GatedTool::with_approval(
        ReadTool::new(),
        gate.clone(),
        context.clone(),
        approval_gate.clone(),
        event_bus.clone(),
        pending_approvals.clone(),
        pending_path_access.clone(),
    ));
    registry.register(GatedTool::with_approval(
        WriteTool::new(),
        gate.clone(),
        context.clone(),
        approval_gate.clone(),
        event_bus.clone(),
        pending_approvals.clone(),
        pending_path_access.clone(),
    ));
    registry.register(GatedTool::with_approval(
        EditTool::new(),
        gate.clone(),
        context.clone(),
        approval_gate.clone(),
        event_bus.clone(),
        pending_approvals.clone(),
        pending_path_access.clone(),
    ));
    registry.register(GatedTool::with_approval(
        GrepTool::new(),
        gate.clone(),
        context.clone(),
        approval_gate.clone(),
        event_bus.clone(),
        pending_approvals.clone(),
        pending_path_access.clone(),
    ));
    registry.register(GatedTool::with_approval(
        FindTool::new(),
        gate.clone(),
        context.clone(),
        approval_gate.clone(),
        event_bus.clone(),
        pending_approvals.clone(),
        pending_path_access.clone(),
    ));
    registry.register(GatedTool::with_approval(
        LsTool::new(),
        gate.clone(),
        context.clone(),
        approval_gate.clone(),
        event_bus.clone(),
        pending_approvals.clone(),
        pending_path_access.clone(),
    ));
    registry.register(GatedTool::with_approval(
        WebSearchTool::new(search_cache.clone()),
        gate.clone(),
        context.clone(),
        approval_gate.clone(),
        event_bus.clone(),
        pending_approvals.clone(),
        pending_path_access.clone(),
    ));
    registry.register(GatedTool::with_approval(
        GetSearchResultsTool::new(search_cache),
        gate,
        context,
        approval_gate,
        event_bus,
        pending_approvals,
        pending_path_access.clone(),
    ));
}

/// Register tools into `registry` based on the agent's [`CSpace`].
///
/// First registers the always-on tier (file ops + web search), then walks
/// every capability in the CSpace and conditionally registers the
/// corresponding kernel tools.
///
/// # Arguments
///
/// * `registry` — The agent's tool registry to populate.
/// * `kernel` — Handle to the kernel for constructing tool instances.
/// * `cspace` — The agent's capability space (determines which tools are available).
/// * `search_cache` — Shared search cache for web search tools.
/// * `agent_id` — The agent's ID (used by A2A tools for routing).
///
/// # CSpace → Tool mapping
///
/// | ResourceRef | Required rights | Registered tools |
/// |-------------|----------------|-----------------|
/// | `Exec { .. }` | `EXECUTE` | `ExecTool` |
/// | `KernelDomain { "memory" }` | — | *(registered unconditionally in `register_all_kernel_tools`)* |
/// | `KernelDomain { "project" }` | any | `ProjectTool` |
/// | `KernelDomain { "agent" }` | any | `KernelAgentTool` |
/// | `KernelDomain { "a2a" }` | any | `A2aDelegateTool`, `A2aSendTool`, `A2aQueryTool` |
/// | `KernelDomain { "persona" }` | any | `PersonaTool` |
/// | `KernelDomain { "program" }` | any | *(deprecated — skills via CSpace)* |
/// | `KernelDomain { "cron" }` | any | `CronTool` |
/// | `KernelDomain { "security" }` | any | `SecurityTool` |
/// | `KernelDomain { "budget" }` | any | `BudgetTool` |
/// | `KernelDomain { "resource" }` | any | `ResourceTool` |
/// | `KernelDomain { "mcp" }` | any | `McpToolWrapper` |
/// | `Program { .. }` | — | *(not registered; surfaced via ToolRetriever)* |
pub fn register_tools_from_cspace(
    registry: &ToolRegistry,
    kernel: &KernelHandle,
    cspace: &CSpace,
    search_cache: Arc<SearchCache>,
    agent_id: AgentId,
) {
    // ── Tier 1: Always-on tools ─────────────────────────────────────
    register_always_on(registry, search_cache);

    // ── Tier 2: CSpace-driven tools ─────────────────────────────────
    for cap in cspace.iter() {
        match &cap.resource {
            // Command execution
            ResourceRef::Exec { .. } if cap.rights.contains(Rights::EXECUTE) => {
                registry.register(ExecTool::from_kernel(kernel));
            }

            // Headless browser — SDK browse tools (pure-Rust oxibrowser-core).
            ResourceRef::Browser if cap.rights.contains(Rights::EXECUTE) => {
                register_browser_tools(kernel, registry);
            }

            // Kernel domain tools
            ResourceRef::KernelDomain { domain } => match domain.as_str() {
                "memory" => { /* Registered unconditionally in register_all_kernel_tools */ }
                "agent" => registry.register(KernelAgentTool::from_kernel(kernel)),
                "a2a" => {
                    registry.register(A2aDelegateTool::from_kernel(kernel, agent_id));
                    registry.register(A2aSendTool::from_kernel(kernel, agent_id));
                    registry.register(A2aQueryTool::from_kernel(kernel));
                }
                "persona" => registry.register(PersonaTool::from_kernel(kernel)),
                "program" => { /* Skills are surfaced through CSpace + semantic retrieval, not individual tools */
                }
                "cron" => registry.register(CronTool::from_kernel(kernel)),
                "security" => registry.register(SecurityTool::from_kernel(kernel)),
                "budget" => registry.register(BudgetTool::from_kernel(kernel)),
                "resource" => registry.register(ResourceTool::from_kernel(kernel)),
                "knowledge" => registry.register(KnowledgeTool::from_kernel(kernel)),
                "mcp" => { /* MCP tools are enumerated dynamically per agent */ }
                _ => {} // Unknown domain — silently skip
            },

            // Programs are not registered as separate tools.
            // ToolRetriever shows them in the capability index;
            // agents use exec to run program commands.
            ResourceRef::Skill { .. } => {}

            // Space, Agent, Mcp resource refs are handled through
            // their respective KernelDomain registrations above
            // or through dedicated tool paths.
            _ => {}
        }
    }
}

/// Register tools into `registry` with access gate + approval gate enforcement.
///
/// Same as [`register_tools_from_cspace`] but:
/// - Always-on tools are wrapped in [`GatedTool`] for permission + approval checks
/// - ExecTool is created with `AgentContext` and wrapped in [`GatedTool`] so
///   RFC-035 Step 2.5 also covers shell + structured exec calls (replacing
///   the bespoke exec-only shell approval block)
///
/// Use this in production. The ungated version exists for backward compatibility.
///
/// # Arguments
///
/// * `registry` — The agent's tool registry to populate.
/// * `kernel` — Handle to the kernel for constructing tool instances.
/// * `cspace` — The agent's capability space (determines which tools are available).
/// * `search_cache` — Shared search cache for web search tools.
/// * `agent_id` — The agent's ID (used by A2A tools for routing).
/// * `gate` — The unified access gate for permission checks.
/// * `context` — The agent's security context.
/// * `approval_gate` — RFC-035 approval gate; consults declared policy,
///   config overrides, and global resolvers per tool call.
/// * `event_bus` — Publishes `KernelEvent::ApprovalRequested` when
///   `RequireApproval` is returned.
/// * `pending_approvals` — Shared registry of pending user decisions.
#[allow(clippy::too_many_arguments)]
pub fn register_tools_from_cspace_gated(
    registry: &ToolRegistry,
    kernel: &KernelHandle,
    cspace: &CSpace,
    search_cache: Arc<SearchCache>,
    agent_id: AgentId,
    gate: Arc<AccessGate>,
    context: AgentContext,
    approval_gate: Option<Arc<crate::approval::ApprovalGate>>,
    event_bus: Option<crate::event_bus::EventBus>,
    pending_approvals: Option<Arc<crate::tools::PendingToolApprovals>>,
    pending_path_access: Option<Arc<crate::tools::PendingPathAccess>>,
) {
    // ── Tier 1: Always-on tools (gated) ──────────────────────────────
    register_always_on_gated(
        registry,
        search_cache,
        gate.clone(),
        context.clone(),
        approval_gate.clone(),
        event_bus.clone(),
        pending_approvals.clone(),
        pending_path_access.clone(),
    );

    // ── Tier 2: CSpace-driven tools ─────────────────────────────────
    for cap in cspace.iter() {
        match &cap.resource {
            // Command execution — wrap in GatedTool so Step 2.5 (RFC-035)
            // also fires for exec. The inner ExecTool retains its own
            // context, binary allowlist + access manager checks; the outer
            // GatedTool runs the unified access gate + approval pipeline.
            ResourceRef::Exec { .. } if cap.rights.contains(Rights::EXECUTE) => {
                registry.register(GatedTool::with_approval(
                    ExecTool::from_kernel_with_context(kernel, context.clone()),
                    gate.clone(),
                    context.clone(),
                    approval_gate.clone(),
                    event_bus.clone(),
                    pending_approvals.clone(),
                    pending_path_access.clone(),
                ));
            }

            // Headless browser — SDK browse tools.
            ResourceRef::Browser if cap.rights.contains(Rights::EXECUTE) => {
                register_browser_tools(kernel, registry);
            }

            // Kernel domain tools (same as ungated — these already use KernelHandle internally)
            ResourceRef::KernelDomain { domain } => match domain.as_str() {
                "memory" => { /* Registered unconditionally in register_all_kernel_tools */ }
                "space" => registry.register(ProjectTool::from_kernel(kernel)),
                "agent" => registry.register(KernelAgentTool::from_kernel(kernel)),
                "a2a" => {
                    registry.register(A2aDelegateTool::from_kernel(kernel, agent_id));
                    registry.register(A2aSendTool::from_kernel(kernel, agent_id));
                    registry.register(A2aQueryTool::from_kernel(kernel));
                }
                "persona" => registry.register(PersonaTool::from_kernel(kernel)),
                "program" => {}
                "cron" => registry.register(CronTool::from_kernel(kernel)),
                "security" => registry.register(SecurityTool::from_kernel(kernel)),
                "budget" => registry.register(BudgetTool::from_kernel(kernel)),
                "resource" => registry.register(ResourceTool::from_kernel(kernel)),
                "knowledge" => registry.register(KnowledgeTool::from_kernel(kernel)),
                "mcp" => {}
                _ => {}
            },

            ResourceRef::Skill { .. } => {}
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_always_on_registers_eight_tools() {
        let registry = ToolRegistry::new();
        let cache = Arc::new(SearchCache::new());
        register_always_on(&registry, cache);

        // The always-on set is: read, write, edit, grep, find, ls, web_search, get_search_results
        // ToolRegistry doesn't expose a count, but we can verify individual tool names.
        let tool_names = registry.names();
        assert!(
            tool_names.contains(&"read".to_string()),
            "read tool should be registered"
        );
        assert!(
            tool_names.contains(&"write".to_string()),
            "write tool should be registered"
        );
        assert!(
            tool_names.contains(&"edit".to_string()),
            "edit tool should be registered"
        );
        assert!(
            tool_names.contains(&"grep".to_string()),
            "grep tool should be registered"
        );
        assert!(
            tool_names.contains(&"find".to_string()),
            "find tool should be registered"
        );
        assert!(
            tool_names.contains(&"ls".to_string()),
            "ls tool should be registered"
        );
        assert!(
            tool_names.contains(&"web_search".to_string()),
            "web_search tool should be registered"
        );
        assert!(
            tool_names.contains(&"get_search_results".to_string()),
            "get_search_results tool should be registered"
        );
    }
}
