#[cfg(test)]
pub(crate) mod fake_server;
pub mod format;
pub mod plugin;

pub use format::TelegramFormatter;
pub use plugin::TelegramPlugin;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use oxios_gateway::GatewayInbox;
use oxios_gateway::channel::Channel;
use oxios_gateway::format::ChannelFormatter;
use oxios_gateway::message::{IncomingMessage, OutgoingMessage};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, watch};

/// Per-chat session state for Telegram.
#[derive(Debug, Clone)]
struct ChatSession {
    /// Current session ID used for multi-turn conversations.
    session_id: String,
    /// When the session was created.
    created_at: DateTime<Utc>,
    /// When the last message was sent/received in this session.
    last_active_at: DateTime<Utc>,
    /// Number of messages exchanged in this session.
    message_count: usize,
}

impl ChatSession {
    fn new() -> Self {
        let now = Utc::now();
        Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            created_at: now,
            last_active_at: now,
            message_count: 0,
        }
    }

    /// Check if this session should be rotated based on configuration.
    fn should_rotate(&self, rotation_hours: u64, max_messages: usize) -> bool {
        // Time-based rotation
        if rotation_hours > 0 {
            let elapsed = Utc::now() - self.last_active_at;
            if elapsed > chrono::Duration::hours(rotation_hours as i64) {
                return true;
            }
        }
        // Message-count based rotation
        if max_messages > 0 && self.message_count >= max_messages {
            return true;
        }
        false
    }

    /// Touch the session (update last_active_at and increment message count).
    fn touch(&mut self) {
        self.last_active_at = Utc::now();
        self.message_count += 1;
    }

    /// Rotate to a new session, returning the old session ID.
    fn rotate(&mut self) -> String {
        let old_id = self.session_id.clone();
        let now = Utc::now();
        self.session_id = uuid::Uuid::new_v4().to_string();
        self.created_at = now;
        self.last_active_at = now;
        self.message_count = 0;
        old_id
    }
}

/// Telegram session configuration.
#[derive(Debug, Clone)]
pub struct TelegramSessionSettings {
    /// Automatically rotate sessions after this many hours of inactivity.
    pub rotation_hours: u64,
    /// Rotate after this many messages (0 = unlimited).
    pub max_messages_per_session: usize,
}

impl Default for TelegramSessionSettings {
    fn default() -> Self {
        Self {
            rotation_hours: 2,
            max_messages_per_session: 0,
        }
    }
}

/// Outcome of a one-shot `getMe` token validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenValidation {
    /// Telegram accepted the token; carries the bot's username.
    Valid(String),
    /// API unreachable (network error, timeout, unexpected payload).
    /// Non-definitive: the polling loop may still recover later.
    Unreachable,
    /// Telegram definitively rejected the token (401/404) with a reason.
    Rejected(String),
}

/// Telegram channel adapter.
///
/// Uses long polling (getUpdates) to receive messages
/// and the Bot API to send responses.
///
/// Session management:
/// - Each `chat_id` gets its own session tracked in memory.
/// - Sessions auto-rotate after a configurable period of inactivity.
/// - Users can force a new session with the `/new` command.
#[derive(Clone)]
pub struct TelegramChannel {
    bot_token: String,
    api_base: String,
    allowed_users: Vec<i64>,
    client: reqwest::Client,
    offset: Arc<RwLock<i64>>,
    /// Maps chat_id → per-chat session state
    chat_sessions: Arc<RwLock<HashMap<i64, ChatSession>>>,
    /// Session rotation settings
    session_settings: TelegramSessionSettings,
    /// Bot username confirmed via getMe at setup time (for status display).
    bot_username: Option<String>,
}

impl TelegramChannel {
    /// Create a new Telegram channel.
    ///
    /// # Arguments
    /// * `bot_token` - Telegram Bot API token from @BotFather
    /// * `allowed_users` - List of allowed Telegram user IDs (empty = allow all)
    pub fn new(bot_token: String, allowed_users: Vec<i64>) -> Self {
        Self {
            bot_token,
            api_base: "https://api.telegram.org".to_string(),
            allowed_users,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            offset: Arc::new(RwLock::new(0)),
            chat_sessions: Arc::new(RwLock::new(HashMap::new())),
            session_settings: TelegramSessionSettings::default(),
            bot_username: None,
        }
    }

