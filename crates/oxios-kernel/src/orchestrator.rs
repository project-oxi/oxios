//! Orchestrator: coordinates the unified intent lifecycle (RFC-027).
//!
//! The orchestrator is the "brain" that processes every user message:
//! 1. assess — classify the message (conversation / clarify / task)
//! 2. crystallize — build a Directive for substantial tasks
//! 3. execute — run the agent via the lifecycle manager
//! 4. review — check the result against acceptance criteria
//! 5. retry — re-execute with feedback if review fails

use std::sync::Arc;

use anyhow::Result;
use oxios_ouroboros::ExecutionResult;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent_lifecycle::AgentLifecycleManager;
use crate::event_bus::EventBus;
use crate::git_layer::GitLayer;
use crate::metrics::get_metrics;
use crate::project::{ConversationBuffer, Project, ProjectManager};
use crate::state_store::StateStore;
use crate::types::AgentId;

/// Role of an agent within a group.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub enum AgentRole {
    /// Executes a specific subtask.
    #[default]
    Worker,
    /// Coordinates subtasks, synthesizes results.
    Manager,
}

/// A subtask within a multi-agent plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTask {
    /// Unique subtask ID.
    pub id: Uuid,
    /// Human-readable description.
    pub description: String,
    /// Capability required (e.g., "code-review", "testing").
    pub required_capability: Option<String>,
    /// Result of the subtask (filled after execution).
    pub result: Option<String>,
    /// Whether this subtask succeeded.
    pub success: bool,
    /// Role of the agent assigned to this subtask.
    #[serde(default)]
    pub role: AgentRole,
}

impl SubTask {
    /// Create a new subtask with the given description.
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            description: description.into(),
            required_capability: None,
            result: None,
            success: false,
            role: AgentRole::default(),
        }
    }

    /// Set the required capability for this subtask.
    pub fn with_capability(mut self, cap: impl Into<String>) -> Self {
        self.required_capability = Some(cap.into());
        self
    }
}

/// Resolved workspace for a single turn, derived from a project alone
/// (design §4.3: `project_id` is the only persisted workspace binding).
#[derive(Debug, Clone, Default)]
pub struct ProjectWorkspace {
    /// The resolved project — `None` when no project is bound or the
    /// binding is stale.
    pub project: Option<Project>,
    /// The project's canonical root paths (empty for unscoped turns).
    pub root_paths: Vec<std::path::PathBuf>,
    /// Rendered `## Workspace Context` body (without the header).
    pub context_body: String,
}

/// The orchestrator coordinates the unified intent lifecycle (RFC-027).
#[allow(dead_code)]
pub struct Orchestrator {
    /// IntentEngine for the unified handle() path (RFC-027).
    /// Lazily available when the kernel wires it; None in legacy constructions.
    intent_engine: RwLock<Option<Arc<dyn oxios_ouroboros::IntentEngineOps>>>,
    event_bus: EventBus,
    state_store: Arc<StateStore>,
    /// Git version control layer for auto-commits.
    git_layer: Option<Arc<GitLayer>>,
    /// Agent lifecycle manager (fork, register, run, cleanup).
    lifecycle: AgentLifecycleManager,
    /// A2A protocol for inter-agent task delegation.
    a2a: Option<Arc<crate::a2a::A2AProtocol>>,
    /// Project manager for context partitioning.
    project_manager: RwLock<Option<Arc<ProjectManager>>>,
    /// Conversation buffer for topic shift detection.
    conversation_buffer: RwLock<ConversationBuffer>,
    /// Orchestrator configuration (Ouroboros protocol settings).
    delegation_config: DelegationConfig,
    /// A2A circuit breaker for delegation reliability.
    a2a_breaker: Arc<crate::a2a::circuit_breaker::A2ACircuitBreaker>,
    /// RFC-027 intent config (retry settings, etc).
    intent_config: RwLock<crate::config::IntentConfig>,
    /// RFC-029 recovery coordinator. When `Some`, the orchestrator's
    /// execute path routes through it (L1 backoff / L2 model swap)
    /// instead of calling lifecycle directly.
    recovery: RwLock<Option<Arc<crate::resilience::RecoveryCoordinator>>>,
}

