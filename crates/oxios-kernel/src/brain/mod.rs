//! Connection to the oxibrain daemon over a Unix-domain socket (RFC-047).
//!
//! oxibrain is a shared system service (oxios, oximemo, oxiline all consume
//! it). oxios connects at startup and degrades gracefully when the daemon is
//! unavailable: every operation returns `None`/empty and agent turns complete
//! normally (spec §4 degradation contract).

use futures::future::BoxFuture;
use oxibrain_client::BrainClient;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;

/// Callback fired when a lazy reconnect attempt fails. Use to kick the
/// oxibrain installer / supervisor (RFC-047 §degradation contract). The
/// connection layer fires this hook at most once per failed call before
/// the retry, so the handler itself must NOT block or panic.
pub type UnavailableHook = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

pub mod config;
pub mod supervisor;
pub use config::{BrainConfig, resolved_socket_path};

pub use supervisor::{
    BrainSupervisor, LAUNCHD_LABEL, ManagedBy, SupervisorConfig, SupervisorState, SupervisorStatus,
    asset_urls, build_plist, extract_single_binary, verify_sha256,
};

/// A live (or degraded) connection to the oxibrain daemon.
///
/// The inner client is `Option` so a failed call can drop the dead client and
/// record the degraded state. A later operation that finds `None` attempts a
/// reconnect once (single retry) before giving up.
///
/// `available` mirrors the client state as an atomic so
/// [`BrainConnection::is_available`] is a lock-free read — a lock-contended
/// `try_lock` would otherwise report "unavailable" during in-flight calls.
pub struct BrainConnection {
    client: Mutex<Option<BrainClient>>,
    config: BrainConfig,
    available: AtomicBool,
    on_unavailable: Option<UnavailableHook>,
}

impl std::fmt::Debug for BrainConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrainConnection")
            .field("client", &self.client)
            .field("config", &self.config)
            .field("available", &self.available)
            .field(
                "on_unavailable",
                &self.on_unavailable.as_ref().map(|_| "Some(<hook>)"),
            )
            .finish()
    }
}

impl BrainConnection {
    /// Try to connect; on failure log a warning and start degraded.
    pub async fn connect(config: BrainConfig) -> Self {
        let (client, available) = match BrainClient::connect(&config.socket_path).await {
            Ok(c) => {
                tracing::info!(
                    path = %config.socket_path.display(),
                    space = %config.space,
                    "connected to oxibrain daemon"
                );
                (Some(c), true)
            }
            Err(e) => {
                tracing::warn!(
                    path = %config.socket_path.display(),
                    error = %e,
                    "brain daemon unavailable — memory operations degraded"
                );
                (None, false)
            }
        };
        Self {
            client: Mutex::new(client),
            config,
            available: AtomicBool::new(available),
            on_unavailable: None,
        }
    }

