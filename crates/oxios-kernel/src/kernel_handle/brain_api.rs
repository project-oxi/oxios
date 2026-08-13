//! Brain API — the oxibrain daemon facade (RFC-047).
//!
//! Replaces the AgentApi memory methods. Wraps the shared
//! [`BrainConnection`](crate::brain::BrainConnection) and exposes the same
//! surface to the web routes and the memory tools. All operations follow the
//! degradation contract: `None`/empty when the daemon is unavailable.

use crate::brain::BrainConnection;
use serde_json::Value;
use std::sync::Arc;

/// Facade over the oxibrain daemon connection.
#[derive(Debug, Clone)]
pub struct BrainApi {
    conn: Arc<BrainConnection>,
}

impl BrainApi {
    /// Wrap a shared connection.
    pub fn new(conn: Arc<BrainConnection>) -> Self {
        Self { conn }
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
