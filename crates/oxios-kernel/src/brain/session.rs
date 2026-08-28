//! Kernel-owned `oxibrain serve --stdio` session (oxibrain 0.8 daemonless).
//!
//! One lazily-spawned caller-owned child per oxios process for all
//! interactive brain surfaces. Nothing spawns until the first brain call;
//! a dead child is respawned on the next call under a capped backoff; a
//! missing binary optionally triggers one install (30 s rate-limit).
//! Degradation contract: every method returns `None` when the child cannot
//! be spawned or a call fails (RFC-047 §4).

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    #[derive(Debug)]
    struct FakeSpawn {
        spawns: AtomicUsize,
        fail_spawn: AtomicBool,
        /// Shared across spawns so toggling is observed by the in-flight
        /// task and any subsequent respawn (matches the brief's respawn
        /// scenario where the test flips `fail_calls` back to false
        /// before the respawned call).
        fail_calls: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl Spawn for FakeSpawn {
        async fn spawn(&self, _dir: &Path) -> anyhow::Result<BrainClient> {
            self.spawns.fetch_add(1, Ordering::SeqCst);
            if self.fail_spawn.load(Ordering::SeqCst) {
                anyhow::bail!("no binary");
            }
            let (c_req, s_req) = tokio::io::duplex(8192);
            let (mut s_resp, c_resp) = tokio::io::duplex(8192);
            // Share the live `fail_calls` AtomicBool with the spawned
            // task. The brief says "fail_calls is read per spawn"; we
            // satisfy that by re-reading self.fail_calls here AND
            // handing the live handle to the task so toggles between
            // spawns are visible to in-flight calls.
            let fail_w = Arc::clone(&self.fail_calls);
            tokio::spawn(async move {
                let mut reader = BufReader::new(s_req);
                let mut line = String::new();
                loop {
                    line.clear();
                    if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                        break;
                    }
                    let v: serde_json::Value = match serde_json::from_str(&line) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let id = v["id"].clone();
                    let method = v["method"].as_str().unwrap_or_default().to_string();
                    let resp = if fail_w.load(Ordering::SeqCst) && method == "tools/call" {
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":"boom"}})
                    } else {
                        respond(&method, &v)
                    };
                    let mut out = serde_json::to_string(&resp).unwrap();
                    out.push('\n');
                    if s_resp.write_all(out.as_bytes()).await.is_err() {
                        break;
                    }
                }
            });
            Ok(BrainClient::from_io(c_resp, c_req))
        }
    }

    fn respond(method: &str, v: &serde_json::Value) -> serde_json::Value {
        let id = v["id"].clone();
        match method {
            "tools/call" => {
                let name = v["params"]["name"].as_str().unwrap_or_default();
                let text = match name {
                    "recall" => json!({
                        "layers":[{"kind":"profile","text":"remembered: test",
                                   "estimated_tokens":5,"provenance":[]}],
                        "total_tokens":5,"budget":3000,"truncated":false
                    })
                    .to_string(),
                    "ingest" => "ep_1".to_string(),
                    "brief" => "# brief".to_string(),
                    "search" => json!({
                        "memory":[{"entity_id":"e1","entity_surface":"Alice",
                                   "entity_type":"Person","score":0.9,"snippet":"s"}],
                        "documents":[{"document_id":"d1","root":"vault",
                                      "locator":"notes/a.md","revision":"r1",
                                      "ordinal":0,"text":"doc text",
                                      "modified_at":1700000000,"score":0.5}],
                        "freshness":{"reconciled_roots":["vault"],
                                     "skipped_roots":[],"skipped_files":0,
                                     "stale_after_retry":[],"dense_coverage":null}
                    })
                    .to_string(),
                    "traverse" | "review_merges" | "why" | "contradictions" | "stats"
                    | "get_entity" | "timeline" => json!({"ok":true}).to_string(),
                    _ => "{}".to_string(),
                };
                json!({"jsonrpc":"2.0","id":id,
                       "result":{"content":[{"type":"text","text":text}]}})
            }
            "spaces/list" => json!({"jsonrpc":"2.0","id":id,
                "result":{"spaces":[{"id":"sp1","name":"personal","created_at":1,
                                     "episode_count":2,"entity_count":3}]}}),
            "document_history" => json!({"jsonrpc":"2.0","id":id,
                "result":[{"revision":"abc","committed_at_ms":1,"content":"v1"}]}),
            "pending_stats" => json!({"jsonrpc":"2.0","id":id,
                "result":{"count":4,"oldest_seq":9}}),
            "resources/read" => json!({"jsonrpc":"2.0","id":id,
                "result":{"contents":[{"text":"{\"ok\":true}"}]}}),
            _ => json!({"jsonrpc":"2.0","id":id,"result":{}}),
        }
    }

    fn session(fake: Arc<FakeSpawn>) -> BrainSession {
        BrainSession::new(
            Arc::new(BrainInstaller::from_brain_section(
                Path::new("/nonexistent"),
                &crate::config::BrainSection::default(),
            )),
            false,
            BrainConfig::new("/tmp/brain", "personal"),
        )
        .with_spawner(fake)
    }

    #[tokio::test]
    async fn spawns_lazily_and_reuses_client() {
        let fake = Arc::new(FakeSpawn {
            spawns: AtomicUsize::new(0),
            fail_spawn: AtomicBool::new(false),
            fail_calls: Arc::new(AtomicBool::new(false)),
        });
        let s = session(fake.clone());
        assert!(!s.is_available());
        let a = s.recall("query", 3000).await;
        let b = s.recall("query", 3000).await;
        assert!(a.is_some() && b.is_some());
        assert_eq!(fake.spawns.load(Ordering::SeqCst), 1, "one child, reused");
        assert!(s.is_available());
        assert_eq!(s.space(), "personal");
    }

    #[tokio::test]
    async fn spawn_failure_degrades_with_backoff() {
        let fake = Arc::new(FakeSpawn {
            spawns: AtomicUsize::new(0),
            fail_spawn: AtomicBool::new(true),
            fail_calls: Arc::new(AtomicBool::new(false)),
        });
        let s = session(fake.clone());
        assert!(s.recall("q", 100).await.is_none());
        assert!(!s.is_available());
        assert_eq!(fake.spawns.load(Ordering::SeqCst), 1);
        // Within the 1s backoff window: no new spawn attempt.
        assert!(s.recall("q", 100).await.is_none());
        assert_eq!(fake.spawns.load(Ordering::SeqCst), 1);
        assert_eq!(
            SpawnBackoff::delay_for(3),
            std::time::Duration::from_secs(30)
        );
    }

    #[tokio::test]
    async fn call_error_drops_client_and_respawns() {
        let fake = Arc::new(FakeSpawn {
            spawns: AtomicUsize::new(0),
            fail_spawn: AtomicBool::new(false),
            fail_calls: Arc::new(AtomicBool::new(false)),
        });
        let s = session(fake.clone());
        assert!(s.remember("fact", "agent").await.is_some());
        fake.fail_calls.store(true, Ordering::SeqCst);
        // Failing call → degraded + client dropped.
        assert!(s.remember("fact", "agent").await.is_none());
        assert!(!s.is_available());
        // Next call respawns (fresh FakeSpawn read of fail_calls = true per
        // spawn… the toggle is read at spawn time, so flip it back first).
        fake.fail_calls.store(false, Ordering::SeqCst);
        assert!(s.remember("fact", "agent").await.is_some());
        assert_eq!(fake.spawns.load(Ordering::SeqCst), 2, "respawned once");
    }

    #[tokio::test]
    async fn search_returns_two_plane_envelope() {
        let fake = Arc::new(FakeSpawn {
            spawns: AtomicUsize::new(0),
            fail_spawn: AtomicBool::new(false),
            fail_calls: Arc::new(AtomicBool::new(false)),
        });
        let s = session(fake);
        let r = s.search("alice", "hybrid", 20, None).await.unwrap();
        assert_eq!(r.memory.len(), 1);
        assert_eq!(r.documents.len(), 1);
        assert_eq!(r.documents[0].root, "vault");
        assert_eq!(r.freshness.reconciled_roots, vec!["vault".to_string()]);
        let mem_only = s.search("alice", "hybrid", 20, Some(&["memory"])).await;
        assert!(mem_only.is_some());
    }

    #[tokio::test]
    async fn native_methods_work() {
        let fake = Arc::new(FakeSpawn {
            spawns: AtomicUsize::new(0),
            fail_spawn: AtomicBool::new(false),
            fail_calls: Arc::new(AtomicBool::new(false)),
        });
        let s = session(fake);
        let hist = s.document_history("vault", "notes/a.md", 10).await.unwrap();
        assert!(hist.is_array());
        let pending = s.pending_stats().await.unwrap();
        assert_eq!(pending.count, 4);
        assert!(s.spaces().await.is_some());
        assert!(s.space_overview().await.is_some());
    }

    #[tokio::test]
    async fn disabled_session_never_spawns_and_returns_none() {
        // Build from the disabled constructor — no installer, no auto_install,
        // just a config + NoopSpawn behind the scenes.
        let s = BrainSession::disabled(BrainConfig::new("/tmp/brain", "personal"));
        assert!(!s.is_available());
        // Every method must degrade without ever touching the installer.
        assert!(s.recall("q", 100).await.is_none());
        assert!(s.remember("fact", "agent").await.is_none());
        assert!(s.search("q", "hybrid", 5, None).await.is_none());
        assert!(s.pending_stats().await.is_none());
        // Backoff was never armed (the NoopSpawn error is the only path),
        // so the session stays available=false but also stays cheap to
        // re-call — no installer attempt fires.
        assert!(!s.is_available());
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
        let text = super::assemble_context_text(&value).unwrap();
        assert!(text.contains("## high_salience_beliefs"));
        assert!(text.contains("Bob likes coffee"));
        assert!(text.contains("## recent_episodes"));
        assert!(text.contains("We discussed Rust"));
    }

    #[test]
    fn assemble_context_empty_layers_is_none() {
        let value = serde_json::json!({ "layers": [], "total_tokens": 0 });
        assert!(super::assemble_context_text(&value).is_none());
    }

    #[test]
    fn parse_resource_json_extracts_first_content_block() {
        let value = serde_json::json!({
            "contents": [
                {"mimeType": "application/json", "text": "[{\"statement\": \"s1\"}]"}
            ]
        });
        let parsed = super::parse_resource_json(&value).expect("parses");
        assert_eq!(parsed[0]["statement"], "s1");
    }

    #[test]
    fn parse_resource_json_degrades_on_malformed_payload() {
        // No contents key.
        assert!(super::parse_resource_json(&serde_json::json!({})).is_none());
        // Contents without text.
        assert!(super::parse_resource_json(&serde_json::json!({"contents": [{}]})).is_none());
        // Text that is not JSON.
        assert!(
            super::parse_resource_json(&serde_json::json!({"contents": [{"text": "nope"}]}))
                .is_none()
        );
    }
}
use futures::future::BoxFuture;
use oxibrain_client::{
    BrainClient, ClientHello, PendingStatsDto, SearchResponseDto, default_client_hello,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

use super::config::BrainConfig;
use super::installer::BrainInstaller;

/// How the session obtains a client. Production spawns a stdio child;
/// tests attach `BrainClient::from_io` to a scripted responder.
#[async_trait::async_trait]
pub trait Spawn: Send + Sync + std::fmt::Debug {
    /// Build a ready client serving the store at `dir`.
    async fn spawn(&self, dir: &Path) -> anyhow::Result<BrainClient>;
}

/// Spawner stub that always errors. Used when `[brain] enabled = false` so
/// every brain call degrades without ever invoking the installer or the
/// auto-install rate-limiter (RFC-047 §4, kernel config docs).
#[derive(Debug)]
pub(crate) struct NoopSpawn;

#[async_trait::async_trait]
impl Spawn for NoopSpawn {
    async fn spawn(&self, _dir: &Path) -> anyhow::Result<BrainClient> {
        anyhow::bail!("brain disabled — set `[brain] enabled = true` to enable")
    }
}

/// Production spawner: resolve the binary (installing once per 30 s when
/// `auto_install`), `spawn_local`, then `bring_up` (capability handshake).
pub struct ChildSpawn {
    installer: Arc<BrainInstaller>,
    auto_install: bool,
    last_install: Mutex<Option<Instant>>,
}

impl std::fmt::Debug for ChildSpawn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildSpawn")
            .field("installer", &self.installer)
            .field("auto_install", &self.auto_install)
            .finish_non_exhaustive()
    }
}