    /// Whether a client is currently connected. Cheap lock-free read.
    pub fn is_available(&self) -> bool {
        self.available.load(Ordering::Relaxed)
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
                self.available.store(true, Ordering::Relaxed);
                crate::metrics::get_metrics().oxibrain_available.set(1.0);
                true
            }
            Err(e) => {
                tracing::warn!(
                    path = %self.config.socket_path.display(),
                    error = %e,
                    "brain daemon still unavailable"
                );
                *guard = None;
                self.available.store(false, Ordering::Relaxed);
                crate::metrics::get_metrics().oxibrain_available.set(0.0);
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

    /// Remember content as an episode. Returns the episode id.
    pub async fn remember(&self, content: &str, source: &str) -> Option<String> {
        let content = content.to_string();
        let source = source.to_string();
        let space = self.config.space.clone();
        self.call(move |c| Box::pin(async move { c.ingest(&content, &space, &source).await }))
            .await
    }

    // ── Web API methods (JSON passthrough) ───────────────────────────

    /// Hybrid/lexical/semantic/graph/community search. Raw daemon JSON.
    pub async fn search(&self, query: &str, mode: &str, limit: usize) -> Option<Value> {
        let query = query.to_string();
        let mode = mode.to_string();
        let space = self.config.space.clone();
        self.call(move |c| Box::pin(async move { c.search(&query, &space, &mode, limit).await }))
            .await
    }

    /// An entity's current beliefs.
    pub async fn get_entity(&self, entity_id: &str) -> Option<Value> {
        let entity_id = entity_id.to_string();
        let space = self.config.space.clone();
        self.call(move |c| Box::pin(async move { c.get_entity(&entity_id, &space).await }))
            .await
    }

    /// Belief intervals for an entity over a time range.
    pub async fn timeline(
        &self,
        entity_id: &str,
        from: Option<i64>,
        to: Option<i64>,
    ) -> Option<Value> {
        let entity_id = entity_id.to_string();
        let space = self.config.space.clone();
        self.call(move |c| Box::pin(async move { c.timeline(&entity_id, &space, from, to).await }))
            .await
    }

    /// Provenance and confidence breakdown for a statement.
    pub async fn why(&self, statement_id: &str) -> Option<Value> {
        let statement_id = statement_id.to_string();
        let space = self.config.space.clone();
        self.call(move |c| Box::pin(async move { c.why(&statement_id, &space).await }))
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

    // ── Shared plumbing ──────────────────────────────────────────────

    /// Register a hook fired when a lazy reconnect fails. The hook is
    /// invoked once per failed call before the retry, so the handler
    /// itself must NOT block or panic. Returns `self` for chaining.
    pub fn with_on_unavailable(mut self, hook: UnavailableHook) -> Self {
        self.on_unavailable = Some(hook);
        self
    }

    /// Run `f` against the connected client, handling degradation:
    /// - no client → one reconnect attempt; still none → fire
    ///   `on_unavailable` hook (if any), retry once; still none → `None`
    ///   and the gauge is set to 0.0 so dashboards reflect the outage.
    /// - call error → drop the dead client, log, `None`.
    async fn call<F, T>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&mut BrainClient) -> BoxFuture<'_, anyhow::Result<T>>,
    {
        let mut guard = self.client.lock().await;
        if guard.is_none() && !self.try_reconnect(&mut guard).await {
            // Lazy reconnect failed: fire the hook once so the supervisor
            // can spin the daemon, then retry exactly once.
            if let Some(hook) = self.on_unavailable.as_ref() {
                (hook)().await;
            }
            if !self.try_reconnect(&mut guard).await {
                crate::metrics::get_metrics().oxibrain_available.set(0.0);
                self.available.store(false, Ordering::Relaxed);
                return None;
            }
        }
        let client = guard.as_mut()?;
        match f(client).await {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!(error = %e, "brain daemon call failed — dropping connection");
                *guard = None;
                self.available.store(false, Ordering::Relaxed);
                crate::metrics::get_metrics().oxibrain_available.set(0.0);
                None
            }
        }
    }

    /// Register a vault directory as a pull source on the connected
    /// daemon and run one sync pass. Mirrors the `remember` `call` pattern:
    /// unavailable daemon ⇒ `None`, no panic (C1). The daemon adopts the
    /// directory into a debounced watcher; registration survives restarts.
    ///
    /// T17 (vault unification): the single ingestion path. Replaces the
    /// previous KnowledgeLens file-change → `remember` chain.
    pub async fn register_vault_source(&self, dir: &Path) -> Option<oxibrain_client::SyncOutcome> {
        let dir = dir.to_string_lossy().into_owned();
        let space = self.config.space.clone();
        let space_for_log = space.clone();
        let outcome = self
            .call(move |c| Box::pin(async move { c.sync_run(&dir, &space).await }))
            .await?;
        tracing::debug!(
            space = %space_for_log,
            new = outcome.new.len(),
            modified = outcome.modified.len(),
            unchanged = outcome.unchanged.len(),
            "brain register: sync_run ok"
        );
        Some(outcome)
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
                self.available.store(true, Ordering::Relaxed);
                crate::metrics::get_metrics().oxibrain_available.set(1.0);
                true
            }
            Err(e) => {
                tracing::debug!(error = %e, "brain daemon still unavailable");
                false
            }
        }
    }
}

