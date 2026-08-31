//! Read-only Operate projections for the web IA redesign (Task 4.1).
//!
//! Four `GET /api/operate/*` endpoints that project EXISTING sources onto the
//! frozen operate contract — no scheduler, no lifecycle manager, no event bus,
//! no second authority:
//!
//! - `/api/operate/attention` — pending HitL approvals (security facade) and
//!   failed/canceled automation runs (`AppState.task_store`).
//! - `/api/operate/runs` — automation runs (task store) merged with the recent
//!   agent page (agents facade), running first then most-recent activity.
//! - `/api/operate/projects/{id}/context` — project record, root liveness,
//!   open issues, milestone counts (derived from labelled-issue listing only),
//!   and running agent runs bound to the project.
//! - `/api/operate/capabilities` — one row per real MCP server, skill, engine
//!   provider, the approval-mode config, and each channel plugin. Command
//!   lines, args, env values, tokens and credentials are NEVER projected.
//!
//! `waiting_input` items: the pending tool-approval / path-access / ask-user
//! registries (`infra.pending_*()`) are resolve-only surfaces — ids are
//! handed out through SSE events and there is no enumeration API. Until the
//! kernel exposes one, this projection truthfully emits no `waiting_input`
//! items rather than inventing them.
//!
//! Redaction rule: summaries and errors are built only from stored
//! name/instruction/resource fields and truncated (200/500 chars). Tool
//! arguments and tool outputs are never included.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use oxios_kernel::access_manager::{Action, ApprovalStatus, PendingApproval, Subject};
use oxios_kernel::agent_log_db::{
    AgentListFilter, AgentStatusFilter, SearchField, SortBy, SortDir,
};
use oxios_kernel::approval::ApprovalMode;
use oxios_kernel::task::{ListTasksParams, Task, TaskRun, TaskRunTrigger};
use oxios_kernel::types::AgentInfo;
use oxios_kernel::{IssueQuery, SkillSource, SkillStatus};

use crate::api::error::AppError;
use crate::api::server::AppState;

/// Maximum characters for attention summaries.
const SUMMARY_MAX_CHARS: usize = 200;
/// Maximum characters for run error text.
const ERROR_MAX_CHARS: usize = 500;
/// Cap on combined `failed_run` attention items.
const FAILED_RUN_CAP: usize = 20;
/// Cap on open issues listed in a project context.
const OPEN_ISSUES_CAP: usize = 20;
/// Cap on running agent runs listed in a project context.
const ACTIVE_RUNS_CAP: usize = 20;
/// Default page size for `/api/operate/runs`.
const DEFAULT_RUNS_LIMIT: u32 = 50;
/// Maximum page size for `/api/operate/runs`.
const MAX_RUNS_LIMIT: u32 = 200;
/// Task-store page size when scanning tasks for their latest run.
const TASK_SCAN_LIMIT: u32 = 500;

// ── Attention ───────────────────────────────────────────────────────────────

/// Response for `GET /api/operate/attention`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttentionResponse {
    items: Vec<OperateAttentionItem>,
    generated_at: String,
}

/// Why an item needs attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperateAttentionKind {
    /// A pending HitL approval.
    PendingApproval,
    /// An agent blocked on user input (no truthful source yet — see module docs).
    #[allow(dead_code)] // frozen contract string; no enumeration source exists yet
    WaitingInput,
    /// An automation run that failed or was canceled.
    FailedRun,
}

/// Safe follow-up actions the web client may offer for an attention item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperateAction {
    /// Approve a pending approval.
    Approve,
    /// Reject a pending approval.
    Reject,
    /// Open the run detail.
    OpenRun,
    /// Open the agent detail.
    #[allow(dead_code)] // frozen contract string; agents emit no attention items yet
    OpenAgent,
    /// Open the owning automation.
    OpenAutomation,
}

/// Where an attention item comes from. Unknown fields are omitted entirely.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OperateOrigin {
    /// Owning agent id, when the source records one.
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    /// Owning session id, when the source records one.
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    /// Owning task id, for automation-run items.
    #[serde(skip_serializing_if = "Option::is_none")]
    automation_id: Option<String>,
    /// Owning task name, for automation-run items.
    #[serde(skip_serializing_if = "Option::is_none")]
    automation_name: Option<String>,
    /// Owning project id, when the source records one.
    #[serde(skip_serializing_if = "Option::is_none")]
    project_id: Option<String>,
}

