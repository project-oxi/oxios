//! Kernel tools — AgentTool wrappers for KernelHandle API domains.
//!
//! These tools expose kernel system calls to the agent's tool-calling loop.
//! Each tool wraps a specific domain API and uses an action-based parameter
//! schema to dispatch operations.
//!
//! ## Tools
//!
//! - [`ProjectTool`] — Project management (list, get, link_memory, unlink_memory)
//! - [`AgentTool`] — Agent lifecycle (list, kill, budget)
//! - [`PersonaTool`] — Persona management (list, set_active, get)
//! - [`CronTool`] — Cron scheduling (list, add, remove, trigger)
//! - [`SecurityTool`] — Security audit (verify_chain, query_audit, audit_count)
//! - [`BudgetTool`] — Budget management (check, set, reserve, reset)
//! - [`ResourceTool`] — Resource monitoring (snapshot, history, overloaded)
//! - [`CalendarTool`] — Calendar events (create, update, delete, list, search, freebusy)

pub mod agent_tool;
pub mod budget_tool;
pub mod calendar_tool;
pub mod cron_tool;
pub mod email_tool;
pub mod image_generation_tool;
pub mod knowledge_tool;
pub mod marketplace_tool;
// First-party app module tools (opt-in, feature-gated).
#[cfg(feature = "memo")]
pub mod memo_tool;
pub mod mount_tool;
pub mod persona_tool;
pub mod project_tool;
pub mod resource_tool;
#[cfg(feature = "browser")]
pub mod screenshot_tool;
pub mod security_tool;
pub mod skill_forge_tool;
#[cfg(feature = "timeline")]
pub mod timeline_tool;

pub use agent_tool::AgentTool as KernelAgentTool;
pub use budget_tool::BudgetTool;
pub use calendar_tool::CalendarTool;
pub use cron_tool::CronTool;
pub use email_tool::EmailTool;
pub use image_generation_tool::ImageGenerationTool;
pub use knowledge_tool::KnowledgeTool;
pub use marketplace_tool::MarketplaceTool;
#[cfg(feature = "memo")]
pub use memo_tool::MemoTool;
pub use mount_tool::MountTool;
pub use persona_tool::PersonaTool;
pub use project_tool::ProjectTool;
pub use resource_tool::ResourceTool;
#[cfg(feature = "browser")]
pub use screenshot_tool::ScreenshotTool;
pub use security_tool::SecurityTool;
pub use skill_forge_tool::SkillForgeTool;
#[cfg(feature = "timeline")]
pub use timeline_tool::TimelineTool;

use crate::KernelHandle;
use crate::tools::{AskUserTool, MemoryReadTool, MemorySearchTool, MemoryWriteTool};
use crate::types::AgentId;
use oxicode_sdk::ToolRegistry;

/// Register all kernel domain tools into the registry.
///
/// Called by [`super::kernel_bridge::OxiosKernelBridge`] during agent build.
/// This is the canonical list of kernel tools available in oxios agents.
pub fn register_all_kernel_tools(registry: &ToolRegistry, kernel: &KernelHandle, _agent_id: &str) {
    let agent_uuid = AgentId::new_v4();

    // ExecTool (stores Arc<KernelHandle>)
    registry.register(crate::tools::ExecTool::from_kernel(kernel));

    // Memory tools — brain-backed (BrainConnection), registered unconditionally.
    // Oxios is a personal agent OS: every agent gets memory read + write.
    // The CSpace-gated path in registration.rs is removed (redundant).
    // See docs/designs/2026-07-11-memory-system-overhaul-design.md Phase 1.
    registry.register(MemoryWriteTool::from_kernel(kernel));
    registry.register(MemoryReadTool::from_kernel(kernel));
    registry.register(MemorySearchTool::from_kernel(kernel));

    // Subagent tool — oxicode-agent's native `subagent` tool (RFC-035 gap 3).
    // Wired via AgentConfig.subagent_runner (set in agent_runtime.rs).
    // When the runner is Some, executes in-process; when None, falls back
    // to the CLI spawn path (which is dormant in oxios — no `oxi` binary).
    registry.register(oxicode_agent::SubagentTool::new());

    // Kernel domain tools (take &KernelHandle)
    registry.register(ProjectTool::from_kernel(kernel));
    registry.register(MountTool::from_kernel(kernel));
    registry.register(KernelAgentTool::from_kernel(kernel));
    registry.register(PersonaTool::from_kernel(kernel));
    registry.register(CronTool::from_kernel(kernel));
    registry.register(SecurityTool::from_kernel(kernel));
    registry.register(BudgetTool::from_kernel(kernel));
    registry.register(ResourceTool::from_kernel(kernel));

    // A2A tools (each stores Arc<KernelHandle>)
    registry.register(crate::tools::A2aDelegateTool::from_kernel(
        kernel, agent_uuid,
    ));
    registry.register(crate::tools::A2aSendTool::from_kernel(kernel, agent_uuid));
    registry.register(crate::tools::A2aQueryTool::from_kernel(kernel));

    // MCP tool wrapper (stores Arc<KernelHandle>)
    registry.register(crate::tools::McpToolWrapper::from_kernel(
        kernel,
        "",
        "",
        "MCP tools via bridge".into(),
        serde_json::json!({"type": "object", "properties": {}}),
    ));

    // KnowledgeTool (markdown note management)
    registry.register(KnowledgeTool::from_kernel(kernel));

    // ask_user (RFC-027): agent-driven clarification via event bus + oneshot
    registry.register(AskUserTool::new(
        kernel.infra.pending_ask_user(),
        kernel.infra.event_bus_clone(),
    ));

    // Marketplace (ClawHub — search, install, update)
    registry.register(MarketplaceTool::from_kernel(kernel));

    // Skill Forge (authoring: create/validate/package/import/list/get/delete)
    registry.register(SkillForgeTool::from_kernel(kernel));

    // Calendar (optional — only if [calendar] is enabled)
    if let Some(calendar_tool) = CalendarTool::try_from_kernel(kernel) {
        registry.register(calendar_tool);
    }

    // oximemo (optional first-party app module — `memo` feature + [memo].enabled)
    #[cfg(feature = "memo")]
    if let Some(memo_tool) = MemoTool::try_from_kernel(kernel) {
        registry.register(memo_tool);
    }

    // oxiline (optional first-party app module — `timeline` feature + [timeline].enabled)
    #[cfg(feature = "timeline")]
    if let Some(timeline_tool) = TimelineTool::try_from_kernel(kernel) {
        registry.register(timeline_tool);
    }

    // Email — always registered; returns a helpful setup error when unconfigured.
    // The shared RwLock<Option<EmailApi>> slot is swapped in at runtime via the
    // web UI setup endpoint, so no daemon restart is needed to activate email.
    registry.register(EmailTool::from_kernel(kernel));
    // Image generation (opt-in — only when [image-gen].enabled = true).
    if kernel.infra.config().image_gen.enabled {
        registry.register(ImageGenerationTool::from_kernel(kernel));
    }

    // Screenshot capture (CSS-aware, Blitz-backed — `browser` feature).
    #[cfg(feature = "browser")]
    registry.register(ScreenshotTool::from_kernel(kernel));
}
