//! API routes for task management (RFC-043).
//!
//! CRUD + scheduling + verify + comments for the task lifecycle.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use oxios_kernel::task::{
    CreateTaskParams, ListTasksParams, SetScheduleParams, SetVerifyParams, TaskRunTrigger,
    TaskStatus, UpdateTaskParams, execute_task_run, migrate_cron_to_tasks,
};

/// Ceiling for a synchronous manual task run (`POST /api/tasks/:id/run`).
/// Longer-running work belongs on a schedule (cron/heartbeat), whose jobs use
/// the CronScheduler's longer `job_timeout_secs`.
const TASK_RUN_TIMEOUT: u64 = 300;

use crate::api::error::AppError;
use crate::api::server::AppState;

// ── List ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListTasksQuery {
    pub statuses: Option<String>,
    pub assignee: Option<String>,
    pub parent: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// GET /api/tasks
pub(crate) async fn handle_tasks_list(
    state: State<Arc<AppState>>,
    Query(q): Query<ListTasksQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let statuses = q
        .statuses
        .map(|s| s.split(',').map(|s| s.trim().to_string()).collect());

    let params = ListTasksParams {
        statuses,
        assignee_agent_id: q.assignee,
        parent_task_id: q.parent,
        limit: q.limit,
        offset: q.offset,
    };

    let store = state.task_store.lock().await;
    match store.list_tasks(params).await {
        Ok(tasks) => Ok(Json(
            serde_json::json!({ "tasks": tasks, "count": tasks.len() }),
        )),
        Err(e) => {
            tracing::error!(error = %e, "Failed to list tasks");
            Err(AppError::Internal(format!("Failed to list tasks: {e}")))
        }
    }
}

// ── Create ────────────────────────────────────────────────────────

/// POST /api/tasks
pub(crate) async fn handle_task_create(
    state: State<Arc<AppState>>,
    Json(params): Json<CreateTaskParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    if params.name.trim().is_empty() || params.instruction.trim().is_empty() {
        return Err(AppError::BadRequest(
            "name and instruction are required".into(),
        ));
    }

    let store = state.task_store.lock().await;
    match store.create_task(params).await {
        Ok(task) => Ok(Json(serde_json::to_value(&task).unwrap_or_default())),
        Err(e) => {
            tracing::error!(error = %e, "Failed to create task");
            Err(AppError::Internal(format!("Failed to create task: {e}")))
        }
    }
}

// ── Get by ID ─────────────────────────────────────────────────────

/// GET /api/tasks/:id
pub(crate) async fn handle_task_get(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let store = state.task_store.lock().await;
    match store.get_task_by_id(&id).await {
        Ok(task) => Ok(Json(serde_json::to_value(&task).unwrap_or_default())),
        Err(e) => {
            tracing::error!(error = %e, id = %id, "Failed to get task");
            Err(AppError::NotFound(format!("Task not found: {id}")))
        }
    }
}

// ── Delete ────────────────────────────────────────────────────────

/// DELETE /api/tasks/:id
pub(crate) async fn handle_task_delete(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let store = state.task_store.lock().await;
    match store.delete_task(&id).await {
        Ok(()) => Ok(Json(serde_json::json!({ "id": id, "deleted": true }))),
        Err(e) => {
            tracing::error!(error = %e, id = %id, "Failed to delete task");
            Err(AppError::Internal(format!("Failed to delete task: {e}")))
        }
    }
}

// ── Update status ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UpdateStatusRequest {
    pub status: String,
}

/// PUT /api/tasks/:id/status
pub(crate) async fn handle_task_update_status(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateStatusRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status: TaskStatus = req
        .status
        .parse()
        .map_err(|e: String| AppError::BadRequest(e))?;

    let store = state.task_store.lock().await;
    match store.update_status(&id, &status).await {
        Ok(()) => Ok(Json(
            serde_json::json!({ "id": id, "status": status.to_string() }),
        )),
        Err(e) => {
            tracing::error!(error = %e, id = %id, "Failed to update task status");
            Err(AppError::Internal(format!("Failed to update status: {e}")))
        }
    }
}

