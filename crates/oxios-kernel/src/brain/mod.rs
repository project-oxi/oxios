//! Connection to the oxibrain daemon over a Unix-domain socket (RFC-047).
//!
//! oxibrain is a shared system service (oxios, oximemo, oxiline all consume
//! it). oxios connects at startup and degrades gracefully when the daemon is
//! unavailable: every operation returns `None`/empty and agent turns complete
//! normally (spec §4 degradation contract).

use crate::brain::config::BrainConfig;
use oxibrain_client::BrainClient;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

/// A live (or degraded) connection to the oxibrain daemon.
///
/// The inner client is `Option` so a failed call can drop the dead client and
/// record the degraded state. A later operation that finds `None` attempts a
/// reconnect once (single retry) before giving up.
#[derive(Debug)]
pub struct BrainConnection {
    client: Mutex<Option<BrainClient>>,
    config: BrainConfig,
}

impl BrainConnection {
    /// Try to connect; on failure log a warning and start degraded.
    pub async fn connect(config: BrainConfig) -> Self {
        let client = match BrainClient::connect(&config.socket_path).await {
            Ok(c) => {
                tracing::info!(
                    path = %config.socket_path.display(),
                    space = %config.space,
                    "connected to oxibrain daemon"
                );
                Some(c)
            }
            Err(e) => {
                tracing::warn!(
                    path = %config.socket_path.display(),
                    error = %e,
                    "brain daemon unavailable — memory operations degraded"
                );
                None
            }
        };
        Self {
            client: Mutex::new(client),
            config,
        }
    }

    /// Whether a client is currently connected. Cheap check (no IO).
    pub fn is_available(&self) -> bool {
        self.client.try_lock().map(|c| c.is_some()).unwrap_or(false)
    }

    /// Drop any dead client and reconnect. Returns `true` on success.
    pub async fn reconnect(&self) -> bool {
        let mut guard = self.client.lock().await;
        match BrainClient::connect(&self.config.socket_path).await {
            Ok(c) => {
                tracing::info!(
                    path = %self.config.socket_path.display(),
                    "reconnected to oxibrain daemon"
                );
                *guard = Some(c);
                true
            }
            Err(e) => {
                tracing::warn!(
                    path = %self.config.socket_path.display(),
                    error = %e,
                    "brain daemon still unavailable"
                );
                *guard = None;
                false
            }
        }
    }

    /// Get the configured space name.
    pub fn space(&self) -> &str {
        &self.config.space
    }

    /// Get the configured socket path.
    pub fn socket_path(&self) -> &Path {
        &self.config.socket_path
    }

    // ── Agent-runtime methods (typed) ────────────────────────────────

    /// Assemble recall context for an agent turn: layered context text within
    /// `budget` tokens. `None` when the daemon is unavailable.
    pub async fn recall(&self, query: &str, budget: usize) -> Option<String> {
        let value = self
            .call(|c| {
                let query = query.to_string();
                let space = self.config.space.clone();
                async move { c.recall(&query, &space, budget).await }
            })
            .await?;
        assemble_context_text(&value)
    }

    /// Remember content as an episode. Returns the episode id.
    pub async fn remember(&self, content: &str, source: &str) -> Option<String> {
        self.call(|c| {
            let content = content.to_string();
            let source = source.to_string();
            let space = self.config.space.clone();
            async move { c.ingest(&content, &space, &source).await }
        })
        .await
    }

    // ── Web API methods (JSON passthrough) ───────────────────────────

    /// Hybrid/lexical/semantic/graph/community search. Raw daemon JSON.
    pub async fn search(&self, query: &str, mode: &str, limit: usize) -> Option<Value> {
        self.call(|c| {
            let query = query.to_string();
            let mode = mode.to_string();
            let space = self.config.space.clone();
            async move { c.search(&query, &space, &mode, limit).await }
        })
        .await
    }

    /// An entity's current beliefs.
    pub async fn get_entity(&self, entity_id: &str) -> Option<Value> {
        self.call(|c| {
            let entity_id = entity_id.to_string();
            let space = self.config.space.clone();
            async move { c.get_entity(&entity_id, &space).await }
        })
        .await
    }

    /// Belief intervals for an entity over a time range.
    pub async fn timeline(
        &self,
        entity_id: &str,
        from: Option<i64>,
        to: Option<i64>,
    ) -> Option<Value> {
        self.call(|c| {
            let entity_id = entity_id.to_string();
            let space = self.config.space.clone();
            async move { c.timeline(&entity_id, &space, from, to).await }
        })
        .await
    }