/// Configuration for A2A delegation retries.
#[allow(dead_code)]
struct DelegationConfig {
    /// Maximum retry attempts for A2A delegation.
    max_retries: u32,
    /// Base delay for exponential backoff (milliseconds).
    base_delay_ms: u64,
    /// Maximum delay cap for exponential backoff (milliseconds).
    max_delay_ms: u64,
    /// Timeout per delegation attempt (milliseconds).
    #[allow(dead_code)]
    timeout_ms: u64,
}

impl Default for DelegationConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay_ms: 100,
            max_delay_ms: 5000,
            timeout_ms: 5000,
        }
    }
}

#[allow(dead_code)]
impl DelegationConfig {
    /// Calculate exponential backoff delay.
    fn backoff_delay(&self, attempt: u32) -> u64 {
        let delay = self.base_delay_ms * 2_u64.saturating_pow(attempt.min(10));
        delay.min(self.max_delay_ms)
    }
}

impl Orchestrator {
    /// Creates a new orchestrator.
    pub fn new(
        event_bus: EventBus,
        state_store: Arc<StateStore>,
        lifecycle: AgentLifecycleManager,
    ) -> Self {
        Self::with_config(
            event_bus,
            state_store,
            lifecycle,
            crate::config::OrchestratorConfig::default(),
        )
    }

    /// Creates a new orchestrator with custom config.
    pub fn with_config(
        event_bus: EventBus,
        state_store: Arc<StateStore>,
        lifecycle: AgentLifecycleManager,
        _config: crate::config::OrchestratorConfig,
    ) -> Self {
        Self {
            intent_engine: RwLock::new(None),
            event_bus,
            state_store,
            git_layer: None,
            lifecycle,
            a2a: None,
            project_manager: RwLock::new(None),
            conversation_buffer: RwLock::new(ConversationBuffer::default()),
            delegation_config: DelegationConfig::default(),
            intent_config: RwLock::new(crate::config::IntentConfig::default()),
            a2a_breaker: Arc::new(crate::a2a::circuit_breaker::A2ACircuitBreaker::new(5, 30)),
            recovery: RwLock::new(None),
        }
    }

    /// Wire the IntentEngine for unified handle() calls (RFC-027).
    /// Called by the kernel assembler after construction.
    pub fn set_intent_engine(&self, engine: Arc<dyn oxios_ouroboros::IntentEngineOps>) {
        *self.intent_engine.write() = Some(engine);
    }

    /// Wire the RFC-027 intent config (retry settings, lightweight model, etc.)
    /// from the parsed TOML. Called by the kernel assembler after construction;
    /// before this, retry thresholds hold their `Default` values.
    pub fn set_intent_config(&self, cfg: crate::config::IntentConfig) {
        *self.intent_config.write() = cfg;
    }

    /// Wire the RFC-029 recovery coordinator. Called by the kernel
    /// assembler after construction (shares `RoutingStats` with
    /// `EngineApi` / `AgentRuntime`).
    pub fn set_recovery(&self, coordinator: Arc<crate::resilience::RecoveryCoordinator>) {
        *self.recovery.write() = Some(coordinator);
    }

    /// Whether the IntentEngine is wired (unified path available).
    pub fn has_intent_engine(&self) -> bool {
        self.intent_engine.read().is_some()
    }

    /// Set the ProjectManager for context partitioning.
    pub fn set_project_manager(&self, manager: Arc<ProjectManager>) {
        *self.project_manager.write() = Some(manager);
    }
    /// Get a reference to the ProjectManager, if set.
    pub fn project_manager(&self) -> Option<Arc<ProjectManager>> {
        self.project_manager.read().as_ref().cloned()
    }

    /// Resolve the workspace for a turn from a project id alone (design §4.3).
    ///
    /// Blank/`None` ids, unparseable ids, and stale bindings (project no
    /// longer exists) all resolve to an empty [`ProjectWorkspace`] — the run
    /// turns scopeless instead of falling back to any ambient workspace.
    pub fn resolve_project_workspace(&self, project_id: Option<&str>) -> ProjectWorkspace {
        let Some(pid) = project_id.map(str::trim).filter(|s| !s.is_empty()) else {
            return ProjectWorkspace::default();
        };
        let Some(pm) = self.project_manager.read().clone() else {
            return ProjectWorkspace::default();
        };
        let Ok(uuid) = uuid::Uuid::parse_str(pid) else {
            return ProjectWorkspace::default();
        };
        match pm.get_project(uuid) {
            Some(p) => ProjectWorkspace {
                context_body: render_project_context(&p),
                root_paths: p.root_paths.clone(),
                project: Some(p),
            },
            // Stale binding: turns run scopeless (design §9).
            _ => ProjectWorkspace::default(),
        }
    }