// ── Set schedule ──────────────────────────────────────────────────

/// PUT /api/tasks/:id/schedule
pub(crate) async fn handle_task_set_schedule(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(params): Json<SetScheduleParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Persist automation fields + compute next_run + set status, all in one
    // store call (replaces the previous buggy handler that only set
    // next_run_at and dropped the automation fields).
    let result = {
        let store = state.task_store.lock().await;
        store.set_automation(&id, params).await
    };
    match result {
        Ok(()) => {
            // Re-read to return the computed next_run/status.
            let task = {
                let store = state.task_store.lock().await;
                store.get_task_by_id(&id).await
            };
            match task {
                Ok(t) => Ok(Json(serde_json::json!({
                    "id": t.id,
                    "automation_mode": t.automation_mode,
                    "schedule_pattern": t.schedule_pattern,
                    "schedule_timezone": t.schedule_timezone,
                    "heartbeat_interval_secs": t.heartbeat_interval_secs,
                    "max_executions": t.max_executions,
                    "next_run_at": t.next_run_at,
                    "status": t.status,
                }))),
                Err(e) => Err(AppError::Internal(format!(
                    "Schedule set, reload failed: {e}"
                ))),
            }
        }
        Err(e) => Err(AppError::Internal(format!("Failed to set schedule: {e}"))),
    }
}

// ── Set verify ────────────────────────────────────────────────────

pub(crate) async fn handle_task_set_verify(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(params): Json<SetVerifyParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let result = {
        let store = state.task_store.lock().await;
        store.set_verify(&id, params).await
    };
    match result {
        Ok(()) => {
            let task = {
                let store = state.task_store.lock().await;
                store.get_task_by_id(&id).await
            };
            match task {
                Ok(t) => Ok(Json(serde_json::to_value(&t).unwrap_or_default())),
                Err(e) => Err(AppError::Internal(format!(
                    "Verify config set, reload failed: {e}"
                ))),
            }
        }
        Err(e) => {
            tracing::error!(error = %e, id = %id, "Failed to set verify config");
            if e.to_string().contains("not found") {
                Err(AppError::NotFound(format!("Task not found: {id}")))
            } else {
                Err(AppError::Internal(format!("Failed to set verify: {e}")))
            }
        }
    }
}

// ── Run task ──────────────────────────────────────────────────────

/// POST /api/tasks/:id/run — trigger manual (synchronous) execution.
///
/// Executes the task's `instruction` through the shared `run_goal` primitive
/// (direct orchestrator path), bounded by `TASK_RUN_TIMEOUT` so a hung agent
/// can't hold the HTTP connection forever. Records the run in `task_runs`
/// and updates the task lifecycle (status, execution_count, etc.).
pub(crate) async fn handle_task_run(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // 1. Load + validate the task exists and isn't already running.
    let task = {
        let store = state.task_store.lock().await;
        store.get_task_by_id(&id).await
    }
    .map_err(|e| AppError::NotFound(format!("Task not found: {id} ({e})")))?;
    if task.status == TaskStatus::Running {
        return Err(AppError::Conflict(format!(
            "Task '{id}' is already running"
        )));
    }

    // Execute + record the full lifecycle via the shared helper (also used by
    // the auto-run tick loop). Bounded by TASK_RUN_TIMEOUT for the HTTP path.
    let (run_id, success, summary) = execute_task_run(
        state.task_store.clone(),
        state.kernel.clone(),
        &id,
        TaskRunTrigger::Manual,
        TASK_RUN_TIMEOUT,
    )
    .await;

    Ok(Json(serde_json::json!({
        "id": id,
        "run_id": run_id,
        "success": success,
        "summary": summary,
    })))
}

// ── Edit (partial update) ─────────────────────────────────────────

