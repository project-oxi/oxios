//! Per-project issue and milestone API routes.
//!
//! Thin transport over [`oxios_kernel::IssueApi`]. The one policy decision
//! that lives here rather than in the facade is CAS handling: the agent tool
//! auto-reconciles a stale `content_hash`, but a human editing an issue in the
//! Web UI must be told that someone else changed it under them. So these
//! routes pass the client's hash straight through and surface `Conflict` as
//! `409` instead of retrying.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use oxicode_sdk::{IssueError, IssuePatch, Priority, Status};
use oxios_kernel::project::MilestonePatch;
use oxios_kernel::{IssueQuery, NewIssue};

use crate::api::error::AppError;
use crate::api::server::AppState;

// ─── Request / query types ──────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct IssueListParams {
    /// `open` | `closed`. Absent means both.
    pub status: Option<String>,
    pub priority: Option<String>,
    pub label: Option<String>,
    pub milestone: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateIssueRequest {
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub priority: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    pub milestone: Option<String>,
    /// Chat session to attribute the issue to. Absent for UI-created issues.
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateIssueRequest {
    pub title: Option<String>,
    pub body: Option<String>,
    pub priority: Option<String>,
    pub status: Option<String>,
    /// Replaces labels wholesale. `[]` clears; absent keeps.
    pub labels: Option<Vec<String>>,
    /// Hash from the last read. Absent skips the conflict check.
    pub content_hash: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct IssueActionRequest {
    pub content_hash: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateMilestoneRequest {
    pub title: String,
    pub description: Option<String>,
    /// `YYYY-MM-DD`.
    pub due: Option<String>,
    pub slug: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateMilestoneRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    /// `Some("")` clears the due date; absent keeps it.
    pub due: Option<String>,
    pub status: Option<String>,
}

// ─── Helpers ────────────────────────────────────────────────

fn project_id(raw: &str) -> Result<uuid::Uuid, AppError> {
    uuid::Uuid::parse_str(raw).map_err(|_| AppError::BadRequest("Invalid project ID".into()))
}

fn priority(raw: Option<&str>) -> Result<Option<Priority>, AppError> {
    Ok(match raw {
        None => None,
        Some("low") => Some(Priority::Low),
        Some("medium") => Some(Priority::Medium),
        Some("high") => Some(Priority::High),
        Some("critical") => Some(Priority::Critical),
        Some(other) => return Err(AppError::BadRequest(format!("Invalid priority: {other}"))),
    })
}

fn status(raw: Option<&str>) -> Result<Option<Status>, AppError> {
    Ok(match raw {
        None => None,
        Some("open") => Some(Status::Open),
        Some("closed") => Some(Status::Closed),
        Some(other) => return Err(AppError::BadRequest(format!("Invalid status: {other}"))),
    })
}

fn due_date(raw: &str) -> Result<Option<chrono::NaiveDate>, AppError> {
    if raw.is_empty() {
        return Ok(None);
    }
    raw.parse::<chrono::NaiveDate>()
        .map(Some)
        .map_err(|_| AppError::BadRequest(format!("Invalid due date (expected YYYY-MM-DD): {raw}")))
}

/// Map a store error onto the status code that says what actually happened.
///
/// `Assigned` is `423 Locked` rather than a generic conflict: the client has
/// to show *who* holds the issue and offer to release it, which is a different
/// remedy from re-reading after a concurrent edit.
fn issue_error(e: IssueError) -> AppError {
    match e {
        IssueError::NotFound { id } => AppError::NotFound(format!("Issue #{id} not found")),
        IssueError::Conflict { id } => AppError::Conflict(format!(
            "Issue #{id} changed since you loaded it; reload and reapply your edit"
        )),
        IssueError::Assigned {
            id,
            owner,
            acquired_at,
        } => AppError::Locked(format!(
            "Issue #{id} is being worked on by {owner} (since {acquired_at})"
        )),
        IssueError::NotAssigned { id, caller } => AppError::Forbidden(format!(
            "Issue #{id} is not claimed by {caller}; claim it first"
        )),
        other => AppError::Internal(other.to_string()),
    }
}

/// The session identity a mutation is attributed to.
///
/// UI edits have no chat session, so they act as a synthetic `web` session.
/// It is a real identity — non-empty and backed by a guard — so a UI claim
/// behaves like any other and is released when the daemon exits.
fn session_of(explicit: Option<String>) -> String {
    explicit
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "web".into())
}

// ─── Issue handlers ─────────────────────────────────────────

pub(crate) async fn handle_issues_list(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<IssueListParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pid = project_id(&id)?;
    let query = IssueQuery {
        status: status(params.status.as_deref())?,
        priority: priority(params.priority.as_deref())?,
        label: params.label,
        milestone: params.milestone,
        text: params.text,
    };
    let issues = state
        .kernel
        .issues
        .list(pid, &query)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(json!({ "items": issues, "total": issues.len() })))
}

pub(crate) async fn handle_issue_get(
    State(state): State<Arc<AppState>>,
    Path((id, number)): Path<(String, u32)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pid = project_id(&id)?;
    let issue = state.kernel.issues.read(pid, number).map_err(issue_error)?;
    Ok(Json(json!(issue)))
}

pub(crate) async fn handle_issue_create(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<CreateIssueRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let pid = project_id(&id)?;
    if body.title.trim().is_empty() {
        return Err(AppError::BadRequest("Title is required".into()));
    }
    let new = NewIssue {
        title: body.title,
        body: body.body,
        priority: priority(body.priority.as_deref())?.unwrap_or_default(),
        labels: body.labels,
        milestone: body.milestone,
        session: Some(session_of(body.session_id)),
    };
    let issue = state
        .kernel
        .issues
        .create(pid, new)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok((StatusCode::CREATED, Json(json!(issue))))
}

pub(crate) async fn handle_issue_update(
    State(state): State<Arc<AppState>>,
    Path((id, number)): Path<(String, u32)>,
    Json(body): Json<UpdateIssueRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pid = project_id(&id)?;
    let patch = IssuePatch {
        title: body.title,
        body: body.body,
        status: status(body.status.as_deref())?,
        priority: priority(body.priority.as_deref())?,
        labels: body.labels,
    };
    let session = session_of(body.session_id);
    let issue = state
        .kernel
        .issues
        .patch(pid, number, patch, Some(&session), body.content_hash)
        .await
        .map_err(issue_error)?;
    Ok(Json(json!(issue)))
}

pub(crate) async fn handle_issue_close(
    State(state): State<Arc<AppState>>,
    Path((id, number)): Path<(String, u32)>,
    Json(body): Json<IssueActionRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pid = project_id(&id)?;
    let session = session_of(body.session_id);
    // Closing requires the claim, so take it first. A live claim held by
    // someone else surfaces as 423 from `start` rather than a confusing
    // "not assigned to you" from `close`.
    state
        .kernel
        .issues
        .start(pid, number, &session, body.content_hash.clone())
        .await
        .map_err(issue_error)?;
    let issue = state
        .kernel
        .issues
        .close(pid, number, &session, None)
        .await
        .map_err(issue_error)?;
    Ok(Json(json!(issue)))
}

pub(crate) async fn handle_issue_reopen(
    State(state): State<Arc<AppState>>,
    Path((id, number)): Path<(String, u32)>,
    Json(body): Json<IssueActionRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pid = project_id(&id)?;
    let issue = state
        .kernel
        .issues
        .reopen(pid, number, body.content_hash)
        .await
        .map_err(issue_error)?;
    Ok(Json(json!(issue)))
}

pub(crate) async fn handle_issue_release(
    State(state): State<Arc<AppState>>,
    Path((id, number)): Path<(String, u32)>,
    Json(body): Json<IssueActionRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pid = project_id(&id)?;
    let session = session_of(body.session_id);
    let issue = state
        .kernel
        .issues
        .release(pid, number, &session, body.content_hash)
        .await
        .map_err(issue_error)?;
    Ok(Json(json!(issue)))
}

// ─── Milestone handlers ─────────────────────────────────────

pub(crate) async fn handle_milestones_list(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pid = project_id(&id)?;
    let milestones = state
        .kernel
        .issues
        .list_milestones(pid)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(
        json!({ "items": milestones, "total": milestones.len() }),
    ))
}