/// One item in the operate attention queue.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OperateAttentionItem {
    kind: OperateAttentionKind,
    /// Approval uuid, registry item id, or task run id.
    id: String,
    /// Exact backend state string ("pending", the run's terminal status, …).
    state: String,
    /// Redacted one-line description built only from stored fields.
    summary: String,
    origin: OperateOrigin,
    /// RFC3339 timestamp of the source record.
    timestamp: String,
    /// Safe actions only.
    actions: Vec<OperateAction>,
    /// Canonical in-app route, when one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    detail_route: Option<String>,
}

/// GET /api/operate/attention — items needing attention, newest source first.
pub(crate) async fn handle_operate_attention(
    State(state): State<Arc<AppState>>,
) -> Result<Json<AttentionResponse>, AppError> {
    let mut items = Vec::new();

    // 1. Pending approvals (status Pending only).
    for (approval, status) in state.kernel.security.list_approvals() {
        if status == ApprovalStatus::Pending {
            items.push(pending_approval_item(&approval));
        }
    }

    // 2. `waiting_input` — the pending tool/path/ask-user registries expose no
    //    enumeration API (resolve-only), so nothing truthful is emitted here.

    // 3. Failed/canceled automation runs — latest run per task.
    let store = state.task_store.lock().await;
    let tasks = store
        .list_tasks(ListTasksParams {
            limit: Some(TASK_SCAN_LIMIT),
            ..ListTasksParams::default()
        })
        .await
        .map_err(|e| AppError::Internal(format!("Failed to list tasks: {e}")))?;
    let mut failed: Vec<(TaskRun, Task)> = Vec::new();
    for task in tasks {
        match store.latest_run(&task.id).await {
            Ok(Some(run)) if run.status == "failed" || run.status == "canceled" => {
                failed.push((run, task));
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(task = %task.id, error = %e, "Skipping unreadable task run"),
        }
    }
    // Newest first by the run's own timestamp, then cap.
    failed.sort_by_key(|(run, _)| std::cmp::Reverse(parse_rfc3339(&run.started_at)));
    failed.truncate(FAILED_RUN_CAP);
    for (run, task) in &failed {
        items.push(failed_run_item(run, task));
    }

    Ok(Json(AttentionResponse {
        items,
        generated_at: Utc::now().to_rfc3339(),
    }))
}

/// Project a pending approval onto an attention item.
fn pending_approval_item(approval: &PendingApproval) -> OperateAttentionItem {
    let origin = match &approval.subject {
        Subject::Agent(id) => OperateOrigin {
            agent_id: Some(id.to_string()),
            ..OperateOrigin::default()
        },
        _ => OperateOrigin::default(),
    };
    OperateAttentionItem {
        kind: OperateAttentionKind::PendingApproval,
        id: approval.id.to_string(),
        state: "pending".to_string(),
        summary: truncate_chars(
            &format!("Approval requested: {}", action_label(&approval.action)),
            SUMMARY_MAX_CHARS,
        ),
        origin,
        timestamp: approval.created_at.to_rfc3339(),
        actions: vec![OperateAction::Approve, OperateAction::Reject],
        // No canonical operate route exists for approvals yet — omitted.
        detail_route: None,
    }
}

/// Short English label for a requested action (stored enum payload only).
fn action_label(action: &Action) -> String {
    match action {
        Action::UseTool(tool) => format!("use tool {tool}"),
        Action::AccessPath(path) => format!("access path {path}"),
        Action::ManageAgents => "manage agents".to_string(),
        Action::ManagePrograms => "manage programs".to_string(),
        Action::ManageWorkspaces => "manage workspaces".to_string(),
        Action::ManageRBAC => "manage RBAC".to_string(),
        Action::ViewAuditLog => "view audit log".to_string(),
        Action::SystemConfig => "system config".to_string(),
    }
}

/// Project a failed/canceled automation run onto an attention item.
fn failed_run_item(run: &TaskRun, task: &Task) -> OperateAttentionItem {
    OperateAttentionItem {
        kind: OperateAttentionKind::FailedRun,
        id: run.id.clone(),
        state: run.status.clone(),
        summary: truncate_chars(
            &format!("Automation \"{}\" run {}", task.name, run.status),
            SUMMARY_MAX_CHARS,
        ),
        origin: OperateOrigin {
            automation_id: Some(task.id.clone()),
            automation_name: Some(task.name.clone()),
            ..OperateOrigin::default()
        },
        timestamp: run.started_at.clone(),
        actions: vec![OperateAction::OpenRun, OperateAction::OpenAutomation],
        detail_route: Some("/operate/runs".to_string()),
    }
}

// ── Runs ────────────────────────────────────────────────────────────────────

/// Response for `GET /api/operate/runs`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunsResponse {
    runs: Vec<OperateRun>,
    total: u64,
}

