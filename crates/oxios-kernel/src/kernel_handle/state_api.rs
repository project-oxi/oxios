//! State API — data persistence, session management.

use crate::state_store::{
    PruneConfig, PruneThrottle, Session, SessionId, SessionSummary, StateStore,
};
use serde::{Serialize, de::DeserializeOwned};
use std::sync::Arc;

/// State management system calls.
///
/// All data persistence: file (JSON/Markdown) storage, session management.
pub struct StateApi {
    pub(crate) state_store: Arc<StateStore>,
    /// Throttle for auto-prune to avoid excessive disk scans.
    pub prune_throttle: PruneThrottle,
}

impl StateApi {
    /// Create a new StateApi.
    pub fn new(state_store: Arc<StateStore>) -> Self {
        Self {
            state_store,
            prune_throttle: PruneThrottle::new(3600), // 1 hour cooldown
        }
    }
    /// Save JSON data.
    pub async fn save<T: Serialize>(
        &self,
        category: &str,
        name: &str,
        data: &T,
    ) -> anyhow::Result<()> {
        self.state_store.save_json(category, name, data).await
    }

    /// Save markdown content.
    pub async fn save_markdown(
        &self,
        category: &str,
        name: &str,
        content: &str,
    ) -> anyhow::Result<()> {
        self.state_store
            .save_markdown(category, name, content)
            .await
    }

    /// Load JSON data.
    pub async fn load<T: DeserializeOwned>(
        &self,
        category: &str,
        name: &str,
    ) -> anyhow::Result<Option<T>> {
        self.state_store.load_json(category, name).await
    }

    /// Load markdown content.
    pub async fn load_markdown(
        &self,
        category: &str,
        name: &str,
    ) -> anyhow::Result<Option<String>> {
        self.state_store.load_markdown(category, name).await
    }

    /// Delete a file.
    pub async fn delete(&self, category: &str, name: &str) -> anyhow::Result<bool> {
        self.state_store.delete_file(category, name).await
    }

    /// List files in a category.
    pub async fn list_category(&self, category: &str) -> anyhow::Result<Vec<String>> {
        self.state_store.list_category(category).await
    }

    /// Commit all changes to git via the provided GitLayer.
    ///
    /// Returns:
    /// - `Ok(None)` when git is disabled (no-op).
    /// - `Ok(Some(info))` on a successful commit.
    /// - `Err(...)` when the commit fails — callers must not conflate this
    ///   with "git disabled", or a broken repo will silently look healthy.
    pub fn commit_all(
        &self,
        git: &crate::git_layer::GitLayer,
        message: &str,
    ) -> anyhow::Result<Option<crate::git_layer::CommitInfo>> {
        if !git.is_enabled() {
            return Ok(None);
        }
        let info = git.commit_file(".", message)?;
        Ok(Some(info))
    }

    /// Save session.
    pub async fn save_session(&self, session: &Session) -> anyhow::Result<()> {
        self.state_store.save_session(session).await
    }

    /// Load session.
    pub async fn load_session(&self, id: &SessionId) -> anyhow::Result<Option<Session>> {
        self.state_store.load_session(id).await
    }

    /// List sessions.
    pub async fn list_sessions(&self) -> anyhow::Result<Vec<SessionSummary>> {
        self.state_store.list_sessions().await
    }

    /// Delete session.
    pub async fn delete_session(&self, id: &SessionId) -> anyhow::Result<bool> {
        self.state_store.delete_session(id).await
    }

    /// RFC-025: Move a session to a different Project (drag-to-reparent).
    pub async fn move_session_to_project(
        &self,
        id: &SessionId,
        project_id: Option<&str>,
    ) -> anyhow::Result<bool> {
        self.state_store
            .move_session_to_project(id, project_id)
            .await
    }

    /// RFC-035: List child sessions (threads) of the given parent.
    pub async fn list_child_sessions(
        &self,
        parent_id: &str,
    ) -> anyhow::Result<Vec<SessionSummary>> {
        self.state_store.list_child_sessions(parent_id).await
    }

    /// RFC-035: Create a thread (sub-session) under the given parent.
    pub async fn create_thread(&self, parent_id: &str, user_id: &str) -> anyhow::Result<Session> {
        self.state_store.create_thread(parent_id, user_id).await
    }

    /// Get workspace base path.
    pub fn workspace_path(&self) -> &std::path::Path {
        &self.state_store.base_path
    }

    /// Access the underlying StateStore (for backup/restore).
    pub fn store(&self) -> &Arc<StateStore> {
        &self.state_store
    }

    /// Prune sessions based on configuration.
    ///
    /// Removes sessions that exceed TTL or exceed the maximum count.
    pub async fn prune_sessions(&self, config: &PruneConfig) -> anyhow::Result<usize> {
        self.state_store.prune_sessions(config).await
    }

    /// Check if auto-prune should run (respects cooldown throttle).
    pub fn should_auto_prune(&self) -> bool {
        self.prune_throttle.should_prune()
    }
}
