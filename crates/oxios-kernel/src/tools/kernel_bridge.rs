//! KernelToolProvider bridge — plugs oxios kernel tools into oxicode-sdk agent builder.
//!
//! Implements [`oxicode_sdk::KernelToolProvider`] so that oxios kernel tools
//! (exec, memory, project, etc.) can be registered into the SDK's

use std::sync::Arc;

use oxicode_sdk::SearchCache;
use oxicode_sdk::ToolRegistry;
use oxicode_sdk::{
    KernelToolContext as SdkKernelToolContext, KernelToolProvider as SdkKernelToolProvider,
};

use crate::KernelHandle;
use crate::tools::registration::register_always_on;

/// Bridges all oxios kernel tools into the oxicode-sdk agent builder.
pub struct OxiosKernelBridge {
    kernel_handle: Arc<KernelHandle>,
    search_cache: Arc<SearchCache>,
}

impl OxiosKernelBridge {
    /// Create a new bridge with the given kernel handle.
    pub fn new(kernel_handle: Arc<KernelHandle>) -> Self {
        Self {
            kernel_handle,
            search_cache: Arc::new(SearchCache::new()),
        }
    }

    /// Create a new bridge with a pre-built search cache.
    pub fn with_cache(kernel_handle: Arc<KernelHandle>, search_cache: Arc<SearchCache>) -> Self {
        Self {
            kernel_handle,
            search_cache,
        }
    }
}

impl SdkKernelToolProvider for OxiosKernelBridge {
    fn tool_names(&self) -> Vec<&str> {
        #[allow(unused_mut)]
        let mut names = vec![
            // Always-on file + web-search tools (registration::register_always_on)
            "read",
            "write",
            "edit",
            "grep",
            "find",
            "ls",
            "web_search",
            "get_search_results",
            // Kernel domain tools (builtin::register_all_kernel_tools)
            "exec",
            "memory_read",
            "memory_write",
            "memory_search",
            "subagent",
            "project",
            "mount",
            "kernel_agent",
            "persona",
            "cron",
            "security",
            "budget",
            "resource",
            "a2a_delegate",
            "a2a_send",
            "a2a_query",
            "knowledge",
            "ask_user",
            "marketplace",
            "skill_forge",
            "calendar", // conditional on [calendar] config
            "send_email",
            // NOTE: MCP tools use a dynamic `full_name` and are enumerated
            // per-server at registration time, so they are not listed here.
        ];

        // Headless browser — oxios-owned browse suite (RFC-046).
        // browse + browse_screenshot share the unified `browser` feature.
        #[cfg(feature = "browser")]
        names.extend([
            "browse",
            "browse_extract",
            "browse_session",
            "browse_script",
            "browse_screenshot",
        ]);

        names
    }

