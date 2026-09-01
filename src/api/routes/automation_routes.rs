//! API routes for automation management.
//!
//! CRUD + trigger scheduling + verify + run history for the automation
//! lifecycle. Only the `/api/automations` prefix is registered; any other
//! prefix 404s.
//!
//! Endpoints (exact, exhaustive):
//!
//! ```text
//! GET/POST        /api/automations
//! GET/PUT/DELETE  /api/automations/:id
//! PUT             /api/automations/:id/status
//! PUT             /api/automations/:id/trigger
//! PUT             /api/automations/:id/verify
//! POST            /api/automations/:id/run
//! GET             /api/automations/:id/runs
//! ```

use std::str::FromStr;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use oxios_kernel::automation::{
    AutomationRunTrigger, AutomationStatus, CreateAutomationParams, ListAutomationsParams,
    SetTriggerParams, SetVerifyParams, UpdateAutomationParams, execute_automation_run,
};

use crate::api::error::AppError;
use crate::api::server::AppState;

/// Ceiling for a synchronous manual run (`POST /api/automations/:id/run`).
/// Longer-running work belongs on a schedule (cron/heartbeat), whose jobs
/// use the longer tick timeout.
const AUTOMATION_RUN_TIMEOUT_SECS: u64 = 300;

// ── List ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListAutomationsQuery {
    pub status: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// GET /api/automations
pub(crate) async fn handle_automations_list(
    state: State<Arc<AppState>>,
    Query(q): Query<ListAutomationsQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let params = ListAutomationsParams {
        status: q.status,
        limit: q.limit,
        offset: q.offset,
    };

    let list = {
        let store = state.automation_store.lock().await;
        store.list_automations(params).await.map_err(|e| {
            tracing::error!(error = %e, "Failed to list automations");
            AppError::BadRequest(format!("Failed to list automations: {e}"))
        })?
    };
    Ok(Json(
        serde_json::json!({ "automations": list, "count": list.len() }),
    ))
}

// ── Create ────────────────────────────────────────────────────────

/// POST /api/automations
pub(crate) async fn handle_automation_create(
    state: State<Arc<AppState>>,
    Json(params): Json<CreateAutomationParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    if params.name.trim().is_empty() || params.instruction.trim().is_empty() {
        return Err(AppError::BadRequest(
            "name and instruction are required".into(),
        ));
    }

    let automation = {
        let store = state.automation_store.lock().await;
        store.create_automation(params).await.map_err(|e| {
            tracing::error!(error = %e, "Failed to create automation");
            AppError::BadRequest(format!("Failed to create automation: {e}"))
        })?
    };
    Ok(Json(serde_json::to_value(&automation).unwrap_or_default()))
}

// ── Get by ID ─────────────────────────────────────────────────────

/// GET /api/automations/:id
pub(crate) async fn handle_automation_get(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let automation = {
        let store = state.automation_store.lock().await;
        store.get_automation(&id).await.map_err(|e| {
            tracing::error!(error = %e, id = %id, "Failed to get automation");
            AppError::NotFound(format!("Automation not found: {id}"))
        })?
    };
    Ok(Json(serde_json::to_value(&automation).unwrap_or_default()))
}

// ── Delete ────────────────────────────────────────────────────────

/// DELETE /api/automations/:id
pub(crate) async fn handle_automation_delete(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    {
        let store = state.automation_store.lock().await;
        store.delete_automation(&id).await.map_err(|e| {
            tracing::error!(error = %e, id = %id, "Failed to delete automation");
            AppError::Internal(format!("Failed to delete automation: {e}"))
        })?;
    }
    Ok(Json(serde_json::json!({ "id": id, "deleted": true })))
}

// ── Update status ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UpdateStatusRequest {
    pub status: String,
}

/// PUT /api/automations/:id/status
pub(crate) async fn handle_automation_update_status(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateStatusRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status =
        AutomationStatus::from_str(&req.status).map_err(|e: String| AppError::BadRequest(e))?;
    {
        let store = state.automation_store.lock().await;
        store.update_status(&id, &status).await.map_err(|e| {
            tracing::error!(error = %e, id = %id, "Failed to update automation status");
            AppError::Internal(format!("Failed to update status: {e}"))
        })?;
    }
    Ok(Json(
        serde_json::json!({ "id": id, "status": status.to_string() }),
    ))
}

