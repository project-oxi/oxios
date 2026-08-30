//! API routes for the oxiline integration (first-party app module).
//!
//! Status + live enable/disable (no restart), mirroring the memo/email setup
//! pattern. Only compiled with the `timeline` cargo feature; when the feature
//! is off the routes are absent (404) and the web UI card tolerates that and
//! hides itself. oxios is always a *co-client* of the store.

use std::sync::Arc;

use axum::{Json, extract::State};
use serde::Deserialize;
use serde_json::json;

use crate::api::error::AppError;
use crate::api::server::AppState;

/// `GET /api/timeline/status` — oxiline connection status.
pub(crate) async fn handle_timeline_status(state: State<Arc<AppState>>) -> Json<serde_json::Value> {
    let cfg = state.config.read();
    let connected = state.kernel.timeline.read().is_some();
    Json(json!({
        "enabled": cfg.timeline.enabled,
        "connected": connected,
        "db_path": cfg.timeline.db_path,
    }))
}

/// Request body for `POST /api/timeline/enable`.
#[derive(Debug, Deserialize, Default)]
pub struct TimelineEnableRequest {
    /// Optional explicit oxiline database path. Empty = oxiline's default
    /// location (`oxiline_core::paths::db_path`, honoring `OXILINE_DB_PATH`).
    #[serde(default)]
    pub db_path: String,
}

/// `POST /api/timeline/enable` — open the store and swap the facade in live.
pub(crate) async fn handle_timeline_enable(
    state: State<Arc<AppState>>,
    Json(body): Json<TimelineEnableRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let db_path = if body.db_path.is_empty() {
        None
    } else {
        Some(std::path::PathBuf::from(&body.db_path))
    };

    let api = oxios_kernel::TimelineApi::open(db_path.as_deref())
        .map_err(|e| AppError::BadRequest(format!("Failed to open oxiline db: {e}")))?;

    // Live swap — agents using the `timeline` tool see this immediately.
    *state.kernel.timeline.write() = Some(api);

    {
        let mut cfg = state.config.write();
        cfg.timeline.enabled = true;
        cfg.timeline.db_path = body.db_path.clone();
    }

    let config_path = state.config_path.clone();
    if config_path.exists() {
        let _ = upsert_timeline_section_in_config(&config_path, &body.db_path, true);
    }

    tracing::info!("oxiline module enabled live (timeline co-client)");
    Ok(Json(json!({
        "status": "ok",
        "message": "oxiline connected.",
        "connected": true,
    })))
}

/// `POST /api/timeline/disable` — drop the live facade (data untouched).
pub(crate) async fn handle_timeline_disable(
    state: State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    *state.kernel.timeline.write() = None;

    let db_path = {
        let mut cfg = state.config.write();
        cfg.timeline.enabled = false;
        cfg.timeline.db_path.clone()
    };

    let config_path = state.config_path.clone();
    if config_path.exists() {
        let _ = upsert_timeline_section_in_config(&config_path, &db_path, false);
    }

    tracing::info!("oxiline module disabled live");
    Ok(Json(json!({
        "status": "ok",
        "message": "oxiline disconnected.",
        "connected": false,
    })))
}

/// Insert or replace the `[timeline]` section in `config.toml` (best-effort).
fn upsert_timeline_section_in_config(
    config_path: &std::path::Path,
    db_path: &str,
    enabled: bool,
) -> std::io::Result<()> {
    let content = std::fs::read_to_string(config_path)?;
    let new_section = format!(
        "# oxiline integration (managed by web UI)\n[timeline]\nenabled = {}\ndb_path = \"{}\"\n",
        enabled, db_path
    );

    if let Some(pos) = content.find("[timeline]") {
        let line_start = content[..pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let rest = &content[pos..];
        let section_end = rest[1..]
            .find("\n[")
            .map(|p| pos + 1 + p + 1)
            .unwrap_or(content.len());
        let mut result = String::with_capacity(content.len());
        result.push_str(&content[..line_start]);
        result.push_str(&new_section);
        result.push_str(&content[section_end..]);
        std::fs::write(config_path, result)?;
    } else {
        let mut result = content;
        if !result.ends_with('\n') {
            result.push('\n');
        }
        result.push('\n');
        result.push_str(&new_section);
        std::fs::write(config_path, result)?;
    }
    Ok(())
}
