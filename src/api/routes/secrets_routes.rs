//! Secrets management API — `/api/secrets`.
//!
//! Stores non-config secrets (telegram token, email password, API keys) in
//! `~/.oxicode/auth.json` via `CredentialStore`, never in `config.toml` plaintext.
//! Provider keys (anthropic, openai, google) share the same auth-store path
//! as the Engine API but are exposed here for unified management.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;

use crate::api::error::AppError;
use crate::api::server::AppState;
use oxios_kernel::credential::{CredentialSource, CredentialStore};

// ── Known secret registry ───────────────────────────────────────────────────

/// Metadata for a known secret key: env var name + whether it's a provider key.
struct SecretMeta {
    /// Env var to check (e.g. `TELEGRAM_BOT_TOKEN`).
    env_var: &'static str,
    /// True for LLM provider keys (resolved via `CredentialStore::resolve`).
    is_provider: bool,
}

/// All secret keys the Web UI can manage. Provider keys reuse the Engine API's
/// `set_api_key` path; non-provider keys use `CredentialStore::store` directly.
const KNOWN_SECRETS: &[(&str, SecretMeta)] = &[
    (
        "telegram_bot_token",
        SecretMeta {
            env_var: "TELEGRAM_BOT_TOKEN",
            is_provider: false,
        },
    ),
    (
        "email_smtp_password",
        SecretMeta {
            env_var: "EMAIL_SMTP_PASSWORD",
            is_provider: false,
        },
    ),
    (
        "oxios_api_key",
        SecretMeta {
            env_var: "OXIOS_API_KEY",
            is_provider: false,
        },
    ),
    (
        "clawhub_api_key",
        SecretMeta {
            env_var: "CLAWHUB_API_KEY",
            is_provider: false,
        },
    ),
    (
        "anthropic",
        SecretMeta {
            env_var: "ANTHROPIC_API_KEY",
            is_provider: true,
        },
    ),
    (
        "openai",
        SecretMeta {
            env_var: "OPENAI_API_KEY",
            is_provider: true,
        },
    ),
    (
        "google",
        SecretMeta {
            env_var: "GOOGLE_API_KEY",
            is_provider: true,
        },
    ),
];

fn lookup_meta(key: &str) -> Option<&'static SecretMeta> {
    KNOWN_SECRETS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, m)| m)
}

// ── Response types ──────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
pub(crate) struct SecretInfo {
    key: String,
    has_value: bool,
    source: String,
    preview: String,
}

/// Mask a secret value: show first 3 chars + asterisks, or `****` if shorter.
fn mask(value: &str) -> String {
    let prefix: String = value.chars().take(3).collect();
    if value.len() > 3 {
        let stars = "*".repeat(value.len() - prefix.len());
        format!("{prefix}{stars}")
    } else {
        "****".to_string()
    }
}

fn source_label(s: &CredentialSource) -> &'static str {
    match s {
        CredentialSource::Config => "config",
        CredentialSource::OxicodeAuthStore => "auth_store",
        CredentialSource::EnvVar => "env",
        CredentialSource::FoundationKeychain => "foundation_keychain",
    }
}

/// Resolve a single secret's status (no raw value in the response).
fn resolve_secret_status(key: &str) -> SecretInfo {
    let meta = lookup_meta(key);
    let env_var = meta.map(|m| m.env_var).unwrap_or("");
    let is_provider = meta.map(|m| m.is_provider).unwrap_or(false);

    let resolved = if is_provider {
        CredentialStore::resolve(key, None)
    } else {
        CredentialStore::resolve_secret(key, env_var)
    };

    match resolved {
        Some((val, src)) => SecretInfo {
            key: key.to_string(),
            has_value: true,
            source: source_label(&src).to_string(),
            preview: mask(&val),
        },
        None => SecretInfo {
            key: key.to_string(),
            has_value: false,
            source: "none".to_string(),
            preview: String::new(),
        },
    }
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// GET /api/secrets — List all known secrets with masked status.
pub(crate) async fn handle_secrets_list(_state: State<Arc<AppState>>) -> Json<Vec<SecretInfo>> {
    let infos: Vec<SecretInfo> = KNOWN_SECRETS
        .iter()
        .map(|(key, _)| resolve_secret_status(key))
        .collect();
    Json(infos)
}

/// Request body for PUT /api/secrets/{key}.
#[derive(Debug, Deserialize)]
pub(crate) struct SetSecretBody {
    pub value: String,
}

/// PUT /api/secrets/{key} — Store a secret value.
pub(crate) async fn handle_secret_set(
    state: State<Arc<AppState>>,
    Path(key): Path<String>,
    Json(body): Json<SetSecretBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let meta = lookup_meta(&key)
        .ok_or_else(|| AppError::BadRequest(format!("unknown secret key: {key}")))?;

    if body.value.is_empty() {
        return Err(AppError::BadRequest("value must not be empty".into()));
    }

    if meta.is_provider {
        // Provider keys go through the engine so the running engine picks
        // them up immediately (not just persisted for next boot).
        state
            .kernel
            .engine
            .set_api_key(&key, &body.value)
            .map_err(|e| AppError::Internal(e.to_string()))?;
    } else {
        CredentialStore::store(&key, &body.value).map_err(|e| AppError::Internal(e.to_string()))?;
    }

    tracing::info!(key = %key, "Secret stored via /api/secrets");
    Ok(Json(serde_json::json!({ "ok": true, "key": key })))
}

/// DELETE /api/secrets/{key} — Remove a secret from the auth store.
pub(crate) async fn handle_secret_delete(
    state: State<Arc<AppState>>,
    Path(key): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let meta = lookup_meta(&key)
        .ok_or_else(|| AppError::BadRequest(format!("unknown secret key: {key}")))?;

    CredentialStore::delete(&key).map_err(|e| AppError::Internal(e.to_string()))?;

    // For provider keys, also clear from the running engine so the old
    // credential is dropped immediately (mirrors the SET path's asymmetry).
    if meta.is_provider {
        state
            .kernel
            .engine
            .clear_api_key(&key)
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    tracing::info!(key = %key, "Secret deleted via /api/secrets");
    Ok(Json(serde_json::json!({ "ok": true, "key": key })))
}

/// GET /api/secrets/{key}/source — Check where a secret is resolved from.
pub(crate) async fn handle_secret_source(
    _state: State<Arc<AppState>>,
    Path(key): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    if lookup_meta(&key).is_none() {
        return Err(AppError::BadRequest(format!("unknown secret key: {key}")));
    }
    let info = resolve_secret_status(&key);
    Ok(Json(serde_json::json!({
        "key": info.key,
        "has_value": info.has_value,
        "source": info.source,
    })))
}