// ── Set trigger ──────────────────────────────────────────────────

/// PUT /api/automations/:id/trigger
pub(crate) async fn handle_automation_set_trigger(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(params): Json<SetTriggerParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Validate + persist the trigger and re-arm scheduling in one store call.
    let set_result = {
        let store = state.automation_store.lock().await;
        store.set_trigger(&id, params).await
    };
    if let Err(e) = set_result {
        return Err(AppError::BadRequest(format!("Failed to set trigger: {e}")));
    }
    // Re-read to return the computed next_run/status.
    let automation = {
        let store = state.automation_store.lock().await;
        store
            .get_automation(&id)
            .await
            .map_err(|e| AppError::Internal(format!("Trigger set, reload failed: {e}")))?
    };
    Ok(Json(serde_json::json!({
        "id": automation.id,
        "trigger": automation.trigger,
        "cronPattern": automation.cron_pattern,
        "timezone": automation.timezone,
        "heartbeatIntervalSecs": automation.heartbeat_interval_secs,
        "maxExecutions": automation.max_executions,
        "nextRunAt": automation.next_run_at,
        "status": automation.status,
    })))
}

// ── Set verify ────────────────────────────────────────────────────

/// PUT /api/automations/:id/verify
pub(crate) async fn handle_automation_set_verify(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(params): Json<SetVerifyParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let set_result = {
        let store = state.automation_store.lock().await;
        store.set_verify(&id, params).await
    };
    if let Err(e) = set_result {
        tracing::error!(error = %e, id = %id, "Failed to set verify config");
        if e.to_string().contains("not found") {
            return Err(AppError::NotFound(format!("Automation not found: {id}")));
        }
        return Err(AppError::Internal(format!("Failed to set verify: {e}")));
    }
    let automation = {
        let store = state.automation_store.lock().await;
        store
            .get_automation(&id)
            .await
            .map_err(|e| AppError::Internal(format!("Verify config set, reload failed: {e}")))?
    };
    Ok(Json(serde_json::to_value(&automation).unwrap_or_default()))
}

// ── Run automation ────────────────────────────────────────────────

