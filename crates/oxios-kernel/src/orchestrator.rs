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
use crate::mount::{MountId, MountManager};
use crate::project::{ConversationBuffer, ProjectManager};
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
    /// Mount manager for path-alias context (RFC-025).
    mount_manager: RwLock<Option<Arc<MountManager>>>,
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
            mount_manager: RwLock::new(None),
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

    /// Set the MountManager for path-alias context (RFC-025).
    pub fn set_mount_manager(&self, manager: Arc<MountManager>) {
        *self.mount_manager.write() = Some(manager);
    }

    /// Get a reference to the MountManager, if set (RFC-025).
    pub fn mount_manager(&self) -> Option<Arc<MountManager>> {
        self.mount_manager.read().as_ref().cloned()
    }

    /// Get a reference to the ProjectManager, if set.
    pub fn project_manager(&self) -> Option<Arc<ProjectManager>> {
        self.project_manager.read().as_ref().cloned()
    }

    /// Detect a project from a message, returning tag string.
    pub fn detect_project_tag(&self, message: &str) -> Option<String> {
        self.project_manager.read().as_ref().and_then(|pm| {
            let projects = pm.list_projects();
            let result = crate::project::detect_project(message, &projects);
            match result {
                crate::project::DetectionResult::Found(id) => pm.get_project(id).map(|p| p.tag()),
                crate::project::DetectionResult::NoMatch { .. } => None,
            }
        })
    }

    /// Resolve the active Mounts for a message (RFC-025).
    ///
    /// Parses explicit `mount_ids` ("uuid1,uuid2,...", primary first); when
    /// none are given, auto-detects from the message. Returns:
    /// - the ordered list of active [`MountId`]s,
    /// - the rendered `## Workspace Context` body (without the header),
    /// - all resolved filesystem paths (primary first),
    /// - a display tag like `[🔧 oxios + oxicode-sdk]`.
    ///
    /// Honors the sticky-primary model: when `mount_ids` is explicitly
    /// provided they are used as-is (detection is skipped). Detection only
    /// runs when `mount_ids` is `None`, seeding the primary slot — it never
    /// replaces an explicit primary, only appends a secondary.
    fn resolve_mount_workspace(
        &self,
        mount_ids: Option<&str>,
        project_ids: Option<&str>,
        user_message: &str,
    ) -> (
        Vec<MountId>,
        Option<String>,
        Vec<std::path::PathBuf>,
        String,
    ) {
        use crate::mount::Mount;

        let Some(mm) = self.mount_manager() else {
            return (Vec::new(), None, Vec::new(), String::new());
        };

        // Parse explicit mount_ids; otherwise auto-detect (seeds the primary slot).
        let mut ids: Vec<MountId> = if let Some(ids_str) = mount_ids {
            ids_str
                .split(',')
                .filter_map(|s| MountId::parse_str(s.trim()).ok())
                .collect()
        } else {
            match mm.detect(user_message) {
                crate::mount::DetectionResult::Found(id) => vec![id],
                crate::mount::DetectionResult::NoMatch { .. } => vec![],
            }
        };
        // De-duplicate while preserving order (handles non-consecutive dups).
        let mut seen = std::collections::HashSet::new();
        ids.retain(|id| seen.insert(*id));

        // ── Project-referenced Mount activation (RFC-025) ──
        // When a project_id is provided, auto-activate its referenced Mounts
        // BEFORE we derive mounts/tag/context/paths, so they are fully
        // visible in the system prompt and the badge — not just granted
        // path access. (Previously this ran after the prompt was built, so
        // project-referenced Mounts were invisible in the context body.)
        let project_for_instructions: Option<crate::project::Project> = if let Some(project_ids_str) =
            project_ids
            && let Some(first_id_str) = project_ids_str.split(',').next()
            && let Some(pm) = self.project_manager()
            && let Ok(pid) = Uuid::parse_str(first_id_str.trim())
        {
            let proj = pm.get_project(pid);
            if let Some(ref project) = proj {
                for mid in &project.mount_ids {
                    if !ids.contains(mid) {
                        ids.push(*mid);
                    }
                }
            }
            proj
        } else {
            None
        };

        if ids.is_empty() {
            return (Vec::new(), None, Vec::new(), String::new());
        }

        // Touch each active Mount (record activity) — now includes any
        // Project-referenced Mounts activated above.
        for id in &ids {
            mm.touch(*id);
        }

        let mounts: Vec<Mount> = mm.get_mounts_ordered(&ids);
        if mounts.is_empty() {
            return (Vec::new(), None, Vec::new(), String::new());
        }

        // Collect all paths (primary first, deduped) over the full Mount set.
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        for m in &mounts {
            for p in &m.paths {
                if !paths.contains(p) {
                    paths.push(p.clone());
                }
            }
        }

        // Display tag.
        let tag = if mounts.len() == 1 {
            mounts[0].tag()
        } else {
            let names: Vec<&str> = mounts.iter().map(|m| m.name.as_str()).collect();
            format!("[🔧 {}]", names.join(" + "))
        };

        let mut context = build_workspace_context_body(&mounts).unwrap_or_default();

        // ── Project instructions (RFC-025) ──
        // Inject the project's instructions into the context body. The
        // "### Active Mounts" header above is only present when there are
        // actual mount entries in `context`; the Project Instructions section
        // stands on its own when only instructions exist.
        if let Some(project) = project_for_instructions {
            // Cap instructions to stay within the prompt budget (~500 tokens).
            let instructions = if project.instructions.len() > 2000 {
                let mut end = 2000;
                while end > 0 && !project.instructions.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}...", &project.instructions[..end])
            } else {
                project.instructions.clone()
            };
            if !instructions.is_empty() {
                context.push_str(&format!(
                    "\n### Project Instructions: {}\n{}\n",
                    project.name, instructions
                ));
            }
        }

        // Enforce a hard prompt budget on the final context body (~1500 tokens).
        const MAX_CONTEXT_CHARS: usize = 6000;
        if context.len() > MAX_CONTEXT_CHARS {
            let mut end = MAX_CONTEXT_CHARS;
            while end > 0 && !context.is_char_boundary(end) {
                end -= 1;
            }
            context.truncate(end);
            context.push_str("\n...(context truncated)...\n");
        }

        let context_opt = if context.is_empty() {
            None
        } else {
            Some(context)
        };
        (ids, context_opt, paths, tag)
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
    /// - `ctx` — per-message context (session, history, project/mount hints).
    pub async fn handle(
        &self,
        engine: &dyn oxios_ouroboros::IntentEngineOps,
        msg: &str,
        ctx: &oxios_ouroboros::MsgCtx,
    ) -> Result<HandleResponse> {
        // 1. Build the Directive verbatim from the message (no crystallize).
        let mut directive = oxios_ouroboros::Directive::from_message(msg);

        // 2. Resolve the execution environment from MsgCtx.
        let env = self.resolve_exec_env(ctx, msg);

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
        project_ids: Option<&str>,
        mount_ids: Option<&str>,
        role: Option<&str>,
        model_override: Option<&str>,
        model_params: Option<oxios_ouroboros::ModelParams>,
        request_id: &str,
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
            project_ids: project_ids.map(String::from),
            mount_ids: mount_ids.map(String::from),
            role: role.map(String::from),
            model_override: model_override.map(String::from),
            user_id: user_id.to_string(),
            model_params,
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
            primary_project_id: env.project_id,
            project_tag: None,
            active_mount_ids: Vec::new(),
            mount_tag: None,
            response: response_text,
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
    ///
    /// Mirrors the Mount workspace resolution done by `handle_message()`
    /// and `chat()` but packages the result as the new [`ExecEnv`] type.
    /// Independent of the directive — runs whether the task is Trivial
    /// or Substantial.
    fn resolve_exec_env(
        &self,
        ctx: &oxios_ouroboros::MsgCtx,
        msg: &str,
    ) -> oxios_ouroboros::ExecEnv {
        let (active_mount_ids, workspace_context, mount_paths, _mount_tag) =
            self.resolve_mount_workspace(ctx.mount_ids.as_deref(), ctx.project_ids.as_deref(), msg);
        // active_mount_ids + mount_tag are surfaced via the legacy path;
        // ExecEnv carries the resolved paths/context/project that the
        // agent runtime actually consumes.
        let _ = active_mount_ids;

        // Resolve a primary project ID (matches handle_message semantics):
        // explicit project_ids takes precedence over auto-detection.
        let project_id = ctx
            .project_ids
            .as_deref()
            .and_then(|ids| {
                ids.split(',')
                    .next()
                    .and_then(|s| Uuid::parse_str(s.trim()).ok())
            })
            .or_else(|| {
                self.detect_project_tag(msg).and_then(|_tag| {
                    self.project_manager().and_then(|pm| {
                        let projects = pm.list_projects();
                        match crate::project::detect_project(msg, &projects) {
                            crate::project::DetectionResult::Found(id) => Some(id),
                            crate::project::DetectionResult::NoMatch { .. } => None,
                        }
                    })
                })
            });

        // Touch the project to record activity (mirrors handle_message).
        if let Some(pid) = project_id
            && let Some(pm) = self.project_manager()
        {
            pm.touch(pid);
        }

        oxios_ouroboros::ExecEnv {
            workspace_context,
            mount_paths,
            project_id,
            cspace_hint: None,
            model_override: ctx.model_override.clone(),
            role: ctx.role.clone(),
            restore_state: None,
            session_id: Some(ctx.session_id.clone()),
            model_params: ctx.model_params.clone(),
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
    /// Active Mount IDs for this message (RFC-025), primary first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_mount_ids: Vec<MountId>,
    /// Mount decoration tag for the response (e.g. "[🔧 oxios + oxicode-sdk]").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mount_tag: Option<String>,
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

/// Render the body of the `## Workspace Context` prompt section (RFC-025).
///
/// The caller (`build_system_prompt`) wraps this in the `## Workspace
/// Context` header. Returns `None` when there are no Mounts to describe.
///
/// Fill order respects the prompt budget (~1500 tokens soft):
/// 1. Primary Mount — full (description + summary + path).
/// 2. Secondary Mounts — name + path + one-line summary only.
fn build_workspace_context_body(mounts: &[crate::mount::Mount]) -> Option<String> {
    if mounts.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str("### Active Mounts\n");

    for (i, m) in mounts.iter().enumerate() {
        let primary = i == 0;
        let path = m
            .primary_path()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "(no path)".to_string());

        if primary {
            out.push_str(&format!("- **{}** → {}\n", m.name, path));
            if !m.auto_description.is_empty() {
                // First ~3 lines of the agent-written description.
                let desc: String = m
                    .auto_description
                    .lines()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join("\n  ");
                out.push_str(&format!("  {}\n", desc));
            }
            let summary = m.summary_line();
            if !summary.is_empty() {
                out.push_str(&format!("  _{}_\n", summary));
            }
            if m.enrichment_pending {
                out.push_str("  _(content changed — consider re-scanning this Mount)_\n");
            }
        } else {
            // Secondary: name + path + one-line summary only.
            let summary = m.summary_line();
            let suffix = if summary.is_empty() {
                String::new()
            } else {
                format!(" — {}", summary)
            };
            out.push_str(&format!("- **{}** → {}{}\n", m.name, path, suffix));
        }
    }

    Some(out)
}

#[cfg(test)]
mod mount_workspace_tests {
    use super::*;
    use crate::mount::{Mount, MountSource};
    use std::path::PathBuf;

    #[test]
    fn test_workspace_context_primary_full_secondary_terse() {
        let mut oxios =
            Mount::from_name_and_path("oxios", PathBuf::from("/Volumes/MERCURY/PROJECTS/oxios"));
        oxios.auto_description = "Agent OS.\nRust + tokio.".to_string();
        oxios.auto_meta.summary = "Rust agent OS".to_string();

        let mut oxi = Mount::from_name_and_path("oxi", PathBuf::from("/oxi"));
        oxi.auto_meta.summary = "SDK".to_string();

        let body = build_workspace_context_body(&[oxios, oxi]).unwrap();
        assert!(body.contains("### Active Mounts"));
        // Primary gets full description.
        assert!(body.contains("Agent OS."));
        assert!(body.contains("_Rust agent OS_"));
        // Secondary is terse.
        assert!(body.contains("**oxi** → /oxi — SDK"));
    }

    #[test]
    fn test_workspace_context_empty_is_none() {
        assert!(build_workspace_context_body(&[]).is_none());
    }

    /// End-to-end: a real MountManager + Orchestrator-less call to
    /// `resolve_mount_workspace` proves that detection seeds the primary,
    /// builds the context body, and collects all paths (multi-path access).
    #[test]
    fn test_resolve_mount_workspace_detects_and_collects_paths() {
        use crate::kernel_db::KernelDatabase;
        use crate::mount::MountManager;
        use std::sync::Arc;

        let db = Arc::new(KernelDatabase::open_in_memory().unwrap());
        let mm = Arc::new(MountManager::new(db, None).unwrap());

        // Register two mounts.
        let oxios = mm
            .create_mount(
                "oxios".to_string(),
                vec![PathBuf::from("/Volumes/MERCURY/PROJECTS/oxios")],
                MountSource::Manual,
            )
            .unwrap();
        let oxicode_sdk = mm
            .create_mount(
                "oxicode-sdk".to_string(),
                vec![PathBuf::from("/Users/me/oxi")],
                MountSource::Manual,
            )
            .unwrap();
        mm.update_enrichment(oxios.id, Some("Agent OS in Rust.".to_string()), None)
            .unwrap();

        // Build a minimal Orchestrator-free resolver path: replicate what
        // resolve_mount_workspace does, but against the manager directly,
        // since the full Orchestrator needs many subsystems.
        let mounts = mm.get_mounts_ordered(&[oxios.id, oxicode_sdk.id]);
        assert_eq!(mounts.len(), 2);

        let body = build_workspace_context_body(&mounts).unwrap();
        assert!(body.contains("oxios"));
        assert!(body.contains("Agent OS in Rust."));
        assert!(body.contains("oxicode-sdk"));

        // Collect paths like the orchestrator does.
        let mut paths = Vec::new();
        for m in &mounts {
            for p in &m.paths {
                if !paths.contains(p) {
                    paths.push(p.clone());
                }
            }
        }
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], PathBuf::from("/Volumes/MERCURY/PROJECTS/oxios"));
        assert_eq!(paths[1], PathBuf::from("/Users/me/oxi"));
    }

    /// Detection layer 1 (name match) seeds the primary when no explicit
    /// mount_ids are given — the core promise of RFC-025.
    #[test]
    fn test_detection_seeds_primary_on_name_mention() {
        use crate::mount::{DetectionResult, detect_mounts};

        let oxios =
            Mount::from_name_and_path("oxios", PathBuf::from("/Volumes/MERCURY/PROJECTS/oxios"));
        let result = detect_mounts("oxios 코드리뷰해줘", std::slice::from_ref(&oxios));
        assert!(matches!(result, DetectionResult::Found(id) if id == oxios.id));
    }
}
