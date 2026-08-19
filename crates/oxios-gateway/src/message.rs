//! Message types for the gateway.
//!
//! Messages are channel-agnostic: they carry content and metadata
//! without depending on any specific channel implementation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// A message arriving from a channel.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IncomingMessage {
    /// Unique message identifier.
    pub id: uuid::Uuid,
    /// Name of the source channel.
    pub channel: String,
    /// Identifier for the user who sent the message.
    pub user_id: String,
    /// Message content.
    pub content: String,
    /// Timestamp of message creation.
    pub timestamp: DateTime<Utc>,
    /// Optional metadata (e.g., session_id for multi-turn conversations).
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl IncomingMessage {
    /// Creates a new incoming message with the current timestamp and empty metadata.
    pub fn new(
        channel: impl Into<String>,
        user_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            channel: channel.into(),
            user_id: user_id.into(),
            content: content.into(),
            timestamp: Utc::now(),
            metadata: HashMap::new(),
        }
    }
}

/// Orchestration result metadata.
///
/// Attached by `Gateway::dispatch()` from `OrchestrationResult`.
/// The legacy `HashMap<String, String>` metadata is retained for channel-specific data
/// (chat_id, message_id, etc.).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseMeta {
    /// Session ID for multi-turn conversations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Primary project ID that handled the message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Project decoration tag (e.g., "[🔧 oxios]").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_tag: Option<String>,
    /// Furthest phase reached (Interview | Seed | Execute | Evaluate | Evolve).
    pub phase: String,
    /// Whether evaluation passed.
    ///
    /// - `None` — evaluation was not applicable (interview, chat).
    /// - `Some(true)` — evaluation passed.
    /// - `Some(false)` — evaluation failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluation_passed: Option<bool>,
    /// Wall-clock duration of the dispatch in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Structured error, if this is an error response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<UserFacingError>,
    /// Structured interview questions (chat UI redesign — interactive
    /// interview). Populated when the interview phase needs clarification
    /// and the LLM produced a structured form. The WebSocket handler
    /// forwards this to the client as an `interview` chunk so the Web
    /// UI can render interactive widgets. When `None`, the frontend
    /// renders the plain `content` as markdown (graceful degradation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interview_questions: Option<Vec<oxios_ouroboros::InterviewQuestionOutput>>,
    /// Current interview round (1-based). Populated alongside
    /// `interview_questions`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interview_round: Option<u32>,
}

/// A user-facing structured error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserFacingError {
    /// User-visible message (Korean).
    pub message: String,
    /// Error classification.
    pub kind: ErrorKind,
    /// Recovery suggestion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// Error kind classification.
///
/// `snake_case` on the wire: the web client narrows on these exact strings
/// (`web/src/stores/chat.ts`). Renaming a variant is a wire-breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Agent execution failed.
    ExecutionFailed,
    /// API key is missing or not configured.
    ApiKeyMissing,
    /// LLM provider error (rate limit, API error, etc.).
    ProviderError,
    /// Timeout.
    Timeout,
    /// Insufficient permissions.
    PermissionDenied,
    /// Input validation failed.
    ValidationError,
    /// The user cancelled the turn (RFC-049). Not a fault.
    Cancelled,
    /// Internal system error (details not exposed to user).
    Internal,
}

/// A message being sent to a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingMessage {
    /// Unique message identifier.
    pub id: uuid::Uuid,
    /// RFC-024 SP1: monotonic sequence number assigned by the gateway's
    /// `ReliabilityLayer`. `None` for messages built outside the delivery
    /// pipeline (tests, direct responses). Consumers use this together
    /// with the message `id` to dedupe (C2 order, C3 idempotency).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    /// Name of the target channel.
    pub channel: String,
    /// Identifier for the user who should receive the message.
    pub user_id: String,
    /// Message content.
    pub content: String,
    /// Timestamp of message creation.
    pub timestamp: DateTime<Utc>,
    /// Optional metadata (e.g., session_id, phase, evaluation_passed).
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    /// RFC-014: typed orchestration metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<ResponseMeta>,
    /// Target connection ID for point-to-point delivery.
    ///
    /// When set, only the WebSocket connection with a matching `conn_id`
    /// should process this message. `None` means broadcast to all connections.
    /// Used to prevent cross-tab message leakage in multi-session scenarios.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_conn_id: Option<String>,
    /// RFC-015 streaming: when `Some(true)`, this is a partial text/thinking
    /// delta. The WS handler forwards it as a bare `token`/`reasoning` chunk
    /// WITHOUT the terminal `done` chunk — the terminal is the single
    /// subsequent message with `partial != Some(true)` (or absent, which is
    /// the legacy/terminal default). `None` preserves legacy behavior
    /// (terminal — emit `token` + `done`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial: Option<bool>,
}

