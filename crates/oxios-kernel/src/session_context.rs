//! Session-level context for managing conversation state across executions.
//!
//! Holds the session's singular project binding (design §4.3). Proactive
//! recall timing moved to the oxibrain daemon — RFC-047.

/// Session-level context for managing conversation state.
///
/// Created when a new session starts, passed to `AgentRuntime::execute()`.
#[derive(Debug, Default)]
pub struct SessionContext {
    /// Project bound to this session, if any.
    pub project_id: Option<String>,
}

impl SessionContext {
    /// Create a new session context with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a session context bound to a project.
    pub fn with_project(project_id: impl Into<String>) -> Self {
        Self {
            project_id: Some(project_id.into()),
        }
    }
}
