//! API routes for the web channel.
//!
//! Route groups are split into sub-modules:
//! - **chat**: Chat and WebSocket streaming
//! - **system**: Health, status, agents, config
//! - **workspace**: File tree, skills, memory
//! - **resource_routes**: System resources
//! - **infra**: Scheduler, audit, permissions, MCP
//! - **events**: Sessions, SSE events, approvals

mod a2a;
mod asset_routes;
mod audit_routes;
mod budget_routes;
mod calendar_routes;
mod chat;
mod cost_routes;
mod cron_jobs;
mod email_routes;
mod engine_routes;
mod events;
mod git_routes;
mod host_tools_routes;
mod image_routes;
mod infra;
#[cfg(feature = "memo")]
mod memo_routes;
#[cfg(feature = "timeline")]
mod timeline_routes;
pub(crate) use host_tools_routes::{handle_host_tools, handle_host_tools_detect};
mod integrations_routes;
pub(crate) use integrations_routes::{
    handle_integration_credential_delete, handle_integration_credential_set,
    handle_integration_credential_status, handle_integration_install,
    handle_integration_oauth_poll, handle_integration_oauth_start, handle_integrations_list,
};
mod knowledge_routes;
mod marketplace;
mod search;
use search::{handle_browse, handle_search};
#[cfg(feature = "screenshot")]
use search::handle_screenshot;
mod mount_routes;
mod project_routes;
mod resource_routes;
mod secrets_routes;
mod security_routes;
mod system;
mod task_routes;
mod token_maxing_routes;
mod tools;
mod workspace;
mod worktree_routes;

use std::sync::Arc;

use axum::{
    Router,
    routing::{delete, get, patch, post, put},
};
use serde::Deserialize;

use crate::api::middleware::{rate_limit_layer, require_auth, require_ready};
use crate::api::persona_routes;
use crate::api::server::AppState;

