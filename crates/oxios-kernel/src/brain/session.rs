//! One-process-per-operation brain transport (oxibrain 0.10 agent-first CLI).
//!
//! Nothing resident — every interaction spawns a short-lived process:
//!
//! - **Agent-facing ops** ([`BrainSession::recall`], [`BrainSession::remember`],
//!   [`BrainSession::search`], [`BrainSession::brief`], [`BrainSession::traverse`],
//!   [`BrainSession::why`], [`BrainSession::contradictions`]) run as one-shot
//!   `oxibrain --dir <dir> <op> --json -` subprocesses. The payload goes over
//!   stdin (spec `agent-first-cli-v1` §2: prose bodies never travel as argv)
//!   and stdout carries the stable envelope `{api, ok, op, space?, data, meta}`
//!   with machine-meaningful exit codes (§3).
//! - **Console-only reads** that the 14-op CLI surface does not expose
//!   (native `stats`, `document_history`, `pending_stats`, `spaces/list`,
//!   and the `entity://` / `timeline://` / `space://` resources) speak
//!   JSON-RPC over a caller-owned `serve --stdio` child that is spawned
//!   per call and reaped when the call returns.
//!
//! Degradation contract (RFC-047 §4): every method returns `None` when the
//! binary is missing, a process fails, or an op errors — the agent turn
//! continues normally. A well-formed envelope proves the transport is alive
//! even when the op itself failed, so availability tracks the transport.