    fn register_tools(&self, registry: &ToolRegistry, context: &SdkKernelToolContext) {
        // 1. Always-on file tools + web search
        register_always_on(registry, Arc::clone(&self.search_cache));

        // 2. Kernel domain tools via KernelHandle
        crate::tools::builtin::register_all_kernel_tools(
            registry,
            &self.kernel_handle,
            &context.agent_id,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that `tool_names()` returns the expected number of tool names.
    #[tokio::test]
    async fn test_tool_names_length() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().to_path_buf();

        // Build a minimal KernelHandle for testing
        let state_store =
            Arc::new(crate::state_store::StateStore::new(base.join("workspace")).unwrap());

        let kernel = Arc::new(crate::KernelHandle::new(
            crate::StateApi::new(state_store.clone()),
            crate::AgentApi::new(
                Arc::new(crate::supervisor::NoOpSupervisor),
                Arc::new(crate::budget::BudgetManager::new()),
            ),
            crate::SecurityApi::new(
                Arc::new(parking_lot::Mutex::new(crate::auth::AuthManager::new())),
                Arc::new(oxicode_sdk::observability::AuditTrail::new(100)),
                Arc::new(parking_lot::Mutex::new(
                    crate::access_manager::AccessManager::new(),
                )),
                state_store.clone(),
            ),
            crate::PersonaApi::new(Arc::new(crate::persona::PersonaManager::new())),
            crate::ExtensionApi::new(Arc::new(crate::skill::SkillManager::new(
                base.join("skills"),
                base.join("share/skills"),
            ))),
            crate::McpApi::new(Arc::new(crate::mcp::McpBridge::new())),
            crate::InfraApi::new(
                Arc::new(crate::git_layer::GitLayer::new(base.join("git"), false).unwrap()),
                Arc::new(crate::cron::CronScheduler::new(state_store.clone(), 60)),
                Arc::new(crate::resource_monitor::ResourceMonitor::new(60, 60)),
                crate::event_bus::EventBus::new(256),
                crate::OxiosConfig::default(),
                std::time::Instant::now(),
                std::sync::Arc::new(crate::tools::PendingToolApprovals::new()),
                std::sync::Arc::new(crate::tools::PendingAskUser::new()),
                std::sync::Arc::new(parking_lot::RwLock::new(
                    crate::approval::ApprovalConfig::default(),
                )),
                std::sync::Arc::new(crate::tools::PendingPathAccess::new()),
            ),
            None,
            crate::ExecApi::new(
                Arc::new(parking_lot::RwLock::new(
                    crate::config::ExecConfig::default(),
                )),
                Arc::new(parking_lot::Mutex::new(
                    crate::access_manager::AccessManager::new(),
                )),
            ),
            crate::A2aApi::new(Arc::new(crate::a2a::A2AProtocol::new(
                crate::event_bus::EventBus::new(256),
            ))),
            crate::EngineApi::new(
                Arc::new(parking_lot::RwLock::new(crate::OxiosConfig::default())),
                base.join("config.toml"),
                Arc::new(crate::RoutingStats::new()),
                Arc::new(crate::engine::EngineHandle::new(Arc::new(
                    crate::OxiosEngine::new("anthropic/claude-sonnet-4-20250514"),
                ))),
            ),
            Arc::new(oxios_markdown::KnowledgeBase::new(base.join("knowledge")).unwrap()),
            Arc::new(
                crate::kernel_handle::KnowledgeLens::new(
                    Arc::new(oxios_markdown::KnowledgeBase::new(base.join("knowledge_lens")).unwrap()),
                    None,
                )
                .unwrap(),
            ),
            crate::MarketplaceApi::new(
                Arc::new(crate::skill::clawhub::ClawHubInstaller::new(
                    base.join("skills"),
                    base.join("workspace"),
                    None,
                )),
                Arc::new(
                    crate::skill::clawhub::ClawHubClient::new(None).expect("valid ClawHub client"),
                ),
                Arc::new(crate::skill::skills_sh::SkillsShInstaller::new(
                    base.join("skills"),
                    None,
                    None,
                )),
                Arc::new(
                    crate::skill::skills_sh::SkillsShClient::new(None, None)
                        .expect("valid Skills.sh client"),
                ),
            ),
            None,                                     // calendar (not configured in test)
            Arc::new(parking_lot::RwLock::new(None)), // email (not configured in test)
        ));

        let bridge = OxiosKernelBridge::new(kernel);

        let names = bridge.tool_names();
        // 8 always-on (file ops + web_search/get_search_results)
        // + 22 kernel domain (exec, memory×3, subagent, project, mount,
        //   kernel_agent, persona, cron, security, budget, resource, a2a×3,
        //   knowledge, ask_user, marketplace, skill_forge, calendar, send_email)
        // = 30. MCP tools are dynamic (per-server) and excluded.
        // +5 tools when browser feature is enabled (browse, browse_extract,
        // browse_session, browse_script, browse_screenshot).
        #[allow(unused_mut)]
        let mut expected = 30_usize;
        #[cfg(feature = "browser")]
        {
            expected += 5;
        }
        assert_eq!(
            names.len(),
            expected,
            "expected {expected} tools, got {:?}",
            names
        );
    }
}
