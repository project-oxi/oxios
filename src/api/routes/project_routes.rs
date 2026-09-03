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
use crate::api::routes::deserialize_some;
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
    /// Default brain space inherited by NEW chats in this Project. Absent =
    /// keep current; JSON `null` = clear; string = set.
    #[serde(default, deserialize_with = "deserialize_some")]
    pub default_brain_space: Option<Option<String>>,
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
        // Client-recoverable misuse.
        ProjectManagerError::Invalid(_) => AppError::BadRequest(e.to_string()),
        // Storage failures are the server's fault, not the client's — and
        // the raw sqlite message must not leak into the response body.
        ProjectManagerError::Database(_) => AppError::Internal("Project storage error".into()),
    }
}

/// Per-root liveness for a project (design §9: a stale project root is
/// surfaced in project editing, never silently replaced by an ambient
/// workspace). No session needed — this is pure project-record inspection.
#[derive(serde::Serialize)]
pub(crate) struct RootStatus {
    path: String,
    exists: bool,
    is_dir: bool,
}

/// GET /api/projects/:id/roots/status — stale-root surfacing.
pub(crate) async fn handle_project_roots_status(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<RootStatus>>, AppError> {
    let api = project_api!(state);
    let project = api
        .get_project(&id)
        .ok_or_else(|| AppError::NotFound("Project not found".into()))?;
    let statuses = project
        .root_paths
        .iter()
        .map(|path| {
            let meta = std::fs::metadata(path);
            RootStatus {
                path: path.clone(),
                exists: meta.is_ok(),
                is_dir: meta.map(|m| m.is_dir()).unwrap_or(false),
            }
        })
        .collect();
    Ok(Json(statuses))
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

    // Default brain space uses the tri-state bundle update: absent = keep
    // current, JSON `null` = clear, string = set. Everything else rides the
    // regular reduced-record update below.
    if body.default_brain_space.is_some() {
        let project = api
            .update_project_bundle(&id, body.instructions.clone(), body.default_brain_space)
            .map_err(|e| AppError::BadRequest(e.to_string()))?;
        // Also apply traditional field updates if provided.
        if body.name.is_some() || body.root_paths.is_some() || body.instructions.is_some() {
            let updated = api
                .update_project(&id, body.name, body.root_paths, body.instructions)
                .map_err(project_error)?;
            return Ok(Json(updated));
        }
        return Ok(Json(project));
    }
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

    api.remove_project(&id).map_err(project_error)?;

    Ok(StatusCode::NO_CONTENT)
}

/// PUT /api/projects/:id/execution-recipe — Bind (or clear) the project's
/// execution recipe (execution-recipe coding host design).
///
/// Writes `[execution_recipe].project_bindings` through the shared config
/// Arc and persists config.toml. The binding is observed by the recipe
/// resolver on the project's NEXT agent turn (it re-reads the config every
/// turn — no restart). `recipe: null` clears the binding.
pub(crate) async fn handle_project_execution_recipe_put(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<SetExecutionRecipeRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let api = project_api!(state);
    api.get_project(&id)
        .ok_or_else(|| AppError::NotFound("Project not found".into()))?;

    // Validate the recipe id up front: the resolver would silently degrade
    // an unknown id to general-v1, which would mask a typo.
    if let Some(recipe) = body
        .recipe
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let known = [
            oxios_kernel::execution_recipe::GENERAL_V1,
            oxios_kernel::execution_recipe::CODING_OMP_V1,
        ];
        if !known.contains(&recipe) {
            return Err(AppError::BadRequest(format!(
                "Unknown recipe '{recipe}'. Known recipes: {}",
                known.join(", ")
            )));
        }
    }

    {
        let mut cfg = state.config.write();
        match body
            .recipe
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(recipe) => {
                cfg.execution_recipe
                    .project_bindings
                    .insert(id.clone(), recipe.to_string());
            }
            None => {
                cfg.execution_recipe.project_bindings.remove(&id);
            }
        }
    }

    // Persist config.toml (same whole-file write as the other section routes).
    let content = {
        let cfg = state.config.read();
        toml::to_string_pretty(&*cfg)
            .map_err(|e: toml::ser::Error| AppError::Internal(e.to_string()))?
    };
    tokio::fs::write(&state.config_path, content)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let bound = state
        .config
        .read()
        .execution_recipe
        .project_bindings
        .get(&id)
        .cloned();
    Ok(Json(serde_json::json!({
        "project_id": id,
        "recipe": bound,
    })))
}