use futures::future::BoxFuture;
use oxibrain_client::{
    BrainClient, ClientHello, LocalProcessEndpoint, PendingStatsDto, SearchResponseDto,
    default_client_hello,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use super::config::BrainConfig;
use super::installer::BrainInstaller;

/// Upper bound on one CLI op process. Agent ops are local SQLite work and
/// inline extraction is opt-in (`extract: true`), so 90 s only bites on
/// pathological lock contention or a wedged binary.
const OP_TIMEOUT: Duration = Duration::from_secs(90);

/// How the console path obtains a client. Production spawns a `serve
/// --stdio` child per call; tests attach [`BrainClient::from_io`] to a
/// scripted responder.
#[async_trait::async_trait]
pub trait Spawn: Send + Sync + std::fmt::Debug {
    /// Build a ready client for the store at `dir`. The executable has
    /// already been resolved (and installed when allowed).
    async fn spawn(&self, exe: &Path, dir: &Path) -> anyhow::Result<BrainClient>;
}

/// Production child transport: `spawn_local` + capability handshake, one
/// child per call. `kill_on_drop` semantics apply — the child dies with the
/// returned client.
#[derive(Debug)]
pub struct StdioChild;

#[async_trait::async_trait]
impl Spawn for StdioChild {
    async fn spawn(&self, exe: &Path, dir: &Path) -> anyhow::Result<BrainClient> {
        let endpoint = LocalProcessEndpoint::new(exe, dir);
        let mut client = BrainClient::spawn_local(endpoint).await?;
        let hello: ClientHello = default_client_hello(env!("CARGO_PKG_NAME"));
        client.bring_up(None, hello).await?;
        Ok(client)
    }
}

/// A degraded-or-live brain connection built entirely from short-lived
/// processes. Cheap to clone via [`Arc`]; no background tasks, no respawn
/// bookkeeping.
#[derive(Debug)]
pub struct BrainSession {
    installer: Arc<BrainInstaller>,
    auto_install: bool,
    /// `false` when `[brain] enabled = false` — every call degrades without
    /// touching the installer or spawning anything (RFC-047 §4, kernel docs).
    enabled: bool,
    available: AtomicBool,
    config: BrainConfig,
    /// One managed install attempt per 30 s when `auto_install` is set.
    last_install: Mutex<Option<Instant>>,
    child: Arc<dyn Spawn>,
}

impl BrainSession {
    /// Build a session with the production transports. Nothing spawns yet.
    pub fn new(installer: Arc<BrainInstaller>, auto_install: bool, config: BrainConfig) -> Self {
        Self::with_child_arc(Arc::new(StdioChild), installer, auto_install, config)
    }

    /// Build a disabled session. Every brain call degrades to `None`
    /// without invoking the installer or spawning a process — used when
    /// `[brain] enabled = false` (kernel boot).
    pub fn disabled(config: BrainConfig) -> Self {
        Self::with_child_arc(Arc::new(StdioChild), disabled_installer(), false, config).disable()
    }

    fn with_child_arc(
        child: Arc<dyn Spawn>,
        installer: Arc<BrainInstaller>,
        auto_install: bool,
        config: BrainConfig,
    ) -> Self {
        Self {
            installer,
            auto_install,
            enabled: true,
            available: AtomicBool::new(false),
            config,
            last_install: Mutex::new(None),
            child,
        }
    }

    fn disable(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Replace the child transport (test injection). Builder style.
    pub fn with_spawner(self, child: Arc<dyn Spawn>) -> Self {
        Self { child, ..self }
    }

    /// Whether the last transport interaction succeeded. Cheap lock-free read.
    pub fn is_available(&self) -> bool {
        self.available.load(Ordering::Relaxed)
    }

    /// The brain data directory.
    pub fn dir(&self) -> &Path {
        &self.config.dir
    }

    // ── Agent-runtime methods (CLI op dispatch) ───────────────────────

    /// Assemble recall context for an agent turn within `budget` tokens.
    pub async fn recall(&self, space: &str, query: &str, budget: usize) -> Option<String> {
        let data = self
            .run_op(
                "recall",
                json!({
                    "query": query,
                    "space": space,
                    "token_budget": budget,
                }),
            )
            .await?;
        let text = assemble_context_text(&data);
        if text.is_some() {
            crate::metrics::get_metrics().oxibrain_recall_total.inc();
        }
        text
    }

    /// Remember content as an episode; returns the episode id. Extraction
    /// is deferred to the backlog drain (`admin extract --pending`, the
    /// kernel's 600 s timer) — the CLI one-shot has no sampling client
    /// (spec §8: `extraction: "pending"` is success, not completion).
    pub async fn remember(&self, space: &str, content: &str, source: &str) -> Option<String> {
        let data = self
            .run_op(
                "ingest",
                json!({
                    "content": content,
                    "space": space,
                    "source_path": source,
                    // Ride a short writer lock instead of failing the write
                    // on first contention; still bounded (spec §6).
                    "wait_lock_ms": 2_000,
                }),
            )
            .await?;
        episode_id_from_ingest(&data)
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
        let mut payload = json!({
            "query": query,
            "space": space,
            "mode": mode,
            "limit": limit,
        });
        if let Some(planes) = planes {
            payload["planes"] = json!(planes);
        }
        let data = self.run_op("search", payload).await?;
        serde_json::from_value(data).ok()
    }

    /// Render a readable page as Markdown (`entity` / `space` / `topic`).
    pub async fn brief(
        &self,
        space: &str,
        target_kind: &str,
        entity_id: Option<&str>,
        topic: Option<&str>,
    ) -> Option<String> {
        let mut payload = json!({ "target_kind": target_kind, "space": space });
        if let Some(id) = entity_id {
            payload["entity_id"] = json!(id);
        }
        if let Some(topic) = topic {
            payload["topic"] = json!(topic);
        }
        let data = self.run_op("brief", payload).await?;
        data.as_str().map(str::to_string)
    }

    /// Bounded belief-filtered subgraph traversal.
    pub async fn traverse(
        &self,
        space: &str,
        start: &[String],
        depth: usize,
        max_nodes: usize,
        direction: &str,
    ) -> Option<Value> {
        self.run_op(
            "traverse",
            json!({
                "start": start,
                "depth": depth,
                "max_nodes": max_nodes,
                "direction": direction,
                "space": space,
            }),
        )
        .await
    }

    /// Provenance and confidence breakdown for a statement.
    pub async fn why(&self, space: &str, statement_id: &str) -> Option<Value> {
        self.run_op(
            "why",
            json!({ "statement_id": statement_id, "space": space }),
        )
        .await
    }

    /// List contradicted statements in the space.
    pub async fn contradictions(&self, space: &str) -> Option<Value> {
        self.run_op("contradictions", json!({ "space": space }))
            .await
    }

    // ── Web console methods (per-call stdio child) ────────────────────

    /// An entity's current beliefs (`entity://` resource — the CLI op set
    /// has no entity-detail op by design; ids are never guessed).
    pub async fn get_entity(&self, space: &str, entity_id: &str) -> Option<Value> {
        let uri = format!("entity://{}?space={}", entity_id, space);
        self.read_resource_json(&uri).await
    }

    /// Belief intervals for an entity over a time range (epoch ms).
    pub async fn timeline(
        &self,
        space: &str,
        entity_id: &str,
        from: Option<i64>,
        to: Option<i64>,
    ) -> Option<Value> {
        let mut uri = format!("timeline://{}?space={}", entity_id, space);
        if let Some(from) = from {
            uri.push_str(&format!("&from={from}"));
        }
        if let Some(to) = to {
            uri.push_str(&format!("&to={to}"));
        }
        self.read_resource_json(&uri).await
    }

    /// Aggregate counts for the space (native JSON-RPC `stats` — the tool
    /// moved off the MCP surface in v2.13; console data stays native).
    pub async fn stats(&self, space: &str) -> Option<Value> {
        self.with_child_client(|c| {
            let space = space.to_string();
            Box::pin(async move { c.call_rpc_json("stats", json!({ "space": space })).await })
        })
        .await
    }

    /// Spaces the server exposes (name, counts, created_at).
    pub async fn spaces(&self) -> Option<Value> {
        let list = self
            .with_child_client(|c| Box::pin(async move { c.list_spaces().await }))
            .await?;
        serde_json::to_value(list).ok()
    }

    /// Overview of one space (`space://{name}` resource).
    pub async fn space_overview(&self, space: &str) -> Option<Value> {
        let uri = format!("space://{}", space);
        self.read_resource_json(&uri).await
    }

    /// gix-backed revision history of one tracked document.
    pub async fn document_history(
        &self,
        space: &str,
        alias: &str,
        locator: &str,
        limit: usize,
    ) -> Option<Value> {
        let list = self
            .with_child_client(|c| {
                let space = space.to_string();
                let alias = alias.to_string();
                let locator = locator.to_string();
                Box::pin(async move { c.document_history(&space, &alias, &locator, limit).await })
            })
            .await?;
        serde_json::to_value(list).ok()
    }

    /// Memory-plane extraction backlog stats.
    pub async fn pending_stats(&self) -> Option<PendingStatsDto> {
        self.with_child_client(|c| Box::pin(async move { c.pending_stats().await }))
            .await
    }

    /// Console merges/failures/sources panels. oxibrain 0.10 removed the
    /// `review_merges` tool (v2.13 slot accounting) and its `admin review`
    /// replacement is not shipped yet — degrade to `None` (the web console
    /// renders empty panels) until upstream lands it (spec agent-first-cli
    /// §10). Revisit on the next oxibrain bump.
    pub async fn review_merges(&self, _space: &str, _section: &str) -> Option<Value> {
        None
    }

    // ── Plumbing ──────────────────────────────────────────────────────

    /// Run one op end to end: resolve the binary (installing once per 30 s
    /// when `auto_install`), spawn `oxibrain --dir D <op> --json -`, feed
    /// the payload over stdin, and parse the response envelope. `None` on
    /// any transport failure or op error.
    async fn run_op(&self, op: &str, payload: Value) -> Option<Value> {
        if !self.enabled {
            return None;
        }
        let Some(exe) = self.executable().await else {
            self.set_available(false);
            return None;
        };
        let mut child = match tokio::process::Command::new(&exe)
            .arg("--dir")
            .arg(self.config.dir.clone())
            .arg(op)
            .args(["--json", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(op, error = %e, "brain op spawn failed — degraded");
                self.set_available(false);
                return None;
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(payload.to_string().as_bytes()).await;
            let _ = stdin.shutdown().await;
        }
        let out = match tokio::time::timeout(OP_TIMEOUT, child.wait_with_output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                tracing::warn!(op, error = %e, "brain op process failed — degraded");
                self.set_available(false);
                return None;
            }
            Err(_) => {
                tracing::warn!(op, timeout = ?OP_TIMEOUT, "brain op timed out — degraded");
                return None; // kill_on_drop reaps the child
            }
        };
        let envelope: serde_json::Value = match serde_json::from_slice(&out.stdout) {
            Ok(v) => v,
            Err(_) => {
                tracing::warn!(
                    op,
                    stdout = %String::from_utf8_lossy(&out.stdout),
                    stderr = %String::from_utf8_lossy(&out.stderr),
                    "brain op printed no envelope — degraded"
                );
                self.set_available(false);
                return None;
            }
        };
        // A well-formed envelope proves the transport works, even when the
        // op itself failed (not_found, locked, …).
        self.set_available(true);
        if envelope.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Some(unwrap_tool_data(
                envelope.get("data").cloned().unwrap_or(Value::Null),
            ));
        }
        let error = envelope.get("error").cloned().unwrap_or(Value::Null);
        let code = error.get("code").and_then(Value::as_str).unwrap_or("?");
        let message = error.get("message").and_then(Value::as_str).unwrap_or("?");
        tracing::warn!(op, code, message, "brain op failed");
        None
    }

    /// Run `f` against a fresh `serve --stdio` child: spawn, handshake,
    /// call, reap. No respawn logic — a failed call simply degrades and
    /// the next call starts a new child.
    async fn with_child_client<F, T>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&mut BrainClient) -> BoxFuture<'_, anyhow::Result<T>>,
    {
        if !self.enabled {
            return None;
        }
        let Some(exe) = self.executable().await else {
            self.set_available(false);
            return None;
        };
        let mut client = match self.child.spawn(&exe, &self.config.dir).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "brain child spawn failed — degraded");
                self.set_available(false);
                return None;
            }
        };
        match f(&mut client).await {
            Ok(v) => {
                self.set_available(true);
                Some(v)
            }
            Err(e) => {
                tracing::warn!(error = %e, "brain child call failed — degraded");
                self.set_available(false);
                None
            }
        }
    }

    /// Read one MCP resource and parse its first content block as JSON.
    async fn read_resource_json(&self, uri: &str) -> Option<Value> {
        let value = self
            .with_child_client(|c| {
                let uri = uri.to_string();
                Box::pin(async move {
                    c.call_rpc_json("resources/read", json!({ "uri": uri }))
                        .await
                })
            })
            .await?;
        parse_resource_json(&value)
    }

    /// Resolve the binary, installing once per 30 s when `auto_install`.
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

    fn set_available(&self, up: bool) {
        self.available.store(up, Ordering::Relaxed);
        crate::metrics::get_metrics()
            .oxibrain_available
            .set(if up { 1.0 } else { 0.0 });
    }
}