// Re-export all handlers for use in build_routes
pub(crate) use a2a::{
    handle_a2a_agent_detail, handle_a2a_agents, handle_a2a_messages, handle_a2a_topology,
};
pub(crate) use asset_routes::{
    handle_asset_delete, handle_asset_get, handle_asset_list, handle_asset_meta_get,
    handle_asset_meta_update, handle_asset_upload,
};
pub(crate) use audit_routes::{
    handle_audit_by_agent, handle_audit_entries, handle_audit_export, handle_audit_flush,
    handle_audit_verify,
};
pub(crate) use budget_routes::{
    handle_budget_get, handle_budget_list, handle_budget_remove, handle_budget_reserve,
    handle_budget_reset, handle_budget_set,
};
pub(crate) use calendar_routes::{
    handle_calendar_by_note, handle_calendar_event_create, handle_calendar_event_delete,
    handle_calendar_event_get, handle_calendar_event_update, handle_calendar_events,
    handle_calendar_freebusy, handle_calendar_search,
};
pub(crate) use chat::{
    handle_ask_user_respond, handle_chat, handle_chat_seed, handle_chat_stream, handle_chat_ticket,
    handle_knowledge_saves, handle_path_access_respond, handle_remove_knowledge_save,
    handle_save_to_knowledge, handle_tool_approval_respond,
};
pub(crate) use cost_routes::{
    handle_cost_by_model, handle_cost_by_project, handle_cost_daily, handle_cost_providers,
    handle_cost_spend_limit_get, handle_cost_spend_limit_set, handle_cost_summary,
};
pub(crate) use cron_jobs::{
    handle_cron_job_create, handle_cron_job_delete, handle_cron_job_get, handle_cron_job_trigger,
    handle_cron_jobs_list, update_cron_job,
};
pub(crate) use email_routes::{
    handle_email_history, handle_email_history_detail, handle_email_setup, handle_email_status,
    handle_email_template_get, handle_email_templates, handle_email_test,
};
pub(crate) use engine_routes::{
    handle_add_custom_provider, handle_check_provider_connection, handle_engine_config,
    handle_engine_delete_api_key, handle_engine_follow_up, handle_engine_models,
    handle_engine_providers, handle_engine_roles, handle_engine_routing_fallbacks,
    handle_engine_routing_stats, handle_engine_set_api_key, handle_engine_set_model,
    handle_engine_set_provider_options, handle_engine_set_quick_ask_model, handle_engine_set_roles,
    handle_engine_set_routing, handle_engine_validate_key, handle_get_provider_config,
    handle_remove_custom_provider, handle_set_model_list, handle_set_provider_config,
};
pub(crate) use events::{
    handle_approval_approve, handle_approval_reject, handle_approvals_list, handle_events,
    handle_session_compress, handle_session_create_thread, handle_session_delete,
    handle_session_get, handle_session_move, handle_session_threads, handle_sessions_list,
    handle_sessions_prune,
};
pub(crate) use git_routes::{
    handle_git_log, handle_git_restore, handle_git_tag_delete, handle_git_tags, handle_git_verify,
};
pub(crate) use infra::{
    handle_audit_log, handle_mcp_server_delete, handle_mcp_server_refresh,
    handle_mcp_server_register, handle_mcp_server_toggle, handle_mcp_server_update,
    handle_mcp_servers_list, handle_mcp_tool_call, handle_mcp_tools_list, handle_metrics,
    handle_permissions_get, handle_permissions_put, handle_security_permissions,
};
pub(crate) use knowledge_routes::{
    handle_knowledge_asset_get, handle_knowledge_backlinks, handle_knowledge_chat_append,
    handle_knowledge_chat_delete, handle_knowledge_chat_messages, handle_knowledge_chat_move,
    handle_knowledge_checklist_add, handle_knowledge_checklist_complete,
    handle_knowledge_checklist_items, handle_knowledge_checklist_remove,
    handle_knowledge_config_get, handle_knowledge_config_put, handle_knowledge_convert_html,
    handle_knowledge_copilot, handle_knowledge_emoji, handle_knowledge_file_diff,
    handle_knowledge_file_or_sub, handle_knowledge_graph, handle_knowledge_habits,
    handle_knowledge_habits_last_week, handle_knowledge_journal_add,
    handle_knowledge_journal_emoji, handle_knowledge_journal_today, handle_knowledge_move,
    handle_knowledge_search, handle_knowledge_stats_done_today, handle_knowledge_stats_today,
    handle_knowledge_tree, handle_knowledge_worker_nightly, handle_knowledge_worker_scheduled,
};
pub(crate) use marketplace::{
    handle_marketplace_install, handle_marketplace_search, handle_marketplace_skill_detail,
    handle_marketplace_updates, handle_skills_sh_install, handle_skills_sh_list,
    handle_skills_sh_search, handle_skills_sh_skill_audit, handle_skills_sh_skill_detail,
};
#[cfg(feature = "memo")]
pub(crate) use memo_routes::{handle_memo_disable, handle_memo_enable, handle_memo_status};
pub(crate) use mount_routes::{
    handle_mount_create, handle_mount_delete, handle_mount_get, handle_mount_rescan,
    handle_mount_update, handle_mounts_list,
};
pub(crate) use project_routes::{
    handle_project_create, handle_project_delete, handle_project_get, handle_project_link_memory,
    handle_project_memories, handle_project_unlink_memory, handle_project_update,
    handle_projects_list,
};
pub(crate) use resource_routes::{
    handle_resource_history, handle_resource_overload, handle_resource_snapshot,
};
pub(crate) use security_routes::{
    handle_approval_config_get, handle_approval_config_patch, handle_approval_grant_add,
    handle_approval_grant_remove,
};
pub(crate) use system::{
    handle_agent_get, handle_agent_kill, handle_agent_logs, handle_agent_stats, handle_agent_trace,
    handle_agents_list, handle_audit_verify_api, handle_backup, handle_config_get,
    handle_config_meta, handle_config_patch, handle_config_put, handle_doctor, handle_health,
    handle_log, handle_readiness, handle_status, handle_update_changelog, handle_update_check,
    handle_update_run,
};
pub(crate) use task_routes::{
    execute_task_run, handle_task_create, handle_task_delete, handle_task_get, handle_task_run,
    handle_task_runs, handle_task_set_schedule, handle_task_set_verify, handle_task_update_status,
    handle_tasks_list,
};
#[cfg(feature = "timeline")]
pub(crate) use timeline_routes::{
    handle_timeline_disable, handle_timeline_enable, handle_timeline_status,
};
pub(crate) use token_maxing_routes::{
    handle_token_maxing_providers, handle_token_maxing_session, handle_token_maxing_sessions,
    handle_token_maxing_start, handle_token_maxing_status, handle_token_maxing_stop,
};
pub(crate) use tools::handle_tools_registry;
pub(crate) use workspace::{
    MemoryMapCache, handle_dream_reports, handle_dream_status, handle_memory_create,
    handle_memory_delete, handle_memory_get, handle_memory_list, handle_memory_map,
    handle_memory_pin, handle_memory_search, handle_memory_semantic_search, handle_memory_stats,
    handle_skill_content, handle_skill_content_update, handle_skill_create, handle_skill_delete,
    handle_skill_disable, handle_skill_enable, handle_skill_get, handle_skill_import_file,
    handle_skill_import_text, handle_skill_import_url, handle_skills_list,
    handle_workspace_file_create, handle_workspace_file_delete, handle_workspace_file_get,
    handle_workspace_file_put, handle_workspace_tree,
};

