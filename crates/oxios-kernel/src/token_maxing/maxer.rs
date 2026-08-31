//! TokenMaxer orchestrator (RFC-031 §5).
//!
//! The drain → rotate → wait → resume loop. Activated by a time window or a
//! manual toggle. Each tick: pick the most-available eligible provider, ask the
//! WorkPlanner for a bounded task, execute it with the provider's model pinned
//! (`ExecEnv.model_override`) under a restricted capability set
//! (`ExecEnv.cspace_hint = "standard"`), credit the self-tracked counter, and
//! record the outcome in the session.
//!
//! ## §6.4 — fail-closed, structurally
//! The maxer sets `ExecEnv.cspace_hint = "standard"`, and the AgentRuntime
//! registers only what the resulting CSpace authorises via
//! `register_tools_from_cspace_gated()` — the single production
//! registration path. Tools the standard capability set does not cover
//! (high-risk selectors such as rm/osascript/wildcard, ManageRBAC,
//! SystemConfig, `ask_user`) are never armed, so the agent cannot reach
//! them even if it tried.
//! As defense-in-depth, each task is wrapped in a hard per-task timeout so any
//! tool that blocks is bounded (`execute_directive` also enforces
//! `max_execution_time`).

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::RwLock;
use tokio::time::timeout;

use oxios_ouroboros::{Directive, ExecEnv};

use crate::agent_lifecycle::AgentLifecycleManager;
use crate::resilience::FailureClass;
use crate::state_store::StateStore;

use super::TokenMaxingConfig;
use super::planner::{PlannedTask, WorkPlanner};
use super::quota_tracker::{Availability, QuotaTracker};
use super::session::{MaxerStatus, MaxingStart, StopReason, TokenMaxingSession};

/// Per-task hard timeout (defense-in-depth). Bounds any blocking tool.
const MAX_TASK_SECS: u64 = 600;
/// Sleep between ticks when every eligible provider is cooled down / drained,
/// before re-checking for a reset (RFC-031 §5 "wait for reset").
const COOLDOWN_POLL_SECS: u64 = 60;
/// Cap on retained session history (memory bound).
const MAX_HISTORY: usize = 50;