/// Installer stand-in for [`BrainSession::disabled`] — never consulted,
/// because `enabled = false` short-circuits before `executable()`.
fn disabled_installer() -> Arc<BrainInstaller> {
    Arc::new(BrainInstaller::from_brain_section(
        Path::new("/nonexistent"),
        &crate::config::BrainSection::default(),
    ))
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

/// Extract the episode id from an `ingest` response. The 0.10 CLI answers
/// in prose ("Ingested as episode: <id> (…)"); prefer a structured
/// `episode_id` field when upstream starts returning one.
fn episode_id_from_ingest(data: &Value) -> Option<String> {
    if let Some(id) = data.get("episode_id").and_then(Value::as_str) {
        return (!id.is_empty()).then(|| id.to_string());
    }
    let text = data.as_str()?;
    let rest = text.strip_prefix("Ingested as episode: ")?;
    let id = rest.split(" (").next().unwrap_or(rest).trim();
    (!id.is_empty()).then(|| id.to_string())
}

/// Unwrap the tool-result envelope. Structured tools (search, recall, …)
/// answer `{data, meta}` (mirroring the MCP `call_tool_data` contract);
/// plain-text tools (ingest prose, brief markdown) surface as bare
/// strings. An object carrying both keys is unwrapped; anything else is
/// the payload itself.
fn unwrap_tool_data(data: Value) -> Value {
    if let (Some(_), Some(_)) = (data.get("data"), data.get("meta")) {
        return data["data"].clone();
    }
    data
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    // ── op transport: a stub `oxibrain` binary emitting canned envelopes ──

    /// Write a stub binary that consumes stdin and prints `stdout_body`.
    fn stub_binary(body: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "oxios-brain-op-stub-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(
            &path,
            format!("#!/bin/sh\ncat >/dev/null\nprintf '%s' '{body}'\n"),
        )
        .unwrap();
        let mut perm = std::fs::metadata(&path).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        perm.set_mode(0o755);
        std::fs::set_permissions(&path, perm).unwrap();
        path
    }

    fn op_session(body: &str) -> (BrainSession, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let section = crate::config::BrainSection {
            binary_path: stub_binary(body).display().to_string(),
            ..crate::config::BrainSection::default()
        };
        let session = BrainSession::new(
            Arc::new(BrainInstaller::from_brain_section(tmp.path(), &section)),
            false,
            BrainConfig::new(tmp.path().join("brain")),
        );
        (session, tmp)
    }

    fn ok_envelope(data: &Value) -> String {
        json!({ "api": 1, "ok": true, "op": "op", "data": data, "meta": {} }).to_string()
    }

    #[tokio::test]
    async fn recall_assembles_envelope_layers() {
        let data = json!({
            "layers": [{"kind": "profile", "text": "remembered: test",
                        "estimated_tokens": 5, "provenance": []}]
        });
        let (s, _tmp) = op_session(&ok_envelope(&data));
        let text = s.recall("personal", "query", 3000).await.unwrap();
        assert!(text.contains("## profile"));
        assert!(text.contains("remembered: test"));
        assert!(s.is_available());
    }

    /// The per-call `space` argument — not the boot-time `BrainConfig`
    /// space — is what reaches the CLI payload.
    #[tokio::test]
    async fn recall_payload_carries_passed_space() {
        // Capture the stdin payload so the test can assert the wire shape.
        let payload_path =
            std::env::temp_dir().join(format!("oxios-brain-recall-payload-{}", std::process::id()));
        let _ = std::fs::remove_file(&payload_path);
        let stub =
            std::env::temp_dir().join(format!("oxios-brain-capture-stub-{}", std::process::id()));
        std::fs::write(
            &stub,
            format!(
                "#!/bin/sh\ncat > '{}'\nprintf '%s' '{}'\n",
                payload_path.display(),
                json!({ "api": 1, "ok": true, "op": "recall", "data": {}, "meta": {} })
            ),
        )
        .unwrap();
        let mut perm = std::fs::metadata(&stub).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        perm.set_mode(0o755);
        std::fs::set_permissions(&stub, perm).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let section = crate::config::BrainSection {
            binary_path: stub.display().to_string(),
            ..crate::config::BrainSection::default()
        };
        let s = BrainSession::new(
            Arc::new(BrainInstaller::from_brain_section(tmp.path(), &section)),
            false,
            // No boot-time binding: the op call itself carries the space.
            BrainConfig::new(tmp.path().join("brain")),
        );
        s.recall("work", "hello", 50).await;

        let payload: Value =
            serde_json::from_str(&std::fs::read_to_string(&payload_path).unwrap()).unwrap();
        assert_eq!(payload["space"], "work");
        assert_eq!(payload["query"], "hello");
        assert_eq!(payload["token_budget"], 50);
    }

    #[tokio::test]
    async fn remember_reads_prose_and_structured_ids() {
        let prose = ok_envelope(&json!(
            "Ingested as episode: ep_1 (extraction skipped: no sampling session)"
        ));
        let (s, _tmp) = op_session(&prose);
        assert_eq!(
            s.remember("personal", "fact", "agent").await.as_deref(),
            Some("ep_1")
        );

        let structured = ok_envelope(&json!({ "episode_id": "ep_9" }));
        let (s, _tmp) = op_session(&structured);
        assert_eq!(
            s.remember("personal", "fact", "agent").await.as_deref(),
            Some("ep_9")
        );
    }

    #[tokio::test]
    async fn search_parses_two_plane_envelope() {
        let data = json!({
            "memory": [{"entity_id": "e1", "entity_surface": "Alice",
                        "entity_type": "Person", "score": 0.9, "snippet": "s"}],
            "documents": [{"document_id": "d1", "root": "vault",
                           "locator": "notes/a.md", "revision": "r1",
                           "ordinal": 0,
                           "text": {"kind": "untrusted_content", "text": "doc text",
                                    "provenance": {"ref": "doc://x", "trust": "unverified"}},
                           "modified_at": 1700000000, "score": 0.5}],
            "freshness": {"reconciled_roots": ["vault"], "skipped_roots": [],
                          "skipped_files": 0, "stale_after_retry": [],
                          "dense_coverage": null}
        });
        let (s, _tmp) = op_session(&ok_envelope(&data));
        let hit = s.search("personal", "q", "hybrid", 5, None).await.unwrap();
        assert_eq!(hit.memory.len(), 1);
        assert_eq!(hit.documents.len(), 1);
        assert_eq!(hit.freshness.reconciled_roots, vec!["vault".to_string()]);
    }

    /// Real 0.10 shape (verified against the stock binary): structured
    /// tools nest the payload as `data.data` next to a `meta` block.
    #[tokio::test]
    async fn search_unwraps_nested_tool_envelope() {
        let body = json!({
            "api": 1, "ok": true, "op": "search", "space": "personal",
            "data": {
                "data": {
                    "memory": [],
                    "documents": [],
                    "freshness": {"reconciled_roots": [], "skipped_roots": [],
                                  "skipped_files": 0, "stale_after_retry": [],
                                  "dense_coverage": null}
                },
                "meta": {"dropped": [{"count": 0, "reason": "below_confidence_or_truncated"}]}
            },
            "meta": {"elapsed_ms": 10}
        })
        .to_string();
        let (s, _tmp) = op_session(&body);
        let hit = s.search("personal", "q", "hybrid", 5, None).await.unwrap();
        assert_eq!(hit.memory.len(), 0);
        assert_eq!(hit.freshness.skipped_files, 0);
    }

    #[tokio::test]
    async fn error_envelope_degrades_but_transport_stays_up() {
        let body = json!({
            "api": 1, "ok": false, "op": "ingest",
            "error": {"code": "locked", "message": "store locked", "retryable": true}
        })
        .to_string();
        let (s, _tmp) = op_session(&body);
        assert!(s.remember("personal", "fact", "agent").await.is_none());
        // The envelope came back: the binary works, the op did not.
        assert!(s.is_available());
    }

    #[tokio::test]
    async fn garbage_stdout_degrades() {
        let (s, _tmp) = op_session("not json at all");
        assert!(s.contradictions("personal").await.is_none());
        assert!(!s.is_available());
    }

    #[tokio::test]
    async fn missing_binary_degrades_fast() {
        let tmp = tempfile::tempdir().unwrap();
        let s = BrainSession::new(
            Arc::new(BrainInstaller::from_brain_section(
                tmp.path(),
                &crate::config::BrainSection::default(),
            )),
            false,
            BrainConfig::new(tmp.path().join("brain")),
        );
        assert!(s.recall("personal", "q", 10).await.is_none());
        assert!(s.stats("personal").await.is_none());
        assert!(!s.is_available());
    }

    #[tokio::test]
    async fn why_and_traverse_pass_data_through() {
        let (s, _tmp) = op_session(&ok_envelope(&json!({ "statement_id": "st1" })));
        assert_eq!(
            s.why("personal", "st1").await.unwrap()["statement_id"],
            "st1"
        );
        let (s, _tmp) = op_session(&ok_envelope(&json!({ "nodes": [], "edges": [] })));
        assert!(
            s.traverse("personal", &["e1".into()], 2, 64, "both")
                .await
                .is_some()
        );
        let (s, _tmp) = op_session(&ok_envelope(&json!("# page")));
        assert_eq!(
            s.brief("personal", "entity", Some("e1"), None)
                .await
                .as_deref(),
            Some("# page")
        );
    }

    // ── console path: scripted JSON-RPC responder over in-memory pipes ──

    #[derive(Debug)]
    struct FakeSpawn {
        spawns: AtomicUsize,
        fail_spawn: AtomicBool,
    }

    #[async_trait::async_trait]
    impl Spawn for FakeSpawn {
        async fn spawn(&self, _exe: &Path, _dir: &Path) -> anyhow::Result<BrainClient> {
            self.spawns.fetch_add(1, Ordering::SeqCst);
            if self.fail_spawn.load(Ordering::SeqCst) {
                anyhow::bail!("no binary");
            }
            let (c_req, s_req) = tokio::io::duplex(8192);
            let (mut s_resp, c_resp) = tokio::io::duplex(8192);
            tokio::spawn(async move {
                let mut reader = BufReader::new(s_req);
                let mut line = String::new();
                loop {
                    line.clear();
                    if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                        break;
                    }
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                        continue;
                    };
                    let mut out = serde_json::to_string(&respond(&v)).expect("serialize response");
                    out.push('\n');
                    if s_resp.write_all(out.as_bytes()).await.is_err() {
                        break;
                    }
                }
            });
            Ok(BrainClient::from_io(c_resp, c_req))
        }
    }

    fn respond(v: &serde_json::Value) -> serde_json::Value {
        let id = v["id"].clone();
        let method = v["method"].as_str().unwrap_or_default();
        match method {
            "stats" => json!({"jsonrpc":"2.0","id":id,
                "result":{"episodes":7,"entities":3,"statements":12}}),
            "spaces/list" => json!({"jsonrpc":"2.0","id":id,
                "result":{"spaces":[{"id":"sp1","name":"personal","created_at":1,
                                     "episode_count":2,"entity_count":3}]}}),
            "document_history" => json!({"jsonrpc":"2.0","id":id,
                "result":[{"revision":"abc","committed_at_ms":1,"content":"v1"}]}),
            "pending_stats" => json!({"jsonrpc":"2.0","id":id,
                "result":{"count":4,"oldest_seq":9}}),
            "resources/read" => {
                let uri = v["params"]["uri"].as_str().unwrap_or_default();
                let text = if uri.starts_with("entity://") {
                    json!([{"statement":"s1","confidence":0.9}]).to_string()
                } else if uri.starts_with("timeline://") {
                    json!([{"statement_id":"s1","valid_from":1}]).to_string()
                } else {
                    json!({"space":"personal","entity_count":3}).to_string()
                };
                json!({"jsonrpc":"2.0","id":id,
                       "result":{"contents":[{"text":text}]}})
            }
            _ => json!({"jsonrpc":"2.0","id":id,"result":{}}),
        }
    }

    /// A dummy explicit binary so `executable()` resolves; the fake child
    /// transport never actually runs it.
    fn dummy_binary() -> PathBuf {
        let path = std::env::temp_dir().join("oxios-brain-child-stub");
        if !path.exists() {
            std::fs::write(&path, b"").unwrap();
        }
        path
    }

    fn child_session(fail_spawn: bool) -> (BrainSession, Arc<FakeSpawn>) {
        let tmp = tempfile::tempdir().unwrap();
        let section = crate::config::BrainSection {
            binary_path: dummy_binary().display().to_string(),
            ..crate::config::BrainSection::default()
        };
        let fake = Arc::new(FakeSpawn {
            spawns: AtomicUsize::new(0),
            fail_spawn: AtomicBool::new(fail_spawn),
        });
        let s = BrainSession::new(
            Arc::new(BrainInstaller::from_brain_section(tmp.path(), &section)),
            false,
            BrainConfig::new(tmp.path().join("brain")),
        )
        .with_spawner(Arc::clone(&fake) as Arc<dyn Spawn>);
        (s, fake)
    }

    #[tokio::test]
    async fn console_native_surfaces_answer_over_per_call_child() {
        let (s, fake) = child_session(false);
        assert_eq!(fake.spawns.load(Ordering::SeqCst), 0, "nothing resident");

        let stats = s.stats("personal").await.unwrap();
        assert_eq!(stats["episodes"], 7);
        let hist = s
            .document_history("personal", "vault", "notes/a.md", 10)
            .await
            .unwrap();
        assert!(
            hist.as_array()
                .unwrap()
                .iter()
                .any(|r| r["revision"] == "abc")
        );
        let pending = s.pending_stats().await.unwrap();
        assert_eq!(pending.count, 4);
        assert!(s.spaces().await.is_some());
        assert!(s.space_overview("personal").await.is_some());
        let beliefs = s.get_entity("personal", "e1").await.unwrap();
        assert!(beliefs.as_array().unwrap().first().unwrap()["statement"] == "s1");
        let _timeline = s
            .timeline("personal", "e1", Some(1), Some(2))
            .await
            .unwrap();

        // Each call spawned its own child; nothing stayed connected.
        let spawns = fake.spawns.load(Ordering::SeqCst);
        assert_eq!(spawns, 7, "one child per console call");
        assert!(s.is_available());
    }

    #[tokio::test]
    async fn spawn_failure_degrades_without_retry_logic() {
        let (s, _fake) = child_session(true);
        assert!(s.stats("personal").await.is_none());
        assert!(!s.is_available());
        // The next call starts fresh — no backoff bookkeeping to reset.
        let (s2, _f) = child_session(false);
        assert!(s2.pending_stats().await.is_some());
    }

    #[tokio::test]
    async fn review_merges_degrades_until_admin_review_ships() {
        let (s, _fake) = child_session(false);
        assert!(s.review_merges("personal", "merges").await.is_none());
    }

    #[tokio::test]
    async fn disabled_session_degrades_without_touching_anything() {
        let s = BrainSession::disabled(BrainConfig::new("/tmp/brain"));
        assert!(!s.is_available());
        assert!(s.recall("personal", "q", 100).await.is_none());
        assert!(s.remember("personal", "fact", "agent").await.is_none());
        assert!(s.search("personal", "q", "hybrid", 5, None).await.is_none());
        assert!(s.stats("personal").await.is_none());
        assert!(s.pending_stats().await.is_none());
        assert!(s.review_merges("personal", "merges").await.is_none());
    }
}