impl ChildSpawn {
    /// Build a production spawner.
    pub fn new(installer: Arc<BrainInstaller>, auto_install: bool) -> Self {
        Self {
            installer,
            auto_install,
            last_install: Mutex::new(None),
        }
    }

    async fn executable(&self) -> Option<PathBuf> {
        if let Some(b) = self.installer.locate_binary() {
            return Some(b);
        }
        if !self.auto_install {
            return None;
        }
        let mut last = self.last_install.lock().await;
        if last
            .map(|t| t.elapsed() < Duration::from_secs(30))
            .unwrap_or(false)
        {
            return None; // rate-limited — one install attempt per 30 s
        }
        *last = Some(Instant::now());
        match self.installer.install().await {
            Ok(b) => {
                tracing::info!(binary = %b.display(), "installed oxibrain binary");
                Some(b)
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "oxibrain install failed (network, sha256, or archive); \
                     degraded — retry on next call"
                );
                None
            }
        }
    }
}

#[async_trait::async_trait]
impl Spawn for ChildSpawn {
    async fn spawn(&self, dir: &Path) -> anyhow::Result<BrainClient> {
        let exe = self.executable().await.ok_or_else(|| {
            anyhow::anyhow!("oxibrain binary not found — run `oxios brain install`")
        })?;
        let endpoint = oxibrain_client::LocalProcessEndpoint::new(exe, dir);
        let mut client = BrainClient::spawn_local(endpoint).await?;
        let hello: ClientHello = default_client_hello(env!("CARGO_PKG_NAME"));
        client.bring_up(None, hello).await?;
        Ok(client)
    }
}