// ---------------------------------------------------------------------------
// Shared pagination types
// ---------------------------------------------------------------------------

/// Pagination query parameters.
#[derive(Debug, Deserialize, Default)]
pub struct PageParams {
    /// Page number (1-indexed).
    #[serde(default = "default_page")]
    pub page: usize,
    /// Items per page.
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_page() -> usize {
    1
}
fn default_limit() -> usize {
    50
}

/// Apply pagination to a slice of items.
/// Returns a JSON value with `{items, total, page, limit}`.
pub fn paginate<T: Clone + serde::Serialize>(
    items: &[T],
    params: &PageParams,
) -> serde_json::Value {
    let total = items.len();
    let limit = params.limit.min(500);
    let offset = (params.page.saturating_sub(1)) * limit;
    serde_json::json!({
        "items": items.iter().skip(offset).take(limit).cloned().collect::<Vec<_>>(),
        "total": total,
        "page": params.page,
        "limit": limit,
    })
}

/// Builds the axum router with all API routes.
///
/// Auth middleware is applied to all `/api/*` routes.
/// `/health` and static assets are excluded from auth.
pub fn build_routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    // Public routes (no auth)
    let public = Router::new()
        .route("/health", get(handle_health))
        .route("/health/ready", get(handle_readiness))
        .route("/metrics", get(handle_metrics))
        // Marketplace (ClawHub) — read-only routes, public
        .route("/api/marketplace/search", get(handle_marketplace_search))
        .route("/api/marketplace/updates", get(handle_marketplace_updates))
        .route(
            "/api/marketplace/skills/{slug}",
            get(handle_marketplace_skill_detail),
        )
        // Marketplace (Skills.sh) — read-only routes, public
        .route(
            "/api/marketplace/skills-sh/search",
            get(handle_skills_sh_search),
        )
        .route(
            "/api/marketplace/skills-sh/list",
            get(handle_skills_sh_list),
        )
        .route(
            "/api/marketplace/skills-sh/skill/{id}",
            get(handle_skills_sh_skill_detail),
        )
        .route(
            "/api/marketplace/skills-sh/skill/{id}/audit",
            get(handle_skills_sh_skill_audit),
        )
        .route("/api/images/{name}", get(image_routes::handle_image_get))
        // Unified asset store — public binary serving (same rationale as
        // /api/images/{name}: <img> tags cannot send Authorization headers).
        .route("/api/assets/{name}", get(handle_asset_get));
    // Protected API routes (auth middleware applied)
    let api = Router::new()
        // Chat
        .route("/api/chat", post(handle_chat))
        .route("/api/chat/ticket", post(handle_chat_ticket))
        .route("/api/chat/seed", post(handle_chat_seed))
        .route("/api/chat/stream", get(handle_chat_stream))
        // RFC-017: runtime tool capability escalation
        .route(
            "/api/chat/tool-approval/{id}/respond",
            post(handle_tool_approval_respond),
        )
        // Interactive path-access cards (Mount / temp-allow / deny)
        .route(
            "/api/chat/path-access/{id}/respond",
            post(handle_path_access_respond),
        )
        // RFC-027: ask_user agent-driven clarification
        .route(
            "/api/chat/ask-user/{id}/respond",
            post(handle_ask_user_respond),
        )
        // RFC-016: Knowledge persistence API
        .route(
            "/api/chat/{session_id}/knowledge-saves",
            get(handle_knowledge_saves),
        )
        .route(
            "/api/chat/{session_id}/messages/{message_index}/save-to-knowledge",
            post(handle_save_to_knowledge),
        )
        .route(
            "/api/chat/{session_id}/messages/{message_index}/knowledge-save",
            delete(handle_remove_knowledge_save),
        )
        // Control
        .route("/api/status", get(handle_status))
        .route("/api/agents", get(handle_agents_list))
        .route("/api/agents/stats", get(handle_agent_stats))
        .route("/api/agents/{id}", get(handle_agent_get))
        .route("/api/agents/{id}/trace", get(handle_agent_trace))
        .route("/api/agents/{id}/logs", get(handle_agent_logs))
        .route("/api/agents/{id}/kill", post(handle_agent_kill))
        // Config
        .route("/api/config", get(handle_config_get))
        .route("/api/config", put(handle_config_put))
        .route("/api/config", patch(handle_config_patch))
        .route("/api/config/meta", get(handle_config_meta))
        // Engine
        .route("/api/engine/follow-up", post(handle_engine_follow_up))
        .route("/api/engine/providers", get(handle_engine_providers))
        .route("/api/engine/models", get(handle_engine_models))
        .route("/api/engine/config", get(handle_engine_config))
        .route("/api/engine/model", put(handle_engine_set_model))
        .route(
            "/api/engine/quick-ask-model",
            put(handle_engine_set_quick_ask_model),
        )
        .route(
            "/api/engine/api-key",
            put(handle_engine_set_api_key).delete(handle_engine_delete_api_key),
        )
        .route(
            "/api/engine/provider-options",
            put(handle_engine_set_provider_options),
        )
        .route("/api/engine/validate-key", post(handle_engine_validate_key))
        .route("/api/engine/routing", put(handle_engine_set_routing))
        // RFC-032: role routing — role → model mapping
        .route(
            "/api/engine/roles",
            get(handle_engine_roles).put(handle_engine_set_roles),
        )
        .route(
            "/api/engine/routing/stats",
            get(handle_engine_routing_stats),
        )
        .route(
            "/api/engine/routing/fallbacks",
            get(handle_engine_routing_fallbacks),
        )
        .route(
            "/api/engine/providers/{id}/config",
            get(handle_get_provider_config).put(handle_set_provider_config),
        )
        .route(
            "/api/engine/providers/{id}/check",
            post(handle_check_provider_connection),
        )
        .route(
            "/api/engine/providers/{id}/models",
            put(handle_set_model_list),
        )
        .route(
            "/api/engine/custom-providers",
            post(handle_add_custom_provider),
        )
        .route(
            "/api/engine/custom-providers/{id}",
            delete(handle_remove_custom_provider),
        )
        // Secrets management (RFC-028 SP-2b)
        .route("/api/secrets", get(secrets_routes::handle_secrets_list))
        .route(
            "/api/secrets/{key}",
            put(secrets_routes::handle_secret_set).delete(secrets_routes::handle_secret_delete),
        )
        .route(
            "/api/secrets/{key}/source",
            get(secrets_routes::handle_secret_source),
        )
        // Workspace
        .route("/api/workspace/tree", get(handle_workspace_tree))
        .route(
            "/api/workspace/file/{*path}",
            get(handle_workspace_file_get),
        )
        .route(
            "/api/workspace/file/{*path}",
            put(handle_workspace_file_put),
        )
        .route(
            "/api/workspace/file/{*path}",
            post(handle_workspace_file_create),
        )
        .route(
            "/api/workspace/file/{*path}",
            delete(handle_workspace_file_delete),
        )
        // Skills
        .route("/api/skills", get(handle_skills_list))
        .route("/api/skills/{name}", get(handle_skill_get))
        .route("/api/skills", post(handle_skill_create))
        .route("/api/skills/{name}", delete(handle_skill_delete))
        .route("/api/skills/{name}/enable", post(handle_skill_enable))
        .route("/api/skills/{name}/disable", post(handle_skill_disable))
        .route("/api/skills/{name}/content", get(handle_skill_content))
        // PUT content — inline edit, frontmatter preserved
        .route(
            "/api/skills/{name}/content",
            put(handle_skill_content_update),
        )
        // Import — text & URL (JSON bodies)
        .route("/api/skills/import/text", post(handle_skill_import_text))
        .route("/api/skills/import/url", post(handle_skill_import_url))
        // Import — file upload (multipart). Relies on the raised global
        // API_BODY_LIMIT (32 MB); a per-route layer could not exceed it.
        .route("/api/skills/import", post(handle_skill_import_file))
        // Memory
        .route("/api/memory", get(handle_memory_list))
        .route("/api/memory", post(handle_memory_create))
        .route("/api/memory/search", post(handle_memory_search))
        .route("/api/memory/semantic", post(handle_memory_semantic_search))
        .route("/api/memory/map", get(handle_memory_map))
        .route("/api/memory/stats", get(handle_memory_stats))
        .route("/api/memory/dream/status", get(handle_dream_status))
        .route("/api/memory/dream/reports", get(handle_dream_reports))
        .route("/api/memory/{id}/pin", put(handle_memory_pin))
        .route(
            "/api/memory/{name}",
            get(handle_memory_get).delete(handle_memory_delete),
        )
        // Audit log
        .route("/api/audit/entries", get(handle_audit_entries))
        .route("/api/audit/verify", get(handle_audit_verify))
        .route("/api/audit/agent/{agent_id}", get(handle_audit_by_agent))
        .route("/api/audit/export", post(handle_audit_export))
        .route("/api/audit/flush", post(handle_audit_flush))
        // Permissions
        .route("/api/audit", get(handle_audit_log))
        .route(
            "/api/security/permissions",
            get(handle_security_permissions),
        )
        .route("/api/permissions/{agent}", get(handle_permissions_get))
        .route("/api/permissions/{agent}", put(handle_permissions_put))
        // MCP
        .route(
            "/api/mcp/servers",
            get(handle_mcp_servers_list).post(handle_mcp_server_register),
        )
        .route(
            "/api/mcp/servers/{name}",
            delete(handle_mcp_server_delete).put(handle_mcp_server_update),
        )
        .route(
            "/api/mcp/servers/{name}/toggle",
            post(handle_mcp_server_toggle),
        )
        .route(
            "/api/mcp/servers/{name}/refresh",
            post(handle_mcp_server_refresh),
        )
        .route(
            "/api/mcp/tools",
            get(handle_mcp_tools_list).post(handle_mcp_tool_call),
        )
        // Prometheus metrics
        .route("/api/metrics", get(handle_metrics))
        // Resources
        .route("/api/resources", get(handle_resource_snapshot))
        .route("/api/resources/history", get(handle_resource_history))
        .route("/api/resources/overload", get(handle_resource_overload))
        // A2A Monitor
        .route("/api/a2a/agents", get(handle_a2a_agents))
        .route("/api/a2a/agents/{id}", get(handle_a2a_agent_detail))
        .route("/api/a2a/messages", get(handle_a2a_messages))
        .route("/api/a2a/topology", get(handle_a2a_topology))
        // Events
        .route("/api/events", get(handle_events))
        // Personas (delegated to persona_routes)
        .route("/api/personas", get(persona_routes::handle_personas_list))
        .route("/api/personas", post(persona_routes::handle_persona_create))
        .route(
            "/api/personas/{id}",
            get(persona_routes::handle_persona_get),
        )
        .route(
            "/api/personas/{id}",
            put(persona_routes::handle_persona_update),
        )
        .route(
            "/api/personas/{id}",
            delete(persona_routes::handle_persona_delete),
        )
        .route(
            "/api/personas/active",
            get(persona_routes::handle_persona_active_get),
        )
        .route(
            "/api/personas/active",
            put(persona_routes::handle_persona_active_set),
        )
        // Sessions
        .route("/api/sessions", get(handle_sessions_list))
        .route("/api/sessions/prune", post(handle_sessions_prune))
        .route("/api/sessions/{id}", get(handle_session_get))
        .route("/api/sessions/{id}", delete(handle_session_delete))
        .route("/api/sessions/{id}/project", patch(handle_session_move))
        .route("/api/sessions/{id}/compress", post(handle_session_compress))
        // RFC-035: Threads (sub-sessions)
        .route("/api/sessions/{id}/threads", get(handle_session_threads))
        .route(
            "/api/sessions/{id}/threads",
            post(handle_session_create_thread),
        )
        // Cron Jobs
        .route("/api/cron-jobs", get(handle_cron_jobs_list))
        .route("/api/cron-jobs", post(handle_cron_job_create))
        .route("/api/cron-jobs/{id}", get(handle_cron_job_get))
        .route("/api/cron-jobs/{id}", delete(handle_cron_job_delete))
        .route("/api/cron-jobs/{id}/edit", post(update_cron_job))
        .route("/api/cron-jobs/{id}/trigger", post(handle_cron_job_trigger))
        // Tasks (RFC-043)
        .route("/api/tasks", get(handle_tasks_list))
        .route("/api/tasks", post(handle_task_create))
        .route("/api/tasks/{id}", get(handle_task_get))
        .route("/api/tasks/{id}", delete(handle_task_delete))
        .route("/api/tasks/{id}/status", put(handle_task_update_status))
        .route("/api/tasks/{id}/schedule", put(handle_task_set_schedule))
        .route("/api/tasks/{id}/verify", put(handle_task_set_verify))
        .route("/api/tasks/{id}/run", post(handle_task_run))
        .route("/api/tasks/{id}/runs", get(handle_task_runs))
        // Calendar
        .route(
            "/api/calendar/events",
            get(handle_calendar_events).post(handle_calendar_event_create),
        )
        .route(
            "/api/calendar/events/{uid}",
            get(handle_calendar_event_get)
                .put(handle_calendar_event_update)
                .delete(handle_calendar_event_delete),
        )
        .route("/api/calendar/search", get(handle_calendar_search))
        .route("/api/calendar/freebusy", get(handle_calendar_freebusy))
        .route("/api/calendar/by-note", get(handle_calendar_by_note))
        // Email
        .route("/api/email/status", get(handle_email_status))
        .route("/api/email/history", get(handle_email_history))
        .route("/api/email/history/{id}", get(handle_email_history_detail))
        .route("/api/email/templates", get(handle_email_templates))
        .route(
            "/api/email/templates/{name}",
            get(handle_email_template_get),
        )
        .route("/api/email/test", post(handle_email_test))
        .route("/api/email/setup", post(handle_email_setup))
        // Approvals (HitL)
        .route("/api/approvals", get(handle_approvals_list))
        .route("/api/approvals/{id}/approve", post(handle_approval_approve))
        .route(
            "/api/security/approval",
            get(handle_approval_config_get).patch(handle_approval_config_patch),
        )
        .route(
            "/api/security/approval/allow-list",
            post(handle_approval_grant_add),
        )
        .route(
            "/api/security/approval/allow-list/{key}",
            delete(handle_approval_grant_remove),
        )
        .route("/api/approvals/{id}/reject", post(handle_approval_reject))
        // Git
        .route("/api/git/log", get(handle_git_log))
        .route("/api/git/tags", get(handle_git_tags))
        .route("/api/git/verify", post(handle_git_verify))
        .route("/api/git/restore", post(handle_git_restore))
        .route("/api/git/tags/{name}", delete(handle_git_tag_delete))
        // Projects
        .route("/api/projects", get(handle_projects_list))
        .route("/api/projects", post(handle_project_create))
        .route("/api/projects/{id}", get(handle_project_get))
        .route("/api/projects/{id}", put(handle_project_update))
        .route("/api/projects/{id}", delete(handle_project_delete))
        .route("/api/projects/{id}/memories", get(handle_project_memories))
        .route(
            "/api/projects/{id}/memories",
            post(handle_project_link_memory),
        )
        .route(
            "/api/projects/{id}/memories/{memoryId}",
            delete(handle_project_unlink_memory),
        )
        // Mounts (RFC-025)
        .route("/api/mounts", get(handle_mounts_list))
        .route("/api/mounts", post(handle_mount_create))
        .route("/api/mounts/{id}", get(handle_mount_get))
        .route("/api/mounts/{id}", put(handle_mount_update))
        .route("/api/mounts/{id}", delete(handle_mount_delete))
        .route("/api/mounts/{id}/rescan", post(handle_mount_rescan))
        // Tool Registry (for settings UI)
        .route("/api/tools/registry", get(handle_tools_registry))
        // Budget
        .route("/api/budget", get(handle_budget_list))
        .route("/api/budget/{agent_id}", get(handle_budget_get))
        .route("/api/budget/{agent_id}", post(handle_budget_set))
        .route("/api/budget/{agent_id}", delete(handle_budget_remove))
        .route(
            "/api/budget/{agent_id}/reserve",
            post(handle_budget_reserve),
        )
        .route("/api/budget/{agent_id}/reset", post(handle_budget_reset))
        // Costs — dollar-based spend views over agent_log_db
        .route("/api/costs/summary", get(handle_cost_summary))
        .route("/api/costs/by-model", get(handle_cost_by_model))
        .route("/api/costs/by-project", get(handle_cost_by_project))
        .route("/api/costs/daily", get(handle_cost_daily))
        .route("/api/costs/providers", get(handle_cost_providers))
        .route(
            "/api/costs/spend-limit",
            get(handle_cost_spend_limit_get).put(handle_cost_spend_limit_set),
        )
        // Token-maxing (RFC-031) — autonomous subscription-quota drain
        .route("/api/token-maxing/start", post(handle_token_maxing_start))
        .route("/api/token-maxing/stop", post(handle_token_maxing_stop))
        .route("/api/token-maxing/status", get(handle_token_maxing_status))
        .route(
            "/api/token-maxing/sessions",
            get(handle_token_maxing_sessions),
        )
        .route(
            "/api/token-maxing/sessions/{id}",
            get(handle_token_maxing_session),
        )
        .route(
            "/api/token-maxing/providers",
            get(handle_token_maxing_providers),
        )
        // Knowledge
        .route("/api/knowledge/tree", get(handle_knowledge_tree))
        .route("/api/knowledge/move", post(handle_knowledge_move))
        // axum 0.8: `{*path}` MUST be the last segment, so we dispatch on method/path
        // in a single handler rather than registering separate sub-path routes.
        .route(
            "/api/knowledge/file/{*path}",
            get(handle_knowledge_file_or_sub),
        )
        .route(
            "/api/knowledge/file/{*path}",
            put(handle_knowledge_file_or_sub),
        )
        .route(
            "/api/knowledge/file/{*path}",
            delete(handle_knowledge_file_or_sub),
        )
        .route(
            "/api/knowledge/asset/{*path}",
            get(handle_knowledge_asset_get),
        )
        .route(
            "/api/knowledge/file/{*path}",
            post(handle_knowledge_file_or_sub),
        )
        .route("/api/knowledge/search", post(handle_knowledge_search))
        .route("/api/knowledge/file-diff", get(handle_knowledge_file_diff))
        .route("/api/knowledge/graph", get(handle_knowledge_graph))
        .route("/api/knowledge/backlinks", get(handle_knowledge_backlinks))
        .route("/api/knowledge/copilot", post(handle_knowledge_copilot))
        // Knowledge — Checklist
        .route(
            "/api/knowledge/checklist/items",
            post(handle_knowledge_checklist_items),
        )
        .route(
            "/api/knowledge/checklist/add",
            post(handle_knowledge_checklist_add),
        )
        .route(
            "/api/knowledge/checklist/complete",
            post(handle_knowledge_checklist_complete),
        )
        .route(
            "/api/knowledge/checklist/remove",
            post(handle_knowledge_checklist_remove),
        )
        // Knowledge — Chat
        .route(
            "/api/knowledge/chat/append",
            post(handle_knowledge_chat_append),
        )
        .route(
            "/api/knowledge/chat/messages",
            get(handle_knowledge_chat_messages),
        )
        .route(
            "/api/knowledge/chat/delete",
            post(handle_knowledge_chat_delete),
        )
        .route("/api/knowledge/chat/move", post(handle_knowledge_chat_move))
        // Knowledge — Journal
        .route(
            "/api/knowledge/journal/add",
            post(handle_knowledge_journal_add),
        )
        .route(
            "/api/knowledge/journal/emoji",
            post(handle_knowledge_journal_emoji),
        )
        .route(
            "/api/knowledge/journal/today",
            get(handle_knowledge_journal_today),
        )
        // Knowledge — Habits
        .route("/api/knowledge/habits", get(handle_knowledge_habits))
        .route(
            "/api/knowledge/habits/last-week",
            get(handle_knowledge_habits_last_week),
        )
        // Knowledge — Stats
        .route(
            "/api/knowledge/stats/today",
            get(handle_knowledge_stats_today),
        )
        .route(
            "/api/knowledge/stats/done-today",
            get(handle_knowledge_stats_done_today),
        )
        // Knowledge — Config
        .route("/api/knowledge/config", get(handle_knowledge_config_get))
        .route("/api/knowledge/config", put(handle_knowledge_config_put))
        // Knowledge — Worker
        .route(
            "/api/knowledge/worker/nightly",
            post(handle_knowledge_worker_nightly),
        )
        .route(
            "/api/knowledge/worker/scheduled",
            post(handle_knowledge_worker_scheduled),
        )
        // Knowledge — Convert & Emoji
        .route(
            "/api/knowledge/convert/html",
            post(handle_knowledge_convert_html),
        )
        .route("/api/knowledge/emoji", get(handle_knowledge_emoji))
        // Marketplace (ClawHub) — install requires auth
        .route(
            "/api/marketplace/skills/{slug}/install",
            post(handle_marketplace_install),
        )
        // Marketplace (Skills.sh) — install requires auth
        .route(
            "/api/marketplace/skills-sh/skill/{id}/install",
            post(handle_skills_sh_install),
        )
        // System Update
        .route("/api/update/check", get(handle_update_check))
        .route("/api/update/changelog", get(handle_update_changelog))
        .route("/api/update/run", post(handle_update_run))
        // System Tools
        .route("/api/system/doctor", post(handle_doctor))
        .route("/api/system/audit-verify", post(handle_audit_verify_api))
        .route("/api/system/backup", post(handle_backup))
        .route("/api/system/log", get(handle_log))
        // Host Tools (RFC-041) — host-CLI discovery inventory
        .route("/api/host-tools", get(handle_host_tools))
        .route("/api/host-tools/detect", post(handle_host_tools_detect))
        // Integrations (RFC-041) — registry + credential status
        .route("/api/integrations", get(handle_integrations_list))
        .route(
            "/api/integrations/{id}/credential",
            get(handle_integration_credential_status)
                .put(handle_integration_credential_set)
                .delete(handle_integration_credential_delete),
        )
        .route(
            "/api/integrations/{id}/install",
            post(handle_integration_install),
        )
        .route(
            "/api/integrations/{id}/oauth/start",
            post(handle_integration_oauth_start),
        )
        .route(
            "/api/integrations/{id}/oauth/poll",
            get(handle_integration_oauth_poll),
        )
        // Search & Browse (Search Panel)
        .route("/api/search", post(handle_search))
        .route("/api/browse", post(handle_browse))
        ;

