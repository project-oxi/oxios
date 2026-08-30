//! Metadata key constants shared across all channels.
//!
//! Standardises the keys used in `IncomingMessage.metadata` and
//! `OutgoingMessage.metadata` `HashMap<String, String>` to prevent typos
//! and improve discoverability.

/// Well-known metadata keys.
#[allow(clippy::module_inception)]
pub mod meta {
    /// Session ID for multi-turn conversations.
    pub const SESSION_ID: &str = "session_id";
    /// Project ID for the turn (singular workspace binding, design §4.3).
    pub const PROJECT_ID: &str = "project_id";
    /// Chat ID (used by Telegram and similar chat channels).
    pub const CHAT_ID: &str = "chat_id";
    /// Message ID for reply correlation.
    pub const MESSAGE_ID: &str = "message_id";
    /// User ID for authentication context.
    pub const USER_ID: &str = "user_id";
    /// Action type for metadata-driven routing (e.g., "switch_model", "get_persona").
    pub const ACTION: &str = "action";
    /// Model ID for switch_model action.
    pub const MODEL_ID: &str = "model_id";
    /// Persona ID for switch_persona action.
    pub const PERSONA_ID: &str = "persona_id";
}