/// Which surface a run row belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperateRunKind {
    /// A scheduled/manual task run.
    Automation,
    /// An agent execution.
    Agent,
}

/// Run trigger, mirrored from the task store (`TaskRunTrigger`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperateRunTrigger {
    /// Manually triggered.
    Manual,
    /// Cron schedule.
    Schedule,
    /// Heartbeat interval.
    Heartbeat,
}

impl From<TaskRunTrigger> for OperateRunTrigger {
    fn from(value: TaskRunTrigger) -> Self {
        match value {
            TaskRunTrigger::Manual => Self::Manual,
            TaskRunTrigger::Schedule => Self::Schedule,
            TaskRunTrigger::Heartbeat => Self::Heartbeat,
        }
    }
}

/// One run row in the merged operate run list.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OperateRun {
    kind: OperateRunKind,
    id: String,
    name: String,
    /// Exact backend status string.
    status: String,
    /// Automation runs only.
    #[serde(skip_serializing_if = "Option::is_none")]
    trigger: Option<OperateRunTrigger>,
    /// Automation runs only.
    #[serde(skip_serializing_if = "Option::is_none")]
    automation_id: Option<String>,
    /// Only when recorded (agents carry project ids; tasks do not).
    #[serde(skip_serializing_if = "Option::is_none")]
    project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    /// Truncated to [`ERROR_MAX_CHARS`].
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    steps_completed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    steps_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens_used: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_secs: Option<f64>,
}

/// Query parameters for `GET /api/operate/runs`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct OperateRunsQuery {
    limit: Option<u32>,
}

/// GET /api/operate/runs — merged automation + agent runs, running first.
pub(crate) async fn handle_operate_runs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<OperateRunsQuery>,
) -> Result<Json<RunsResponse>, AppError> {
    let limit = params
        .limit
        .unwrap_or(DEFAULT_RUNS_LIMIT)
        .clamp(1, MAX_RUNS_LIMIT);

    // Rows carry an internal sort key: (active first, most recent activity).
    let mut rows: Vec<(bool, DateTime<Utc>, OperateRun)> = Vec::new();
    let mut automation_total: u64 = 0;

    {
        let store = state.task_store.lock().await;
        let tasks = store
            .list_tasks(ListTasksParams {
                limit: Some(TASK_SCAN_LIMIT),
                ..ListTasksParams::default()
            })
            .await
            .map_err(|e| AppError::Internal(format!("Failed to list tasks: {e}")))?;
        for task in &tasks {
            let runs = store
                .list_runs(&task.id)
                .await
                .map_err(|e| AppError::Internal(format!("Failed to list runs: {e}")))?;
            automation_total += runs.len() as u64;
            for run in &runs {
                let activity = run.completed_at.as_deref().unwrap_or(&run.started_at);
                rows.push((
                    run.status == "running",
                    parse_rfc3339(activity),
                    automation_run_row(run, task),
                ));
            }
        }
    }

    // Agent rows — same recent-first filter defaults as `/api/agents`.
    let filter = AgentListFilter {
        q: None,
        search_field: SearchField::All,
        status: None,
        session_id: None,
        project_id: None,
        model_id: None,
        tool: None,
        has_error: None,
        date_from: None,
        date_to: None,
        cost_min: None,
        cost_max: None,
        tokens_min: None,
        tokens_max: None,
        duration_min: None,
        duration_max: None,
        sort_by: SortBy::CreatedAt,
        sort_dir: SortDir::Desc,
        page: 1,
        per_page: limit,
    };
    let agents = state
        .kernel
        .agents
        .query(&filter)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to list agents: {e}")))?;
    let agent_total = agents.total;
    for agent in &agents.items {
        rows.push(agent_sort_row(agent));
    }

    // Running first, then most-recent-activity first.
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    rows.truncate(limit as usize);

    Ok(Json(RunsResponse {
        runs: rows.into_iter().map(|(_, _, run)| run).collect(),
        total: automation_total + agent_total,
    }))
}

