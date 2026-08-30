//! API routes for email management.
//!
//! Provides endpoints for SMTP setup, viewing sent history,
//! and browsing templates. Actual email sending is done by
//! agents via the `send_email` tool.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::Value;

use crate::api::error::AppError;
use crate::api::server::AppState;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// Query parameters for email history listing.
#[derive(Debug, Deserialize, Default)]
pub struct EmailHistoryParams {
    /// Maximum number of records to return.
    #[serde(default = "default_history_limit")]
    pub limit: usize,
}

fn default_history_limit() -> usize {
    100
}

/// A single sent email record returned by the API.
#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SentEmailResponse {
    /// Unique ID.
    pub id: String,
    /// Timestamp (RFC 3339).
    pub sent_at: String,
    /// Email subject.
    pub subject: String,
    /// Recipient.
    pub to: String,
    /// Template used (if any).
    pub template_used: Option<String>,
    /// SMTP message ID.
    pub message_id: String,
    /// First 500 chars of HTML for preview.
    pub html_preview: String,
    /// Full HTML body (원문). May be large.
    pub html_full: String,
    /// Plain text fallback if provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
    /// Associated cron job name.
    pub cron_job: Option<String>,
}

/// Email setup status response.
#[derive(Debug, Serialize)]
pub struct EmailStatusResponse {
    /// Whether email is configured and ready.
    pub configured: bool,
    /// Configured email address.
    pub email: Option<String>,
    /// SMTP provider.
    pub provider: Option<String>,
    /// Number of templates available.
    pub template_count: usize,
    /// Number of emails sent (total).
    pub total_sent: usize,
}

/// Email test request. In v1, the `to` field is accepted but ignored
/// (test emails always go to `my_email`).
#[derive(Debug, Deserialize)]
pub struct EmailTestRequest {
    /// Optional custom recipient (ignored in v1).
    #[serde(default)]
    pub to: Option<String>,
}

/// Template info returned by the API.
#[derive(Debug, Serialize)]
pub struct TemplateResponse {
    /// Template name.
    pub name: String,
    /// First 200 chars of template content.
    pub preview: String,
    /// Template file size in bytes.
    pub size: usize,
}

/// Email config update request (for setup).
#[derive(Debug, Deserialize)]
pub struct EmailConfigRequest {
    /// User's email address.
    pub my_email: String,
    /// SMTP provider.
    #[serde(default = "default_provider")]
    pub provider: String,
    /// SMTP host (for custom).
    #[serde(default)]
    pub host: Option<String>,
    /// SMTP port (for custom).
    #[serde(default)]
    pub port: Option<u16>,
    /// SMTP auth username (defaults to my_email).
    #[serde(default)]
    pub user: Option<String>,
    /// SMTP password / app password.
    pub password: String,
}

fn default_provider() -> String {
    "resend".to_string()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the email API, returning 503 if unavailable.
macro_rules! email_api {
    ($state:expr) => {
        $state
            .kernel
            .email
            .read()
            .as_ref()
            .ok_or_else(|| {
                AppError::ServiceUnavailable(
                    "Email subsystem not available. Set up email in Settings → Email.".into(),
                )
            })?
            .clone()
    };
}

/// Load sent records from the state store directory.
///
/// Reads raw JSON files (which include `html_full` and `body_text`),
/// then strips large fields when `full=false` (for list view) to reduce payload.
fn load_sent_records(
    state_store: &oxios_kernel::state_store::StateStore,
    limit: usize,
    full: bool,
) -> Vec<serde_json::Value> {
    let sent_dir = state_store.base_path.join("email_sent");
    if !sent_dir.exists() {
        return Vec::new();
    }

    let mut records = Vec::new();
    let Ok(entries) = std::fs::read_dir(&sent_dir) else {
        return records;
    };

    for entry in entries.flatten() {
        if entry.path().extension().is_some_and(|ext| ext == "json") {
            match std::fs::read_to_string(entry.path()) {
                Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(mut val) => {
                        if !full {
                            // Strip heavy fields for list view
                            if let Some(obj) = val.as_object_mut() {
                                obj.remove("html_full");
                                obj.remove("body_text");
                            }
                        }
                        records.push(val);
                    }
                    Err(e) => {
                        tracing::warn!(
                            file = ?entry.path(),
                            error = %e,
                            "Skipping corrupted email record"
                        );
                    }
                },
                Err(e) => {
                    tracing::warn!(
                        file = ?entry.path(),
                        error = %e,
                        "Failed to read email record"
                    );
                }
            }
        }
    }

    // Sort by sent_at descending (newest first)
    records.sort_by(|a, b| {
        let sa = a.get("sent_at").and_then(|v| v.as_str()).unwrap_or("");
        let sb = b.get("sent_at").and_then(|v| v.as_str()).unwrap_or("");
        sb.cmp(sa)
    });
    records.truncate(limit);
    records
}