/// PUT /api/tasks/:id — partial update; `None` fields unchanged.
pub(crate) async fn handle_task_update(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(params): Json<UpdateTaskParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let result = {
        let store = state.task_store.lock().await;
        store.update_task(&id, params).await
    };
    match result {
        Ok(()) => {
            let task = {
                let store = state.task_store.lock().await;
                store.get_task_by_id(&id).await
            };
            match task {
                Ok(t) => Ok(Json(serde_json::to_value(&t).unwrap_or_default())),
                Err(e) => Err(AppError::Internal(format!(
                    "Task updated, reload failed: {e}"
                ))),
            }
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                Err(AppError::NotFound(format!("Task not found: {id}")))
            } else if msg.contains("no fields") {
                Err(AppError::BadRequest(msg))
            } else {
                tracing::error!(error = %e, id = %id, "Failed to update task");
                Err(AppError::Internal(format!("Failed to update task: {e}")))
            }
        }
    }
}

// ── Batch create ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateTasksBatchRequest {
    pub tasks: Vec<CreateTaskParams>,
}

/// POST /api/tasks/batch — create several tasks in one call.
pub(crate) async fn handle_task_create_batch(
    state: State<Arc<AppState>>,
    Json(req): Json<CreateTasksBatchRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if req.tasks.is_empty() {
        return Err(AppError::BadRequest("'tasks' must not be empty".into()));
    }
    if req
        .tasks
        .iter()
        .any(|t| t.name.trim().is_empty() || t.instruction.trim().is_empty())
    {
        return Err(AppError::BadRequest(
            "each task requires name and instruction".into(),
        ));
    }
    let mut created = Vec::with_capacity(req.tasks.len());
    {
        let store = state.task_store.lock().await;
        for params in req.tasks {
            let task = store
                .create_task(params)
                .await
                .map_err(|e| AppError::Internal(format!("Failed to create task: {e}")))?;
            created.push(task);
        }
    }
    Ok(Json(
        serde_json::json!({ "tasks": created, "count": created.len() }),
    ))
}

// ── Comments ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddCommentRequest {
    pub content: String,
    pub author_agent_id: Option<String>,
    pub author_session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCommentRequest {
    pub content: String,
}

/// GET /api/tasks/:id/comments — oldest first.
pub(crate) async fn handle_task_comments_list(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let comments = {
        let store = state.task_store.lock().await;
        store
            .list_comments(&id)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to list comments: {e}")))?
    };
    Ok(Json(
        serde_json::json!({ "comments": comments, "count": comments.len() }),
    ))
}

/// POST /api/tasks/:id/comments
pub(crate) async fn handle_task_comment_create(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AddCommentRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if req.content.trim().is_empty() {
        return Err(AppError::BadRequest("'content' must not be empty".into()));
    }
    let comment = {
        let store = state.task_store.lock().await;
        store
            .add_comment(
                &id,
                &req.content,
                req.author_agent_id.as_deref(),
                req.author_session_id.as_deref(),
            )
            .await
    };
    match comment {
        Ok(c) => Ok(Json(serde_json::to_value(&c).unwrap_or_default())),
        Err(e) => {
            tracing::error!(error = %e, id = %id, "Failed to add comment");
            Err(AppError::NotFound(format!("Task not found: {id} ({e})")))
        }
    }
}

/// PUT /api/tasks/:id/comments/:cid
pub(crate) async fn handle_task_comment_update(
    state: State<Arc<AppState>>,
    Path((_id, cid)): Path<(String, String)>,
    Json(req): Json<UpdateCommentRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let result = {
        let store = state.task_store.lock().await;
        store.update_comment(&cid, &req.content).await
    };
    match result {
        Ok(()) => Ok(Json(serde_json::json!({ "id": cid, "updated": true }))),
        Err(e) => {
            tracing::error!(error = %e, cid, "Failed to update comment");
            Err(AppError::NotFound(format!(
                "Comment not found: {cid} ({e})"
            )))
        }
    }
}