/// Sort key + row for an agent run.
fn agent_sort_row(agent: &AgentInfo) -> (bool, DateTime<Utc>, OperateRun) {
    let active = matches!(agent.status.to_string().as_str(), "running" | "starting");
    let activity = agent
        .completed_at
        .or(agent.started_at)
        .unwrap_or(agent.created_at);
    (active, activity, agent_run_row(agent))
}

/// Map a `TaskRun` plus its owning `Task` onto the operate contract.
fn automation_run_row(run: &TaskRun, task: &Task) -> OperateRun {
    let duration_secs = run.completed_at.as_deref().and_then(|completed| {
        DateTime::parse_from_rfc3339(completed)
            .ok()
            .zip(DateTime::parse_from_rfc3339(&run.started_at).ok())
            .map(|(end, start)| (end - start).num_seconds().max(0) as f64)
    });
    OperateRun {
        kind: OperateRunKind::Automation,
        id: run.id.clone(),
        name: task.name.clone(),
        status: run.status.clone(),
        trigger: Some(OperateRunTrigger::from(run.trigger.clone())),
        automation_id: Some(task.id.clone()),
        // Tasks do not record a project.
        project_id: None,
        session_id: run.session_id.clone(),
        error: run
            .error
            .as_deref()
            .map(|e| truncate_chars(e, ERROR_MAX_CHARS)),
        steps_completed: None,
        steps_total: None,
        tokens_used: run.tokens_used,
        cost_usd: run.cost_usd,
        model_id: None,
        started_at: Some(run.started_at.clone()),
        completed_at: run.completed_at.clone(),
        duration_secs,
    }
}

/// Map an `AgentInfo` onto the operate contract (same shapes as
/// `/api/agents` summaries).
fn agent_run_row(agent: &AgentInfo) -> OperateRun {
    let model_id = if agent.model_id.is_empty() {
        None
    } else {
        Some(agent.model_id.clone())
    };
    OperateRun {
        kind: OperateRunKind::Agent,
        id: agent.id.to_string(),
        name: agent.name.clone(),
        status: agent.status.to_string(),
        // Agents carry no trigger.
        trigger: None,
        automation_id: None,
        project_id: agent.project_id.map(|id| id.to_string()),
        session_id: agent.session_id.clone(),
        error: agent
            .error
            .as_deref()
            .map(|e| truncate_chars(e, ERROR_MAX_CHARS)),
        steps_completed: Some(agent.steps_completed as u64),
        steps_total: agent.steps_total.map(|t| t as u64),
        tokens_used: Some(agent.tokens_input + agent.tokens_output),
        cost_usd: Some(agent.cost_usd),
        model_id,
        started_at: agent.started_at.map(|t| t.to_rfc3339()),
        completed_at: agent.completed_at.map(|t| t.to_rfc3339()),
        duration_secs: agent
            .completed_at
            .zip(agent.started_at)
            .map(|(end, start)| (end - start).num_seconds().max(0) as f64),
    }
}

// ── Project context ─────────────────────────────────────────────────────────

/// Response for `GET /api/operate/projects/{id}/context`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OperateProjectContext {
    project: OperateProjectSummary,
    /// Root liveness from the same source `/api/projects/{id}/roots/status`
    /// reads; `None` when the project has no roots.
    #[serde(skip_serializing_if = "Option::is_none")]
    roots: Option<Vec<OperateProjectRoot>>,
    issues_open: Vec<OperateIssueRef>,
    milestones: Vec<OperateMilestoneSummary>,
    active_runs: Vec<OperateRun>,
}

/// The stored project record fields relevant to operate.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OperateProjectSummary {
    id: String,
    name: String,
    root_paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_brain_space: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_active_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
}

/// One project root. `branch`/`dirty` are omitted — the root-liveness source
/// supplies neither.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OperateProjectRoot {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dirty: Option<bool>,
}

/// Open issue reference.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OperateIssueRef {
    id: String,
    title: String,
    state: String,
}

/// Milestone with counts derived from the labelled-issue listing.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OperateMilestoneSummary {
    slug: String,
    open_count: u64,
    closed_count: u64,
}

