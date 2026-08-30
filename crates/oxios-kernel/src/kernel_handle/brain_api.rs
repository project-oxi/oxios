//! Brain API — the oxibrain 0.8 daemonless facade (RFC-047, RFC-049).
//!
//! Wraps the kernel-owned [`BrainSession`] (a lazily-spawned stdio child)
//! and the [`BrainInstaller`] (binary location / install). Routes and
//! memory tools consume this facade instead of a raw connection. All
//! operations follow the degradation contract: `None`/empty when the
//! brain is unavailable, and the agent turn continues normally.

use crate::brain::{BrainInstaller, BrainSession};
use oxibrain_client::{PendingStatsDto, SearchResponseDto};
use serde::Serialize;
use serde_json::Value;
use std::fmt;
use std::sync::Arc;

/// Facade over the oxibrain stdio session + installer.
#[derive(Clone)]
pub struct BrainApi {
    session: Arc<BrainSession>,
    installer: Arc<BrainInstaller>,
}

impl fmt::Debug for BrainApi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrainApi")
            .field("session", &self.session)
            .field("installer", &self.installer)
            .finish()
    }
}

/// Snapshot of brain availability + binary metadata for `/api/brain/status`.
#[derive(Debug, Clone, Serialize)]
pub struct BrainStatusSnapshot {
    /// Whether the brain session child is currently up.
    pub available: bool,
    /// Whether the managed binary is on disk (or an explicit binary is set).
    pub binary_installed: bool,
    /// Resolved binary path when installed.
    pub binary_path: Option<String>,
    /// `<binary> --version` output when installed.
    pub binary_version: Option<String>,
}

impl BrainApi {
    /// Build a facade from a session + installer (kernel boot path).
    pub fn new(session: Arc<BrainSession>, installer: Arc<BrainInstaller>) -> Self {
        Self { session, installer }
    }

    /// Whether the brain session child is currently connected.
    pub fn is_available(&self) -> bool {
        self.session.is_available()
    }

    /// Underlying session (for tests + ad-hoc callers).
    pub fn session(&self) -> Arc<BrainSession> {
        Arc::clone(&self.session)
    }

    /// Snapshot of availability + binary metadata for `/api/brain/status`.
    pub fn status_snapshot(&self) -> BrainStatusSnapshot {
        let binary = self.installer.locate_binary();
        let version = binary
            .as_deref()
            .and_then(|p| self.installer.version_of_cached(p));
        BrainStatusSnapshot {
            available: self.session.is_available(),
            binary_installed: binary.is_some(),
            binary_path: binary.as_ref().and_then(|p| p.to_str().map(str::to_string)),
            binary_version: version,
        }
    }

    /// Assemble recall context for an agent turn.
    pub async fn recall(&self, space: &str, query: &str, budget: usize) -> Option<String> {
        self.session.recall(space, query, budget).await
    }

    /// Remember content as an episode; returns the episode id.
    pub async fn remember(&self, space: &str, content: &str, source: &str) -> Option<String> {
        self.session.remember(space, content, source).await
    }

    /// Two-plane search (`memory` + `documents`); `planes=None` → both.
    pub async fn search(
        &self,
        space: &str,
        query: &str,
        mode: &str,
        limit: usize,
        planes: Option<&[&str]>,
    ) -> Option<SearchResponseDto> {
        self.session.search(space, query, mode, limit, planes).await
    }

    /// An entity's current beliefs.
    pub async fn get_entity(&self, space: &str, entity_id: &str) -> Option<Value> {
        self.session.get_entity(space, entity_id).await
    }

    /// Belief intervals for an entity over a time range.
    pub async fn timeline(
        &self,
        space: &str,
        entity_id: &str,
        from: Option<i64>,
        to: Option<i64>,
    ) -> Option<Value> {
        self.session.timeline(space, entity_id, from, to).await
    }

    /// Provenance and confidence breakdown for a statement.
    pub async fn why(&self, space: &str, statement_id: &str) -> Option<Value> {
        self.session.why(space, statement_id).await
    }

    /// List contradicted statements in the space.
    pub async fn contradictions(&self, space: &str) -> Option<Value> {
        self.session.contradictions(space).await
    }

    /// Aggregate counts for the space.
    pub async fn stats(&self, space: &str) -> Option<Value> {
        self.session.stats(space).await
    }

    /// Render a readable page as Markdown (`entity` / `space` / `topic`).
    pub async fn brief(
        &self,
        space: &str,
        target_kind: &str,
        entity_id: Option<&str>,
        topic: Option<&str>,
    ) -> Option<String> {
        self.session
            .brief(space, target_kind, entity_id, topic)
            .await
    }

    /// Bounded belief-filtered subgraph traversal from start entities.
    pub async fn traverse(
        &self,
        space: &str,
        start: &[String],
        depth: usize,
        max_nodes: usize,
        direction: &str,
    ) -> Option<Value> {
        self.session
            .traverse(space, start, depth, max_nodes, direction)
            .await
    }
    /// Console data: `merges` / `failures` / `sources`.
    pub async fn review_merges(&self, space: &str, section: &str) -> Option<Value> {
        self.session.review_merges(space, section).await
    }

    /// Spaces the server exposes.
    pub async fn spaces(&self) -> Option<Value> {
        self.session.spaces().await
    }

    /// Overview of one space (counts + recent entities).
    pub async fn space_overview(&self, space: &str) -> Option<Value> {
        self.session.space_overview(space).await
    }

    /// gix-backed revision history of one tracked document.
    pub async fn document_history(
        &self,
        space: &str,
        alias: &str,
        locator: &str,
        limit: usize,
    ) -> Option<Value> {
        self.session
            .document_history(space, alias, locator, limit)
            .await
    }

    /// Memory-plane extraction backlog stats.
    pub async fn pending_stats(&self) -> Option<PendingStatsDto> {
        self.session.pending_stats().await
    }
}