/// DELETE /api/tasks/:id/comments/:cid
pub(crate) async fn handle_task_comment_delete(
    state: State<Arc<AppState>>,
    Path((_id, cid)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let result = {
        let store = state.task_store.lock().await;
        store.delete_comment(&cid).await
    };
    match result {
        Ok(()) => Ok(Json(serde_json::json!({ "id": cid, "deleted": true }))),
        Err(e) => {
            tracing::error!(error = %e, cid, "Failed to delete comment");
            Err(AppError::NotFound(format!(
                "Comment not found: {cid} ({e})"
            )))
        }
    }
}

// ── Dependencies ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddDependencyRequest {
    pub depends_on_task_id: String,
}

/// GET /api/tasks/:id/dependencies — full dependency task objects.
pub(crate) async fn handle_task_dependencies_list(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let deps = {
        let store = state.task_store.lock().await;
        let ids = store
            .dependency_ids(&id)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to list dependencies: {e}")))?;
        let mut tasks = Vec::with_capacity(ids.len());
        for dep_id in ids {
            if let Ok(t) = store.get_task_by_id(&dep_id).await {
                tasks.push(t);
            }
        }
        tasks
    };
    Ok(Json(
        serde_json::json!({ "dependencies": deps, "count": deps.len() }),
    ))
}

/// POST /api/tasks/:id/dependencies — cycle/self-edge/duplicate are 400s.
pub(crate) async fn handle_task_dependency_add(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AddDependencyRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let result = {
        let store = state.task_store.lock().await;
        store.add_dependency(&id, &req.depends_on_task_id).await
    };
    match result {
        Ok(()) => {
            let task = {
                let store = state.task_store.lock().await;
                store.get_task_by_id(&id).await
            };
            match task {
                Ok(t) => Ok(Json(serde_json::to_value(&t).unwrap_or_default())),
                Err(e) => Err(AppError::Internal(format!(
                    "Dependency added, reload failed: {e}"
                ))),
            }
        }
        Err(e) => Err(AppError::BadRequest(format!(
            "Failed to add dependency: {e}"
        ))),
    }
}

/// DELETE /api/tasks/:id/dependencies/:dep_id
pub(crate) async fn handle_task_dependency_remove(
    state: State<Arc<AppState>>,
    Path((id, dep_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let result = {
        let store = state.task_store.lock().await;
        store.remove_dependency(&id, &dep_id).await
    };
    match result {
        Ok(()) => Ok(Json(
            serde_json::json!({ "id": id, "removed": dep_id, "deleted": true }),
        )),
        Err(e) => Err(AppError::BadRequest(format!(
            "Failed to remove dependency: {e}"
        ))),
    }
}

// ── Cron migration ────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MigrateCronRequest {
    pub dry_run: Option<bool>,
}

/// POST /api/tasks/migrate-cron — copy cron jobs into scheduled tasks.
pub(crate) async fn handle_task_migrate_cron(
    state: State<Arc<AppState>>,
    Json(req): Json<MigrateCronRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let report = {
        let store = state.task_store.lock().await;
        migrate_cron_to_tasks(
            &state.kernel.infra.list_crons(),
            &store,
            req.dry_run.unwrap_or(false),
        )
        .await
    };
    match report {
        Ok(r) => Ok(Json(serde_json::to_value(&r).unwrap_or_default())),
        Err(e) => {
            tracing::error!(error = %e, "Cron -> task migration failed");
            Err(AppError::Internal(format!("Migration failed: {e}")))
        }
    }
}

// ── Run history ───────────────────────────────────────────────────

/// GET /api/tasks/:id/runs — execution history (newest first).
pub(crate) async fn handle_task_runs(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let runs = {
        let store = state.task_store.lock().await;
        store
            .list_runs(&id)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to list runs: {e}")))?
    };
    Ok(Json(
        serde_json::json!({ "runs": runs, "count": runs.len() }),
    ))
}