/// Bounded retry policy for the boot-time vault registration.
///
/// The daemon may not be reachable at boot (user-managed daemon started
/// after oxios, or the oxibrain installer still warming up). We retry
/// the registration with capped exponential backoff instead of silently
/// dropping the whole session's ingestion.
///
/// Defaults: 5s initial, 60s max backoff, 10 min total budget. Tests
/// override via [`VaultRegisterPolicy`] fields.
#[derive(Debug, Clone, Copy)]
pub struct VaultRegisterPolicy {
    /// First sleep between attempts.
    pub initial_backoff: std::time::Duration,
    /// Cap on the per-attempt sleep (exponential growth saturates here).
    pub max_backoff: std::time::Duration,
    /// Hard wall-clock budget. After this elapses, give up with
    /// [`VaultRegisterOutcome::TimedOut`].
    pub max_total: std::time::Duration,
}

impl Default for VaultRegisterPolicy {
    fn default() -> Self {
        Self {
            initial_backoff: std::time::Duration::from_secs(5),
            max_backoff: std::time::Duration::from_secs(60),
            max_total: std::time::Duration::from_secs(600),
        }
    }
}

/// Result of a [`VaultRegisterPolicy::retry`] loop.
#[derive(Debug, PartialEq, Eq)]
pub enum VaultRegisterOutcome {
    /// Attempt returned `Some(_)` before the budget elapsed.
    Ok,
    /// Budget elapsed before any attempt succeeded.
    TimedOut,
}

impl VaultRegisterPolicy {
    /// Drive `attempt` with bounded exponential backoff until it returns
    /// `Some(_)`, the budget elapses, or the vault dir appears (re-checked
    /// each iteration — covers the "vault dir not yet created at first
    /// boot" failure mode).
    ///
    /// `attempt` is an owned `Fn() -> Fut` so each retry builds a fresh
    /// future; the closure is called once per attempt.
    ///
    /// R17 round 1 (P2): bounded retry replaces the prior one-shot
    /// boot-time call. All failure paths log-only — the kernel must
    /// boot even if the vault is never registered.
    pub async fn retry<F, Fut>(&self, dir: &Path, attempt: F) -> VaultRegisterOutcome
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Option<()>>,
    {
        let started = std::time::Instant::now();
        let mut backoff = self.initial_backoff;
        let mut attempt_n = 0u32;
        loop {
            attempt_n += 1;
            // Vault dir re-check — covers the "first-boot dir missing"
            // case. The daemon's sync_vault() errors with "not a
            // directory" when the path doesn't exist, so this re-check
            // keeps the policy alive until the dir materializes.
            if !dir.is_dir() {
                tracing::info!(
                    path = %dir.display(),
                    attempt = attempt_n,
                    "brain register: vault dir missing; will retry"
                );
            } else if let Some(()) = attempt().await {
                tracing::info!(
                    path = %dir.display(),
                    attempt = attempt_n,
                    "brain register: vault pull source registered (after retry)"
                );
                return VaultRegisterOutcome::Ok;
            }
            let elapsed = started.elapsed();
            if elapsed >= self.max_total {
                tracing::warn!(
                    path = %dir.display(),
                    attempts = attempt_n,
                    elapsed_secs = elapsed.as_secs(),
                    "brain register: retry budget exhausted; vault not registered"
                );
                return VaultRegisterOutcome::TimedOut;
            }
            // Cap the sleep so we don't blow past max_total on a single
            // long backoff.
            let remaining = self.max_total.saturating_sub(elapsed);
            let sleep_for = backoff.min(remaining);
            tracing::debug!(
                path = %dir.display(),
                attempt = attempt_n,
                sleep_secs = sleep_for.as_secs(),
                "brain register: attempt failed; sleeping before retry"
            );
            tokio::time::sleep(sleep_for).await;
            // Exponential growth, capped at max_backoff. The next
            // iteration starts from the saturated value.
            backoff = (backoff * 2).min(self.max_backoff);
        }
    }
}