/// The token-maxing orchestrator. Constructed once at boot with a clone of the
/// `AgentLifecycleManager`, the shared `QuotaTracker`, a `WorkPlanner`, and the
/// `StateStore` for session persistence.
pub struct TokenMaxer {
    lifecycle: AgentLifecycleManager,
    tracker: Arc<QuotaTracker>,
    planner: WorkPlanner,
    state_store: Arc<StateStore>,
    state: RwLock<MaxerRuntimeState>,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct MaxerRuntimeState {
    running: bool,
    current: Option<TokenMaxingSession>,
    current_provider: Option<String>,
    current_task: Option<String>,
    history: Vec<TokenMaxingSession>,
}

impl TokenMaxer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        lifecycle: AgentLifecycleManager,
        tracker: Arc<QuotaTracker>,
        planner: WorkPlanner,
        state_store: Arc<StateStore>,
    ) -> Self {
        Self {
            lifecycle,
            tracker,
            planner,
            state_store,
            state: RwLock::new(MaxerRuntimeState::default()),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Launch a session. Refuses if the mode is disabled or no eligible provider
    /// has at least one model to pin. Returns the new session id; the drain loop
    /// runs on a spawned task.
    pub fn launch(self: &Arc<Self>, start: MaxingStart) -> anyhow::Result<String> {
        let cfg = self.tracker.config();
        if !cfg.enabled {
            anyhow::bail!("token-maxing is disabled (token-maxing.enabled = false)");
        }
        let eligible = cfg
            .providers
            .iter()
            .filter(|p| cfg.is_eligible(&p.provider) && !p.models.is_empty());
        if eligible.count() == 0 {
            anyhow::bail!(
                "no eligible token-maxing provider: each needs \
                 billing_model = \"subscription\" and a non-empty models list"
            );
        }

        let (window, manual) = match &start {
            MaxingStart::Scheduled(w) => (Some(w.clone()), false),
            MaxingStart::Manual => (None, true),
        };
        let session = TokenMaxingSession::start(window, manual);
        let id = session.id.clone();

        {
            let mut st = self.state.write();
            if st.running {
                anyhow::bail!("a token-maxing session is already running");
            }
            st.running = true;
            st.current = Some(session);
        }
        self.cancel.store(false, Ordering::Relaxed);

        let me = Arc::clone(self);
        tokio::spawn(async move {
            me.run_loop(start).await;
        });
        Ok(id)
    }

    /// Request a graceful stop after the in-flight task completes.
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Live status (RFC-031 §9 `GET /api/token-maxing/status`).
    pub fn status(&self) -> MaxerStatus {
        let st = self.state.read();
        let current = st.current.as_ref();
        MaxerStatus {
            running: st.running,
            current_session_id: current.map(|s| s.id.clone()),
            current_provider: st.current_provider.clone(),
            current_task: st.current_task.clone(),
            manual: current.map(|s| s.manual).unwrap_or(false),
            window: current.and_then(|s| s.window.clone()),
            tokens_this_session: current.map(|s| s.totals.tokens).unwrap_or(0),
            tasks_this_session: current.map(|s| s.totals.tasks).unwrap_or(0),
            providers: self.tracker.snapshots(),
        }
    }

    /// Past sessions (most-recent last), capped at [`MAX_HISTORY`].
    pub fn sessions(&self) -> Vec<TokenMaxingSession> {
        self.state.read().history.clone()
    }

    /// One past or in-flight session by id.
    pub fn session(&self, id: &str) -> Option<TokenMaxingSession> {
        let st = self.state.read();
        if let Some(c) = &st.current
            && c.id == id
        {
            return Some(c.clone());
        }
        st.history.iter().find(|s| s.id == id).cloned()
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// The drain → rotate → wait → resume loop.
    async fn run_loop(self: Arc<Self>, start: MaxingStart) {
        let (window, manual) = match &start {
            MaxingStart::Scheduled(w) => (Some(w.clone()), false),
            MaxingStart::Manual => (None, true),
        };
        let mut session = TokenMaxingSession::start(window, manual);
        self.persist(&session).await;

        let mut done_goals: HashSet<String> = HashSet::new();

        let stop_reason = loop {
            if self.cancelled() {
                break StopReason::Cancelled;
            }
            if !session.within_window() {
                break StopReason::WindowEnded;
            }

            let cfg = self.tracker.config();

            // Pick the most-available eligible provider with a model to pin.
            let provider = match self.pick_provider(&cfg) {
                Some(p) => p,
                None => {
                    // All cooled/drained — wait for a reset, then re-check.
                    self.sleep_cancellable(COOLDOWN_POLL_SECS).await;
                    continue;
                }
            };
            let model = match self.pick_model(&cfg, &provider, &session) {
                Some(m) => m,
                None => {
                    self.sleep_cancellable(COOLDOWN_POLL_SECS).await;
                    continue;
                }
            };

            let task = match self.planner.next_task(&done_goals).await {
                Some(t) => t,
                None => break StopReason::NoWork,
            };
            done_goals.insert(task.goal.clone());

            {
                let mut st = self.state.write();
                st.current_provider = Some(provider.clone());
                st.current_task = Some(task.source_name.clone());
            }

            self.dispatch(&mut session, task, provider, model).await;
            self.persist(&session).await;
            self.state.write().current = Some(session.clone());
        };

        session.finalize(stop_reason);
        self.persist(&session).await;

        let mut st = self.state.write();
        st.running = false;
        st.current_provider = None;
        st.current_task = None;
        st.current = None;
        st.history.push(session);
        if st.history.len() > MAX_HISTORY {
            let drop_n = st.history.len() - MAX_HISTORY;
            st.history.drain(0..drop_n);
        }
    }

    /// Execute one task, credit the self-tracked counter, and record the outcome.
    async fn dispatch(
        &self,
        session: &mut TokenMaxingSession,
        task: PlannedTask,
        provider: String,
        model: String,
    ) {
        let env = ExecEnv {
            // §6.1/§6.4: restricted capability set → high-risk tools DENY at the
            // gate; ask_user is not registered on this path either.
            cspace_hint: Some("standard".into()),
            model_override: Some(model.clone()),
            root_paths: task.root_paths.clone(),
            ..Default::default()
        };
        let directive = Directive {
            goal: task.goal.clone(),
            ..Default::default()
        };

        let t0 = std::time::Instant::now();
        let result = timeout(
            Duration::from_secs(MAX_TASK_SECS),
            self.lifecycle.execute_directive(&directive, &env),
        )
        .await;
        let dur = t0.elapsed().as_secs_f64();

        let source = task.source;
        let source_name = task.source_name;
        let goal = task.goal;

        match result {
            Ok(Ok(r)) => {
                let tokens = r.tokens_input + r.tokens_output;
                // Phase 1 feed: credit the self-tracked counter.
                let _ = self.tracker.reserve(&provider, tokens);
                // Reactive override: a classified failure cools the provider.
                if let Some(class) = r.failure_class {
                    self.tracker.record_failure(&provider, class, None);
                }
                session.record_task(
                    source,
                    source_name,
                    goal,
                    provider,
                    model,
                    r.success,
                    tokens,
                    dur,
                    truncate(&r.output, 800),
                );
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "token-maxing task failed");
                session.record_task(
                    source,
                    source_name,
                    goal,
                    provider,
                    model,
                    false,
                    0,
                    dur,
                    format!("error: {e}"),
                );
            }
            Err(_) => {
                tracing::warn!("token-maxing task timed out — cooling provider");
                // A timeout often signals a rate-limit/quota wall — cool the
                // provider so the loop rotates to another.
                self.tracker
                    .record_failure(&provider, FailureClass::Transient, None);
                session.record_task(
                    source,
                    source_name,
                    goal,
                    provider,
                    model,
                    false,
                    0,
                    dur,
                    "timed out".into(),
                );
            }
        }
    }

    /// First dispatchable eligible provider — Available preferred over Draining
    /// (the rotation across providers when one hits its floor/cooldown).
    fn pick_provider(&self, cfg: &TokenMaxingConfig) -> Option<String> {
        let snaps = self.tracker.snapshots();
        if let Some(s) = snaps.iter().find(|s| {
            matches!(s.availability, Availability::Available { .. })
                && self.has_model(cfg, &s.provider)
        }) {
            return Some(s.provider.clone());
        }
        snaps
            .iter()
            .find(|s| {
                matches!(s.availability, Availability::Draining { .. })
                    && self.has_model(cfg, &s.provider)
            })
            .map(|s| s.provider.clone())
    }

    fn has_model(&self, cfg: &TokenMaxingConfig, provider: &str) -> bool {
        cfg.get(provider)
            .map(|p| !p.models.is_empty())
            .unwrap_or(false)
    }

    /// Round-robin a model from the provider's configured list.
    fn pick_model(
        &self,
        cfg: &TokenMaxingConfig,
        provider: &str,
        session: &TokenMaxingSession,
    ) -> Option<String> {
        let p = cfg.get(provider)?;
        if p.models.is_empty() {
            return None;
        }
        let used = session
            .tasks
            .iter()
            .filter(|t| t.provider == provider)
            .count();
        Some(p.models[used % p.models.len()].clone())
    }

    async fn persist(&self, session: &TokenMaxingSession) {
        if let Err(e) = self
            .state_store
            .save_json("token-maxing", &session.id, session)
            .await
        {
            tracing::warn!(error = %e, "failed to persist token-maxing session");
        }
    }

    async fn sleep_cancellable(&self, secs: u64) {
        tokio::time::sleep(Duration::from_secs(secs)).await;
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // Cut on a char boundary to avoid splitting a multi-byte sequence.
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}
