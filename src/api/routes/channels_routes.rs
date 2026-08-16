//! Runtime channel control — `/api/channels`.
//!
//! Lists channel availability/state and drives runtime connect/disconnect
//! for the Telegram channel (Web UI instant connect). `channels.enabled` in
//! config.toml stays the source of truth for boot activation; connect
//! persists it *after* a successful runtime start so a rejected token never
//! leaves the channel enabled.

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use crate::api::error::AppError;
use crate::api::server::AppState;
use oxios_kernel::credential::{CredentialSource, CredentialStore, TELEGRAM_TOKEN_STORE_KEY};

/// Only channels that make sense to control at runtime. The CLI channel is
/// interactive (stdin loop) and must not be started from an HTTP call.
const RUNTIME_CONTROLLABLE: &[&str] = &["telegram"];

// ── Pure helpers ─────────────────────────────────────────────────────────────

/// Add `name` to the enabled list if absent (dedup).
pub(crate) fn upsert_enabled(enabled: &mut Vec<String>, name: &str) {
    if !enabled.iter().any(|c| c == name) {
        enabled.push(name.to_string());
    }
}

/// Remove `name` from the enabled list; no-op when absent.
pub(crate) fn remove_enabled(enabled: &mut Vec<String>, name: &str) {
    enabled.retain(|c| c != name);
}

/// Map a credential source to the label the Web UI Secrets section uses.
pub(crate) fn token_source_label(source: Option<CredentialSource>) -> Option<&'static str> {
    match source {
        Some(CredentialSource::EnvVar) => Some("env"),
        Some(CredentialSource::OxicodeAuthStore) => Some("auth_store"),
        Some(CredentialSource::Config) => Some("config"),
        None => None,
    }
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// GET /api/channels — availability, config state, runtime state.
pub(crate) async fn handle_channels_list(
    state: State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (enabled, telegram_env) = {
        let config = state.config.read();
        (
            config.channels.enabled.clone(),
            config.channels.telegram.bot_token_env.clone(),
        )
    };
    let running = state.gateway.channel_names().await;

    let mut channels = Vec::new();
    for plugin in crate::build_channel_plugins() {
        let name = plugin.name().to_string();
        let token_source: Option<&'static str> = if name == "telegram" {
            #[cfg(feature = "telegram")]
            {
                token_source_label(
                    CredentialStore::resolve_secret(TELEGRAM_TOKEN_STORE_KEY, &telegram_env)
                        .map(|(_, src)| src),
                )
            }
            #[cfg(not(feature = "telegram"))]
            {
                None
            }
        } else {
            None
        };
        channels.push(json!({
            "name": name,
            "available": true,
            "enabled": enabled.contains(&name),
            "running": running.contains(&name),
            "token_source": token_source,
        }));
    }

    Ok(Json(json!({ "channels": channels })))
}

/// Body for POST /api/channels/{name}/connect.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ConnectBody {
    /// Optional bot token. When present and non-empty it is stored in the
    /// credential store before connecting (single-call connect flow).
    pub token: Option<String>,
}

/// POST /api/channels/{name}/connect — store optional token, start the
/// channel now, persist `channels.enabled` after success.
pub(crate) async fn handle_channel_connect(
    state: State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<ConnectBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !RUNTIME_CONTROLLABLE.contains(&name.as_str()) {
        return Err(AppError::BadRequest(format!(
            "channel '{name}' cannot be connected at runtime (available: telegram)"
        )));
    }
    connect_telegram(&state, body).await
}

/// POST /api/channels/{name}/disconnect — stop the channel now and persist
/// its removal from `channels.enabled`. Idempotent.
pub(crate) async fn handle_channel_disconnect(
    state: State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !RUNTIME_CONTROLLABLE.contains(&name.as_str()) {
        return Err(AppError::BadRequest(format!(
            "channel '{name}' cannot be disconnected at runtime (available: telegram)"
        )));
    }

    // Idempotent: unregister returns Ok even when not registered.
    state
        .gateway
        .unregister(&name)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    persist_enabled_change(&state, |enabled| remove_enabled(enabled, &name)).await?;

    tracing::info!(channel = %name, "Channel disconnected via /api/channels");
    Ok(Json(json!({ "status": "disconnected" })))
}

// ── Telegram connect (feature-gated) ─────────────────────────────────────────

#[cfg(not(feature = "telegram"))]
async fn connect_telegram(
    _state: &State<Arc<AppState>>,
    _body: ConnectBody,
) -> Result<Json<serde_json::Value>, AppError> {
    Err(AppError::BadRequest(
        "the telegram channel is not compiled into this build".into(),
    ))
}