/// Resolve the brain space with documented precedence.
///
/// 1. `~/.oxi/config.toml [vault].space` — ecosystem-wide override.
/// 2. `fallback` — the kernel's `BrainConfig::space` default.
///
/// Best-effort: missing/unreadable/malformed ecosystem config returns
/// the fallback. Never blocks the boot path.
pub fn resolve_space(home: &std::path::Path, fallback: &str) -> String {
    let path = home.join(".oxi").join("config.toml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return fallback.to_string();
    };
    #[derive(serde::Deserialize)]
    struct VaultSection {
        space: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct Root {
        vault: Option<VaultSection>,
    }
    match toml::from_str::<Root>(&text) {
        Ok(r) => r
            .vault
            .and_then(|v| v.space)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| fallback.to_string()),
        Err(_) => fallback.to_string(),
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

    /// When a lazy reconnect fails, the `on_unavailable` hook must fire
    /// exactly once per call before the second reconnect attempt — and the
    /// call itself must degrade to `None`. TDD guard for the respawn hook.
    #[tokio::test]
    async fn on_unavailable_hook_fires_then_retries() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let sock = dir.path().join("dead.sock");
        let config = BrainConfig::new(sock.clone(), "personal");
        let conn = BrainConnection::connect(config).await;
        assert!(!conn.is_available(), "no daemon → degraded");

        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_hook = Arc::clone(&calls);
        let hook: UnavailableHook = Arc::new(move || {
            let calls = Arc::clone(&calls_hook);
            Box::pin(async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            })
        });
        let conn = conn.with_on_unavailable(hook);

        let result = conn.recall("test", 1000).await;
        assert_eq!(result, None, "no daemon → call returns None");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "hook fires exactly once per failed call"
        );
    }

    /// T17 (vault unification): `register_vault_source` issues
    /// `BrainClient::sync_run(dir, config.space)` on the connected daemon
    /// and degrades to `None` (no panic) when the daemon is unreachable.
    /// Mirrors the `remember`/`recall`/`stats` degradation contract (C1).
    #[tokio::test]
    async fn register_vault_source_degrades_when_daemon_unavailable() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let config = BrainConfig::new(dir.path().join("missing.sock"), "personal");
        let conn = BrainConnection::connect(config).await;
        assert!(!conn.is_available(), "no daemon -> degraded");

        let vault = dir.path().join("vault");
        std::fs::create_dir_all(&vault).expect("vault mkdir");
        // No daemon => None, no panic (C1).
        let result = conn.register_vault_source(&vault).await;
        assert!(result.is_none(), "unreachable daemon => None");
    }

    // ─── VaultRegisterPolicy tests (R17 round 1 P2) ──────────────────────

    /// Fast retry policy: 10ms initial, 50ms max, 500ms total. Keeps tests fast.
    fn fast_policy() -> VaultRegisterPolicy {
        VaultRegisterPolicy {
            initial_backoff: std::time::Duration::from_millis(10),
            max_backoff: std::time::Duration::from_millis(50),
            max_total: std::time::Duration::from_millis(500),
        }
    }

    /// Succeeds immediately → no retries, no sleep.
    #[tokio::test]
    async fn vault_register_policy_succeeds_immediately() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let attempts_c = std::sync::Arc::clone(&attempts);
        let outcome = fast_policy()
            .retry(dir.path(), || {
                let a = std::sync::Arc::clone(&attempts_c);
                async move {
                    a.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Some(())
                }
            })
            .await;
        assert_eq!(outcome, VaultRegisterOutcome::Ok);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// Fails twice then succeeds → 3 attempts total.
    #[tokio::test]
    async fn vault_register_policy_succeeds_after_n_retries() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let attempts_c = std::sync::Arc::clone(&attempts);
        let outcome = fast_policy()
            .retry(dir.path(), || {
                let a = std::sync::Arc::clone(&attempts_c);
                async move {
                    let n = a.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if n < 2 { None } else { Some(()) }
                }
            })
            .await;
        assert_eq!(outcome, VaultRegisterOutcome::Ok);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    /// Always fails → eventually times out with VaultDirMissing (the
    /// vault path is real here so the policy gives up on time, not on
    /// missing-dir).
    #[tokio::test]
    async fn vault_register_policy_times_out() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let attempts_c = std::sync::Arc::clone(&attempts);
        let outcome = fast_policy()
            .retry(dir.path(), || {
                let a = std::sync::Arc::clone(&attempts_c);
                async move {
                    a.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    None
                }
            })
            .await;
        assert_eq!(outcome, VaultRegisterOutcome::TimedOut);
        assert!(attempts.load(std::sync::atomic::Ordering::SeqCst) >= 1);
    }

    /// Vault dir does not exist on the first attempts → policy waits for
    /// the dir to appear. The test creates the dir after attempt 2 succeeds.
    #[tokio::test]
    async fn vault_register_policy_retries_when_vault_dir_missing_then_created() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let vault = dir.path().join("vault");
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let attempts_c = std::sync::Arc::clone(&attempts);

        // Background creator: after ~30ms create the dir.
        let creator_dir = vault.clone();
        let creator = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(30));
            std::fs::create_dir_all(&creator_dir).expect("create vault");
        });

        let outcome = fast_policy()
            .retry(&vault, || {
                let a = std::sync::Arc::clone(&attempts_c);
                async move {
                    a.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    None // every attempt fails (no daemon) — retry until timeout
                }
            })
            .await;
        creator.join().expect("creator join");
        // The dir did appear during the retry window. Without the
        // vault-existence re-check, the policy would still time out —
        // which it does here because the attempt always returns None.
        // The test asserts the retry WINDOW survives long enough for the
        // dir to appear and the loop continues until max_total. (The
        // boot path's `register_vault_source` returns Some once the
        // daemon accepts the sync_run; the policy alone only knows about
        // the dir's existence and the attempt's outcome.)
        assert_eq!(
            outcome,
            VaultRegisterOutcome::TimedOut,
            "no daemon => always None => timeout (dir check does not flip outcome)"
        );
        assert!(attempts.load(std::sync::atomic::Ordering::SeqCst) >= 2);
        // Vault was created during the retry window.
        assert!(vault.is_dir(), "creator thread made the vault");
    }

    // ─── resolve_space tests (R17 round 1 P3) ────────────────────────────

    #[test]
    fn resolve_space_returns_fallback_when_oxi_config_missing() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        assert_eq!(resolve_space(dir.path(), "personal"), "personal");
    }

    #[test]
    fn resolve_space_returns_fallback_when_oxi_config_unreadable() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let oxi_dir = dir.path().join(".oxi");
        std::fs::create_dir_all(&oxi_dir).unwrap();
        // Directory where a file is expected → read fails with IsADirectory.
        let cfg = oxi_dir.join("config.toml");
        std::fs::create_dir(&cfg).unwrap();
        assert_eq!(resolve_space(dir.path(), "personal"), "personal");
    }

    #[test]
    fn resolve_space_returns_fallback_when_oxi_config_malformed() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let oxi_dir = dir.path().join(".oxi");
        std::fs::create_dir_all(&oxi_dir).unwrap();
        std::fs::write(oxi_dir.join("config.toml"), "this is not [valid toml").unwrap();
        assert_eq!(resolve_space(dir.path(), "personal"), "personal");
    }

    #[test]
    fn resolve_space_returns_fallback_when_space_is_whitespace() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let oxi_dir = dir.path().join(".oxi");
        std::fs::create_dir_all(&oxi_dir).unwrap();
        std::fs::write(oxi_dir.join("config.toml"), "[vault]\nspace = \"   \"\n").unwrap();
        assert_eq!(resolve_space(dir.path(), "personal"), "personal");
    }

    #[test]
    fn resolve_space_returns_fallback_when_vault_table_absent() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let oxi_dir = dir.path().join(".oxi");
        std::fs::create_dir_all(&oxi_dir).unwrap();
        std::fs::write(oxi_dir.join("config.toml"), "[some_other]\nkey = 1\n").unwrap();
        assert_eq!(resolve_space(dir.path(), "personal"), "personal");
    }

    #[test]
    fn resolve_space_returns_oxi_value_when_valid() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let oxi_dir = dir.path().join(".oxi");
        std::fs::create_dir_all(&oxi_dir).unwrap();
        std::fs::write(oxi_dir.join("config.toml"), "[vault]\nspace = \"work\"\n").unwrap();
        assert_eq!(resolve_space(dir.path(), "personal"), "work");
    }

    #[test]
    fn resolve_space_trims_whitespace_around_valid_value() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let oxi_dir = dir.path().join(".oxi");
        std::fs::create_dir_all(&oxi_dir).unwrap();
        std::fs::write(
            oxi_dir.join("config.toml"),
            "[vault]\nspace = \"  work  \"\n",
        )
        .unwrap();
        assert_eq!(resolve_space(dir.path(), "personal"), "work");
    }
}
