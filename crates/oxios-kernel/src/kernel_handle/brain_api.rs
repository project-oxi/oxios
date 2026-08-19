//! Brain API — the oxibrain daemon facade (RFC-047).
//!
//! Replaces the AgentApi memory methods. Wraps the shared
//! [`BrainConnection`](crate::brain::BrainConnection) and exposes the same
//! surface to the web routes and the memory tools. All operations follow the
//! degradation contract: `None`/empty when the daemon is unavailable.
//!
//! The supervisor handle is shared with the kernel so `/api/brain/status`
//! can surface install / launchd state without re-running `ensure`.

use crate::brain::{BrainConnection, BrainSupervisor, SupervisorStatus};
use serde_json::Value;
use std::fmt;
use std::sync::Arc;

/// Facade over the oxibrain daemon connection.
#[derive(Clone)]
pub struct BrainApi {
    conn: Arc<BrainConnection>,
    supervisor: Arc<BrainSupervisor>,
}

impl fmt::Debug for BrainApi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The supervisor holds Arc<dyn Installer>/ProbeFn which aren't
        // Debug; mask them like the sibling facades (BrowserApi, EngineApi…).
        f.debug_struct("BrainApi")
            .field("conn", &self.conn)
            .field("supervisor", &"Some(<supervisor>)")
            .finish()
    }
}

impl BrainApi {
    /// Wrap a shared connection and supervisor.
    pub fn new(conn: Arc<BrainConnection>, supervisor: Arc<BrainSupervisor>) -> Self {
        Self { conn, supervisor }
    }

    /// Live snapshot of the supervisor (install / launchd / spawn state).
    /// Cheap clone — the supervisor caches the value internally.
    pub fn supervisor_state(&self) -> SupervisorStatus {
        self.supervisor.status()
    }

    /// Whether the daemon is currently reachable.
    pub fn is_available(&self) -> bool {
        self.conn.is_available()
    }

    /// Drop the dead client and reconnect. Returns `true` on success.
    pub async fn reconnect(&self) -> bool {
        self.conn.reconnect().await
    }

    /// The configured space name.
    pub fn space(&self) -> &str {
        self.conn.space()
    }

    /// Assemble recall context for an agent turn.
    pub async fn recall(&self, query: &str, budget: usize) -> Option<String> {
        self.conn.recall(query, budget).await
    }

    /// Remember content as an episode; returns the episode id.
    pub async fn remember(&self, content: &str, source: &str) -> Option<String> {
        self.conn.remember(content, source).await
    }

    /// Hybrid/lexical/semantic/graph/community search.
    pub async fn search(&self, query: &str, mode: &str, limit: usize) -> Option<Value> {
        self.conn.search(query, mode, limit).await
    }

    /// An entity's current beliefs.
    pub async fn get_entity(&self, entity_id: &str) -> Option<Value> {
        self.conn.get_entity(entity_id).await
    }

    /// Belief intervals for an entity over a time range.
    pub async fn timeline(
        &self,
        entity_id: &str,
        from: Option<i64>,
        to: Option<i64>,
    ) -> Option<Value> {
        self.conn.timeline(entity_id, from, to).await
    }

    /// Provenance and confidence breakdown for a statement.
    pub async fn why(&self, statement_id: &str) -> Option<Value> {
        self.conn.why(statement_id).await
    }

    /// List contradicted statements in the space.
    pub async fn contradictions(&self) -> Option<Value> {
        self.conn.contradictions().await
    }

    /// Aggregate counts for the space.
    pub async fn stats(&self) -> Option<Value> {
        self.conn.stats().await
    }

    /// The underlying connection (for tools that need raw access).
    pub fn connection(&self) -> Arc<BrainConnection> {
        Arc::clone(&self.conn)
    }
}