        // Screenshot (CSS-rendered, Blitz-backed — `screenshot` feature)
        #[cfg(feature = "screenshot")]
        let api = api.route("/api/screenshot", get(handle_screenshot));

        let api = api
        // Unified asset store — protected CRUD
        .route(
            "/api/assets",
            post(handle_asset_upload).get(handle_asset_list),
        )
        .route(
            "/api/assets/{name}/meta",
            get(handle_asset_meta_get).put(handle_asset_meta_update),
        )
        .route("/api/assets/{name}", delete(handle_asset_delete))
        // Worktree fan-out (RFC-044 Phase 4) — protected
        .route(
            "/api/worktree/fanout",
            post(worktree_routes::handle_worktree_fanout),
        )
        .route(
            "/api/worktree/diff",
            post(worktree_routes::handle_worktree_diff),
        )
        .route(
            "/api/worktree/merge",
            post(worktree_routes::handle_worktree_merge),
        );

    // oximemo integration (first-party app module) — only when the `memo`
    // cargo feature is compiled in. Absent (404) otherwise; the web UI card
    // 404-tolerates and hides itself.
    #[cfg(feature = "memo")]
    let api = api
        .route("/api/memo/status", get(handle_memo_status))
        .route("/api/memo/enable", post(handle_memo_enable))
        .route("/api/memo/disable", post(handle_memo_disable));
    #[cfg(feature = "timeline")]
    let api = api
        .route("/api/timeline/status", get(handle_timeline_status))
        .route("/api/timeline/enable", post(handle_timeline_enable))
        .route("/api/timeline/disable", post(handle_timeline_disable));

    let api = api
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_auth,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_ready,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone().rate_limiter.clone(),
            rate_limit_layer,
        ))
        .layer(axum::extract::DefaultBodyLimit::max(API_BODY_LIMIT))
        .with_state(state.clone());

    public.merge(api).with_state(state)
}

/// Body size limit for API requests (32 MB).
///
/// Oxios is local-first / single-user (AGENTS.md: no containers, direct host
/// execution), so this is a sanity guard against accidental huge payloads, not
/// a multi-tenant DoS threshold. 32 MB comfortably covers skill archive uploads.
const API_BODY_LIMIT: usize = 32 * 1024 * 1024;