/// GET /api/operate/projects/{id}/context — everything operate shows for a
/// project. Unknown projects (and unparseable ids, which cannot exist in the
/// project database) are 404.
pub(crate) async fn handle_operate_project_context(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<OperateProjectContext>, AppError> {
    let not_found = || AppError::NotFound("Project not found".to_string());
    let api = state.kernel.projects.as_ref().ok_or_else(not_found)?;
    let project = api.get_project(&id).ok_or_else(not_found)?;

    // Root liveness — same source `/api/projects/{id}/roots/status` reads.
    let roots = if project.root_paths.is_empty() {
        None
    } else {
        Some(
            project
                .root_paths
                .iter()
                .map(|path| OperateProjectRoot {
                    path: path.clone(),
                    branch: None,
                    dirty: None,
                })
                .collect(),
        )
    };

    let pid = uuid::Uuid::parse_str(&project.id).map_err(|_| not_found())?;

    // Open issues, capped.
    let issues_open = state
        .kernel
        .issues
        .list(
            pid,
            &IssueQuery {
                status: Some(oxicode_sdk::Status::Open),
                ..IssueQuery::default()
            },
        )
        .map_err(|e| AppError::Internal(e.to_string()))?
        .into_iter()
        .take(OPEN_ISSUES_CAP)
        .map(|issue| OperateIssueRef {
            id: issue.number.to_string(),
            title: issue.title,
            state: issue.status,
        })
        .collect();

    // Milestones — `list_milestones` derives its counts by listing the real
    // labelled issues (never stored progress).
    let milestones = state
        .kernel
        .issues
        .list_milestones(pid)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .into_iter()
        .map(|milestone| OperateMilestoneSummary {
            slug: milestone.slug,
            open_count: (milestone.total - milestone.closed) as u64,
            closed_count: milestone.closed as u64,
        })
        .collect();

    // Running agent runs bound to this project, capped.
    let filter = AgentListFilter {
        q: None,
        search_field: SearchField::All,
        status: Some(AgentStatusFilter::Running),
        session_id: None,
        project_id: Some(project.id.clone()),
        model_id: None,
        tool: None,
        has_error: None,
        date_from: None,
        date_to: None,
        cost_min: None,
        cost_max: None,
        tokens_min: None,
        tokens_max: None,
        duration_min: None,
        duration_max: None,
        sort_by: SortBy::CreatedAt,
        sort_dir: SortDir::Desc,
        page: 1,
        per_page: ACTIVE_RUNS_CAP as u32,
    };
    let active_runs = state
        .kernel
        .agents
        .query(&filter)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to list agents: {e}")))?
        .items
        .iter()
        .take(ACTIVE_RUNS_CAP)
        .map(agent_run_row)
        .collect();

    Ok(Json(OperateProjectContext {
        project: OperateProjectSummary {
            id: project.id,
            name: project.name,
            root_paths: project.root_paths,
            instructions: Some(project.instructions),
            default_brain_space: project.default_brain_space,
            last_active_at: Some(project.last_active_at),
            created_at: Some(project.created_at),
        },
        roots,
        issues_open,
        milestones,
        active_runs,
    }))
}

// ── Capabilities ────────────────────────────────────────────────────────────

/// Response for `GET /api/operate/capabilities`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CapabilitiesResponse {
    families: Vec<OperateCapability>,
}

/// Capability family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperateCapabilityFamily {
    /// MCP servers.
    Mcp,
    /// Skills.
    Skill,
    /// Engine providers.
    Engine,
    /// Security configuration.
    Security,
    /// Channels.
    Channel,
}

/// One capability row. Only non-sensitive fields are projected.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OperateCapability {
    family: OperateCapabilityFamily,
    id: String,
    name: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    /// Canonical `/operate/system/*` deep route.
    deep_route: String,
}

