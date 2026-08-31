//! API routes for automation management.
//!
//! CRUD + trigger scheduling + verify + run history for the automation
//! lifecycle. Mechanical cutover note (web IA major version): the route
//! paths keep their `/api/tasks` prefix until the HTTP-rename task lands.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use oxios_kernel::automation::{
    AutomationRunTrigger, AutomationStatus, CreateAutomationParams, ListAutomationsParams,
    SetTriggerParams, SetVerifyParams, UpdateAutomationParams, execute_automation_run,
};

/// Ceiling for a synchronous manual run (`POST /api/tasks/:id/run`).
/// Longer-running work belongs on a schedule (cron/heartbeat), whose jobs
/// use the longer tick timeout.
const TASK_RUN_TIMEOUT: u64 = 300;

use crate::api::error::AppError;
use crate::api::server::AppState;

// ── List ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListAutomationsQuery {
    pub status: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// GET /api/tasks
pub(crate) async fn handle_tasks_list(
    state: State<Arc<AppState>>,
    Query(q): Query<ListAutomationsQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let params = ListAutomationsParams {
        status: q.status,
        limit: q.limit,
        offset: q.offset,
    };

    let store = state.automation_store.lock().await;
    match store.list_automations(params).await {
        Ok(list) => Ok(Json(
            serde_json::json!({ "tasks": list, "count": list.len() }),
        )),
        Err(e) => {
            tracing::error!(error = %e, "Failed to list tasks");
            Err(AppError::BadRequest(format!("Failed to list tasks: {e}")))
        }
    }
}

// ── Create ────────────────────────────────────────────────────────

/// POST /api/tasks
pub(crate) async fn handle_task_create(
    state: State<Arc<AppState>>,
    Json(params): Json<CreateAutomationParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    if params.name.trim().is_empty() || params.instruction.trim().is_empty() {
        return Err(AppError::BadRequest(
            "name and instruction are required".into(),
        ));
    }

    let store = state.automation_store.lock().await;
    match store.create_automation(params).await {
        Ok(automation) => Ok(Json(serde_json::to_value(&automation).unwrap_or_default())),
        Err(e) => {
            tracing::error!(error = %e, "Failed to create task");
            Err(AppError::BadRequest(format!("Failed to create task: {e}")))
        }
    }
}

// ── Get by ID ─────────────────────────────────────────────────────

/// GET /api/tasks/:id
pub(crate) async fn handle_task_get(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let store = state.automation_store.lock().await;
    match store.get_automation(&id).await {
        Ok(automation) => Ok(Json(serde_json::to_value(&automation).unwrap_or_default())),
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
    let store = state.automation_store.lock().await;
    match store.delete_automation(&id).await {
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
    let status: AutomationStatus = req
        .status
        .parse()
        .map_err(|e: String| AppError::BadRequest(e))?;

    let store = state.automation_store.lock().await;
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

// ── Set schedule (trigger) ────────────────────────────────────────

/// PUT /api/tasks/:id/schedule
pub(crate) async fn handle_task_set_schedule(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(params): Json<SetTriggerParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Validate + persist the trigger and re-arm scheduling in one store call.
    let result = {
        let store = state.automation_store.lock().await;
        store.set_trigger(&id, params).await
    };
    match result {
        Ok(()) => {
            // Re-read to return the computed next_run/status.
            let automation = {
                let store = state.automation_store.lock().await;
                store.get_automation(&id).await
            };
            match automation {
                Ok(t) => Ok(Json(serde_json::json!({
                    "id": t.id,
                    "trigger": t.trigger,
                    "cron_pattern": t.cron_pattern,
                    "timezone": t.timezone,
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
        Err(e) => Err(AppError::BadRequest(format!("Failed to set schedule: {e}"))),
    }
}

// ── Set verify ────────────────────────────────────────────────────

pub(crate) async fn handle_task_set_verify(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(params): Json<SetVerifyParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let result = {
        let store = state.automation_store.lock().await;
        store.set_verify(&id, params).await
    };
    match result {
        Ok(()) => {
            let automation = {
                let store = state.automation_store.lock().await;
                store.get_automation(&id).await
            };
            match automation {
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
/// Executes the automation's snapshot instruction through the shared
/// `run_goal` primitive (direct orchestrator path), bounded by
/// `TASK_RUN_TIMEOUT` so a hung agent can't hold the HTTP connection
/// forever. The run row (with its context snapshot) is opened and the
/// automation lifecycle updated by the shared runner.
pub(crate) async fn handle_task_run(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // 1. Validate the automation exists.
    {
        let store = state.automation_store.lock().await;
        store
            .get_automation(&id)
            .await
            .map_err(|e| AppError::NotFound(format!("Task not found: {id} ({e})")))?;
    }

    // Execute + record the full lifecycle via the shared helper (also used by
    // the auto-run tick loop). Bounded by TASK_RUN_TIMEOUT for the HTTP path.
    let (run_id, success, summary) = execute_automation_run(
        state.automation_store.clone(),
        state.kernel.clone(),
        &id,
        AutomationRunTrigger::Manual,
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
    Json(params): Json<UpdateAutomationParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let result = {
        let store = state.automation_store.lock().await;
        store.update_automation(&id, params).await
    };
    match result {
        Ok(()) => {
            let automation = {
                let store = state.automation_store.lock().await;
                store.get_automation(&id).await
            };
            match automation {
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
pub struct CreateAutomationsBatchRequest {
    pub tasks: Vec<CreateAutomationParams>,
}

/// POST /api/tasks/batch — create several automations in one call.
pub(crate) async fn handle_task_create_batch(
    state: State<Arc<AppState>>,
    Json(req): Json<CreateAutomationsBatchRequest>,
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
        let store = state.automation_store.lock().await;
        for params in req.tasks {
            let automation = store
                .create_automation(params)
                .await
                .map_err(|e| AppError::BadRequest(format!("Failed to create task: {e}")))?;
            created.push(automation);
        }
    }
    Ok(Json(
        serde_json::json!({ "tasks": created, "count": created.len() }),
    ))
}

// ── Run history ───────────────────────────────────────────────────

/// GET /api/tasks/:id/runs — execution history (newest first).
pub(crate) async fn handle_task_runs(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let runs = {
        let store = state.automation_store.lock().await;
        store
            .list_runs(&id)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to list runs: {e}")))?
    };
    Ok(Json(
        serde_json::json!({ "runs": runs, "count": runs.len() }),
    ))
}