/// Capped spawn-backoff schedule: [0, 1s, 5s, 30s…].
pub struct SpawnBackoff;

impl SpawnBackoff {
    /// Delay before spawn attempt number `consecutive_failures + 1`.
    pub fn delay_for(consecutive_failures: u32) -> Duration {
        match consecutive_failures {
            0 => Duration::ZERO,
            1 => Duration::from_secs(1),
            2 => Duration::from_secs(5),
            _ => Duration::from_secs(30),
        }
    }
}

/// A live (or degraded) session with the oxibrain stdio child.
#[derive(Debug)]
pub struct BrainSession {
    client: Mutex<Option<BrainClient>>,
    config: BrainConfig,
    available: AtomicBool,
    spawner: Arc<dyn Spawn>,
    failures: AtomicU32,
    next_retry: Mutex<Option<Instant>>,
}

impl BrainSession {
    /// Build a session with the production spawner. Nothing spawns yet.
    pub fn new(installer: Arc<BrainInstaller>, auto_install: bool, config: BrainConfig) -> Self {
        Self::with_spawner_arc(Arc::new(ChildSpawn::new(installer, auto_install)), config)
    }

    /// Build a session whose spawner is `NoopSpawn`. Every brain call
    /// degrades to `None` without invoking the installer or spawning a
    /// child — used when `[brain] enabled = false` (kernel boot).
    pub fn disabled(config: BrainConfig) -> Self {
        Self::with_spawner_arc(Arc::new(NoopSpawn), config)
    }