impl OutgoingMessage {
    /// Creates a new outgoing message with the current timestamp.
    pub fn new(
        channel: impl Into<String>,
        user_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self::with_id(uuid::Uuid::new_v4(), channel, user_id, content)
    }

    /// Creates a new outgoing message with a specific ID (preserving correlation with the request).
    pub fn with_id(
        id: uuid::Uuid,
        channel: impl Into<String>,
        user_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id,
            seq: None,
            channel: channel.into(),
            user_id: user_id.into(),
            content: content.into(),
            timestamp: Utc::now(),
            metadata: HashMap::new(),
            meta: None,
            target_conn_id: None,
            partial: None,
        }
    }

    /// Creates a new outgoing message with metadata.
    pub fn with_metadata(
        channel: impl Into<String>,
        user_id: impl Into<String>,
        content: impl Into<String>,
        metadata: HashMap<String, String>,
    ) -> Self {
        Self::with_id(uuid::Uuid::new_v4(), channel, user_id, content).with_metadata_only(metadata)
    }

    /// Creates a new outgoing message with a specific ID and metadata.
    pub fn with_id_and_metadata(
        id: uuid::Uuid,
        channel: impl Into<String>,
        user_id: impl Into<String>,
        content: impl Into<String>,
        metadata: HashMap<String, String>,
    ) -> Self {
        Self {
            id,
            seq: None,
            channel: channel.into(),
            user_id: user_id.into(),
            content: content.into(),
            timestamp: Utc::now(),
            metadata,
            meta: None,
            target_conn_id: None,
            partial: None,
        }
    }

    /// Sets metadata on this message (builder pattern).
    pub fn with_metadata_only(mut self, metadata: HashMap<String, String>) -> Self {
        self.metadata = metadata;
        self
    }

    /// Sets the streaming partial flag (builder pattern).
    ///
    /// `Some(true)` marks this message as a streaming delta — the WS handler
    /// forwards it as a bare `token` chunk WITHOUT the terminal `done`.
    /// `Some(false)` explicitly marks terminal (legacy default behavior).
    pub fn with_partial(mut self, partial: bool) -> Self {
        self.partial = Some(partial);
        self
    }

    /// Creates a success response with typed metadata.
    ///
    /// Combines channel-specific metadata (chat_id, message_id, etc.) with
    /// structured orchestration metadata.
    pub fn success(
        correlation_id: Uuid,
        channel: &str,
        user_id: &str,
        content: &str,
        channel_meta: HashMap<String, String>,
        response_meta: ResponseMeta,
    ) -> Self {
        Self {
            id: correlation_id,
            seq: None,
            channel: channel.to_string(),
            user_id: user_id.to_string(),
            content: content.to_string(),
            timestamp: Utc::now(),
            metadata: channel_meta,
            meta: Some(response_meta),
            target_conn_id: None,
            partial: None,
        }
    }

    /// Creates an error response.
    ///
    /// The `UserFacingError` provides structured error information.
    /// Callers can set `session_id` etc. on the returned message's metadata
    /// to preserve conversation continuity.
    pub fn error(correlation_id: Uuid, channel: &str, user_id: &str, err: UserFacingError) -> Self {
        Self {
            id: correlation_id,
            seq: None,
            channel: channel.to_string(),
            user_id: user_id.to_string(),
            content: err.message.clone(),
            timestamp: Utc::now(),
            metadata: HashMap::new(),
            meta: Some(ResponseMeta {
                session_id: None,
                project_id: None,
                project_tag: None,
                phase: String::new(),
                evaluation_passed: None,
                duration_ms: None,
                error: Some(err),
                interview_questions: None,
                interview_round: None,
            }),
            target_conn_id: None,
            partial: None,
        }
    }
}
