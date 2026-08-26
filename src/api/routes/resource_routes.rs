//! REST API routes for system resource monitoring.
//!
//! Exposes endpoints for current resource snapshots, historical data,
//! and overload detection via the Oxios kernel's ResourceMonitor.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;

use crate::api::error::AppError;
use crate::api::server::AppState;

// ---------------------------------------------------------------------------
// Query Parameters
// ---------------------------------------------------------------------------

/// Query parameters for history endpoint.
#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    /// Number of most recent snapshots to return.
    #[serde(default = "default_last_n")]
    pub last_n: usize,
}

fn default_last_n() -> usize {
    30
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/resources — Get current resource snapshot.
pub(crate) async fn handle_resource_snapshot(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let snapshot = state.kernel.infra.resource_snapshot();
    serde_json::to_value(&snapshot)
        .map(Json)
        .map_err(|e| AppError::Internal(format!("failed to serialize snapshot: {e}")))
}

/// GET /api/resources/history — Get historical snapshots.
pub(crate) async fn handle_resource_history(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HistoryQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let snapshots = state.kernel.infra.resource_history(params.last_n);
    let count = snapshots.len();
    serde_json::to_value(serde_json::json!({
        "snapshots": snapshots,
        "count": count,
    }))
    .map(Json)
    .map_err(|e| AppError::Internal(format!("failed to serialize history: {e}")))
}

/// GET /api/resources/overload — Check if system is overloaded.
pub(crate) async fn handle_resource_overload(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let overloaded = state.kernel.infra.is_overloaded();
    // Live thresholds — `infra.config()` is a boot snapshot; hot-reload
    // (PATCH /api/config) updates the ResourceMonitor directly.
    let threshold = state.kernel.infra.resource_monitor().overload_threshold();

    serde_json::to_value(serde_json::json!({
        "overloaded": overloaded,
        "threshold": {
            "cpu_percent": threshold.cpu_percent,
            "memory_percent": threshold.memory_percent,
            "load_avg": threshold.load_avg,
        },
    }))
    .map(Json)
    .map_err(|e| AppError::Internal(format!("failed to serialize overload status: {e}")))
}