    /// Override API base URL (for local Bot API servers).
    pub fn with_api_base(mut self, base: String) -> Self {
        self.api_base = base;
        self
    }

    /// Attach the bot username confirmed at setup time (status display).
    pub fn with_bot_username(mut self, username: Option<String>) -> Self {
        self.bot_username = username;
        self
    }

    /// Validate the bot token with a single `getMe` call.
    ///
    /// Uses a short per-request timeout (5 s) so callers (channel setup,
    /// connect endpoint) fail fast instead of waiting on the polling loop's
    /// 60 s client ceiling. Only definitive Telegram rejections (401/404)
    /// map to [`TokenValidation::Rejected`]; transient failures map to
    /// [`TokenValidation::Unreachable`] so boot activation stays resilient.
    pub async fn validate_token(&self) -> TokenValidation {
        let resp = self
            .client
            .post(self.api_url("getMe"))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await;
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "Telegram getMe unreachable");
                return TokenValidation::Unreachable;
            }
        };
        let status = resp.status();
        let body: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "Telegram getMe body unparseable");
                return TokenValidation::Unreachable;
            }
        };
        if status.is_success() && body.get("ok").and_then(|o| o.as_bool()).unwrap_or(false) {
            return match body.pointer("/result/username").and_then(|u| u.as_str()) {
                Some(username) => TokenValidation::Valid(username.to_string()),
                None => {
                    tracing::warn!("Telegram getMe response missing bot username");
                    TokenValidation::Unreachable
                }
            };
        }
        let description = body
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("unknown error");
        if status.as_u16() == 401 || status.as_u16() == 404 {
            TokenValidation::Rejected(description.to_string())
        } else {
            tracing::warn!(
                status = status.as_u16(),
                description = %description,
                "Telegram getMe failed (non-definitive)"
            );
            TokenValidation::Unreachable
        }
    }

    /// Set session management settings.
    pub fn with_session_settings(mut self, settings: TelegramSessionSettings) -> Self {
        self.session_settings = settings;
        self
    }

    fn api_url(&self, method: &str) -> String {
        format!("{}/bot{}/{}", self.api_base, self.bot_token, method)
    }

    /// Check if user is allowed.
    fn is_user_allowed(&self, user_id: i64) -> bool {
        self.allowed_users.is_empty() || self.allowed_users.contains(&user_id)
    }

    /// Get or create a session for a chat, auto-rotating if needed.
    async fn get_or_create_session(&self, chat_id: i64) -> String {
        let mut sessions = self.chat_sessions.write().await;
        let session = sessions.entry(chat_id).or_insert_with(ChatSession::new);

        // Check if rotation is needed
        if session.should_rotate(
            self.session_settings.rotation_hours,
            self.session_settings.max_messages_per_session,
        ) {
            session.rotate();
            tracing::info!(
                chat_id = chat_id,
                new_session = %session.session_id,
                "Telegram session auto-rotated"
            );
        }

        session.touch();
        session.session_id.clone()
    }

    /// Force-rotate a chat's session (used for /new command).
    async fn force_new_session(&self, chat_id: i64) -> String {
        let mut sessions = self.chat_sessions.write().await;
        let session = sessions.entry(chat_id).or_insert_with(ChatSession::new);
        let old_id = session.rotate();
        tracing::info!(
            chat_id = chat_id,
            old_session = %old_id,
            new_session = %session.session_id,
            "Telegram session force-rotated via /new command"
        );
        session.session_id.clone()
    }

    /// Poll for updates using getUpdates (long polling).
    async fn poll_updates(&self) -> Result<Vec<serde_json::Value>> {
        let offset = *self.offset.read().await;
        let mut body = serde_json::json!({
            "timeout": 30,
            "limit": 100,
        });
        if offset > 0 {
            body["offset"] = serde_json::Value::Number(offset.into());
        }

        let resp = self
            .client
            .post(self.api_url("getUpdates"))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await.unwrap_or_default();
            anyhow::bail!("Telegram getUpdates failed: {err}");
        }

        let json: serde_json::Value = resp.json().await?;
        let updates = json
            .get("result")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        // Update offset
        if let Some(last) = updates.last()
            && let Some(id) = last.get("update_id").and_then(|id| id.as_i64())
        {
            *self.offset.write().await = id + 1;
        }

        Ok(updates)
    }

    /// Send a chat action indicator (e.g. "typing").
    async fn send_chat_action(&self, chat_id: i64, action: &str) -> Result<()> {
        self.client
            .post(self.api_url("sendChatAction"))
            .json(&serde_json::json!({ "chat_id": chat_id, "action": action }))
            .send()
            .await?;
        Ok(())
    }

    /// Send a text message to a chat.
    async fn send_text(&self, chat_id: i64, text: &str, reply_to: Option<i64>) -> Result<()> {
        for chunk in split_message(text, 4000) {
            let mut body = serde_json::json!({
                "chat_id": chat_id,
                "text": &chunk,
                "parse_mode": "Markdown",
            });
            if let Some(msg_id) = reply_to {
                body["reply_to_message_id"] = serde_json::Value::Number(msg_id.into());
            }
            let resp = self
                .client
                .post(self.api_url("sendMessage"))
                .json(&body)
                .send()
                .await?;
            if !resp.status().is_success() {
                // Fallback: send without parse_mode
                body["parse_mode"] = serde_json::Value::Null;
                self.client
                    .post(self.api_url("sendMessage"))
                    .json(&body)
                    .send()
                    .await?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Channel for TelegramChannel {
    fn name(&self) -> &str {
        "telegram"
    }

    fn status(&self) -> serde_json::Value {
        match &self.bot_username {
            Some(username) => serde_json::json!({ "bot_username": username }),
            None => serde_json::Value::Null,
        }
    }

    async fn start(
        &self,
        tx: mpsc::Sender<GatewayInbox>,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<tokio::task::JoinHandle<()>> {
        let this = Arc::new(self.clone());
        let channel_name = this.name().to_owned();

        let handle = tokio::spawn(async move {
            let mut retry_count: u32 = 0;
            loop {
                tokio::select! {
                    updates_result = this.poll_updates() => {
                        match updates_result {
                            Ok(updates) => {
                                retry_count = 0;
                                for update in updates {
                                    let message = update
                                        .get("message")
                                        .or_else(|| update.get("channel_post"))
                                        .or_else(|| update.get("edited_message"));
                                    let Some(msg) = message else { continue };

                                    let chat_id = msg
                                        .get("chat")
                                        .and_then(|c| c.get("id"))
                                        .and_then(|id| id.as_i64());
                                    let user_id = msg
                                        .get("from")
                                        .and_then(|f| f.get("id"))
                                        .and_then(|id| id.as_i64());
                                    let text = msg
                                        .get("text")
                                        .and_then(|t| t.as_str())
                                        .unwrap_or("");
                                    let message_id = msg
                                        .get("message_id")
                                        .and_then(|id| id.as_i64())
                                        .unwrap_or(0);

                                    if text.is_empty() {
                                        continue;
                                    }

                                    // Permission check. When an allowlist is
                                    // configured (non-empty), a message with no
                                    // recognizable `from` user_id (e.g. some
                                    // channel posts) is rejected outright —
                                    // treating it as "unknown user" would let it
                                    // bypass the whitelist.
                                    let user_unauthorized = match user_id {
                                        Some(uid) => !this.is_user_allowed(uid),
                                        None => !this.allowed_users.is_empty(),
                                    };
                                    if user_unauthorized {
                                        tracing::warn!(
                                            user_id = ?user_id,
                                            "Unauthorized Telegram user"
                                        );
                                        if let Some(cid) = chat_id {
                                            let _ = this
                                                .send_text(
                                                    cid,
                                                    "Unauthorized. Your user ID is not in the allowed list.",
                                                    None,
                                                )
                                                .await;
                                        }
                                        continue;
                                    }


                                    let Some(cid) = chat_id else { continue };
                                    let user_id_str = user_id
                                        .map(|id| id.to_string())
                                        .unwrap_or_else(|| "unknown".to_string());

                                    // /new command — start a new session
                                    let trimmed = text.trim();
                                    if trimmed == "/new" || trimmed == "/new@me" {
                                        let new_session_id = this.force_new_session(cid).await;
                                        let _ = this
                                            .send_text(
                                                cid,
                                                &format!("🔄 Starting a new session.\\n`{}`", new_session_id.chars().take(8).collect::<String>()),
                                                Some(message_id),
                                            )
                                            .await;
                                        continue;
                                    }

                                    // /session command — show current session info
                                    if trimmed == "/session" || trimmed == "/session@me" {
                                        let sessions = this.chat_sessions.read().await;
                                        if let Some(session) = sessions.get(&cid) {
                                            let info = format!(
                                                "📋 Current session\\n• ID: `{}`\\n• Messages: {}\\n• Started: {}\\n• Last activity: {}",
                                                session.session_id.chars().take(8).collect::<String>(),
                                                session.message_count,
                                                session.created_at.format("%m/%d %H:%M"),
                                                session.last_active_at.format("%m/%d %H:%M"),
                                            );
                                            drop(sessions);
                                            let _ = this.send_text(cid, &info, Some(message_id)).await;
                                        } else {
                                            drop(sessions);
                                            let _ = this
                                                .send_text(cid, "📋 No active sessions.", Some(message_id))
                                                .await;
                                        }
                                        continue;
                                    }

                                    // /spaces command — channels don't have kernel access
                                    if trimmed == "/spaces" || trimmed.starts_with("/spaces@") {
                                        let _ = this.send_text(cid, "Space management is available in the Web dashboard.", Some(message_id)).await;
                                        continue;
                                    }

                                    // /space command — channels don't have kernel access
                                    if trimmed.starts_with("/space") && !trimmed.starts_with("/spaces") {
                                        let _ = this.send_text(cid, "Space management is available in the Web dashboard.", Some(message_id)).await;
                                        continue;
                                    }

                                    // Skip other /command messages
                                    if text.starts_with('/') {
                                        continue;
                                    }

                                    // Get or auto-rotate session
                                    let session_id = this.get_or_create_session(cid).await;

                                    let mut metadata = HashMap::new();
                                    metadata.insert("chat_id".to_string(), cid.to_string());
                                    metadata.insert("message_id".to_string(), message_id.to_string());
                                    metadata.insert("session_id".to_string(), session_id);

                                    let incoming = IncomingMessage {
                                        channel: "telegram".to_string(),
                                        user_id: user_id_str,
                                        content: text.to_string(),
                                        metadata,
                                        ..Default::default()
                                    };

                                    tracing::info!(
                                        chat_id = cid,
                                        text = %text.chars().take(50).collect::<String>(),
                                        "Telegram message received"
                                    );

                                    if tx.send((channel_name.clone(), incoming)).await.is_err() {
                                        break; // Gateway receiver closed
                                    }
                                    // Send typing indicator
                                    let _ = this.send_chat_action(cid, "typing").await;
                                }
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "Telegram poll error");
                                let delay = std::time::Duration::from_secs(5 * 2u64.pow(retry_count.min(4)));
                                tokio::time::sleep(delay).await;
                                retry_count += 1;
                            }
                        }
                    }

                    _ = shutdown.changed() => {
                        tracing::info!(channel = %channel_name, "Telegram channel stopped");
                        break;
                    }
                }
            }
        });

        Ok(handle)
    }

    async fn send(&self, msg: OutgoingMessage) -> Result<()> {
        let chat_id: i64 = msg
            .metadata
            .get("chat_id")
            .and_then(|id| id.parse().ok())
            .or_else(|| msg.user_id.parse().ok())
            .ok_or_else(|| anyhow::anyhow!("No chat_id for Telegram message"))?;

        let reply_to = msg
            .metadata
            .get("message_id")
            .and_then(|id| id.parse().ok());

        let formatter = TelegramFormatter;
        let raw = match &msg.meta {
            Some(meta) if meta.error.is_some() => formatter.format_error(&msg),
            Some(_) => formatter.format_success(&msg),
            None => msg.content.clone(),
        };

        for chunk in split_message(&raw, 4000) {
            self.send_text(chat_id, &chunk, reply_to).await?;
        }

        tracing::debug!(chat_id = chat_id, "Telegram response sent");
        Ok(())
    }
}

/// Split a message into chunks of at most `max_chars` Unicode characters.
///
/// Unlike byte-based splitting, this is safe for multi-byte UTF-8
/// (Korean, Chinese, emoji, etc.).
fn split_message(text: &str, max_chars: usize) -> Vec<String> {
    if text.chars().count() <= max_chars {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if current.chars().count() >= max_chars {
            chunks.push(std::mem::take(&mut current));
        }
        current.push(ch);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telegram_channel_new() {
        let channel = TelegramChannel::new("test-token".to_string(), vec![12345]);
        assert_eq!(channel.name(), "telegram");
        assert!(channel.is_user_allowed(12345));
        assert!(!channel.is_user_allowed(99999));
    }

    mod validation_tests {
        use super::*;
        use crate::channels::telegram::fake_server::{self, FakeResponse};

        #[tokio::test]
        async fn validate_token_ok_reports_username() {
            let server = fake_server::spawn(FakeResponse::Ok {
                username: "oxios_bot".into(),
            })
            .await;
            let channel =
                TelegramChannel::new("tok".into(), vec![]).with_api_base(server.base_url.clone());
            assert_eq!(
                channel.validate_token().await,
                TokenValidation::Valid("oxios_bot".into())
            );
        }

        #[tokio::test]
        async fn validate_token_unauthorized_is_rejected() {
            let server = fake_server::spawn(FakeResponse::Unauthorized).await;
            let channel =
                TelegramChannel::new("tok-bad".into(), vec![]).with_api_base(server.base_url);
            match channel.validate_token().await {
                TokenValidation::Rejected(msg) => {
                    assert!(msg.contains("Unauthorized"), "unexpected message: {msg}")
                }
                other => panic!("expected Rejected, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn validate_token_connection_refused_is_unreachable() {
            // Bind then drop a listener so the port is (almost certainly)
            // closed — the next connect is refused.
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            drop(listener);
            let channel = TelegramChannel::new("tok".into(), vec![]).with_api_base(base);
            assert_eq!(channel.validate_token().await, TokenValidation::Unreachable);
        }

        #[tokio::test]
        async fn status_reports_bot_username() {
            let channel =
                TelegramChannel::new("tok".into(), vec![]).with_bot_username(Some("b".into()));
            assert_eq!(channel.status()["bot_username"], "b");
        }

        #[tokio::test]
        async fn status_null_without_bot_username() {
            let channel = TelegramChannel::new("tok".into(), vec![]);
            assert!(channel.status().is_null());
        }
    }
    #[test]
    fn test_telegram_channel_allow_all() {
        let channel = TelegramChannel::new("test-token".to_string(), vec![]);
        assert!(channel.is_user_allowed(12345));
        assert!(channel.is_user_allowed(99999));
    }

    #[test]
    fn test_api_url() {
        let channel = TelegramChannel::new("123:ABC".to_string(), vec![]);
        assert_eq!(
            channel.api_url("getMe"),
            "https://api.telegram.org/bot123:ABC/getMe"
        );
    }

    #[test]
    fn test_chat_session_rotation_by_time() {
        let mut session = ChatSession::new();
        assert!(!session.should_rotate(2, 0)); // Just created, should not rotate

        // Simulate 3 hours of inactivity
        session.last_active_at = Utc::now() - chrono::Duration::hours(3);
        assert!(session.should_rotate(2, 0)); // Should rotate
        assert!(!session.should_rotate(0, 0)); // Disabled, should not rotate
    }

    #[test]
    fn test_chat_session_rotation_by_message_count() {
        let mut session = ChatSession::new();
        session.message_count = 50;
        assert!(session.should_rotate(0, 50)); // At limit
        assert!(session.should_rotate(0, 49)); // Over limit
        assert!(!session.should_rotate(0, 51)); // Under limit
        assert!(!session.should_rotate(0, 0)); // Disabled
    }

    #[test]
    fn test_chat_session_rotate_resets_state() {
        let mut session = ChatSession::new();
        let original_id = session.session_id.clone();
        session.message_count = 100;

        let old_id = session.rotate();
        assert_eq!(old_id, original_id);
        assert_ne!(session.session_id, original_id);
        assert_eq!(session.message_count, 0);
    }

    #[test]
    fn test_chat_session_touch() {
        let mut session = ChatSession::new();
        assert_eq!(session.message_count, 0);
        session.touch();
        assert_eq!(session.message_count, 1);
        session.touch();
        assert_eq!(session.message_count, 2);
    }

    #[test]
    fn test_split_message_ascii() {
        let text = "hello world";
        let chunks = split_message(text, 5);
        assert_eq!(chunks, vec!["hello", " worl", "d"]);
    }

    #[test]
    fn test_split_message_utf8() {
        let text = "안녕하세요세계"; // 7 Korean chars
        let chunks = split_message(text, 3);
        assert_eq!(chunks, vec!["안녕하", "세요세", "계"]);
    }

    #[test]
    fn test_split_message_short() {
        let text = "hello";
        let chunks = split_message(text, 10);
        assert_eq!(chunks, vec!["hello"]);
    }
}