    /// Set the A2A protocol for inter-agent task delegation.
    pub fn set_a2a(&mut self, a2a: Arc<crate::a2a::A2AProtocol>) {
        self.a2a = Some(a2a);
    }

    /// Set the GitLayer for auto-commits after state saves.
    pub fn set_git_layer(&mut self, git_layer: Arc<GitLayer>) {
        self.git_layer = Some(git_layer);
    }

    /// Restore sessions from persisted state.
    ///
    /// RFC-027: the in-memory interview session map is no longer used.
    /// Clarify state is restored from the session store's conversation
    /// history on demand by `handle_unified`. This function is a no-op.
    pub async fn restore_sessions(&self) {
        // No-op — see doc comment above.
    }

    #[allow(dead_code)]
    fn git_commit(&self, rel_path: &str, message: &str) {
        if let Some(ref gl) = self.git_layer
            && gl.is_enabled()
        {
            let _ = gl.commit_file(rel_path, message);
        }
    }

    // ──────────────────────────────────────────────────────────────────
    // RFC-033 — Unified streaming orchestration
    // ──────────────────────────────────────────────────────────────────
    //
    // The assess/crystallize external LLM gates were removed. Every message
    // streams through the agent loop directly — the agent's own
    // UNDERSTAND → PLAN → EXECUTE → VERIFY → REPORT protocol classifies and
    // plans inline. The only surviving external call is `review`, which
    // fires when a Directive carries acceptance criteria.

    /// Unified entry point for every user message (RFC-033).
    ///
    /// A single path with no routing gate: build a [`Directive`] verbatim
    /// from the message, resolve the [`ExecEnv`], execute via the agent
    /// loop (which streams every token/tool/thinking event), and — only
    /// when the directive carries acceptance criteria — run an external
    /// [`IntentEngineOps::review`] with one retry.
    ///
    /// Conversation, clarification, and task depth are all decided *inside*
    /// the agent loop now (simple chat → plain streaming reply; ambiguity →
    /// `ask_user` / `pi-questionnaire` tool; complex work → tool calls).
    /// This matches Claude.ai / Gemini Web, where the model's intelligence
    /// is the classifier and there is no pre-classification step.
    ///
    /// # Why `review` is gated on `needs_review()`
    /// `Directive::from_message` (used for interactive chat) carries no
    /// acceptance criteria, so interactive chat never triggers external
    /// review — the agent's internal VERIFY step replaces it. The review
    /// path survives for any future/automated producer of criteria-bearing
    /// directives; until one is wired, `verify_or_retry` is dormant.
    ///
    /// # Parameters
    /// - `engine` — the LLM-backed intent engine (review only, RFC-033).
    /// - `msg` — the user's raw message text.
    /// - `ctx` — per-message context (session, history, project hints).
    pub async fn handle(
        &self,
        engine: &dyn oxios_ouroboros::IntentEngineOps,
        msg: &str,
        ctx: &oxios_ouroboros::MsgCtx,
    ) -> Result<HandleResponse> {
        // 1. Build the Directive verbatim from the message (no crystallize).
        let mut directive = oxios_ouroboros::Directive::from_message(msg);

        // 2. Resolve the execution environment from MsgCtx.
        let env = self.resolve_exec_env(ctx);

        // 3. Execute — every message streams through the agent loop.
        let mut result = self.execute_directive(&directive, &env).await?;

        // 4. Optional external review — only when the directive carries
        //    acceptance criteria (RFC-033 §3.5). Interactive chat uses
        //    Directive::from_message (no criteria), so this is skipped and
        //    the agent's internal VERIFY step stands in for review.
        let (verdict, evaluation_passed) = if directive.needs_review() {
            let (r, v) = self
                .verify_or_retry(engine, &mut directive, &env, result, msg, ctx)
                .await?;
            result = r;
            let passed = v.all_passed();
            (Some(v), Some(passed))
        } else {
            (None, None)
        };

        Ok(HandleResponse {
            directive: Box::new(directive),
            env: Box::new(env),
            result: Box::new(result),
            verdict,
            evaluation_passed,
        })
    }