/// POST /api/automations/:id/run — trigger manual (synchronous) execution.
///
/// Executes the automation's snapshot instruction through the shared
/// `run_goal` primitive (direct orchestrator path), bounded by
/// `AUTOMATION_RUN_TIMEOUT_SECS` so a hung agent can't hold the HTTP
/// connection forever. The run row (with its context snapshot) is opened
/// and the automation lifecycle updated by the shared runner.
pub(crate) async fn handle_automation_run(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // 1. Validate the automation exists.
    {
        let store = state.automation_store.lock().await;
        store
            .get_automation(&id)
            .await
            .map_err(|e| AppError::NotFound(format!("Automation not found: {id} ({e})")))?;
    }

    // Execute + record the full lifecycle via the shared helper (also used by
    // the auto-run tick loop). Bounded for the HTTP path.
    let (run_id, success, summary) = execute_automation_run(
        state.automation_store.clone(),
        state.kernel.clone(),
        &id,
        AutomationRunTrigger::Manual,
        AUTOMATION_RUN_TIMEOUT_SECS,
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

/// PUT /api/automations/:id — partial update; `None` fields unchanged.
pub(crate) async fn handle_automation_update(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(params): Json<UpdateAutomationParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let update_result = {
        let store = state.automation_store.lock().await;
        store.update_automation(&id, params).await
    };
    if let Err(e) = update_result {
        let msg = e.to_string();
        if msg.contains("not found") {
            return Err(AppError::NotFound(format!("Automation not found: {id}")));
        }
        if msg.contains("no fields") {
            return Err(AppError::BadRequest(msg));
        }
        tracing::error!(error = %e, id = %id, "Failed to update automation");
        return Err(AppError::Internal(format!(
            "Failed to update automation: {e}"
        )));
    }
    let automation = {
        let store = state.automation_store.lock().await;
        store
            .get_automation(&id)
            .await
            .map_err(|e| AppError::Internal(format!("Automation updated, reload failed: {e}")))?
    };
    Ok(Json(serde_json::to_value(&automation).unwrap_or_default()))
}

// ── Run history ───────────────────────────────────────────────────

/// GET /api/automations/:id/runs — execution history (newest first).
pub(crate) async fn handle_automation_runs(
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // `.unwrap()` on setup ops is idiomatic in tests

    use crate::api::routes::{build_routes, test_support};
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::json;
    use tower::Service;

    fn router(app: &test_support::TestApp) -> Router {
        build_routes(app.state.clone()).with_state(app.state.clone())
    }

    async fn send(
        svc: &mut Router,
        method: &str,
        uri: &str,
        body: Option<&serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(v) => {
                builder = builder.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let request = builder.body(body).unwrap();
        let response = svc.call(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or_else(|_| {
                serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned())
            })
        };
        (status, json)
    }

    async fn create_automation(svc: &mut Router, name: &str, instruction: &str) -> String {
        let (status, body) = send(
            svc,
            "POST",
            "/api/automations",
            Some(&json!({
                "name": name,
                "instruction": instruction,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create failed: {body}");
        body["id"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn post_create_automation_returns_id() {
        let tmp = tempfile::tempdir().unwrap();
        let app = test_support::test_app(tmp.path());
        let mut svc = router(&app);

        let id = create_automation(&mut svc, "fonts", "recommend fonts").await;
        assert!(!id.is_empty(), "create did not return an id");

        // GET returns it (handler now wired into the router).
        let (status, body) = send(&mut svc, "GET", &format!("/api/automations/{id}"), None).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(body["id"], id);
        assert_eq!(body["name"], "fonts");
        assert_eq!(body["instruction"], "recommend fonts");
    }

    #[tokio::test]
    async fn put_trigger_schedules_next_run_for_cron() {
        let tmp = tempfile::tempdir().unwrap();
        let app = test_support::test_app(tmp.path());
        let mut svc = router(&app);

        let id = create_automation(&mut svc, "every-hour", "do it").await;
        let (status, body) = send(
            &mut svc,
            "PUT",
            &format!("/api/automations/{id}/trigger"),
            Some(&json!({
                "trigger": "cron",
                "cronPattern": "0 * * * *",
                "timezone": "UTC",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(body["trigger"], "cron");
        assert!(
            body["nextRunAt"].is_string(),
            "expected computed nextRunAt, got: {body}"
        );
    }

    #[tokio::test]
    async fn post_run_creates_run_row_with_context_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let app = test_support::test_app(tmp.path());
        let mut svc = router(&app);

        let id = create_automation(&mut svc, "manual", "do the thing").await;
        let (status, body) = send(
            &mut svc,
            "POST",
            &format!("/api/automations/{id}/run"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "manual run failed: {body}");
        let run_id = body["run_id"].as_str().unwrap_or("").to_string();
        assert!(
            !run_id.is_empty(),
            "manual run did not return a run_id: {body}"
        );

        // The stored run row must carry the immutable context snapshot.
        let (status, body) = send(
            &mut svc,
            "GET",
            &format!("/api/automations/{id}/runs"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        let runs = body["runs"].as_array().expect("runs array");
        assert_eq!(runs.len(), 1, "expected one run row: {body}");
        let snap = &runs[0]["contextSnapshot"];
        assert_eq!(snap["trigger"], "manual");
        assert_eq!(snap["instruction"], "do the thing");
    }

    #[tokio::test]
    async fn get_legacy_tasks_endpoint_does_not_resolve() {
        // No compatibility routes are registered for the retired surface;
        // the router must 404 these paths instead of aliasing to automations.
        let tmp = tempfile::tempdir().unwrap();
        let app = test_support::test_app(tmp.path());
        let mut svc = router(&app);

        for path in [
            "/api/tasks",
            "/api/tasks/abc",
            "/api/tasks/abc/status",
            "/api/tasks/abc/run",
            "/api/tasks/abc/runs",
            "/api/tasks/batch",
        ] {
            let (status, _body) = send(&mut svc, "GET", path, None).await;
            assert_eq!(
                status,
                StatusCode::NOT_FOUND,
                "legacy path {path} must 404, got {status}"
            );
        }
    }
}