pub(crate) async fn handle_milestone_create(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<CreateMilestoneRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let pid = project_id(&id)?;
    if body.title.trim().is_empty() {
        return Err(AppError::BadRequest("Title is required".into()));
    }
    let due = match body.due.as_deref() {
        Some(d) => due_date(d)?,
        None => None,
    };
    let milestone = state
        .kernel
        .issues
        .create_milestone(pid, body.title, body.description, due, body.slug)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok((StatusCode::CREATED, Json(json!(milestone))))
}

pub(crate) async fn handle_milestone_update(
    State(state): State<Arc<AppState>>,
    Path((id, slug)): Path<(String, String)>,
    Json(body): Json<UpdateMilestoneRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    use oxios_kernel::project::MilestoneStatus;
    let pid = project_id(&id)?;
    let patch = MilestonePatch {
        title: body.title,
        description: body.description,
        // `Some("")` clears the date, absent keeps it — the outer Option is
        // "did the client mention `due` at all".
        due: match body.due.as_deref() {
            Some(d) => Some(due_date(d)?),
            None => None,
        },
        status: match body.status.as_deref() {
            None => None,
            Some("open") => Some(MilestoneStatus::Open),
            Some("closed") => Some(MilestoneStatus::Closed),
            Some(other) => {
                return Err(AppError::BadRequest(format!(
                    "Invalid milestone status: {other}"
                )));
            }
        },
    };
    let milestone = state
        .kernel
        .issues
        .update_milestone(pid, &slug, patch)
        .map_err(|e| AppError::NotFound(e.to_string()))?;
    Ok(Json(json!(milestone)))
}

pub(crate) async fn handle_milestone_delete(
    State(state): State<Arc<AppState>>,
    Path((id, slug)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pid = project_id(&id)?;
    let failed = state
        .kernel
        .issues
        .delete_milestone(pid, &slug)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    if !failed.is_empty() {
        // The milestone is intentionally still there: removing the definition
        // while members still carry its label would orphan them.
        return Err(AppError::Conflict(format!(
            "Could not clear the milestone from issue(s) {}; the milestone was kept",
            failed
                .iter()
                .map(|n| format!("#{n}"))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Ok(Json(json!({ "deleted": slug })))
}

pub(crate) async fn handle_issue_set_milestone(
    State(state): State<Arc<AppState>>,
    Path((id, number)): Path<(String, u32)>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pid = project_id(&id)?;
    // An explicit `null` clears membership; that is the whole point of the
    // endpoint, so absent and null are treated the same.
    let slug = body.get("milestone").and_then(|v| v.as_str());
    let session = session_of(
        body.get("session_id")
            .and_then(|v| v.as_str())
            .map(String::from),
    );
    let issue = state
        .kernel
        .issues
        .set_milestone(pid, number, slug, Some(&session))
        .await
        .map_err(issue_error)?;
    Ok(Json(json!(issue)))
}