    /// Unified entry point that accepts legacy-style parameters and returns
    /// an `OrchestrationResult` (RFC-027).
    ///
    /// Builds a [`MsgCtx`] from the session history (if any), then delegates
    /// to [`handle`](Self::handle). Falls back to `handle_message` if no
    /// `IntentEngine` is wired.
    #[allow(clippy::too_many_arguments)]
    pub async fn handle_unified(
        &self,
        user_id: &str,
        msg: &str,
        session_id: Option<&str>,
        project_id: Option<&str>,
        role: Option<&str>,
        model_override: Option<&str>,
        model_params: Option<oxios_ouroboros::ModelParams>,
        persona_id: Option<&str>,
        brain_space: Option<&str>,
        request_id: &str,
        turn_command: Option<oxios_ouroboros::TurnCommand>,
    ) -> Result<OrchestrationResult> {
        // Get the IntentEngine (always wired by the kernel assembler).
        let engine = self
            .intent_engine
            .read()
            .clone()
            .expect("IntentEngine not wired — kernel assembler bug");

        // Build MsgCtx.
        let sid = session_id.unwrap_or(request_id).to_string();
        let history = self.load_session_history(&sid).await;
        let ctx = oxios_ouroboros::MsgCtx {
            session_id: sid.clone(),
            history,
            project_id: project_id.map(String::from),
            role: role.map(String::from),
            model_override: model_override.map(String::from),
            persona_id: persona_id.map(String::from),
            brain_space: brain_space.map(String::from),
            user_id: user_id.to_string(),
            model_params,
            turn_command,
        };

        // Call the unified path.
        let start = std::time::Instant::now();
        let response = self.handle(engine.as_ref(), msg, &ctx).await?;
        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(self.handle_response_to_orchestration_result(response, &ctx, duration_ms))
    }