    fn with_spawner_arc(spawner: Arc<dyn Spawn>, config: BrainConfig) -> Self {
        Self {
            client: Mutex::new(None),
            config,
            available: AtomicBool::new(false),
            spawner,
            failures: AtomicU32::new(0),
            next_retry: Mutex::new(None),
        }
    }

    /// Replace the spawner (test injection). Builder style.
    pub fn with_spawner(self, spawner: Arc<dyn Spawn>) -> Self {
        Self::with_spawner_arc(spawner, self.config)
    }

    /// Whether a child is currently connected. Cheap lock-free read.
    pub fn is_available(&self) -> bool {
        self.available.load(Ordering::Relaxed)
    }

    /// The configured space name.
    pub fn space(&self) -> &str {
        &self.config.space
    }

    /// The brain data directory.
    pub fn dir(&self) -> &Path {
        &self.config.dir
    }

    // ── Agent-runtime methods ─────────────────────────────────────────

    /// Assemble recall context for an agent turn within `budget` tokens.
    pub async fn recall(&self, query: &str, budget: usize) -> Option<String> {
        let query = query.to_string();
        let space = self.config.space.clone();
        let value = self
            .call(move |c| Box::pin(async move { c.recall(&query, &space, budget).await }))
            .await?;
        let text = assemble_context_text(&value);
        if text.is_some() {
            crate::metrics::get_metrics().oxibrain_recall_total.inc();
        }
        text
    }