/// GET /api/operate/capabilities — one row per real source record.
///
/// Never includes tokens, secrets, credentials, raw args, or env values.
pub(crate) async fn handle_operate_capabilities(
    State(state): State<Arc<AppState>>,
) -> Result<Json<CapabilitiesResponse>, AppError> {
    let mut families = Vec::new();

    // MCP servers — same source `/api/mcp/servers` reads, minus command/args/env.
    for name in state.kernel.mcp.list_servers() {
        let enabled = state
            .kernel
            .mcp
            .get_server(&name)
            .map(|server| server.enabled)
            .unwrap_or(false);
        let initialized = state.kernel.mcp.client_status(&name).await.unwrap_or(false);
        let status = if !enabled {
            "disabled"
        } else if initialized {
            "connected"
        } else {
            "enabled"
        };
        families.push(OperateCapability {
            family: OperateCapabilityFamily::Mcp,
            id: name.clone(),
            name,
            status: status.to_string(),
            scope: None,
            deep_route: "/operate/system/mcp".to_string(),
        });
    }

    // Skills — same source `/api/skills` reads.
    for entry in state.kernel.extensions.list_skills_entries().await {
        let status = match entry.status {
            SkillStatus::Ready => "ready",
            SkillStatus::NeedsSetup => "needs_setup",
            SkillStatus::Disabled => "disabled",
        };
        let scope = match entry.source {
            SkillSource::Bundled => "bundled",
            SkillSource::Managed => "managed",
            SkillSource::Workspace => "workspace",
            SkillSource::Foundation => "foundation",
        };
        families.push(OperateCapability {
            family: OperateCapabilityFamily::Skill,
            id: entry.skill.name.clone(),
            name: entry.skill.name,
            status: status.to_string(),
            scope: Some(scope.to_string()),
            deep_route: "/operate/system/skills".to_string(),
        });
    }

    // Engine providers — same source `/api/engine/providers` reads, minus
    // key material (`has_key` becomes a status string).
    for provider in state.kernel.engine.providers() {
        let status = if provider.has_key {
            "configured"
        } else {
            "unconfigured"
        };
        families.push(OperateCapability {
            family: OperateCapabilityFamily::Engine,
            id: provider.id.clone(),
            name: provider.name,
            status: status.to_string(),
            scope: None,
            deep_route: "/operate/system/settings".to_string(),
        });
    }

    // Security — the approval-mode config is one row.
    let mode = match state.kernel.infra.approval_config().mode {
        ApprovalMode::Manual => "manual",
        ApprovalMode::AllowList => "allow-list",
        ApprovalMode::AutoRun => "auto-run",
    };
    families.push(OperateCapability {
        family: OperateCapabilityFamily::Security,
        id: "approval-mode".to_string(),
        name: "Approval mode".to_string(),
        status: mode.to_string(),
        scope: None,
        deep_route: "/operate/system/security".to_string(),
    });

    // Channels — same plugin/config/gateway source `/api/channels` reads.
    let enabled_channels = state.config.read().channels.enabled.clone();
    let running_channels = state.gateway.channel_names().await;
    for plugin in crate::build_channel_plugins() {
        let name = plugin.name().to_string();
        let status = if running_channels.contains(&name) {
            "running"
        } else if enabled_channels.contains(&name) {
            "enabled"
        } else {
            "available"
        };
        families.push(OperateCapability {
            family: OperateCapabilityFamily::Channel,
            id: name.clone(),
            name,
            status: status.to_string(),
            scope: None,
            deep_route: "/operate/system/connections".to_string(),
        });
    }

    Ok(Json(CapabilitiesResponse { families }))
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Truncate to at most `max` characters (char-boundary safe).
fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        value.chars().take(max).collect()
    }
}

