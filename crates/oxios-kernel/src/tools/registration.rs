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
    ToolRegistry, WriteTool,
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
/// and web search tools. The web-search tools are the kernel wrappers
/// ([`KernelWebSearchTool`] / [`KernelGetSearchResultsTool`]) which stash
/// structured results into `bus` for the RFC-015 transparency stream and
/// prefer a managed provider per the `[search]` config snapshot.
pub fn register_always_on(
    registry: &ToolRegistry,
    search_cache: Arc<SearchCache>,
    bus: Arc<crate::tools::SearchResultBus>,
    search: crate::config::SearchConfig,
) {
    registry.register(ReadTool::new());
    registry.register(WriteTool::new());
    registry.register(EditTool::new());
    registry.register(GrepTool::new());
    registry.register(FindTool::new());
    registry.register(LsTool::new());
    registry.register(crate::tools::KernelWebSearchTool::new(
        search_cache.clone(),
        bus.clone(),
        search,
    ));
    registry.register(crate::tools::KernelGetSearchResultsTool::new(
        GetSearchResultsTool::new(search_cache),
        bus,
    ));
}

/// Register the headless-browser browse tools when the engine is available.
///
/// Uses oxios-owned tools (RFC-046) backed by `oxibrowser-core` 0.21 directly.
/// Only compiled with the `browser` feature; without it this is a no-op stub.
#[cfg(feature = "browser")]
fn register_browser_tools(kernel: &KernelHandle, registry: &ToolRegistry) {
    use crate::tools::browse::{
        BrowseExtractTool, BrowseScriptTool, BrowseSessionTool, BrowseTool,
    };
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
/// all operations pass through the access gate and approval gate.
#[allow(clippy::too_many_arguments)]
pub fn register_always_on_gated(
    registry: &ToolRegistry,
    search_cache: Arc<SearchCache>,
    bus: Arc<crate::tools::SearchResultBus>,
    search: crate::config::SearchConfig,
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
        crate::tools::KernelWebSearchTool::new(
            search_cache.clone(),
            bus.clone(),
            search,
        ),
        gate.clone(),
        context.clone(),
        approval_gate.clone(),
        event_bus.clone(),
        pending_approvals.clone(),
        pending_path_access.clone(),
    ));
    registry.register(GatedTool::with_approval(
        crate::tools::KernelGetSearchResultsTool::new(
            GetSearchResultsTool::new(search_cache),
            bus,
        ),
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
    register_always_on(
        registry,
        search_cache,
        Arc::clone(&kernel.search_results),
        kernel.engine.search_config(),
    );

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
                "memory" => {
                    // Memory tools removed with the oxibrain 0.8 daemonless
                    // cutover — agents use the `brain` skill + `oxibrain`
                    // CLI (exec structured mode). Domain stays recognized so
                    // persona grants keep their meaning.
                }
                "agent" => registry.register(KernelAgentTool::from_kernel(kernel)),
                "a2a" => {
                    registry.register(A2aDelegateTool::from_kernel(kernel, agent_id));
                    registry.register(A2aSendTool::from_kernel(kernel, agent_id));
                    registry.register(A2aQueryTool::from_kernel(kernel));
                }
                "persona" => registry.register(PersonaTool::from_kernel(kernel)),
                "program" => { /* Skills are surfaced through CSpace + semantic retrieval, not individual tools */
                }

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
/// - When `include_always_on_tools` is `false`, Tier 1 (file ops + web search)
///   is skipped entirely — used for control-only personas (`ToolProfile::Control`)
///   whose settings-copilot scope must never include the file system or web
///
/// Use this in production. The ungated version exists for backward compatibility.
///
/// # Arguments
///
/// * `registry` — The agent's tool registry to populate.
/// * `kernel` — Handle to the kernel for constructing tool instances.
/// * `cspace` — The agent's capability space (determines which tools are available).
/// * `search_cache` — Shared search cache for web search tools.
/// * `agent_id` — The agent's ID (used by A2A + Task tools for routing).
/// * `gate` — The unified access gate for permission checks.
/// * `context` — The agent's security context.
/// * `approval_gate` — RFC-035 approval gate; consults declared policy,
///   config overrides, and global resolvers per tool call.
/// * `event_bus` — Publishes `KernelEvent::ApprovalRequested` when
///   `RequireApproval` is returned.
/// * `pending_approvals` — Shared registry of pending user decisions.
/// * `include_always_on_tools` — When `false`, skip Tier 1 entirely
///   (used for `ToolProfile::Control`).
///
/// # CSpace → Tool mapping
///
/// | ResourceRef | Required rights | Registered tools |
/// |-------------|----------------|-----------------|
/// | `Exec { .. }` | `EXECUTE` | `ExecTool` (gated) |
/// | `Browser` | `EXECUTE` | SDK browse tools |
/// | `KernelDomain { "memory" }` | — | *(registered unconditionally in `register_all_kernel_tools`)* |
/// | `KernelDomain { "space" }` | any | `ProjectTool` + `MountTool` |
/// | `KernelDomain { "agent" }` | any | `KernelAgentTool` |
/// | `KernelDomain { "a2a" }` | any | `A2aDelegateTool`, `A2aSendTool`, `A2aQueryTool` |
/// | `KernelDomain { "persona" }` | any | `PersonaTool` |
/// | `KernelDomain { "program" }` | any | *(deprecated — skills via CSpace)* |
/// | `KernelDomain { "security" }` | any | `SecurityTool` |
/// | `KernelDomain { "budget" }` | any | `BudgetTool` |
/// | `KernelDomain { "resource" }` | any | `ResourceTool` |
/// | `KernelDomain { "knowledge" }` | any | `KnowledgeTool` |
/// | `KernelDomain { "task" }` | any | `TaskTool` (when task store attached) |
/// | `KernelDomain { "marketplace" }` | any | `MarketplaceTool` |
/// | `KernelDomain { "mcp_manage" }` | any | `McpManageTool` (approval-gated) |
/// | `KernelDomain { "engine" }` | any | `EngineTool` (approval-gated) |
/// | `Program { .. }` | — | *(not registered; surfaced via ToolRetriever)* |
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
    include_always_on_tools: bool,
) {
    // ── Tier 1: Always-on tools (gated) ──────────────────────────────
    // Skipped for control-only personas (ToolProfile::Control): the
    // always-on tier is file ops + web search, which a settings copilot
    // must not receive.
    if include_always_on_tools {
        register_always_on_gated(
            registry,
            search_cache,
            Arc::clone(&kernel.search_results),
            kernel.engine.search_config(),
            gate.clone(),
            context.clone(),
            approval_gate.clone(),
            event_bus.clone(),
            pending_approvals.clone(),
            pending_path_access.clone(),
        );
    }

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
                "memory" => {
                    // Memory tools removed with the oxibrain 0.8 daemonless
                    // cutover — agents use the `brain` skill + `oxibrain`
                    // CLI (exec structured mode). Domain stays recognized so
                    // persona grants keep their meaning.
                }
                "space" => {
                    registry.register(ProjectTool::from_kernel(kernel));
                    registry.register(MountTool::from_kernel(kernel));
                }
                "agent" => registry.register(KernelAgentTool::from_kernel(kernel)),
                "a2a" => {
                    registry.register(A2aDelegateTool::from_kernel(kernel, agent_id));
                    registry.register(A2aSendTool::from_kernel(kernel, agent_id));
                    registry.register(A2aQueryTool::from_kernel(kernel));
                }
                "persona" => registry.register(PersonaTool::from_kernel(kernel)),
                "program" => {}

                "security" => registry.register(SecurityTool::from_kernel(kernel)),
                "budget" => registry.register(BudgetTool::from_kernel(kernel)),
                "resource" => registry.register(ResourceTool::from_kernel(kernel)),
                "knowledge" => registry.register(KnowledgeTool::from_kernel(kernel)),
                "task" => {
                    if let Some(store) = kernel.task_store.clone() {
                        registry.register(TaskTool::for_store(store, Some(agent_id.to_string())));
                    }
                }
                "marketplace" => registry.register(MarketplaceTool::from_kernel(kernel)),
                "mcp_manage" => registry.register(GatedTool::with_approval(
                    McpManageTool::from_kernel(kernel),
                    gate.clone(),
                    context.clone(),
                    approval_gate.clone(),
                    event_bus.clone(),
                    pending_approvals.clone(),
                    pending_path_access.clone(),
                )),
                "engine" => registry.register(GatedTool::with_approval(
                    EngineTool::from_kernel(kernel),
                    gate.clone(),
                    context.clone(),
                    approval_gate.clone(),
                    event_bus.clone(),
                    pending_approvals.clone(),
                    pending_path_access.clone(),
                )),
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
        register_always_on(
            &registry,
            cache,
            Arc::new(crate::tools::SearchResultBus::new()),
            crate::config::SearchConfig::default(),
        );

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