/// Count sent email records by counting `.json` files in the `email_sent/`
/// directory. Does NOT read file contents — O(entries) time, O(1) memory,
/// safe for directories with tens of thousands of records.
fn count_sent_records(state_store: &oxios_kernel::state_store::StateStore) -> usize {
    let sent_dir = state_store.base_path.join("email_sent");
    let Ok(entries) = std::fs::read_dir(&sent_dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .count()
}

/// Load a single sent record by exact ID match.
fn load_sent_record(
    state_store: &oxios_kernel::state_store::StateStore,
    id: &str,
) -> Option<serde_json::Value> {
    let sent_dir = state_store.base_path.join("email_sent");
    if !sent_dir.exists() {
        return None;
    }

    let Ok(entries) = std::fs::read_dir(&sent_dir) else {
        return None;
    };

    for entry in entries.flatten() {
        if entry.path().extension().is_some_and(|ext| ext == "json")
            && let Ok(content) = std::fs::read_to_string(entry.path())
            && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
            && val.get("id").and_then(|v| v.as_str()) == Some(id)
        {
            return Some(val);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/email/status — Email subsystem status.
pub(crate) async fn handle_email_status(
    state: State<Arc<AppState>>,
) -> Result<Json<EmailStatusResponse>, AppError> {
    let provider_name = match state.config.read().email.provider {
        oxios_kernel::email::SmtpProvider::Resend => "resend",
        oxios_kernel::email::SmtpProvider::Gmail => "gmail",
        oxios_kernel::email::SmtpProvider::Icloud => "icloud",
        oxios_kernel::email::SmtpProvider::Fastmail => "fastmail",
        oxios_kernel::email::SmtpProvider::Custom => "custom",
    }
    .to_string();

    let email_guard = state.kernel.email.read();
    let configured = email_guard.is_some();
    let (email, provider, template_count) = if let Some(api) = &*email_guard {
        let templates = api.list_templates().unwrap_or_default();
        (
            Some(api.default_to().to_string()),
            Some(provider_name),
            templates.len(),
        )
    } else {
        (None, None, 0)
    };

    let total_sent = count_sent_records(state.kernel.state.store());

    Ok(Json(EmailStatusResponse {
        configured,
        email,
        provider,
        template_count,
        total_sent,
    }))
}

/// GET /api/email/history — List sent emails.
pub(crate) async fn handle_email_history(
    state: State<Arc<AppState>>,
    Query(params): Query<EmailHistoryParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Email subsystem doesn't need to be configured to view history
    let limit = params.limit.min(500);
    let records = load_sent_records(state.kernel.state.store(), limit, false);

    Ok(Json(serde_json::json!({
        "emails": records,
        "total": records.len(),
        "limit": limit,
    })))
}

/// GET /api/email/history/{id} — Get a specific sent email.
pub(crate) async fn handle_email_history_detail(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let record = load_sent_record(state.kernel.state.store(), &id)
        .ok_or_else(|| AppError::NotFound(format!("Email record '{id}' not found")))?;

    Ok(Json(record))
}

/// GET /api/email/templates — List email templates.
pub(crate) async fn handle_email_templates(
    state: State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let api = email_api!(state);
    let names = api
        .list_templates()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut templates = Vec::new();
    for name in &names {
        match api.load_template(name) {
            Ok(content) => templates.push(TemplateResponse {
                name: name.clone(),
                preview: content.chars().take(200).collect(),
                size: content.len(),
            }),
            Err(e) => {
                tracing::warn!(template = %name, error = %e, "Failed to load template");
            }
        }
    }

    Ok(Json(serde_json::json!({
        "templates": templates,
    })))
}

/// GET /api/email/templates/{name} — Get a specific template.
pub(crate) async fn handle_email_template_get(
    state: State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let api = email_api!(state);
    let content = api
        .load_template(&name)
        .map_err(|e| AppError::NotFound(e.to_string()))?;

    Ok(Json(serde_json::json!({
        "name": name,
        "content": content,
        "size": content.len(),
    })))
}

/// POST /api/email/test — Send a test email.
///
/// Body (optional): `{"to": "custom@example.com"}`
/// Note: In v1, the `to` field is ignored — the test email always goes to
/// `my_email` (the user's own address). External recipients are a v2 feature.
pub(crate) async fn handle_email_test(
    state: State<Arc<AppState>>,
    body: Option<Json<EmailTestRequest>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let api = email_api!(state);
    let default_to = api.default_to().to_string();
    api.test_connection()
        .await
        .map_err(|e| AppError::Internal(format!("SMTP test failed: {e}")))?;

    let requested_to = body.and_then(|Json(b)| b.to);
    let recipient = requested_to.unwrap_or_else(|| default_to.clone());
    let override_note = if recipient != default_to {
        format!(" (requested '{recipient}' ignored in v1, sent to '{default_to}')")
    } else {
        String::new()
    };

    Ok(Json(serde_json::json!({
        "status": "ok",
        "message": format!("Test email sent successfully{override_note}"),
        "to": default_to,
    })))
}

/// POST /api/email/setup — Configure email from web UI (live, no restart).
pub(crate) async fn handle_email_setup(
    state: State<Arc<AppState>>,
    Json(body): Json<EmailConfigRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Validate we can connect with these credentials
    let provider = match body.provider.as_str() {
        "resend" => oxios_kernel::email::SmtpProvider::Resend,
        "gmail" => oxios_kernel::email::SmtpProvider::Gmail,
        "icloud" => oxios_kernel::email::SmtpProvider::Icloud,
        "fastmail" => oxios_kernel::email::SmtpProvider::Fastmail,
        "custom" => oxios_kernel::email::SmtpProvider::Custom,
        _ => {
            return Err(AppError::BadRequest(format!(
                "Unknown provider: {}",
                body.provider
            )));
        }
    };

    let config = oxios_kernel::config::EmailConfig {
        enabled: true,
        my_email: body.my_email.clone(),
        provider,
        host: body.host.unwrap_or_default(),
        port: body.port.unwrap_or(0),
        tls: None,
        user: body.user.unwrap_or_default(),
        secret_ref: "email_smtp".to_string(),
        rate_limit_per_hour: 10,
    };

    let smtp = oxios_kernel::SmtpClient::from_config(&config, &body.password)
        .map_err(|e| AppError::BadRequest(format!("SMTP config error: {e}")))?;

    // Test the connection before saving
    smtp.test_connection()
        .await
        .map_err(|e| AppError::BadRequest(format!("SMTP test failed: {e}")))?;

    // Save password to the credential store
    let token = oxicode_sdk::TokenBundle {
        access_token: body.password,
        refresh_token: None,
        token_type: "Bearer".to_string(),
        obtained_at: chrono::Utc::now(),
        expires_in: 0,
        scope: None,
    };
    oxicode_sdk::save_token("email_smtp", &token)
        .map_err(|e| AppError::Internal(format!("Failed to save credentials: {e}")))?;

    // Persist to config.toml (upsert — replaces existing [email] section)
    let config_path = state.config_path.clone();
    if config_path.exists() {
        let _ = upsert_email_section_in_config(&config_path, &config);
    }

    // ── Live kernel update (no restart needed) ───────────────────
    // Build a new EmailApi and swap it into the shared RwLock slot.
    // The already-registered EmailTool picks this up immediately.
    let workspace = std::path::PathBuf::from(&state.config.read().kernel.workspace);
    let template_dir = workspace.join("email_templates");
    let _ = std::fs::create_dir_all(&template_dir);

    let new_api = oxios_kernel::EmailApi::new(
        smtp,
        template_dir,
        state.kernel.state.store().clone(),
        Some(state.kernel.infra.event_bus_clone()),
        config.rate_limit_per_hour,
    );

    // Swap into the shared slot — agents using send_email see this instantly
    *state.kernel.email.write() = Some(new_api);

    // Update in-memory config so GET /api/email/status stays consistent
    state.config.write().email = config;

    tracing::info!(email = %body.my_email, "Email configured live (no restart)");

    Ok(Json(serde_json::json!({
        "status": "ok",
        "message": "Email configured successfully.",
        "email": body.my_email,
    })))
}

/// Insert or replace the [email] section in config.toml.
fn upsert_email_section_in_config(
    config_path: &std::path::Path,
    config: &oxios_kernel::config::EmailConfig,
) -> std::io::Result<()> {
    let content = std::fs::read_to_string(config_path)?;
    let provider_str = match config.provider {
        oxios_kernel::email::SmtpProvider::Resend => "resend",
        oxios_kernel::email::SmtpProvider::Gmail => "gmail",
        oxios_kernel::email::SmtpProvider::Icloud => "icloud",
        oxios_kernel::email::SmtpProvider::Fastmail => "fastmail",
        oxios_kernel::email::SmtpProvider::Custom => "custom",
    };
    let new_section = format!(
        "# Email (configured by web UI)\n[email]\nenabled = true\nmy_email = \"{}\"\nprovider = \"{}\"\n",
        config.my_email, provider_str
    );

    if let Some(email_pos) = content.find("[email]") {
        // Replace existing section — from start of [email] line to next section or EOF
        let line_start = content[..email_pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let rest = &content[email_pos..];
        let section_end = rest[1..]
            .find("\n[")
            .map(|p| email_pos + 1 + p + 1)
            .unwrap_or(content.len());
        let mut result = String::with_capacity(content.len());
        result.push_str(&content[..line_start]);
        result.push_str(&new_section);
        result.push_str(&content[section_end..]);
        std::fs::write(config_path, result)?;
    } else {
        // Append new section
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a temp StateStore with N sent email records.
    fn make_store_with_records(
        records: &[(&str, &str, &str)],
    ) -> (tempfile::TempDir, oxios_kernel::state_store::StateStore) {
        let tmp = tempfile::tempdir().unwrap();
        let sent_dir = tmp.path().join("email_sent");
        fs::create_dir_all(&sent_dir).unwrap();
        for (id, subject, html) in records {
            let record = serde_json::json!({
                "id": id,
                "sent_at": "2026-06-06T08:00:12+00:00",
                "subject": subject,
                "to": "me@gmail.com",
                "template_used": Value::Null,
                "message_id": format!("<{id}@test>"),
                "html_preview": html.chars().take(100).collect::<String>(),
                "html_full": html,
                "body_text": Value::Null,
                "cron_job": Value::Null,
            });
            fs::write(sent_dir.join(format!("{id}.json")), record.to_string()).unwrap();
        }
        let store = oxios_kernel::state_store::StateStore::new(tmp.path().to_path_buf()).unwrap();
        (tmp, store)
    }

    #[test]
    fn test_load_sent_records_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let store = oxios_kernel::state_store::StateStore::new(tmp.path().to_path_buf()).unwrap();
        let records = load_sent_records(&store, 100, false);
        assert!(records.is_empty());
    }

    #[test]
    fn test_load_sent_records_limit_and_sort() {
        let data = vec![
            ("a", "First", "<p>A</p>"),
            ("b", "Second", "<p>B</p>"),
            ("c", "Third", "<p>C</p>"),
        ];
        let (_tmp, store) = make_store_with_records(&data);
        let records = load_sent_records(&store, 100, false);
        assert_eq!(records.len(), 3);
        // All should be there (sorted by sent_at descending — same timestamp, so stable)
    }

    #[test]
    fn test_load_sent_records_strips_full_when_not_requested() {
        let data = vec![(
            "a",
            "Subject",
            "<p>very long html body that should be stripped</p>",
        )];
        let (_tmp, store) = make_store_with_records(&data);
        let records = load_sent_records(&store, 100, false);
        let rec = &records[0];
        assert!(
            rec.get("html_full").is_none(),
            "html_full should be stripped"
        );
        assert!(
            rec.get("body_text").is_none(),
            "body_text should be stripped"
        );
        assert!(rec.get("html_preview").is_some());
    }

    #[test]
    fn test_load_sent_records_keeps_full_when_requested() {
        let data = vec![("a", "Subject", "<p>full body</p>")];
        let (_tmp, store) = make_store_with_records(&data);
        let records = load_sent_records(&store, 100, true);
        let rec = &records[0];
        assert_eq!(
            rec.get("html_full").and_then(|v| v.as_str()),
            Some("<p>full body</p>")
        );
    }

    #[test]
    fn test_load_sent_record_exact_id_match() {
        // Critical: substring must NOT match — "abc" must not match "abcd1234"
        let data = vec![
            ("abc", "Short ID", "<p>abc</p>"),
            ("abcd1234", "Long ID", "<p>abcd</p>"),
        ];
        let (_tmp, store) = make_store_with_records(&data);

        let r1 = load_sent_record(&store, "abc").expect("abc should match");
        assert_eq!(r1.get("subject").and_then(|v| v.as_str()), Some("Short ID"));

        let r2 = load_sent_record(&store, "abcd1234").expect("abcd1234 should match");
        assert_eq!(r2.get("subject").and_then(|v| v.as_str()), Some("Long ID"));

        // Substring must NOT match
        assert!(
            load_sent_record(&store, "ab").is_none(),
            "ab must not match abc"
        );
        assert!(
            load_sent_record(&store, "abc1").is_none(),
            "abc1 must not match abc"
        );
    }

    #[test]
    fn test_load_sent_record_handles_corruption() {
        let tmp = tempfile::tempdir().unwrap();
        let sent_dir = tmp.path().join("email_sent");
        fs::create_dir_all(&sent_dir).unwrap();
        // Write a valid and a corrupted file
        let good = serde_json::json!({
            "id": "good", "sent_at": "2026-06-06T00:00:00+00:00",
            "subject": "Good", "to": "x", "message_id": "m",
            "html_preview": "", "html_full": "", "template_used": null, "cron_job": null
        });
        fs::write(sent_dir.join("good.json"), good.to_string()).unwrap();
        fs::write(sent_dir.join("corrupt.json"), "NOT VALID JSON {{{").unwrap();

        let store = oxios_kernel::state_store::StateStore::new(tmp.path().to_path_buf()).unwrap();
        // Should not panic; good record should be returned
        let r = load_sent_record(&store, "good");
        assert!(r.is_some());
        // Corrupt file should be silently skipped, not crash
        let records = load_sent_records(&store, 100, false);
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn test_upsert_email_section_creates_block() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        fs::write(&path, "[kernel]\nworkspace = \"/tmp\"").unwrap();
        let config = oxios_kernel::config::EmailConfig {
            enabled: true,
            my_email: "me@gmail.com".to_string(),
            provider: oxios_kernel::email::SmtpProvider::Gmail,
            host: String::new(),
            port: 0,
            tls: None,
            user: String::new(),
            secret_ref: "email_smtp".to_string(),
            rate_limit_per_hour: 10,
        };
        upsert_email_section_in_config(&path, &config).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("[email]"));
        assert!(content.contains("my_email = \"me@gmail.com\""));
        assert!(content.contains("provider = \"gmail\""));
        // Original [kernel] must be preserved
        assert!(content.contains("[kernel]"));
    }

    #[test]
    fn test_upsert_email_section_replaces_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let config = oxios_kernel::config::EmailConfig {
            enabled: true,
            my_email: "me@gmail.com".to_string(),
            provider: oxios_kernel::email::SmtpProvider::Gmail,
            host: String::new(),
            port: 0,
            tls: None,
            user: String::new(),
            secret_ref: "email_smtp".to_string(),
            rate_limit_per_hour: 10,
        };
        // Pre-populate with stale [email] section
        fs::write(
            &path,
            "[email]\nenabled = false\nmy_email = \"old@example.com\"\nprovider = \"icloud\"\n",
        )
        .unwrap();
        // Should replace the section with new config
        upsert_email_section_in_config(&path, &config).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        // Should have exactly one [email] section, with new values
        assert_eq!(content.matches("[email]").count(), 1);
        assert!(content.contains("me@gmail.com"));
        assert!(!content.contains("old@example.com"));
    }
}
