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
//! - [`TaskTool`] — Task management (create, list, view, edit, schedule, verify, run)

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
pub mod task_tool;
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
pub use task_tool::TaskTool;
#[cfg(feature = "timeline")]
pub use timeline_tool::TimelineTool;