#[cfg(feature = "telegram")]
async fn connect_telegram(
    state: &State<Arc<AppState>>,
    body: ConnectBody,
) -> Result<Json<serde_json::Value>, AppError> {
    use oxios_gateway::plugin::{ChannelContext, ChannelPlugin};
    use oxios_kernel::OxiosConfig;

    // 1. Optional token: persist first so a later reconnect (boot or
    //    reconnect button) resolves the same credential.
    if let Some(token) = body.token.as_deref().filter(|t| !t.is_empty()) {
        CredentialStore::store(TELEGRAM_TOKEN_STORE_KEY, token)
            .map_err(|e| AppError::Internal(format!("failed to store bot token: {e}")))?;
    }

    // 2. Token must resolve from store or env before we touch the channel.
    let telegram_env = state.config.read().channels.telegram.bot_token_env.clone();
    if CredentialStore::resolve_secret(TELEGRAM_TOKEN_STORE_KEY, &telegram_env).is_none() {
        return Err(AppError::BadRequest(
            "no telegram bot token found — provide a token or set it in Settings → Secrets".into(),
        ));
    }

    // 3. Reconnect semantics: stop any running instance so a fresh setup
    //    picks up current config + credential.
    if state
        .gateway
        .channel_names()
        .await
        .contains(&"telegram".to_string())
    {
        state
            .gateway
            .unregister("telegram")
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    // 4. Build from a snapshot of the live config.
    let config: OxiosConfig = state.config.read().clone();
    let ctx = ChannelContext {
        config: std::sync::Arc::new(parking_lot::RwLock::new(config)),
        config_path: state.config_path.clone(),
    };
    let bundle = crate::channels::telegram::TelegramPlugin::new()
        .setup(ctx)
        .await
        .map_err(|e| AppError::BadRequest(format!("telegram connect failed: {e:#}")))?;

    if !bundle.tasks.is_empty() {
        tracing::warn!(
            tasks = bundle.tasks.len(),
            "channel plugin returned background tasks; /api/channels does not supervise them"
        );
    }

    // 5. Register → the polling loop starts now.
    state
        .gateway
        .register(bundle.channel)
        .await
        .map_err(|e| AppError::Internal(format!("failed to register telegram channel: {e}")))?;

    // 6. Persist enabled AFTER a successful start.
    persist_enabled_change(state, |enabled| upsert_enabled(enabled, "telegram")).await?;

    let info = state.gateway.channel_status("telegram").await;
    tracing::info!("Telegram channel connected via /api/channels");
    Ok(Json(json!({ "status": "connected", "info": info })))
}

// ── Persistence ──────────────────────────────────────────────────────────────

/// Apply `mutate` to `channels.enabled` on the live config, then persist the
/// whole config to disk (same write path as PUT /api/config).
async fn persist_enabled_change(
    state: &State<Arc<AppState>>,
    mutate: impl FnOnce(&mut Vec<String>),
) -> Result<(), AppError> {
    let updated = {
        let mut config = state.config.write();
        mutate(&mut config.channels.enabled);
        config.clone()
    };

    let content = toml::to_string_pretty(&updated)
        .map_err(|e| AppError::Internal(format!("config serialization failed: {e}")))?;
    tokio::fs::write(&state.config_path, content)
        .await
        .map_err(|e| AppError::Internal(format!("failed to persist config: {e}")))?;

    tracing::info!(path = %state.config_path.display(), "channels.enabled persisted");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_enabled_dedups() {
        let mut enabled = vec!["cli".to_string()];
        upsert_enabled(&mut enabled, "telegram");
        assert_eq!(enabled, vec!["cli", "telegram"]);
        upsert_enabled(&mut enabled, "telegram");
        assert_eq!(enabled.len(), 2, "must not duplicate: {enabled:?}");
    }

    #[test]
    fn remove_enabled_is_noop_when_absent() {
        let mut enabled = vec!["cli".to_string()];
        remove_enabled(&mut enabled, "telegram");
        assert_eq!(enabled, vec!["cli"]);
        remove_enabled(&mut enabled, "cli");
        assert!(enabled.is_empty());
    }

    #[test]
    fn token_source_label_mapping() {
        assert_eq!(
            token_source_label(Some(CredentialSource::EnvVar)),
            Some("env")
        );
        assert_eq!(
            token_source_label(Some(CredentialSource::OxicodeAuthStore)),
            Some("auth_store")
        );
        assert_eq!(token_source_label(None), None);
    }
}