    /// Provenance and confidence breakdown for a statement.
    pub async fn why(&self, statement_id: &str) -> Option<Value> {
        self.call(|c| {
            let statement_id = statement_id.to_string();
            let space = self.config.space.clone();
            async move { c.why(&statement_id, &space).await }
        })
        .await
    }

    /// List contradicted statements in the space.
    pub async fn contradictions(&self) -> Option<Value> {
        self.call(|c| {
            let space = self.config.space.clone();
            async move { c.contradictions(&space).await }
        })
        .await
    }

    /// Aggregate counts for the space.
    pub async fn stats(&self) -> Option<Value> {
        self.call(|c| {
            let space = self.config.space.clone();
            async move { c.stats(&space).await }
        })
        .await
    }

    // ── Shared plumbing ──────────────────────────────────────────────

    /// Run `f` against the connected client, handling degradation:
    /// - no client → one reconnect attempt; still none → `None`
    /// - call error → drop the dead client, log, `None`
    async fn call<F, Fut, T>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&mut BrainClient) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<T>>,
    {
        let mut guard = self.client.lock().await;
        if guard.is_none() && !self.try_reconnect(&mut guard).await {
            return None;
        }
        let client = guard.as_mut()?;
        match f(client).await {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!(error = %e, "brain daemon call failed — dropping connection");
                *guard = None;
                None
            }
        }
    }

    /// Single-shot reconnect inside a held lock. `true` if a client is now present.
    async fn try_reconnect(&self, guard: &mut Option<BrainClient>) -> bool {
        if guard.is_some() {
            return true;
        }
        match BrainClient::connect(&self.config.socket_path).await {
            Ok(c) => {
                tracing::info!(
                    path = %self.config.socket_path.display(),
                    "reconnected to oxibrain daemon"
                );
                *guard = Some(c);
                true
            }
            Err(e) => {
                tracing::debug!(error = %e, "brain daemon still unavailable");
                false
            }
        }
    }
}

/// Join a daemon `recall` (ContextResult) payload into a single text block.
///
/// The daemon returns `{layers: [{kind, text, estimated_tokens, provenance}],
/// total_tokens, budget, truncated}`. Each layer becomes a headed section so
/// the agent can see why the context was included.
fn assemble_context_text(value: &Value) -> Option<String> {
    let layers = value.get("layers")?.as_array()?;
    if layers.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for layer in layers {
        let kind = layer
            .get("kind")
            .and_then(|k| k.as_str())
            .unwrap_or("context");
        let text = layer.get("text").and_then(|t| t.as_str()).unwrap_or("");
        if text.is_empty() {
            continue;
        }
        parts.push(format!("## {kind}\n{text}"));
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("\n\n"))
}

/// Convenience handle shared by tools and the runtime: an owned
/// [`BrainConnection`] behind an `Arc`.
pub type SharedBrain = Arc<BrainConnection>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_to_missing_socket_is_degraded() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let config = BrainConfig::new(dir.path().join("missing.sock"), "personal");
        let conn = BrainConnection::connect(config).await;
        assert!(!conn.is_available(), "no daemon → degraded");
        assert_eq!(conn.recall("test", 1000).await, None);
        assert_eq!(conn.remember("hello", "test").await, None);
        assert_eq!(conn.stats().await, None);
    }

    #[test]
    fn assemble_context_joins_layers() {
        let value = serde_json::json!({
            "layers": [
                {"kind": "high_salience_beliefs", "text": "Bob likes coffee", "estimated_tokens": 4},
                {"kind": "recent_episodes", "text": "We discussed Rust", "estimated_tokens": 5}
            ],
            "total_tokens": 9,
            "truncated": false
        });
        let text = assemble_context_text(&value).unwrap();
        assert!(text.contains("## high_salience_beliefs"));
        assert!(text.contains("Bob likes coffee"));
        assert!(text.contains("## recent_episodes"));
        assert!(text.contains("We discussed Rust"));
    }

    #[test]
    fn assemble_context_empty_layers_is_none() {
        let value = serde_json::json!({ "layers": [], "total_tokens": 0 });
        assert!(assemble_context_text(&value).is_none());
    }

    #[test]
    fn config_roundtrip() {
        let cfg = BrainConfig::new("/tmp/x.sock", "work");
        assert_eq!(cfg.socket_path, std::path::PathBuf::from("/tmp/x.sock"));
        assert_eq!(cfg.space, "work");
    }
}
