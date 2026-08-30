//! API routes for the oximemo integration (first-party app module).
//!
//! Status + live enable/disable (no restart), mirroring the email setup
//! pattern. Only compiled with the `memo` cargo feature; when the feature is
//! off the routes are absent (404) and the web UI card tolerates that and
//! hides itself. oxios is always a *co-client* of the vault — disable never
//! touches the user's data.

use std::sync::Arc;

use axum::{Json, extract::State};
use serde::Deserialize;
use serde_json::json;

use crate::api::error::AppError;
use crate::api::server::AppState;

/// `GET /api/memo/status` — oximemo connection status.
///
/// `connected` reflects the live runtime slot (the facade may be swapped in
/// after boot via `POST /api/memo/enable`); `enabled` is the persisted config.
pub(crate) async fn handle_memo_status(state: State<Arc<AppState>>) -> Json<serde_json::Value> {
    let cfg = state.config.read();
    let connected = state.kernel.memo.read().is_some();
    Json(json!({
        "enabled": cfg.memo.enabled,
        "connected": connected,
        "vault_path": cfg.memo.vault_path,
    }))
}

/// Request body for `POST /api/memo/enable`.
#[derive(Debug, Deserialize, Default)]
pub struct MemoEnableRequest {
    /// Optional explicit vault path. Empty = oximemo's default location,
    /// resolved by `oximemo_core::Paths`.
    #[serde(default)]
    pub vault_path: String,
}

/// `POST /api/memo/enable` — open the vault and swap the facade in live.
///
/// Validates the vault opens before persisting; on success the already-
/// registered `memo` agent tool picks the facade up immediately.
pub(crate) async fn handle_memo_enable(
    state: State<Arc<AppState>>,
    Json(body): Json<MemoEnableRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let vault_path = if body.vault_path.is_empty() {
        None
    } else {
        Some(std::path::PathBuf::from(&body.vault_path))
    };

    let api = oxios_kernel::MemoApi::open(
        vault_path.as_deref(),
        Some(state.kernel.infra.event_bus_clone()),
    )
    .map_err(|e| AppError::BadRequest(format!("Failed to open oximemo vault: {e}")))?;

    // Live swap — agents using the `memo` tool see this immediately.
    *state.kernel.memo.write() = Some(api);

    // Keep in-memory config consistent with the slot.
    {
        let mut cfg = state.config.write();
        cfg.memo.enabled = true;
        cfg.memo.vault_path = body.vault_path.clone();
    }

    // Persist to config.toml (best-effort; the in-memory change already took effect).
    let config_path = state.config_path.clone();
    if config_path.exists() {
        let _ = upsert_memo_section_in_config(&config_path, &body.vault_path, true);
    }

    tracing::info!("oximemo module enabled live (vault co-client)");
    Ok(Json(json!({
        "status": "ok",
        "message": "oximemo connected.",
        "connected": true,
    })))
}

/// `POST /api/memo/disable` — drop the live facade.
///
/// Data is never touched — oxios is a co-client, not the owner. The `memo`
/// tool starts erroring "not connected" on its next call.
pub(crate) async fn handle_memo_disable(
    state: State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    *state.kernel.memo.write() = None;

    let vault_path = {
        let mut cfg = state.config.write();
        cfg.memo.enabled = false;
        cfg.memo.vault_path.clone()
    };

    let config_path = state.config_path.clone();
    if config_path.exists() {
        let _ = upsert_memo_section_in_config(&config_path, &vault_path, false);
    }

    tracing::info!("oximemo module disabled live");
    Ok(Json(json!({
        "status": "ok",
        "message": "oximemo disconnected.",
        "connected": false,
    })))
}

/// Insert or replace the `[memo]` section in `config.toml`.
///
/// Best-effort persistence mirroring [`email_routes::upsert_email_section_in_config`]:
/// matches the exact `[memo]` header (not e.g. a hypothetical `[memos]`) and
/// replaces through the next section or EOF, appending if absent.
fn upsert_memo_section_in_config(
    config_path: &std::path::Path,
    vault_path: &str,
    enabled: bool,
) -> std::io::Result<()> {
    let content = std::fs::read_to_string(config_path)?;
    let new_section = format!(
        "# oximemo integration (managed by web UI)\n[memo]\nenabled = {}\nvault_path = \"{}\"\n",
        enabled, vault_path
    );

    if let Some(pos) = content.find("[memo]") {
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
