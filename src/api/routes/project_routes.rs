//! Project management API routes.
//!
//! CRUD over the reduced project record: `{ name, root_paths, instructions }`.
//! Root-path validation failures and name conflicts return 422 with an
//! English message (design §9).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use std::sync::Arc;

use oxios_kernel::{ProjectInfo, ProjectManagerError};

use crate::api::error::AppError;
use crate::api::server::AppState;

// ─── Request / Query types ──────────────────────────────────

/// List query parameters with search.
#[derive(Debug, Deserialize)]
pub(crate) struct ProjectListParams {
    #[serde(default = "default_page")]
    pub(crate) page: usize,
    #[serde(default = "default_limit")]
    pub(crate) limit: usize,
    pub(crate) search: Option<String>,
}

fn default_page() -> usize {
    1
}

fn default_limit() -> usize {
    50
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateProjectRequest {
    pub name: String,
    /// Canonical, existing directories. Order matters — the first root is
    /// the working directory. May be empty. Duplicates are deduped.
    #[serde(default)]
    pub root_paths: Vec<String>,
    pub instructions: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateProjectRequest {
    pub name: Option<String>,
    pub root_paths: Option<Vec<String>>,
    pub instructions: Option<String>,
}

// ─── Helpers ────────────────────────────────────────────────

/// Shorthand to get ProjectApi from AppState, or error.
macro_rules! project_api {
    ($state:expr) => {
        $state
            .kernel
            .projects
            .as_ref()
            .ok_or_else(|| AppError::Internal("Projects not available".into()))?
    };
}

/// Map a typed project error onto its HTTP status (design §9).
fn project_error(e: ProjectManagerError) -> AppError {
    match e {
        ProjectManagerError::NotFound(_) => AppError::NotFound(e.to_string()),
        ProjectManagerError::InvalidRoot { .. } | ProjectManagerError::NameExists(_) => {
            AppError::UnprocessableEntity(e.to_string())
        }
        ProjectManagerError::Invalid(_) | ProjectManagerError::Database(_) => {
            AppError::BadRequest(e.to_string())
        }
    }
}

// ─── Handlers ───────────────────────────────────────────────

/// GET /api/projects — List all projects with pagination and search.
pub(crate) async fn handle_projects_list(
    state: State<Arc<AppState>>,
    Query(params): Query<ProjectListParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let api = project_api!(state);
    let all = api.list_projects();

    // Filter by search — name and instructions only (design §4.4).
    let filtered: Vec<ProjectInfo> = match &params.search {
        Some(search) => {
            let lower = search.to_lowercase();
            all.into_iter()
                .filter(|p| {
                    p.name.to_lowercase().contains(&lower)
                        || p.instructions.to_lowercase().contains(&lower)
                })
                .collect()
        }
        None => all,
    };

    // Sort by last_active_at descending
    let mut sorted = filtered;
    sorted.sort_by(|a, b| b.last_active_at.cmp(&a.last_active_at));

    let total = sorted.len();
    let limit = params.limit.min(500);
    let offset = params.page.saturating_sub(1) * limit;
    let items: Vec<&ProjectInfo> = sorted.iter().skip(offset).take(limit).collect();

    Ok(Json(serde_json::json!({
        "items": items,
        "total": total,
        "page": params.page,
        "limit": limit,
    })))
}

/// GET /api/projects/:id — Get a single project.
pub(crate) async fn handle_project_get(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ProjectInfo>, AppError> {
    let api = project_api!(state);
    api.get_project(&id)
        .ok_or_else(|| AppError::NotFound("Project not found".into()))
        .map(Json)
}

/// POST /api/projects — Create a new project.
pub(crate) async fn handle_project_create(
    state: State<Arc<AppState>>,
    Json(body): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<ProjectInfo>), AppError> {
    let api = project_api!(state);

    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("Project name is required".into()));
    }

    let project = api
        .create_project(
            &body.name,
            body.root_paths,
            body.instructions.as_deref().unwrap_or(""),
        )
        .map_err(project_error)?;

    Ok((StatusCode::CREATED, Json(project)))
}

/// PUT /api/projects/:id — Update a project.
pub(crate) async fn handle_project_update(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateProjectRequest>,
) -> Result<Json<ProjectInfo>, AppError> {
    let api = project_api!(state);

    let project = api
        .update_project(&id, body.name, body.root_paths, body.instructions)
        .map_err(project_error)?;

    Ok(Json(project))
}

/// DELETE /api/projects/:id — Remove a project.
pub(crate) async fn handle_project_delete(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let api = project_api!(state);

    api.remove_project(&id)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}