/// Request body for [`handle_project_execution_recipe_put`].
#[derive(Debug, Deserialize)]
pub(crate) struct SetExecutionRecipeRequest {
    /// Recipe id to bind (e.g. `"coding-omp-v1"`), or `null` to clear.
    #[serde(default)]
    pub(crate) recipe: Option<String>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // `.unwrap()` on setup ops is idiomatic in tests

    use crate::api::routes::{build_routes, test_support};
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
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
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, json)
    }

    /// Create a project over the real route table; returns its id.
    async fn create_project(svc: &mut Router, name: &str, roots: Vec<String>) -> String {
        let body = serde_json::json!({ "name": name, "root_paths": roots });
        let request = Request::builder()
            .method("POST")
            .uri("/api/projects")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = svc.call(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        json["id"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn roots_status_reports_exists_and_is_dir_per_root() {
        let tmp = tempfile::tempdir().unwrap();
        let live = tmp.path().join("live-repo");
        std::fs::create_dir_all(&live).unwrap();
        let app = test_support::test_app(tmp.path());
        let mut svc = router(&app);
        let id = create_project(
            &mut svc,
            "status-live",
            vec![live.to_string_lossy().to_string()],
        )
        .await;
        // validate_root_paths canonicalizes at create time (macOS /var is a
        // symlink to /private/var), so the stored root is the canonical form.
        let canon = live.canonicalize().unwrap();

        let (status, body) =
            send(&mut svc, "GET", &format!("/api/projects/{id}/roots/status")).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        let entries = body.as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["path"], canon.to_string_lossy().to_string());
        assert_eq!(entries[0]["exists"], serde_json::json!(true));
        assert_eq!(entries[0]["is_dir"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn roots_status_flags_root_deleted_after_creation() {
        // Design §9: a root that vanishes after validation is surfaced as
        // stale (exists=false) — never silently replaced by ambient scope.
        let tmp = tempfile::tempdir().unwrap();
        let doomed = tmp.path().join("doomed-repo");
        std::fs::create_dir_all(&doomed).unwrap();
        let app = test_support::test_app(tmp.path());
        let mut svc = router(&app);
        let id = create_project(
            &mut svc,
            "status-stale",
            vec![doomed.to_string_lossy().to_string()],
        )
        .await;
        std::fs::remove_dir_all(&doomed).unwrap();

        let (status, body) =
            send(&mut svc, "GET", &format!("/api/projects/{id}/roots/status")).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        let entries = body.as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["exists"], serde_json::json!(false));
        assert_eq!(entries[0]["is_dir"], serde_json::json!(false));
    }

    #[tokio::test]
    async fn roots_status_unknown_project_is_404_and_folderless_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let app = test_support::test_app(tmp.path());
        let mut svc = router(&app);

        let unknown = uuid::Uuid::new_v4().to_string();
        let (status, _) = send(
            &mut svc,
            "GET",
            &format!("/api/projects/{unknown}/roots/status"),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let id = create_project(&mut svc, "status-folderless", vec![]).await;
        let (status, body) =
            send(&mut svc, "GET", &format!("/api/projects/{id}/roots/status")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!([]));
    }

    #[tokio::test]
    async fn delete_unknown_project_maps_to_404_via_shared_mapper() {
        // F1: DELETE goes through project_error like create/update — a
        // missing project is 404, a storage failure is 500 without the raw
        // sqlite message (never a blanket 400).
        let tmp = tempfile::tempdir().unwrap();
        let app = test_support::test_app(tmp.path());
        let mut svc = router(&app);
        let unknown = uuid::Uuid::new_v4().to_string();
        let (status, body) = send(&mut svc, "DELETE", &format!("/api/projects/{unknown}")).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
        assert!(
            !body["error"].as_str().unwrap_or("").contains("database"),
            "sqlite message must not leak: {body}"
        );
    }
}