    /// Remember content as an episode; returns the episode id. Extraction
    /// is deferred to the backlog drain (the MCP surface cannot extract
    /// inline without a sampling session).
    pub async fn remember(&self, content: &str, source: &str) -> Option<String> {
        let content = content.to_string();
        let source = source.to_string();
        let space = self.config.space.clone();
        self.call(move |c| Box::pin(async move { c.ingest(&content, &space, &source).await }))
            .await
    }

    // ── Web API methods ───────────────────────────────────────────────

    /// Two-plane search (`memory` + `documents`); `planes=None` → both.
    pub async fn search(
        &self,
        query: &str,
        mode: &str,
        limit: usize,
        planes: Option<&[&str]>,
    ) -> Option<SearchResponseDto> {
        let query = query.to_string();
        let mode = mode.to_string();
        let space = self.config.space.clone();
        let planes = planes.map(|p| p.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        self.call(move |c| {
            Box::pin(async move {
                match planes {
                    Some(p) => {
                        let refs: Vec<&str> = p.iter().map(String::as_str).collect();
                        c.search_planes(&query, &space, &mode, limit, &refs).await
                    }
                    None => c.search(&query, &space, &mode, limit).await,
                }
            })
        })
        .await
    }

    /// An entity's current beliefs.
    pub async fn get_entity(&self, entity_id: &str) -> Option<Value> {
        let id = entity_id.to_string();
        let space = self.config.space.clone();
        self.call(move |c| Box::pin(async move { c.get_entity(&id, &space).await }))
            .await
    }

    /// Belief intervals for an entity over a time range (epoch ms).
    pub async fn timeline(
        &self,
        entity_id: &str,
        from: Option<i64>,
        to: Option<i64>,
    ) -> Option<Value> {
        let id = entity_id.to_string();
        let space = self.config.space.clone();
        self.call(move |c| Box::pin(async move { c.timeline(&id, &space, from, to).await }))
            .await
    }

    /// Provenance and confidence breakdown for a statement.
    pub async fn why(&self, statement_id: &str) -> Option<Value> {
        let id = statement_id.to_string();
        let space = self.config.space.clone();
        self.call(move |c| Box::pin(async move { c.why(&id, &space).await }))
            .await
    }

    /// List contradicted statements in the space.
    pub async fn contradictions(&self) -> Option<Value> {
        let space = self.config.space.clone();
        self.call(move |c| Box::pin(async move { c.contradictions(&space).await }))
            .await
    }

    /// Aggregate counts for the space.
    pub async fn stats(&self) -> Option<Value> {
        let space = self.config.space.clone();
        self.call(move |c| Box::pin(async move { c.stats(&space).await }))
            .await
    }

    // ── Web console methods (passthrough) ─────────────────────────────

    /// Render a readable page as Markdown (`entity` / `space` / `topic`).
    pub async fn brief(
        &self,
        target_kind: &str,
        entity_id: Option<&str>,
        topic: Option<&str>,
    ) -> Option<String> {
        let args = json!({
            "target_kind": target_kind,
            "entity_id": entity_id,
            "topic": topic,
            "space": self.config.space,
        });
        self.call(move |c| Box::pin(async move { c.call_tool("brief", args).await }))
            .await
    }

    /// Bounded belief-filtered subgraph traversal.
    pub async fn traverse(
        &self,
        start: &[String],
        depth: usize,
        max_nodes: usize,
        direction: &str,
    ) -> Option<Value> {
        let args = json!({
            "start": start,
            "depth": depth,
            "max_nodes": max_nodes,
            "direction": direction,
            "space": self.config.space,
        });
        self.call(move |c| Box::pin(async move { c.call_tool_json("traverse", args).await }))
            .await
    }

    /// Console data: `merges`, `failures`, or `sources`.
    pub async fn review_merges(&self, section: &str) -> Option<Value> {
        let args = json!({ "section": section, "space": self.config.space });
        self.call(move |c| Box::pin(async move { c.call_tool_json("review_merges", args).await }))
            .await
    }

    /// Spaces the server exposes (name, counts, created_at).
    pub async fn spaces(&self) -> Option<Value> {
        let list = self
            .call(move |c| Box::pin(async move { c.list_spaces().await }))
            .await?;
        Some(serde_json::to_value(list).unwrap_or(Value::Null))
    }

    /// Overview of the configured space (`space://{name}` resource).
    pub async fn space_overview(&self) -> Option<Value> {
        let uri = format!("space://{}", self.config.space);
        self.read_resource_json(&uri).await
    }

    /// gix-backed revision history of one tracked document.
    pub async fn document_history(
        &self,
        alias: &str,
        locator: &str,
        limit: usize,
    ) -> Option<Value> {
        let alias = alias.to_string();
        let locator = locator.to_string();
        let space = self.config.space.clone();
        let list = self
            .call(move |c| {
                Box::pin(async move { c.document_history(&space, &alias, &locator, limit).await })
            })
            .await?;
        Some(serde_json::to_value(list).unwrap_or(Value::Null))
    }

    /// Memory-plane extraction backlog stats.
    pub async fn pending_stats(&self) -> Option<PendingStatsDto> {
        self.call(move |c| Box::pin(async move { c.pending_stats().await }))
            .await
    }

    /// Read one MCP resource and parse its first content block as JSON.
    async fn read_resource_json(&self, uri: &str) -> Option<Value> {
        let uri = uri.to_string();
        let value = self
            .call(move |c| {
                Box::pin(async move {
                    c.call_rpc_json("resources/read", json!({ "uri": uri }))
                        .await
                })
            })
            .await?;
        parse_resource_json(&value)
    }

    // ── Plumbing ──────────────────────────────────────────────────────

    /// Run `f` against the session child: spawn when absent (subject to
    /// backoff), degrade to `None` when the spawn fails; on a call error
    /// drop the client (`kill_on_drop` reaps the child) so the next call
    /// respawns.
    async fn call<F, T>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&mut BrainClient) -> BoxFuture<'_, anyhow::Result<T>>,
    {
        let mut guard = self.client.lock().await;
        if guard.is_none() && !self.try_spawn(&mut guard).await {
            return None;
        }
        let client = guard.as_mut()?;
        match f(client).await {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!(error = %e, "brain call failed — dropping session child");
                *guard = None;
                self.available.store(false, Ordering::Relaxed);
                crate::metrics::get_metrics().oxibrain_available.set(0.0);
                None
            }
        }
    }

    /// Single spawn attempt inside a held lock. `true` when a client exists.
    async fn try_spawn(&self, guard: &mut Option<BrainClient>) -> bool {
        if let Some(next) = *self.next_retry.lock().await
            && tokio::time::Instant::now() < next
        {
            return false; // inside a backoff window
        }
        match self.spawner.spawn(&self.config.dir).await {
            Ok(c) => {
                tracing::info!(dir = %self.config.dir.display(), "oxibrain session child up");
                *guard = Some(c);
                self.available.store(true, Ordering::Relaxed);
                self.failures.store(0, Ordering::Relaxed);
                *self.next_retry.lock().await = None;
                crate::metrics::get_metrics().oxibrain_available.set(1.0);
                true
            }
            Err(e) => {
                let n = self.failures.fetch_add(1, Ordering::SeqCst) + 1;
                let delay = SpawnBackoff::delay_for(n);
                *self.next_retry.lock().await = Some(tokio::time::Instant::now() + delay);
                tracing::warn!(error = %e, fails = n, ?delay,
                    "brain session spawn failed — degraded");
                self.available.store(false, Ordering::Relaxed);
                crate::metrics::get_metrics().oxibrain_available.set(0.0);
                false
            }
        }
    }
}

/// Join a `recall` (ContextResult) payload into a single text block.
fn assemble_context_text(value: &Value) -> Option<String> {
    let layers = value.get("layers")?.as_array()?;
    let mut out = String::new();
    for layer in layers {
        let kind = layer
            .get("kind")
            .and_then(|k| k.as_str())
            .unwrap_or("context");
        let text = layer.get("text").and_then(|t| t.as_str()).unwrap_or("");
        if !text.is_empty() {
            out.push_str(&format!("## {kind}\n{text}\n\n"));
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Parse a `resources/read` result: `{contents: [{text: "<json>"}]}`.
fn parse_resource_json(value: &Value) -> Option<Value> {
    let text = value
        .get("contents")?
        .as_array()?
        .first()?
        .get("text")?
        .as_str()?;
    serde_json::from_str(text).ok()
}