/// Parse an RFC3339 timestamp; unparseable values sort as the epoch rather
/// than failing the projection.
fn parse_rfc3339(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_default()
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // `.unwrap()` on setup ops is idiomatic in tests

    use super::*;
    use crate::api::routes::{build_routes, test_support};
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use oxios_kernel::McpServer;
    use tower::Service;

    fn router(app: &test_support::TestApp) -> Router {
        build_routes(app.state.clone()).with_state(app.state.clone())
    }

    async fn send(svc: &mut Router, method: &str, uri: &str) -> (StatusCode, serde_json::Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let response = svc.call(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, json)
    }

    /// Test 1: fresh fixture → empty attention, 200, generatedAt present.
    #[tokio::test]
    async fn attention_empty_fixture_returns_empty_items() {
        let tmp = tempfile::tempdir().unwrap();
        let app = test_support::test_app(tmp.path());
        let mut svc = router(&app);

        let (status, body) = send(&mut svc, "GET", "/api/operate/attention").await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(body["items"], serde_json::json!([]));
        assert!(body["generatedAt"].as_str().is_some());
    }

    /// Test 2 (mapping level): a pending approval maps to a
    /// `pending_approval` item with the exact source id and approve+reject
    /// actions. (The fixture's access manager is not exposed for seeding, so
    /// the approval → item mapping is asserted directly — see report.)
    #[tokio::test]
    async fn pending_approval_maps_source_fields_and_safe_actions() {
        let id = uuid::Uuid::new_v4();
        let agent_id = uuid::Uuid::new_v4();
        let approval = PendingApproval {
            id,
            subject: Subject::Agent(agent_id),
            action: Action::UseTool("rm".to_string()),
            resource: "rm".to_string(),
            reason: "destructive tool".to_string(),
            created_at: Utc::now(),
        };

        let item = pending_approval_item(&approval);
        assert_eq!(item.kind, OperateAttentionKind::PendingApproval);
        assert_eq!(item.id, id.to_string());
        assert_eq!(item.state, "pending");
        assert_eq!(
            item.actions,
            vec![OperateAction::Approve, OperateAction::Reject]
        );
        assert_eq!(
            item.origin.agent_id.as_deref(),
            Some(agent_id.to_string()).as_deref()
        );

        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["kind"], "pending_approval");
        assert_eq!(json["state"], "pending");
        assert_eq!(json["id"], id.to_string());
        assert!(
            json.get("detailRoute").is_none(),
            "no canonical route yet: {json}"
        );
        // Approve/reject serialized as snake_case action strings.
        assert_eq!(json["actions"], serde_json::json!(["approve", "reject"]));
    }

    /// Test 3: a failed task run surfaces in attention (with automation
    /// origin) and in runs (kind "automation"); healthy runs never do.
    #[tokio::test]
    async fn failed_run_surfaces_in_attention_and_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let app = test_support::test_app(tmp.path());
        let mut svc = router(&app);

        let store = app.state.task_store.lock().await;
        let failed_task = store
            .create_task(oxios_kernel::task::CreateTaskParams {
                name: "nightly-sync".to_string(),
                instruction: "sync the vault".to_string(),
                identifier: None,
                description: None,
                priority: None,
                parent_task_id: None,
                assignee_agent_id: None,
                created_by_agent_id: None,
                created_by_session_id: None,
                sort_order: None,
            })
            .await
            .unwrap();
        let run_id = store
            .mark_running(&failed_task.id, TaskRunTrigger::Schedule)
            .await
            .unwrap();
        store
            .mark_finished(
                &failed_task.id,
                &run_id,
                false,
                false,
                "run failed".to_string(),
                Some("boom".to_string()),
            )
            .await
            .unwrap();

        // A healthy run on a second task must NOT appear in attention.
        let healthy_task = store
            .create_task(oxios_kernel::task::CreateTaskParams {
                name: "healthy-task".to_string(),
                instruction: "keep going".to_string(),
                identifier: None,
                description: None,
                priority: None,
                parent_task_id: None,
                assignee_agent_id: None,
                created_by_agent_id: None,
                created_by_session_id: None,
                sort_order: None,
            })
            .await
            .unwrap();
        let healthy_run = store
            .mark_running(&healthy_task.id, TaskRunTrigger::Manual)
            .await
            .unwrap();
        store
            .mark_finished(
                &healthy_task.id,
                &healthy_run,
                true,
                false,
                "done".to_string(),
                None,
            )
            .await
            .unwrap();
        drop(store);

        let (status, body) = send(&mut svc, "GET", "/api/operate/attention").await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        let items = body["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "only the failed run: {body}");
        let item = &items[0];
        assert_eq!(item["kind"], "failed_run");
        assert_eq!(item["state"], "failed");
        assert_eq!(item["origin"]["automationId"], failed_task.id);
        assert_eq!(item["origin"]["automationName"], "nightly-sync");
        assert_eq!(item["detailRoute"], "/operate/runs");
        assert_eq!(
            item["actions"],
            serde_json::json!(["open_run", "open_automation"])
        );

        let (status, body) = send(&mut svc, "GET", "/api/operate/runs").await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        let runs = body["runs"].as_array().unwrap();
        let row = runs
            .iter()
            .find(|r| r["id"] == run_id)
            .expect("failed run row present");
        assert_eq!(row["kind"], "automation");
        assert_eq!(row["status"], "failed");
        assert_eq!(row["automationId"], failed_task.id);
        assert_eq!(row["trigger"], "schedule");
        assert_eq!(row["error"], "boom");
        assert!(row.get("projectId").is_none(), "tasks carry no project");
    }

    /// Test 4 (redaction): a seeded MCP server's env secret and args never
    /// reach the capabilities response — only the whitelisted row fields do.
    #[tokio::test]
    async fn capabilities_never_leak_seeded_secrets() {
        let tmp = tempfile::tempdir().unwrap();
        let app = test_support::test_app(tmp.path());

        let mut server = McpServer::new("secret-mcp", "/bin/echo");
        server.args = vec!["--token".to_string(), "arg-secret-value".to_string()];
        server.env.insert(
            "SECRET_TOKEN".to_string(),
            "sk-live-super-secret".to_string(),
        );
        app.state.kernel.mcp.register_server(server);

        let mut svc = router(&app);
        let (status, body) = send(&mut svc, "GET", "/api/operate/capabilities").await;
        assert_eq!(status, StatusCode::OK, "body: {body}");

        let body_str = serde_json::to_string(&body).unwrap();
        assert!(body_str.contains("secret-mcp"), "row is listed: {body}");
        assert!(
            !body_str.contains("sk-live-super-secret"),
            "env secret leaked: {body}"
        );
        assert!(!body_str.contains("arg-secret-value"), "arg leaked: {body}");
        assert!(!body_str.contains("\"command\""), "{body}");
        assert!(!body_str.contains("\"args\""), "{body}");
        assert!(!body_str.contains("\"env\""), "{body}");

        let mcp_row = body["families"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["family"] == "mcp" && f["id"] == "secret-mcp")
            .expect("mcp row present");
        assert_eq!(mcp_row["status"], "enabled");
        assert_eq!(mcp_row["deepRoute"], "/operate/system/mcp");
    }

    /// Test 5: a project with no roots/issues/milestones/active agents gets
    /// empty relations — nothing invented.
    #[tokio::test]
    async fn project_context_with_no_relations_omits_them() {
        let tmp = tempfile::tempdir().unwrap();
        let app = test_support::test_app(tmp.path());
        let project = app
            .project_manager
            .create("context-proj", vec![], "")
            .unwrap();
        let pid = project.id.to_string();
        let mut svc = router(&app);

        let (status, body) = send(
            &mut svc,
            "GET",
            &format!("/api/operate/projects/{pid}/context"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(body["project"]["id"], pid);
        assert_eq!(body["project"]["name"], "context-proj");
        assert_eq!(body["issuesOpen"], serde_json::json!([]));
        assert_eq!(body["milestones"], serde_json::json!([]));
        assert_eq!(body["activeRuns"], serde_json::json!([]));
        assert!(
            body.get("roots").is_none() || body["roots"].is_null(),
            "no roots → omitted: {body}"
        );
    }

    /// Test 6: unparseable and unknown project ids are 404.
    #[tokio::test]
    async fn project_context_invalid_and_unknown_ids_are_404() {
        let tmp = tempfile::tempdir().unwrap();
        let app = test_support::test_app(tmp.path());
        let mut svc = router(&app);

        for id in ["definitely-not-a-uuid", &uuid::Uuid::new_v4().to_string()] {
            let (status, _) = send(
                &mut svc,
                "GET",
                &format!("/api/operate/projects/{id}/context"),
            )
            .await;
            assert_eq!(status, StatusCode::NOT_FOUND, "id: {id}");
        }
    }

    /// Test 7: the existing task surface is untouched.
    #[tokio::test]
    async fn existing_tasks_endpoint_still_works() {
        let tmp = tempfile::tempdir().unwrap();
        let app = test_support::test_app(tmp.path());
        let mut svc = router(&app);

        let (status, body) = send(&mut svc, "GET", "/api/tasks").await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert!(body["tasks"].is_array());
    }

    /// The truncation helper respects char boundaries (redaction rule).
    #[test]
    fn truncate_chars_caps_length_at_char_boundary() {
        let long = "x".repeat(500);
        assert_eq!(truncate_chars(&long, 200).chars().count(), 200);
        assert_eq!(truncate_chars("short", 200), "short");
        let unicode = "한글테스트".repeat(100);
        let capped = truncate_chars(&unicode, 10);
        assert_eq!(capped.chars().count(), 10);
    }
}