    /// Load conversation history for a session from the state store.
    async fn load_session_history(&self, session_id: &str) -> Vec<oxios_ouroboros::Exchange> {
        let sid = crate::state_store::SessionId(session_id.to_string());
        match self.state_store.load_session(&sid).await {
            Ok(Some(session)) => session
                .user_messages
                .iter()
                .zip(session.agent_responses.iter())
                .map(|(u, a)| oxios_ouroboros::Exchange {
                    user: u.content.clone(),
                    agent: a.content.clone(),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn handle_response_to_orchestration_result(
        &self,
        response: HandleResponse,
        ctx: &oxios_ouroboros::MsgCtx,
        duration_ms: u64,
    ) -> OrchestrationResult {
        let metrics = get_metrics();
        metrics.orch_duration.observe(duration_ms as f64 / 1000.0);

        let HandleResponse {
            directive,
            env,
            result,
            verdict,
            evaluation_passed,
        } = response;

        let project_id = env.project_id.map(|u| u.to_string());
        // RFC-032: when execution failed (budget/quota/auth/etc) and the
        // output is empty, generate a user-friendly error message so the WS
        // handler can relay it as an `type: "error"` chunk.
        let failure_class: Option<oxios_ouroboros::FailureClass> = result.failure_class;
        let response_text = if !result.success && result.output.trim().is_empty() {
            failure_class_to_user_message(failure_class.as_ref())
        } else if directive.acceptance_criteria.is_empty() {
            result.output.clone()
        } else {
            match &verdict {
                Some(v) if v.all_passed() => result.output.clone(),
                Some(v) => format!(
                    "{}\n\n⚠ Review notes:\n{}",
                    result.output,
                    v.notes.join("\n")
                ),
                None => result.output.clone(),
            }
        };
        if evaluation_passed.unwrap_or(false) {
            metrics.agents_completed.inc();
        } else {
            metrics.agents_failed.inc();
        }
        OrchestrationResult {
            session_id: Some(ctx.session_id.clone()),
            response: response_text,
            primary_project_id: env.project_id,
            project_tag: None,
            project_id,
            agent_id: None,
            phase_reached: "execute".to_string(),
            evaluation_passed,
            output: Some(result.output.clone()),
            tool_calls: result.tool_calls.clone(),
            failure_class,
            interview_questions: None,
            interview_round: None,
            reasoning_text: result.reasoning_text.clone(),
            reasoning_segments: result.reasoning_segments.clone(),
        }
    }
    /// Resolve the [`ExecEnv`] from the message context: the workspace is
    /// derived purely from the bound project (roots + context body).
    /// Independent of the directive — runs for every message.
    fn resolve_exec_env(&self, ctx: &oxios_ouroboros::MsgCtx) -> oxios_ouroboros::ExecEnv {
        let workspace = self.resolve_project_workspace(ctx.project_id.as_deref());

        // Touch the project to record activity.
        if let Some(project) = &workspace.project
            && let Some(pm) = self.project_manager()
        {
            pm.touch(project.id);
        }

        oxios_ouroboros::ExecEnv {
            workspace_context: if workspace.context_body.is_empty() {
                None
            } else {
                Some(workspace.context_body)
            },
            root_paths: workspace.root_paths,
            project_id: workspace.project.map(|p| p.id),
            cspace_hint: None,
            model_override: ctx.model_override.clone(),
            role: ctx.role.clone(),
            persona_id: ctx.persona_id.clone(),
            restore_state: None,
            session_id: Some(ctx.session_id.clone()),
            brain_space: ctx.brain_space.clone(),
            model_params: ctx.model_params.clone(),
            turn_command: ctx.turn_command.clone(),
        }
    }

    /// Execute a [`Directive`] under an [`ExecEnv`].
    ///
    async fn execute_directive(
        &self,
        directive: &oxios_ouroboros::Directive,
        env: &oxios_ouroboros::ExecEnv,
    ) -> Result<ExecutionResult> {
        // RFC-029: route through the recovery coordinator when wired
        // (L1 backoff / L2 model swap on provider failure). Falls back
        // to a direct lifecycle call when no coordinator is set.
        //
        // Clone the Arc out of the read guard so the parking_lot guard
        // (which is !Send) is dropped before the .await — otherwise the
        // future is !Send and breaks tokio::spawn in the gateway.
        let coordinator = self.recovery.read().as_ref().cloned();
        if let Some(coordinator) = coordinator {
            coordinator.execute(&self.lifecycle, directive, env).await
        } else {
            self.lifecycle.execute_directive(directive, env).await
        }
    }

    /// Review the result against the directive's criteria; on failure,
    /// retry once with the verdict's gaps folded back as constraints.
    ///
    /// RFC-033: this is the sole surviving external LLM gate. It is reached
    /// only when `Directive::needs_review()` is true (acceptance criteria or
    /// output schema present). `Orchestrator::handle` builds directives via
    /// `Directive::from_message` for interactive chat, which carries no
    /// criteria — so for interactive chat this method is **dormant** and the
    /// agent's internal VERIFY step stands in for review. It remains wired so
    /// any future/automated producer of criteria-bearing directives gets
    /// impartial post-execution review. Retries are capped at one attempt.
    async fn verify_or_retry(
        &self,
        engine: &dyn oxios_ouroboros::IntentEngineOps,
        directive: &mut oxios_ouroboros::Directive,
        env: &oxios_ouroboros::ExecEnv,
        initial_result: ExecutionResult,
        _msg: &str,
        _ctx: &oxios_ouroboros::MsgCtx,
    ) -> Result<(ExecutionResult, oxios_ouroboros::Verdict)> {
        let verdict = engine.review(directive, &initial_result).await?;

        if verdict.all_passed() || verdict.gaps.is_empty() {
            return Ok((initial_result, verdict));
        }

        // Check if retry is enabled (RFC-027 Decision 6).
        // When disabled, return the initial result with the failed verdict.
        let enable_retry = self.intent_config.read().enable_retry;
        if !enable_retry {
            tracing::info!("Review failed but retry disabled (enable_retry=false)");
            return Ok((initial_result, verdict));
        }

        let metrics = get_metrics();
        metrics.retry_attempted.inc();

        tracing::info!(
            gaps = verdict.gaps.len(),
            "Review failed — retrying with feedback"
        );

        // Execute with feedback: previous output + gaps injected.
        let retry_result = self
            .lifecycle
            .execute_with_feedback(directive, env, &initial_result, &verdict.gaps)
            .await?;

        // Re-review.
        let retry_verdict = engine.review(directive, &retry_result).await?;

        // Track retry effectiveness.
        if retry_verdict.score > verdict.score {
            metrics.retry_improved.inc();
        } else if retry_verdict.score < verdict.score {
            metrics.retry_degraded.inc();
        } else {
            metrics.retry_unchanged.inc();
        }

        // Return best result.
        let chosen_result = if retry_verdict.score >= verdict.score {
            retry_result
        } else {
            initial_result
        };

        Ok((chosen_result, retry_verdict))
    }
}

/// Response envelope for [`Orchestrator::handle`] (RFC-033).
///
/// RFC-033 collapsed the former `Reply` / `Clarify` / `Task` variants into a
/// single shape: every message now executes through the agent loop, so there
/// is only ever one terminal state. The agent's reply text, tool calls, and
/// reasoning live in `result`; `verdict` / `evaluation_passed` are `Some`
/// only when an external review ran (a criteria-bearing directive).
#[derive(Debug, Clone)]
pub struct HandleResponse {
    /// The directive that was executed (post-retry if a retry ran).
    pub directive: Box<oxios_ouroboros::Directive>,
    /// The execution environment resolved for this message.
    pub env: Box<oxios_ouroboros::ExecEnv>,
    /// The execution result (agent reply text, tool calls, reasoning).
    pub result: Box<ExecutionResult>,
    /// The external review verdict — `None` when no review ran.
    pub verdict: Option<oxios_ouroboros::Verdict>,
    /// Whether the (final) verdict passed — `None` when no review ran.
    pub evaluation_passed: Option<bool>,
}

/// Result of a full orchestration cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationResult {
    /// Session ID for multi-turn interviews. Pass this on follow-up messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The Space ID that handled this message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_project_id: Option<Uuid>,
    /// Space decoration tag for the response (e.g. "[🔧 oxios]").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_tag: Option<String>,
    /// The project bound to this turn, as a string id — `None` when the
    /// turn ran scopeless (no project or stale binding).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// The response to send back to the user.
    pub response: String,
    /// The agent that executed (if execute phase was reached).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// The furthest phase reached: "interview" (conversation/clarify) or "execute" (task executed).
    pub phase_reached: String,
    /// Whether evaluation passed.
    ///
    /// - `None` — evaluation was not applicable (interview, chat, non-task).
    /// - `Some(true)` — evaluation passed.
    /// - `Some(false)` — evaluation failed or execution unsuccessful.
    pub evaluation_passed: Option<bool>,
    /// Output or notes from evaluation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Tool calls recorded during execution.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<oxios_ouroboros::ToolCallRecord>,
    /// Structured interview questions (chat UI redesign — interactive
    /// interview). Populated when the interview phase needs clarification
    /// and the LLM produced a structured form of the questions. The
    /// Gateway forwards this to the WebSocket as an `interview` chunk;
    /// the Web UI renders it as interactive widgets (chips, yes/no
    /// buttons). When `None`, the frontend falls back to rendering
    /// `response` as plain markdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interview_questions: Option<Vec<oxios_ouroboros::InterviewQuestionOutput>>,
    /// Current interview round (1-based). Populated alongside
    /// `interview_questions`. Drives the "Round N/M" indicator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interview_round: Option<u32>,

    /// P4 (§7 persistence): full concatenated reasoning text from the
    /// agent's `ThinkingDelta` stream. Surfaced into the terminal
    /// `OutgoingMessage` metadata so chat.rs can persist it alongside
    /// `tool_calls` and restore on session reopen.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reasoning_text: String,
    /// Block-stream transparency: positioned reasoning spans, interleaved
    /// with tool calls on session reopen. Threaded from ExecutionResult.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning_segments: Vec<oxios_ouroboros::ReasoningSegment>,
    /// Provider failure classification (RFC-029). `Some` when execution
    /// failed with a classifiable provider/infra error; `None` on success,
    /// interview, clarify, or unclassified failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<oxios_ouroboros::FailureClass>,
}
/// Generate a user-facing error message based on the failure class.
/// Used when execution failed with no output text to show.
fn failure_class_to_user_message(class: Option<&oxios_ouroboros::FailureClass>) -> String {
    use oxios_ouroboros::FailureClass;
    match class {
        Some(FailureClass::BudgetExceeded) => {
            "\u{26a0}\u{fe0f} Token budget exceeded for this provider. \
             Try selecting a different model or configuring additional providers \
             in Settings \u{2192} Engine."
                .to_string()
        }
        Some(FailureClass::QuotaExhausted) => "\u{26a0}\u{fe0f} Provider quota exhausted. \
             The selected provider has reached its rate or usage limit. \
             Wait a moment and retry, or switch to a different model."
            .to_string(),
        Some(FailureClass::AuthFailure) => "\u{26a0}\u{fe0f} Authentication failed. \
             Your API key for this provider may be invalid or expired. \
             Check your credentials in Settings \u{2192} Engine."
            .to_string(),
        Some(FailureClass::ModelUnavailable) => "\u{26a0}\u{fe0f} Model unavailable. \
             The selected model is no longer available or was not found. \
             Choose a different model in Settings \u{2192} Engine."
            .to_string(),
        Some(FailureClass::ContextOverflow) => "\u{26a0}\u{fe0f} Context window exceeded. \
             The conversation is too long for this model's context limit. \
             Start a new session or switch to a model with a larger context window."
            .to_string(),
        Some(FailureClass::Transient) => {
            "\u{26a0}\u{fe0f} A temporary error occurred while contacting the provider. \
             The system will retry automatically. If the issue persists, \
             try a different model or check your network connection."
                .to_string()
        }
        Some(FailureClass::Unknown) | None => {
            "\u{26a0}\u{fe0f} An unexpected error occurred during execution. \
             Please try again. If the problem persists, check your provider \
             configuration in Settings \u{2192} Engine."
                .to_string()
        }
    }
}

/// Render the body of the `## Workspace Context` prompt section for a
/// project (design §4.3). The caller wraps this in the `## Workspace
/// Context` header. Roots and instructions are rendered only when present,
/// so a folderless project contributes instructions alone.
fn render_project_context(p: &Project) -> String {
    /// Char-boundary-safe truncation with an ellipsis marker.
    fn cap(s: &str, max: usize) -> String {
        if s.chars().count() <= max {
            return s.to_string();
        }
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }

    const MAX_INSTRUCTIONS_CHARS: usize = 2000;
    const MAX_CONTEXT_CHARS: usize = 6000;

    let mut s = format!("### Project: {}\n", p.name);
    if !p.root_paths.is_empty() {
        s.push_str("Filesystem roots (the first root is the working directory):\n");
        for r in &p.root_paths {
            s.push_str(&format!("- {}\n", r.display()));
        }
    }
    if !p.instructions.trim().is_empty() {
        s.push_str(&format!(
            "\nProject instructions:\n{}\n",
            cap(p.instructions.trim(), MAX_INSTRUCTIONS_CHARS)
        ));
    }
    // Whole-body cap: the context block must stay a
    // bounded slice of the system prompt.
    cap(&s, MAX_CONTEXT_CHARS)
}

#[cfg(test)]
mod project_workspace_tests {
    use super::*;
    use crate::supervisor::NoOpSupervisor;

    /// Minimal orchestrator wired with a real ProjectManager over an
    /// in-memory DB — enough to exercise `resolve_project_workspace`.
    fn workspace_orchestrator() -> (Orchestrator, Arc<ProjectManager>) {
        let event_bus = EventBus::new(64);
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let state_store = Arc::new(StateStore::new(tmp.path().to_path_buf()).expect("state store"));
        let access_manager = Arc::new(parking_lot::Mutex::new(
            crate::access_manager::AccessManager::new(),
        ));
        let a2a = Arc::new(crate::a2a::A2AProtocol::new(event_bus.clone()));
        let lifecycle = AgentLifecycleManager::new(
            Arc::new(NoOpSupervisor),
            access_manager,
            a2a,
            event_bus.clone(),
            300,
            vec![],
            true,
            "/tmp/oxios-test-workspace".to_string(),
            Arc::new(crate::turn_registry::TurnRegistry::new()),
        );
        let orch = Orchestrator::new(event_bus, state_store, lifecycle);
        let db = Arc::new(crate::kernel_db::KernelDatabase::open_in_memory().expect("db"));
        let pm = Arc::new(ProjectManager::new(db, None).expect("project manager"));
        orch.set_project_manager(pm.clone());
        (orch, pm)
    }

    #[test]
    fn resolution_by_id_returns_roots_and_context() {
        let (orch, pm) = workspace_orchestrator();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).expect("mkdir");
        let project = pm
            .create("oxios", vec![root.clone()], "keep tests fast")
            .expect("create");

        let ws = orch.resolve_project_workspace(Some(&project.id.to_string()));
        let resolved = ws.project.expect("project resolved");
        assert_eq!(resolved.id, project.id);
        assert_eq!(ws.root_paths.len(), 1);
        assert!(ws.context_body.contains("### Project: oxios"));
        assert!(ws.context_body.contains("Filesystem roots"));
        assert!(ws.context_body.contains("keep tests fast"));
        // The canonicalized root appears in the context body.
        let canon = std::fs::canonicalize(&root).expect("canonicalize");
        assert!(
            ws.context_body
                .contains(&canon.to_string_lossy().to_string())
        );
    }

    #[test]
    fn unknown_id_returns_default() {
        let (orch, _pm) = workspace_orchestrator();
        let ws = orch.resolve_project_workspace(Some(&Uuid::new_v4().to_string()));
        assert!(ws.project.is_none());
        assert!(ws.root_paths.is_empty());
        assert!(ws.context_body.is_empty());
    }

    #[test]
    fn none_or_blank_or_malformed_returns_default() {
        let (orch, _pm) = workspace_orchestrator();
        for input in [None, Some(""), Some("   "), Some("not-a-uuid")] {
            let ws = orch.resolve_project_workspace(input);
            assert!(
                ws.project.is_none(),
                "input {input:?} must resolve scopeless"
            );
            assert!(ws.root_paths.is_empty());
            assert!(ws.context_body.is_empty());
        }
    }

    #[test]
    fn instructions_only_project_renders_instructions_and_empty_roots() {
        let (orch, pm) = workspace_orchestrator();
        let project = pm
            .create("writing", vec![], "write in English")
            .expect("create");

        let ws = orch.resolve_project_workspace(Some(&project.id.to_string()));
        assert!(ws.project.is_some());
        assert!(ws.root_paths.is_empty());
        assert!(ws.context_body.contains("### Project: writing"));
        assert!(ws.context_body.contains("write in English"));
        assert!(!ws.context_body.contains("Filesystem roots"));
    }

    #[test]
    fn long_instructions_and_context_are_capped() {
        let (orch, pm) = workspace_orchestrator();
        let big = "x".repeat(5000);
        let project = pm.create("big", vec![], &big).expect("create");
        let ws = orch.resolve_project_workspace(Some(&project.id.to_string()));
        assert!(ws.context_body.chars().count() <= 6000);

        // Instructions themselves cap at 2000 chars, with an ellipsis marker.
        let ctx = super::render_project_context(&project);
        let instructions = ctx.split("Project instructions:\n").nth(1).unwrap_or("");
        let trimmed = instructions.trim_end_matches('\n');
        assert!(trimmed.chars().count() <= 2001, "instructions capped");
        assert!(trimmed.ends_with('…'), "ellipsis marker present");
    }

    #[test]
    fn removed_project_detaches_scope() {
        // Design §9: deleting the active project cannot silently keep a
        // scope — a stale binding resolves scopeless.
        let (orch, pm) = workspace_orchestrator();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let project = pm
            .create("gone", vec![dir.path().to_path_buf()], "")
            .expect("create");
        let pid = project.id.to_string();
        pm.remove_project(project.id).expect("remove");

        let ws = orch.resolve_project_workspace(Some(&pid));
        assert!(ws.project.is_none());
        assert!(ws.root_paths.is_empty());
        assert!(ws.context_body.is_empty());
    }
}
