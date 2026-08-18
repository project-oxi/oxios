//! Agent runtime: wraps oxicode-sdk's Agent for directive execution.
//!
//! The AgentRuntime uses `OxiosEngine.oxi().agent()` (AgentBuilder pattern)
//! to construct agents with full middleware, observability, and security
//! integration from oxicode-sdk 0.24.0.
//!
//! # Architecture
//!
//! All tool access goes through `KernelHandle` — the single syscall-table-like
//! path for agent OS control. The runtime:
//!
//! 1. Resolves the agent's CSpace from persona/role/hint
//! 2. Registers tools via `register_tools_from_cspace()`
//! 3. Optionally queries `ToolRetriever` for semantic capability hints
//! 4. Builds an `Agent` via `AgentBuilder` with middleware pipeline
//! 5. Runs via `Agent::run_streaming()` for real-time event processing
//!
//! # oxicode-sdk 0.23.0 Integration
//!
//! Uses `AgentBuilder` for agent construction with:
//! - `.with_rate_limit()` — tool call rate limiting
//! - `.with_token_budget()` — per-execution token caps
//! - `.tracer()` / `.cost_tracker()` — observability hooks
//! ## Routing integration (RFC-011)
//!
//! Model usage events (`AgentEvent::Usage`) are recorded to the shared
//! `RoutingStats` so the Web dashboard can display per-model call counts
//! and estimated costs.

use anyhow::Result;
use oxicode_sdk::observability::AuditTrail;
use oxicode_sdk::{Agent, AgentConfig, AgentEvent, CompactionEvent, CompactionStrategy};
use oxicode_sdk::{SearchCache, ToolExecutionMode, ToolRegistry};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
// RFC-014 Phase D: `ToolRegistry::register_arc` is used in the AgentBuilder
// path to attach CSpace tools after `builder.build()` returns.

use crate::access_manager::{AccessGate, AgentContext, TracingAuditSink, TrailAuditSink};
use crate::capability::resolve::resolve_cspace;
use crate::engine::OxiosEngine;
use crate::persona::PersonaManager;
use crate::tools::registration::register_tools_from_cspace_gated;

use crate::KernelHandle;
use crate::event_bus::KernelEvent;
use crate::session_context::SessionContext;
use crate::types::AgentId;
use oxios_ouroboros::{Directive, ExecEnv, ExecutionResult};

use oxicode_sdk::{BreakerState, CircuitBreaker, DefaultCircuitBreaker};
use std::time::Duration;

/// Global LLM circuit breaker — oxicode-sdk 0.64 `DefaultCircuitBreaker` (R6).
///
/// Metrics-only today: it records success/failure and surfaces `.state()` to
/// the `llm_circuit_breaker_state` gauge, but does not gate calls. The
/// dedicated per-provider health registry (`resilience/health.rs`) owns actual
/// gating. oxicode-sdk 0.61 removed the old `ProviderCircuitBreaker`/
/// `CircuitBreakerConfig`; 0.64 reintroduced a minimal `CircuitBreaker` trait
/// + this reference impl (R6) which oxios now adopts.
///
/// Thresholds (5 consecutive failures, 30 s reset) mirror the per-provider
/// health registry's documented policy. They are SDK-illustrative reference
/// values — this breaker is observational, not a reliability boundary.
static LLM_CIRCUIT_BREAKER: std::sync::LazyLock<DefaultCircuitBreaker> =
    std::sync::LazyLock::new(|| DefaultCircuitBreaker::new(5, Duration::from_secs(30)));

/// Streaming delta emitted by the runtime's `AgentEvent` callback.
///
/// P1 wires only `Text` (one `AgentEvent::TextChunk { text }` → one delta).
/// P4 adds `Thinking` / `ThinkingDelta` for the live 추론 panel. The enum
/// is `#[non_exhaustive]` so adding variants later doesn't break collectors.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum StreamDelta {
    /// One-shot model announcement. Emitted exactly once at the start of a
    /// streaming turn so the chat UI can mark the response with the actual
    /// model after fallback resolution.
    Model(String),
    /// One text chunk from the model.
    Text(String),

    /// The model has entered extended thinking (no payload — signal only).
    /// Used by the LiveActivityBar to transition into "추론 중" state.
    Thinking,
    /// One batched chunk of reasoning text. The runtime coalesces
    /// `AgentEvent::ThinkingDelta { text }` into ~50ms batches before
    /// emitting this delta to avoid flooding the mpsc.
    ThinkingDelta(String),
    /// The model finished a reasoning span (signal-only, no payload).
    ///
    /// Emitted by oxi 0.58+ (`AgentEvent::ThinkingEnd`). For interleaved
    /// reasoning models (Claude 4, o-series) this fires once per span. The
    /// gateway collector uses it as the authoritative `reasoning.end`
    /// marker; models that never emit it (reasoning_content via openai.rs —
    /// GLM/DeepSeek/Qwen) fall back to the collector's first-Text heuristic.
    ThinkingEnd,
}

/// Connection-scoped streaming sink sender.
///
/// Wrapped in `Arc` so it can be cloned cheaply across the
/// orchestrator → lifecycle → runtime boundary. The receiver lives in a
/// collector task owned by the gateway dispatch layer; see the design doc
/// §8.1 for the conn_id scoping rationale.
pub type StreamingSinkTx = std::sync::Arc<tokio::sync::mpsc::Sender<StreamDelta>>;

/// Configuration for creating AgentRuntime instances.
#[derive(Debug, Clone)]
pub struct AgentRuntimeConfig {
    /// Model ID in `provider/model` format (e.g. `anthropic/claude-sonnet-4-20250514`).
    pub model_id: String,
    /// How to execute tool calls within a single turn.
    pub tool_execution: ToolExecutionMode,
    /// Whether auto-retry is enabled for retryable LLM errors.
    pub auto_retry_enabled: bool,
    /// Scratch workspace directory for temp files.
    pub workspace_dir: Option<std::path::PathBuf>,
    /// API key resolved from CredentialStore at build time.
    pub api_key: Option<String>,
    /// Per-provider options for fine-grained control.
    pub provider_options: Option<oxicode_sdk::ProviderOptions>,
    /// Rate limit for tool calls (requests per minute). 0 = unlimited.
    pub rate_limit_per_minute: usize,
    /// Token budget per agent execution. 0 = unlimited.
    pub token_budget: usize,
    /// Enable audit logging for all tool executions.
    pub audit_tool_calls: bool,
    /// Maximum bytes of a tool result before truncation (RFC-035 gap 1).
    /// When set, tool results exceeding this are truncated in the message
    /// history with a `"... [truncated: N bytes omitted]"` marker.
    /// `None` = unlimited (opt-in). Threaded to `AgentConfig::max_tool_result_bytes`.
    pub max_tool_result_bytes: Option<usize>,
    /// Per-message model params (temperature / max_tokens). When set,
    /// these override the hardcoded defaults in AgentConfig construction.
    /// Populated from `ExecEnv::model_params` which the gateway fills
    /// from the WS payload.
    pub model_params: Option<oxios_ouroboros::ModelParams>,
    // NOTE: subagent_max_depth was removed — oxicode-agent hardcodes the
    // in-process recursion cap to 3 (subagent.rs:649). `AgentConfig.subagent_depth`
    // is the CURRENT depth (always 0 for top-level agents), not a max.
}

impl Default for AgentRuntimeConfig {
    fn default() -> Self {
        Self {
            model_id: String::new(),
            tool_execution: ToolExecutionMode::Parallel,
            auto_retry_enabled: true,
            workspace_dir: None,
            api_key: None,
            provider_options: None,
            rate_limit_per_minute: 0,
            token_budget: 0,
            audit_tool_calls: false,
            max_tool_result_bytes: None,
            model_params: None,
        }
    }
}

/// Mutable state shared between the event callback and the main execute flow.
#[derive(Default)]
struct ExecuteState {
    final_content: String,
    steps_completed: usize,
    success: bool,
    /// Collected trajectory steps for SONA learning (RFC-020 Phase 2).
    /// P4 (§7 persistence): concatenated reasoning text from
    /// `AgentEvent::ThinkingDelta { text }`. Surfaced via `ExecutionResult`
    /// metadata on turn completion, capped at ~4 KB to bound storage.
    reasoning_text: String,
    /// Block-stream transparency: positioned reasoning spans for reopen
    /// fidelity. `reasoning_bytes` bounds total captured text (char-safe).
    reasoning_segments: Vec<oxios_ouroboros::ReasoningSegment>,
    reasoning_bytes: usize,
    /// Ordered by insertion — parallel tools get their final position
    /// resolved when they complete, preserving approximate execution order.
    trajectory_steps: Vec<crate::memory_agent::sona::TrajectoryStep>,
    /// Map of tool_call_id → (start instant, index into trajectory_steps).
    /// Used to correlate ToolExecutionEnd with the correct step when
    /// parallel tool calls complete out of order.
    pending_tools: std::collections::HashMap<String, (std::time::Instant, usize)>,
    /// Ordered tool_call_ids matching trajectory_steps indices.
    /// Pushed in ToolExecutionStart, same order as trajectory_steps.
    tool_call_ids: Vec<String>,
    /// Per-step tool args (JSON string) captured from ToolExecutionStart.
    tool_args_map: std::collections::HashMap<String, String>,
    /// Per-step error flag from ToolExecutionEnd.
    tool_error_map: std::collections::HashMap<String, bool>,
    /// Per-step start timestamp (UTC) from ToolExecutionStart.
    tool_timestamps: std::collections::HashMap<String, chrono::DateTime<chrono::Utc>>,
    /// Cumulative input tokens from AgentEvent::Usage.
    total_input_tokens: u64,
    /// Cumulative output tokens from AgentEvent::Usage.
    total_output_tokens: u64,
}

/// Runtime that wraps an oxicode-sdk `Agent` for executing directives.
///
/// Each call to [`AgentRuntime::execute_directive`] creates a fresh `Agent`,
/// builds a ToolRegistry based on the agent's CSpace, and runs it to completion.
///
/// All OS-level access goes through `KernelHandle` — the single syscall table
/// for agent control. Provider/model resolution goes through `EngineHandle`,
/// which returns the latest `OxiosEngine` (hot-swapped on config change).
pub struct AgentRuntime {
    engine_handle: Arc<crate::engine::EngineHandle>,
    config: AgentRuntimeConfig,
    /// Single path to all kernel services.
    kernel_handle: Arc<KernelHandle>,
    /// Persona manager for system prompt injection.
    persona_manager: Option<Arc<PersonaManager>>,
    /// Semantic tool retriever for capability discovery.
    tool_retriever: Option<Arc<crate::tools::retrieval::ToolRetriever>>,
    /// Shared routing stats (shared with EngineApi).
    routing_stats: Option<Arc<crate::kernel_handle::RoutingStats>>,
    /// Autonomous persistence hook (RFC-016).
    persistence_hook: Option<Arc<crate::persistence_hook::PersistenceHook>>,
    /// SONA trajectory pattern engine (RFC-020 Phase 2). `None` until attached
    /// by the kernel assembler.
    sona: Option<Arc<crate::memory_agent::sona::SonaEngine>>,
    /// Per-session assistant message index counter (RFC-016).
    session_msg_counter: Arc<Mutex<HashMap<String, usize>>>,
}

impl AgentRuntime {
    /// Creates a new agent runtime with engine handle and kernel access.
    ///
    /// The active model is resolved live from `engine_handle` on each
    /// `execute()` (reads the post-hot-swap default) — there is no frozen
    /// model id at construction. Tool access goes through `kernel_handle`.
    pub fn new(
        engine_handle: Arc<crate::engine::EngineHandle>,
        kernel_handle: Arc<KernelHandle>,
        routing_stats: Option<Arc<crate::kernel_handle::RoutingStats>>,
    ) -> Self {
        Self {
            engine_handle,
            config: AgentRuntimeConfig::default(),
            kernel_handle,
            persona_manager: None,
            tool_retriever: None,
            routing_stats,
            persistence_hook: None,
            sona: None,
            session_msg_counter: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Attach the SONA trajectory pattern engine.
    pub fn with_sona(mut self, sona: Arc<crate::memory_agent::sona::SonaEngine>) -> Self {
        self.sona = Some(sona);
        self
    }

    /// Attach a PersonaManager for persona system prompt injection.
    pub fn with_persona_manager(mut self, pm: Arc<PersonaManager>) -> Self {
        self.persona_manager = Some(pm);
        self
    }

    /// Set the runtime config (overrides defaults).
    pub fn with_config(mut self, config: AgentRuntimeConfig) -> Self {
        self.config = config;
        self
    }

    /// Attach a ToolRetriever for semantic capability discovery.
    pub fn with_tool_retriever(
        mut self,
        retriever: Arc<crate::tools::retrieval::ToolRetriever>,
    ) -> Self {
        self.tool_retriever = Some(retriever);
        self
    }

    /// Attach a PersistenceHook for autonomous persistence (RFC-016).
    pub fn with_persistence_hook(
        mut self,
        hook: Arc<crate::persistence_hook::PersistenceHook>,
    ) -> Self {
        self.persistence_hook = Some(hook);
        self
    }

    /// Execute a Directive with its ExecEnv (RFC-027 unified intent handling).
    ///
    /// Maps Directive/ExecEnv fields to the agent's runtime inputs and runs
    /// the tool-calling loop to completion. The persistence hook (RFC-016)
    /// runs on this path.
    pub async fn execute_directive(
        &self,
        agent_id: AgentId,
        directive: &Directive,
        env: &ExecEnv,
        session_ctx: &mut SessionContext,
    ) -> Result<ExecutionResult> {
        // RFC-033: prefer the chat session id the gateway registered its
        // streaming sink under (set by the orchestrator from ctx.session_id)
        // so token/tool/thinking deltas and RFC-015 events correlate with the
        // live WS sink. Fall back to the agent id for non-chat callers
        // (token-maxing, A2A) that leave env.session_id unset.
        let session_id: Option<String> = env
            .session_id
            .clone()
            .or_else(|| Some(agent_id.to_string()));
        self.execute_directive_with_session(agent_id, directive, env, session_ctx, session_id)
            .await
    }
    /// Like [`execute_directive`](Self::execute_directive) but with an
    /// explicit session_id for RFC-015 chat transparency event publishing.
    pub async fn execute_directive_with_session(
        &self,
        agent_id: AgentId,
        directive: &Directive,
        env: &ExecEnv,
        session_ctx: &mut SessionContext,
        session_id: Option<String>,
    ) -> Result<ExecutionResult> {
        self.execute_inner(
            agent_id,
            &directive.goal,
            &directive.original_request,
            &directive.constraints,
            &directive.acceptance_criteria,
            env.cspace_hint.as_deref(),
            &env.mount_paths,
            env.workspace_context.as_deref(),
            session_ctx,
            session_id,
            Some(directive),
            env.model_override.as_deref(),
            env.role.as_deref(),
            env.restore_state.as_ref(),
        )
        .await
    }

    /// Shared execution body for the directive path.
    ///
    /// Performs the full agent-runtime pipeline: prompt assembly, capability
    /// retrieval, memory + knowledge recall, CSpace tool registration,
    /// model resolution, agent run, post-execution summary, and the
    /// autonomous persistence hook (RFC-016).
    #[allow(clippy::too_many_arguments)]
    async fn execute_inner(
        &self,
        agent_id: AgentId,
        goal: &str,
        original_request: &str,
        constraints: &[String],
        acceptance_criteria: &[String],
        cspace_hint: Option<&str>,
        mount_paths: &[std::path::PathBuf],
        workspace_context: Option<&str>,
        // Retained for API compatibility; recall timing moved to the brain.
        _session_ctx: &mut SessionContext,
        session_id: Option<String>,
        persistence_directive: Option<&Directive>,
        model_override: Option<&str>,
        role: Option<&str>,
        restore_state: Option<&serde_json::Value>,
    ) -> Result<ExecutionResult> {
        let prompt = build_user_prompt_inner(goal, acceptance_criteria);

        // Get active persona system prompt.
        let persona_prompt = self
            .persona_manager
            .as_ref()
            .map(|pm| pm.active_system_prompt())
            .filter(|s| !s.trim().is_empty());

        // Determine persona role for CSpace resolution.
        let persona_role = self
            .persona_manager
            .as_ref()
            .and_then(|pm| pm.get_active_persona().map(|p| p.role.clone()));

        // Resolve CSpace from persona role, hint, or default.
        let cspace = resolve_cspace(
            cspace_hint,
            persona_role.as_deref(),
            Some("worker"),
            agent_id,
        );

        // Build system prompt (without SKILL.md injection — capabilities are
        // surfaced through the CSpace tool set + semantic retrieval instead).
        let mut system_prompt = build_system_prompt_inner(
            goal,
            original_request,
            constraints,
            acceptance_criteria,
            workspace_context,
            persona_prompt.as_deref(),
            None,
            None,
        );

        // Semantic capability retrieval: find tools relevant to this task's goal.
        let capabilities_xml = if let Some(ref retriever) = self.tool_retriever {
            match retriever.embedder().embed(goal).await {
                Ok(query_vec) => {
                    let results = retriever.retrieve(&query_vec, 8);
                    if results.is_empty() {
                        None
                    } else {
                        let xml = crate::tools::retrieval::format_capability_index(&results);
                        tracing::info!(count = results.len(), "Retrieved relevant capabilities");
                        Some(xml)
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to embed goal for retrieval");
                    None
                }
            }
        } else {
            None
        };

        // Build kernel manifest from CSpace active domains.
        let kernel_manifest = {
            let domains = cspace.active_domains();
            if domains.is_empty() {
                None
            } else {
                Some(crate::tools::retrieval::build_kernel_manifest(&domains))
            }
        };

        // Rebuild system prompt with capabilities and manifest if available.
        if capabilities_xml.is_some() || kernel_manifest.is_some() {
            system_prompt = build_system_prompt_inner(
                goal,
                original_request,
                constraints,
                acceptance_criteria,
                workspace_context,
                persona_prompt.as_deref(),
                capabilities_xml.as_deref(),
                kernel_manifest.as_deref(),
            );
        }

        // Blend relevant memories into system prompt (RFC-047: brain recall).
        if let Some(brain) = &self.kernel_handle.brain {
            match brain.recall(goal, 3000).await {
                Some(context) => {
                    tracing::info!("Recalled brain context for task");
                    system_prompt.push_str(&format!("\n\n## Relevant Memory\n{context}\n",));
                }
                None => tracing::debug!("No brain context recalled"),
            }
        }

        // Inject learned strategy from SONA (RFC-020 Phase 2).
        if let Some(sona) = &self.sona {
            match sona.adapt(goal).await {
                Ok(Some(pattern)) if pattern.confidence > 0.5 => {
                    tracing::info!(
                        domain = %pattern.domain,
                        confidence = pattern.confidence,
                        "SONA learned pattern injected"
                    );
                    system_prompt.push_str(&format!(
                        "\n\n## Learned Strategy (confidence: {:.0}%)\n{}\n",
                        pattern.confidence * 100.0,
                        pattern.strategy,
                    ));
                }
                Ok(_) => tracing::debug!("No high-confidence SONA pattern found"),
                Err(e) => tracing::debug!(error = %e, "SONA adapt failed (non-fatal)"),
            }
        }

        // Blend relevant knowledge notes into system prompt (KnowledgeLens, RFC-003 Phase 3).
        match self
            .kernel_handle
            .knowledge_lens
            .recall_for_context(goal, 5)
            .await
        {
            Ok(ctx) if !ctx.notes.is_empty() => {
                tracing::info!(
                    notes = ctx.notes.len(),
                    memories = ctx.memories.len(),
                    "Recalled knowledge context for task"
                );
                let knowledge_blend = ctx
                    .notes
                    .iter()
                    .take(3)
                    .map(|n| format!("## {}\n\n{}", n.name, n.content))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                system_prompt.push_str("\n\n## Relevant Knowledge\n\n");
                system_prompt.push_str(&knowledge_blend);
            }
            Ok(_) => tracing::debug!("No knowledge recalled"),
            Err(e) => tracing::warn!(error = %e, "Failed to recall knowledge context"),
        }

        // RFC-032 + RFC-029 P2 + RFC-039: resolve the model. Precedence:
        //   1. `model_override` — set by RecoveryCoordinator during fallback
        //      retries. MUST win over role routing: if a role-mapped model
        //      is the one that just failed, letting role override recovery
        //      would loop the failure.
        //   2. `effective_role` — when the WS client supplied a per-message
        //      role hint (`env.role`), use it; otherwise fall back to the
        //      active persona's `role`. Read from config directly (not via
        //      the EngineApi facade) so the resolution stays on the hot
        //      path. RFC-039 makes the persona role participate here so
        //      `engine.role_routing[persona_role]` actually fires.
        //   3. — the configured default.
        let effective_role = role.or(persona_role.as_deref());
        let engine = self.engine_handle.get();
        let model_id = model_override
            .map(|s| s.to_string())
            .or_else(|| effective_role.and_then(|r| self.kernel_handle.engine.model_for_role(r)))
            .unwrap_or_else(|| engine.default_model_id().to_string());
        // Validates fail-fast: a bad model ID is rejected here at execute entry.
        engine.resolve_model(&model_id)?;
        // Synthetic per-execution ID for tracing.
        let exec_id = uuid::Uuid::new_v4();

        // Build the agent. Refresh config.model_id to the live value so every
        // downstream consumer (AgentConfig, legacy provider path, usage callback)
        // uses the same model as the interview/crystallize phases — no frozen boot
        // string that silently diverges from what interview used.
        let mut config = self.config.clone();
        config.model_id = model_id;
        let kernel_handle = Arc::clone(&self.kernel_handle);

        // Extract audit trail from kernel for TrailAuditSink wiring.
        let audit_trail: Option<Arc<AuditTrail>> =
            Some(Arc::clone(&self.kernel_handle.security.audit_trail));

        let (
            mut final_content,
            steps_completed,
            success,
            trajectory_steps,
            agent,
            tool_call_ids,
            tool_args_map,
            tool_error_map,
            tool_timestamps,
            total_input_tokens,
            total_output_tokens,
            reasoning_text,
            reasoning_segments,
        ) = {
            run_agent(
                &config,
                &engine,
                kernel_handle,
                self.sona.clone(),
                system_prompt,
                prompt,
                exec_id,
                goal.to_string(),
                agent_id,
                cspace,
                audit_trail,
                self.routing_stats.clone(),
                session_id.clone(),
                mount_paths,
                restore_state,
            )
            .await?
        };

        // ── Post-execution: safety net for empty final content ──
        //
        // oxi 0.32.0 removed max_iterations — the loop now exits naturally
        // when the LLM produces a text-only response (pi-agent behavior).
        // This block is kept as a safety net in case the LLM returns empty
        // text despite a natural exit (rare, but possible).
        if final_content.is_empty() && !trajectory_steps.is_empty() {
            let tool_summary: Vec<String> = trajectory_steps
                .iter()
                .enumerate()
                .map(|(i, step)| {
                    let truncated = if step.output.len() > 800 {
                        // Char-boundary safe truncation: roll back to the
                        // nearest UTF-8 boundary so multibyte sequences
                        // (Korean, CJK, emoji) don't panic on byte slicing.
                        let mut end = 800;
                        while end > 0 && !step.output.is_char_boundary(end) {
                            end -= 1;
                        }
                        format!("{}...", &step.output[..end])
                    } else {
                        step.output.clone()
                    };
                    format!("{}. [{}] {}", i + 1, step.input, truncated)
                })
                .collect();
            let summary_prompt = format!(
                "도구 실행 결과:\n\n{}\n\n\
                 위 결과를 바탕으로 사용자의 요청에 대해 자연스럽게 한국어로 답변해주세요. \
                 도구의 원시 출력을 그대로 복사하지 말고, 의미 있는 내용만 정리해서 전달하세요.",
                tool_summary.join("\n")
            );
            match agent.run(summary_prompt).await {
                Ok((response, _events)) => {
                    if !response.content.is_empty() {
                        tracing::info!(exec_id = %exec_id, "Post-execution summary generated");
                        final_content = response.content;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Post-execution summary failed");
                }
            }
        }

        // Map trajectory steps to tool call records for the execution result.
        // tool_call_ids[i] corresponds to trajectory_steps[i].
        let tool_calls: Vec<oxios_ouroboros::ToolCallRecord> = trajectory_steps
            .iter()
            .enumerate()
            .map(|(i, step)| {
                let tc_id = tool_call_ids.get(i).cloned().unwrap_or_default();
                let args_str = tool_call_ids
                    .get(i)
                    .and_then(|id| tool_args_map.get(id))
                    .cloned()
                    .unwrap_or_default();
                let is_error = tool_call_ids
                    .get(i)
                    .and_then(|id| tool_error_map.get(id))
                    .copied()
                    .unwrap_or(false);
                let timestamp = tool_call_ids
                    .get(i)
                    .and_then(|id| tool_timestamps.get(id))
                    .copied();
                let input_str = truncate_json_str(&args_str, 500);
                oxios_ouroboros::ToolCallRecord {
                    tool: step.input.clone(),
                    input: input_str,
                    output: step.output.clone(),
                    duration_ms: step.duration_ms,
                    is_error,
                    tool_call_id: tc_id,
                    timestamp,
                }
            })
            .collect();

        tracing::info!(
            exec_id = %exec_id,
            steps = steps_completed,
            success,
            tool_calls = tool_calls.len(),
            "AgentRuntime finished"
        );

        let result = ExecutionResult {
            output: final_content.clone(),
            steps_completed,
            success,
            tool_calls,
            failure_class: None,
            restore_state: None,
            tokens_input: total_input_tokens,
            tokens_output: total_output_tokens,
            model_id: self.engine_handle.get().default_model_id().to_string(),
            reasoning_text,
            reasoning_segments,
        };

        // RFC-016: Autonomous persistence hook.
        // Runs after successful execution, fire-and-forget.
        if let Some(directive) = persistence_directive
            && success
            && let Some(hook) = &self.persistence_hook
        {
            let already_saved_knowledge = trajectory_steps
                .iter()
                .any(|s| s.input == "knowledge" && s.output.contains("written successfully"));
            let hook = hook.clone();
            let directive_clone = directive.clone();
            let traj_clone = trajectory_steps.clone();
            let output_clone = final_content.clone();
            let sid = session_id.clone();
            // Compute the assistant message index for this execution.
            // Increment per-session counter, then use the pre-increment value.
            let msg_index = {
                let mut counter = self.session_msg_counter.lock();
                let idx = counter.entry(sid.clone().unwrap_or_default()).or_insert(0);
                let current = *idx;
                *idx += 1;
                current
            };
            tokio::spawn(async move {
                match hook
                    .evaluate(
                        &directive_clone,
                        &traj_clone,
                        &output_clone,
                        already_saved_knowledge,
                    )
                    .await
                {
                    Ok(plan) => {
                        if !plan.memory.is_empty() || !plan.knowledge.is_empty() {
                            tracing::info!(
                                memory = plan.memory.len(),
                                knowledge = plan.knowledge.len(),
                                message_index = msg_index,
                                "PersistenceHook executing plan"
                            );
                            let session_id = sid.unwrap_or_default();
                            hook.execute_plan(plan, &session_id, msg_index).await;
                        }
                    }
                    Err(e) => tracing::warn!(error = %e, "PersistenceHook evaluate failed"),
                }
            });
        }

        Ok(result)
    }
}

/// Create and run an oxicode-sdk `Agent` with CSpace-based tool registration.
///
/// Uses `engine.oxi().agent()` (AgentBuilder) for full middleware,
/// observability, and security integration from oxicode-sdk 0.23.0.
#[allow(clippy::too_many_arguments)]
async fn run_agent(
    config: &AgentRuntimeConfig,
    engine: &OxiosEngine,
    kernel_handle: Arc<KernelHandle>,
    sona: Option<Arc<crate::memory_agent::sona::SonaEngine>>,
    system_prompt: String,
    prompt: String,
    exec_id: uuid::Uuid,
    goal: String,
    agent_id: AgentId,
    cspace: crate::capability::CSpace,
    audit_trail: Option<Arc<AuditTrail>>,
    routing_stats: Option<Arc<crate::kernel_handle::RoutingStats>>,
    session_id: Option<String>,
    mount_paths: &[std::path::PathBuf],
    restore_state: Option<&serde_json::Value>,
) -> Result<(
    String,
    usize,
    bool,
    Vec<crate::memory_agent::sona::TrajectoryStep>,
    Arc<Agent>,
    Vec<String>,
    std::collections::HashMap<String, String>,
    std::collections::HashMap<String, bool>,
    std::collections::HashMap<String, chrono::DateTime<chrono::Utc>>,
    u64,
    u64,
    String,
    Vec<oxios_ouroboros::ReasoningSegment>,
)> {
    // Extract workspace.
    // RFC-025: the primary Mount's first path is the CWD; otherwise the
    // configured workspace_dir, otherwise a per-agent temp dir. Paths now
    // come only from Mounts — the legacy config.project_paths fallback was
    // removed when the RFC-025 migration completed.
    let workspace = if !mount_paths.is_empty() {
        mount_paths[0].clone()
    } else if let Some(ws) = &config.workspace_dir {
        ws.clone()
    } else {
        std::env::temp_dir()
            .join("oxios-agent-workspace")
            .join(agent_id.to_string())
    };

    // Ensure workspace exists.
    let _ = std::fs::create_dir_all(&workspace);

    tracing::debug!(workspace = %workspace.display(), "Agent workspace scoped");

    // Ensure all paths the agent might access are in allowed_paths.
    //
    // AgentLifecycleManager::ensure_permissions() adds kernel.workspace (~/.oxios/workspace),
    // but the agent operates in different directories depending on context:
    //
    //   1. Process CWD — oxicode-sdk 0.35+ bakes `workspace_dir` into file tools
    //      via `with_cwd`, so ReadTool/LsTool resolve relatives against the
    //      workspace, NOT the process CWD. However, oxios's own CSpace tools
    //      (kernel-bridge tools wrapped in GatedTool) and bash/exec
    //      subprocesses may still resolve against the process CWD. We grant
    //      it as a safety net so those tools aren't denied by GatedTool.
    //   2. The designated workspace — computed from mount_paths / workspace_dir / temp.
    //   3. Kernel workspace — state store path for sessions, etc.
    //   4. /tmp -- general temp file access.
    //
    // All four must be in allowed_paths before GatedTool wraps any tool.
    {
        use crate::access_manager::{Role, Subject};
        let agent_name = format!("agent-{agent_id}");
        let mut am = kernel_handle.exec.access_manager().lock();
        let perms = am.get_or_create_permissions(&agent_name);

        // 1. CWD -- critical: oxicode-sdk resolves relative paths here
        if let Ok(cwd) = std::env::current_dir() {
            let cwd_pattern = format!("{}/**", cwd.to_string_lossy().trim_end_matches('/'));
            if !perms.allowed_paths.iter().any(|p| p == &cwd_pattern) {
                perms.allow_path(&cwd_pattern);
                tracing::debug!(
                    agent = %agent_name,
                    path = %cwd_pattern,
                    "Added CWD to agent allowed paths"
                );
            }
        }

        // 2. Designated workspace
        let ws_pattern = format!("{}/**", workspace.to_string_lossy().trim_end_matches('/'));
        if !perms.allowed_paths.iter().any(|p| p == &ws_pattern) {
            perms.allow_path(&ws_pattern);
        }

        // 2b. RFC-025: every bound Mount grants path access.
        //     This fixes the latent gap where only project_paths[0] was
        //     accessible — now all Mount paths (multi-path work) are allowed.
        //     Parent patterns already covering a path are skipped.
        for mount_path in mount_paths {
            let pattern = format!("{}/**", mount_path.to_string_lossy().trim_end_matches('/'));
            if !perms.allowed_paths.iter().any(|p| p == &pattern) {
                perms.allow_path(&pattern);
                tracing::debug!(
                    agent = %agent_name,
                    path = %pattern,
                    "Added Mount path to agent allowed paths (RFC-025)"
                );
            }
        }

        // 3. Kernel workspace (state store path)
        let kernel_ws = kernel_handle
            .state
            .workspace_path()
            .to_string_lossy()
            .to_string();
        let kernel_ws_pattern = format!("{}/**", kernel_ws.trim_end_matches('/'));
        if kernel_ws_pattern != ws_pattern
            && !perms.allowed_paths.iter().any(|p| p == &kernel_ws_pattern)
        {
            perms.allow_path(&kernel_ws_pattern);
        }

        // 4. /tmp -- for general temp file access
        if !perms.allowed_paths.iter().any(|p| p == "/tmp/**") {
            perms.allow_path("/tmp/**");
        }

        // Ensure RBAC Superuser role so AccessGate Layer 1 passes.
        let rbac_subject = Subject::Agent(agent_id);
        am.rbac_manager_mut()
            .assign_role(rbac_subject, Role::Superuser);
    }

    // Start distributed trace span for this agent execution.
    let _trace_guard = crate::observability::tracer().start(
        format!("exec-{}", &exec_id.to_string()[..8]).as_str(),
        oxicode_sdk::SpanKind::Agent,
    );

    // ── Register tools based on CSpace (with access gate) ──
    let registry = ToolRegistry::new();
    let search_cache = Arc::new(SearchCache::new());

    // Build agent context for security
    let agent_context = AgentContext {
        agent_id,
        agent_name: format!("agent-{agent_id}"),
        cspace: Arc::new(cspace.clone()),
    };

    // Build audit sink: TrailAuditSink (Merkle chain + JSONL) when audit_trail
    // is available, otherwise fall back to TracingAuditSink.
    let audit_sink: Arc<dyn crate::access_manager::AuditSink> = if let Some(trail) = audit_trail {
        let audit_path = kernel_handle
            .state
            .workspace_path()
            .join("audit")
            .join("access.jsonl");
        Arc::new(TrailAuditSink::new(trail, audit_path))
    } else {
        Arc::new(TracingAuditSink)
    };
    // Build access gate from kernel's security infrastructure
    let access_gate = Arc::new(AccessGate::new(
        kernel_handle.exec.access_manager().clone(),
        Arc::new(kernel_handle.exec.config_snapshot()),
        audit_sink,
    ));

    // Build the RFC-035 ApprovalGate: declared policies + config overrides +
    // the always-on SecurityBlacklist as the single global resolver, plus
    // the ExecPolicyResolver as a dynamic resolver for `exec` so structured
    // exec with an allowed binary auto-runs without approval in Manual mode.
    // The resolver takes a SNAPSHOT of `ExecConfig.allowed_commands` at
    // construction time; runtime reloads via `PUT /api/config` do NOT
    // propagate. Sharing the live `Arc<RwLock<Vec<String>>>` would require
    // touching `config.rs` (off-staging), so the brief sanctions snapshot
    // + note the limitation. Reload takes effect on next agent run.
    let approval_config = kernel_handle.infra.approval_config_handle();
    let exec_snapshot = kernel_handle.exec.config_snapshot();
    let exec_resolver = crate::approval::ExecPolicyResolver {
        allowed_commands: Arc::new(parking_lot::RwLock::new(
            exec_snapshot.allowed_commands.clone(),
        )),
    };
    let mut dynamic_resolvers = std::collections::HashMap::new();
    dynamic_resolvers.insert(
        "exec".to_string(),
        Box::new(exec_resolver) as Box<dyn crate::approval::ToolPolicyResolver>,
    );
    let approval_gate = Arc::new(crate::approval::ApprovalGate::with_dynamic_resolvers(
        crate::approval::default_tool_policy_map(),
        approval_config,
        vec![Box::new(crate::approval::SecurityBlacklist::new(
            crate::approval::default_blacklist_rules(),
        ))],
        dynamic_resolvers,
    ));
    let approval_event_bus = kernel_handle.infra.event_bus_clone();
    let approval_pending = kernel_handle.infra.pending_tool_approvals();
    let path_access_pending = kernel_handle.infra.pending_path_access();

    // Pre-initialize the browser engine so the synchronous registration path
    // can attach browse tools. The engine is a shared `OnceCell` on the
    // KernelHandle, so this is a one-time cost; agents without a Browser
    // capability still skip tool registration. No-op without `browser`.
    #[cfg(feature = "browser")]
    if let Some(browser) = &kernel_handle.browser
        && let Err(e) = browser.engine().await
    {
        tracing::warn!("browser engine unavailable, browse tools disabled: {e:#}");
    }

    register_tools_from_cspace_gated(
        &registry,
        &kernel_handle,
        &cspace,
        search_cache,
        agent_id,
        access_gate,
        agent_context,
        Some(approval_gate),
        Some(approval_event_bus),
        Some(approval_pending),
        Some(path_access_pending),
    );

    tracing::info!(
        exec_id = %exec_id,
        capabilities = cspace.len(),
        "Tools registered from CSpace"
    );

    // ── Build AgentConfig ──
    //
    // RFC-014 Phase D: `system_prompt` is also passed to the new
    // `AgentBuilder::system_prompt()` (which overrides the value embedded
    // in `AgentConfig` at build time). We clone here so the builder path
    // can consume the value while the legacy `Agent::new_with_resolver`
    // path still sees it in the config.
    let agent_config = AgentConfig {
        name: format!("agent-{agent_id}"),
        description: None,
        model_id: config.model_id.clone(),
        system_prompt: Some(system_prompt.clone()),
        timeout_seconds: 300,
        temperature: config
            .model_params
            .as_ref()
            .and_then(|p| p.temperature)
            .or(Some(0.7)),
        max_tokens: config
            .model_params
            .as_ref()
            .and_then(|p| p.max_tokens)
            .map(|v| v as usize)
            .or(Some(8192)),
        compaction_strategy: CompactionStrategy::Threshold(0.8),
        compaction_instruction: None,
        context_window: 128_000,
        workspace_dir: Some(workspace.clone()),
        output_mode: None,
        provider_options: config.provider_options.clone(),
        session_id: None,
        // RFC-035 Phase B/C: pass through gap 1/3 config to oxicode-sdk 0.54.0+.
        max_tool_result_bytes: config.max_tool_result_bytes,
        // subagent_depth = CURRENT depth (0 = top-level). The in-process
        // max is hardcoded to 3 in oxicode-agent (subagent.rs:649). Do NOT
        // wire a "max depth" config here — it would make the agent start
        // at depth N and fail every subagent call immediately.
        subagent_depth: 0,
        // RFC-035 Phase C: wire the in-process sub-agent runner so the
        // `subagent` tool delegates in-process (no CLI subprocess).
        subagent_runner: Some(
            crate::subagent_runner::OxiosSubagentRunner::new(engine.oxi().clone())
                .into_trait_object(),
        ),
        ..Default::default()
    };

    // ── Build Agent via AgentBuilder (RFC-014 Phase D) ──
    //
    // The legacy rate-limited `ProviderPool` path (oxicode-sdk `ProviderPool` /
    // `RateLimitPolicy`) was removed in oxicode-sdk 0.61 with no direct
    // replacement; oxios never set `provider_rpm > 0` in production, so the
    // single AgentBuilder path below is the only path. Engine-level
    // `authorizer` / `tracer` / `cost_tracker` are propagated through the
    // builder methods.
    let mut builder = engine
        .oxi()
        .agent(agent_config)
        .workspace(&workspace)
        .system_prompt(system_prompt);

    // CSpace-based tool registration is oxios-specific and is preserved.
    //
    // The builder's `.tool()` method takes `impl AgentTool + 'static`
    // (a concrete value), but oxios' CSpace tools are `Arc<dyn AgentTool>`.
    // The SDK does not expose a way to inject a pre-built `ToolRegistry`
    // into the builder, so we register them on the agent's tool registry
    // after `build()` returns. This keeps CSpace semantics intact.
    //
    // We capture the tool names now and apply them once the agent exists.
    let cspace_tool_arcs: Vec<Arc<dyn oxicode_sdk::AgentTool>> = registry
        .names()
        .into_iter()
        .filter_map(|name| registry.get(&name))
        .collect();

    // Engine-level observability/security → AgentBuilder (new API).
    if let Some(auth) = engine.authorizer() {
        builder = builder.authorizer(auth.clone());
    }
    if let Some(tracer) = engine.tracer() {
        builder = builder.tracer(tracer.clone());
    }
    if let Some(ct) = engine.cost_tracker() {
        builder = builder.cost_tracker(ct.clone());
    }

    // Middleware: AgentBuilder convenience helpers replace the manual
    // `MiddlewarePipeline` + `build_hooks()` + `set_hooks()` triple.
    if config.rate_limit_per_minute > 0 {
        builder = builder.with_rate_limit(config.rate_limit_per_minute);
    }
    if config.token_budget > 0 {
        builder = builder.with_token_budget(config.token_budget);
    }
    if config.audit_tool_calls {
        builder = builder.with_logging();
    }

    let built = builder.build()?;
    let agent = Arc::new(built);

    // Attach CSpace tools to the agent's tool registry.
    // `Agent::tools()` returns the same `Arc<ToolRegistry>` that
    // `AgentBuilder` populated, so `register_arc` is the canonical
    // extension point for `Arc<dyn AgentTool>` values.
    let agent_tools = agent.tools();
    for tool in cspace_tool_arcs {
        agent_tools.register_arc(tool);
    }

    // RFC-029 P2b: restore conversation state from a prior failed run
    // so the new agent (with a fallback model) continues from the
    // checkpoint rather than restarting from scratch.
    if let Some(state) = restore_state {
        agent.import_state(state.clone()).unwrap_or_else(|e| {
            tracing::warn!(agent_id = %agent_id, error = %e, "Failed to restore agent state");
        });
    }

    // Shared mutable state for the event callback.
    let exec_state = Arc::new(Mutex::new(ExecuteState::default()));
    let exec_state_cb = Arc::clone(&exec_state);
    let brain_for_callback: Option<Arc<crate::brain::BrainConnection>> =
        kernel_handle.brain.as_ref().map(|b| b.connection());
    let session_id_for_callback = exec_id.to_string();
    let model_id_for_callback = config.model_id.clone();
    let agent_id_for_callback = agent_id.to_string();
    let routing_stats_for_cb = routing_stats.clone();
    // RFC-015: real-time event publishing for chat transparency.
    // Falls back to None when the caller did not opt in.
    let transparency_session: Option<String> = session_id.clone();
    let kernel_handle_for_cb: Arc<KernelHandle> = Arc::clone(&kernel_handle);
    // P1 chat transparency: per-session streaming sink registry. The
    // callback looks up the sink for this session and pushes live text
    // deltas. Lookup misses silently (no gateway registered → not a chat).
    let streaming_sinks_for_cb: Arc<crate::streaming_sink::StreamingSinkRegistry> =
        Arc::clone(&kernel_handle.streaming_sinks);
    // Run the agent with live-streaming events.
    //
    // oxicode-sdk's `Agent::run_streaming` awaits the FULL `run_with_channel`
    // future before draining its internal std channel, so every callback —
    // text/reasoning deltas included — fires only after the response
    // completes, and the Web UI received each answer as one end-of-turn
    // burst. `run_tokio_stream` hands back the live receiver instead, so
    // deltas reach the streaming sink as the provider emits them. Its
    // completion `Response` is a stub (`content` empty) — this call site
    // already assembles the result from `exec_state`, so only the Ok/Err
    // signal is taken from it.
    let mut sent_model_for_cb: bool = false;
    let mut on_event = move |event: AgentEvent| {
        if !sent_model_for_cb
            && let Some(ref sid) = transparency_session
            && !model_id_for_callback.is_empty()
            && let Some(tx) = streaming_sinks_for_cb.lookup(sid)
        {
            let _ = tx.try_send(StreamDelta::Model(model_id_for_callback.clone()));
            sent_model_for_cb = true;
        }
        let mut s = exec_state_cb.lock();
        match event {
            AgentEvent::ToolExecutionStart {
                tool_name,
                tool_call_id,
                args,
                context,
                ..
            } => {
                // Record start time and push a placeholder step.
                let idx = s.trajectory_steps.len();
                s.pending_tools
                    .insert(tool_call_id.clone(), (std::time::Instant::now(), idx));
                s.tool_args_map.insert(
                    tool_call_id.clone(),
                    serde_json::to_string(&args).unwrap_or_default(),
                );
                s.tool_timestamps
                    .insert(tool_call_id.clone(), chrono::Utc::now());
                s.tool_call_ids.push(tool_call_id.clone());
                s.trajectory_steps
                    .push(crate::memory_agent::sona::TrajectoryStep {
                        input: tool_name.clone(),
                        output: String::new(),
                        duration_ms: 0,
                        confidence: 0.0,
                    });
                // RFC-015: broadcast tool start so Web UI can show progress.
                if let Some(ref sid) = transparency_session {
                    let context_json = context
                        .as_ref()
                        .map(serde_json::to_value)
                        .transpose()
                        .unwrap_or(None);
                    let _ = kernel_handle_for_cb
                        .infra
                        .publish(KernelEvent::ToolExecutionStarted {
                            session_id: sid.clone(),
                            tool_name: tool_name.clone(),
                            tool_call_id: tool_call_id.clone(),
                            tool_args: args.clone(),
                            context: context_json,
                        });
                }
            }
            AgentEvent::ToolExecutionUpdate {
                tool_call_id,
                tool_name,
                partial_result,
                tab_id,
                context,
            } => {
                // RFC-015: forward real-time progress to the event bus
                // so the Web UI can show a spinner and progress text
                // while the tool is still executing. Best-effort —
                // publish failures (e.g. lagged subscribers) are ignored.
                //
                // `tab_id` and `context` come from oxicode-agent 0.29+
                // (ToolCallContext: PageVisit, WebSearch, etc.).
                // Older agent versions won't send these — they default
                // to None and the UI gracefully ignores them.
                if let Some(ref sid) = transparency_session {
                    let context_json = context
                        .as_ref()
                        .map(serde_json::to_value)
                        .transpose()
                        .unwrap_or(None);
                    let _ =
                        kernel_handle_for_cb
                            .infra
                            .publish(KernelEvent::ToolExecutionProgress {
                                session_id: sid.clone(),
                                tool_call_id: tool_call_id.clone(),
                                tool_name: tool_name.clone(),
                                progress: partial_result,
                                tab_id,
                                context: context_json,
                            });
                }
            }
            AgentEvent::ToolExecutionEnd {
                tool_name,
                tool_call_id,
                is_error,
                result,
                ..
            } => {
                if !is_error {
                    s.steps_completed += 1;
                }
                // Look up the exact step by tool_call_id.
                let mut duration_ms: u64 = 0;
                let mut summary = String::new();
                if let Some((start, idx)) = s.pending_tools.remove(tool_call_id.as_str()) {
                    duration_ms = start.elapsed().as_millis() as u64;
                    if let Some(step) = s.trajectory_steps.get_mut(idx) {
                        summary = summarize_tool_result(&result.content, 200);
                        step.output = summary.clone();
                        step.duration_ms = duration_ms;
                        step.confidence = if is_error { 0.3 } else { 0.8 };
                    }
                }
                s.tool_error_map.insert(tool_call_id.clone(), is_error);
                // RFC-015: broadcast tool completion.
                if let Some(ref sid) = transparency_session {
                    let _ =
                        kernel_handle_for_cb
                            .infra
                            .publish(KernelEvent::ToolExecutionFinished {
                                session_id: sid.clone(),
                                tool_call_id: tool_call_id.clone(),
                                tool_name: tool_name.clone(),
                                duration_ms,
                                is_error,
                                output_summary: summary,
                            });
                }
            }
            AgentEvent::AgentEnd {
                messages,
                stop_reason,
                ..
            } => {
                if let Some(oxicode_sdk::Message::Assistant(a)) = messages.last() {
                    s.final_content = a.text_content();
                }
                // oxi 0.32.0: loop exits naturally when LLM produces text-only
                // response (StopReason::Stop). Error/Aborted = failure.
                // ToolUse should not occur at AgentEnd in 0.32.0 (the loop
                // continues until text-only), but treat it as non-failure
                // since tool calls were executed successfully.
                s.success = matches!(stop_reason.as_deref(), Some("Stop") | Some("ToolUse"));
            }
            AgentEvent::Error { message, .. } => {
                s.final_content = message.clone();
                s.success = false;
            }
            AgentEvent::Usage {
                input_tokens,
                output_tokens,
            } => {
                // Accumulate totals for ExecutionResult.
                s.total_input_tokens += input_tokens as u64;
                s.total_output_tokens += output_tokens as u64;

                // Record token usage to cost tracker (existing).
                let agent_label = format!("agent-{agent_id_for_callback}");
                crate::observability::cost_tracker().record(
                    &agent_label,
                    &oxicode_sdk::Model::new(
                        &model_id_for_callback,
                        &model_id_for_callback,
                        oxicode_sdk::Api::OpenAiCompletions,
                        "unknown",
                        "https://unknown.com",
                    ),
                    oxicode_sdk::TokenUsage {
                        input: input_tokens as u64,
                        output: output_tokens as u64,
                        cache_read: 0,
                        cache_write: 0,
                    },
                );

                // Record to routing stats (RFC-011).
                if let Some(stats) = &routing_stats_for_cb {
                    let cost = crate::kernel_handle::engine_api::estimate_cost(
                        &model_id_for_callback,
                        input_tokens as u64,
                        output_tokens as u64,
                    );
                    stats.record_model_usage(&model_id_for_callback, cost);
                }
                // RFC-015: publish cumulative token usage.
                if let Some(ref sid) = transparency_session {
                    let _ = kernel_handle_for_cb
                        .infra
                        .publish(KernelEvent::TokenUsageUpdate {
                            session_id: sid.clone(),
                            input_tokens: input_tokens as u64,
                            output_tokens: output_tokens as u64,
                        });
                }
            }
            AgentEvent::Compaction {
                event: CompactionEvent::Completed { result, .. },
            } => {
                handle_compaction(
                    result.summary.clone(),
                    session_id_for_callback.clone(),
                    brain_for_callback.clone(),
                );
                // RFC-015: compaction is a form of reasoning — expose it.
                if let Some(ref sid) = transparency_session {
                    let _ = kernel_handle_for_cb
                        .infra
                        .publish(KernelEvent::ReasoningFragment {
                            session_id: sid.clone(),
                            content: result.summary.clone(),
                            source: "compaction".to_string(),
                        });
                }
            }
            AgentEvent::Compaction {
                event: CompactionEvent::Triggered { source, .. },
            } => {
                // RFC-035 gap 2: surface the trigger source so the
                // 3-4× heuristic drift (pre-0.53 silent no-op) is
                // observable end-to-end. The match arm itself does
                // not act on compaction — the SDK handles the
                // actual trigger — we only publish a KernelEvent.
                if let Some(ref sid) = transparency_session {
                    let _ = kernel_handle_for_cb
                        .infra
                        .publish(KernelEvent::CompactionTriggered {
                            session_id: Some(sid.clone()),
                            source,
                        });
                } else {
                    let _ = kernel_handle_for_cb
                        .infra
                        .publish(KernelEvent::CompactionTriggered {
                            session_id: None,
                            source,
                        });
                }
            }
            AgentEvent::MessageUpdate { delta, .. } => {
                // SDK 0.66.0: unified streaming delta. Text deltas arrive
                // ONLY here (the legacy TextChunk variant is never emitted
                // in 0.66.0), so this arm is the live text path.
                match delta {
                    oxicode_agent::StreamDelta::Text(text) => {
                        if let Some(ref sid) = transparency_session
                            && let Some(tx) = streaming_sinks_for_cb.lookup(sid)
                        {
                            let _ = tx.try_send(StreamDelta::Text(text));
                        }
                    }
                    oxicode_agent::StreamDelta::Thinking(text) if !text.is_empty() => {
                        // SDK emits thinking deltas BOTH as legacy
                        // ThinkingDelta and as MessageUpdate::Thinking.
                        // We handle them here and drop the legacy arm to
                        // avoid double-emission to the UI.
                        const REASONING_CAP: usize = 4096;
                        if s.reasoning_text.len() < REASONING_CAP {
                            s.reasoning_text.push_str(&text);
                            if s.reasoning_text.len() > REASONING_CAP {
                                s.reasoning_text.truncate(REASONING_CAP);
                            }
                        }
                        // Block-stream transparency: capture into a
                        // positioned segment keyed by tools started so far.
                        if s.reasoning_bytes < REASONING_CAP {
                            let budget = REASONING_CAP - s.reasoning_bytes;
                            let mut chunk = String::new();
                            for ch in text.chars() {
                                if chunk.len() + ch.len_utf8() > budget {
                                    break;
                                }
                                chunk.push(ch);
                            }
                            if !chunk.is_empty() {
                                s.reasoning_bytes += chunk.len();
                                let pos = s.trajectory_steps.len();
                                if s.reasoning_segments
                                    .last()
                                    .is_some_and(|l| l.before_step == pos)
                                {
                                    s.reasoning_segments
                                        .last_mut()
                                        .expect("checked Some in the enclosing if condition")
                                        .text
                                        .push_str(&chunk);
                                } else {
                                    s.reasoning_segments
                                        .push(oxios_ouroboros::ReasoningSegment {
                                            before_step: pos,
                                            text: chunk,
                                        });
                                }
                            }
                        }
                        if let Some(ref sid) = transparency_session
                            && let Some(tx) = streaming_sinks_for_cb.lookup(sid)
                        {
                            let _ = tx.try_send(StreamDelta::Thinking);
                            let _ = tx.try_send(StreamDelta::ThinkingDelta(text));
                        }
                    }
                    oxicode_agent::StreamDelta::Sync => {
                        // Tool-call end marker — not a reasoning span end.
                        // The authoritative reasoning.end marker comes from
                        // the legacy ThinkingEnd event (handled below).
                    }
                    // Empty Thinking delta — the guarded
                    // `Thinking(text) if !text.is_empty()` arm above
                    // doesn't count for exhaustiveness, so handle the
                    // empty case here as a no-op.
                    _ => {}
                }
            }
            AgentEvent::Thinking => {
                // P4: signal-only — LiveActivityBar flips to "추론 중".
                // Sent through the same connection-scoped sink so the
                // state change is visible to the live chat only (no
                // EventBus broadcast for a transient UI signal).
                if let Some(ref sid) = transparency_session
                    && let Some(tx) = streaming_sinks_for_cb.lookup(sid)
                {
                    let _ = tx.try_send(StreamDelta::Thinking);
                }
            }
            AgentEvent::ThinkingEnd => {
                // oxi 0.58+: explicit reasoning-end signal. Drives the
                // authoritative `reasoning.end` close for providers that
                // emit it. The collector resets its `was_reasoning` flag
                // so the first-Text fallback (reasoning_content models)
                // never double-fires.
                if let Some(ref sid) = transparency_session
                    && let Some(tx) = streaming_sinks_for_cb.lookup(sid)
                {
                    let _ = tx.try_send(StreamDelta::ThinkingEnd);
                }
            }
            AgentEvent::ToolCallDelta {
                tool_call_id,
                args_delta,
            } => {
                // oxi 0.58+: partial tool-call args streamed by the LLM
                // while it is still constructing the call (before
                // ToolExecutionStart). Each `args_delta` is a raw JSON
                // fragment; consumers accumulate per `tool_call_id`.
                if let Some(ref sid) = transparency_session {
                    let _ = kernel_handle_for_cb
                        .infra
                        .publish(KernelEvent::ToolArgsDelta {
                            session_id: sid.clone(),
                            tool_call_id: tool_call_id.clone(),
                            args_delta: args_delta.clone(),
                        });
                }
            }
            _ => {}
        }
    };

    let (mut event_rx, run_handle) = match agent.run_tokio_stream(prompt).await {
        Ok(pair) => pair,
        Err(e) => {
            // Mirror the terminal-error path below (breaker + restore state).
            LLM_CIRCUIT_BREAKER.record_failure();
            crate::metrics::get_metrics()
                .llm_circuit_breaker_state
                .set(match LLM_CIRCUIT_BREAKER.state() {
                    BreakerState::Closed => 0.0,
                    BreakerState::HalfOpen => 0.5,
                    BreakerState::Open => 1.0,
                });
            let restore_state = agent.export_state().ok();
            return Err(crate::resilience::AgentRunError::wrap(e, restore_state).into());
        }
    };
    while let Some(event) = event_rx.recv().await {
        on_event(event);
    }
    // The handle resolves to the stub `Response` on success; a JoinError
    // means the spawned agent task panicked or was cancelled.
    let result = run_handle
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("agent stream task failed: {e}").into()));

    // Record circuit breaker result after agent execution. The breaker is
    // observational (see `LLM_CIRCUIT_BREAKER`); the gauge reflects its real
    // state machine rather than just "last call errored".
    if result.is_err() {
        LLM_CIRCUIT_BREAKER.record_failure();
    } else {
        LLM_CIRCUIT_BREAKER.record_success();
    }
    let breaker_state = match LLM_CIRCUIT_BREAKER.state() {
        BreakerState::Closed => 0.0,
        BreakerState::HalfOpen => 0.5,
        BreakerState::Open => 1.0,
    };
    crate::metrics::get_metrics()
        .llm_circuit_breaker_state
        .set(breaker_state);

    if let Err(e) = result {
        tracing::error!(exec_id = %exec_id, error = %e, "Agent failed");
        // RFC-029 P2b: capture the agent's accumulated conversation state
        // before returning. The supervisor's Err arm unwraps AgentRunError
        // and populates ExecutionResult.restore_state so the coordinator
        // can inject it into a retry with a different model (snapshot→restore).
        let restore_state = agent.export_state().ok();
        return Err(crate::resilience::AgentRunError::wrap(e, restore_state).into());
    }

    let s = exec_state.lock();
    tracing::info!(
        exec_id = %exec_id,
        steps = s.steps_completed,
        success = s.success,
        "Agent completed"
    );

    // Record trajectory to SONA learning engine (RFC-020 Phase 2).
    // Fire-and-forget: don't block the result on learning.
    if !s.trajectory_steps.is_empty()
        && let Some(sona) = &sona
    {
        let steps = s.trajectory_steps.clone();
        let success = s.success;
        let sona = Arc::clone(sona);
        let domain = infer_domain(&goal);
        tokio::spawn(async move {
            let verdict = if success {
                crate::memory_agent::sona::Verdict::Success
            } else {
                crate::memory_agent::sona::Verdict::Failure
            };
            let trajectory = crate::memory_agent::sona::Trajectory::new(steps, verdict, &domain);
            if let Err(e) = sona.record(trajectory).await {
                tracing::debug!(error = %e, "SONA trajectory recording failed (non-fatal)");
            }
        });
    }

    Ok((
        s.final_content.clone(),
        s.steps_completed,
        s.success,
        s.trajectory_steps.clone(),
        agent,
        s.tool_call_ids.clone(),
        s.tool_args_map.clone(),
        s.tool_error_map.clone(),
        s.tool_timestamps.clone(),
        s.total_input_tokens,
        s.total_output_tokens,
        s.reasoning_text.clone(),
        s.reasoning_segments.clone(),
    ))
}

/// Summarize a tool result string to fit within `max_len` characters.
///
/// Uses char-aware truncation to avoid panicking on multi-byte UTF-8
/// (e.g., Korean, CJK, emoji).
fn summarize_tool_result(result: &str, max_len: usize) -> String {
    let trimmed = result.trim();
    if trimmed.chars().count() <= max_len {
        return trimmed.to_string();
    }
    // Take the first line or truncate.
    let first_line = trimmed.lines().next().unwrap_or("");
    if first_line.chars().count() <= max_len {
        first_line.to_string()
    } else {
        let take = max_len.saturating_sub(3);
        let truncated: String = if take == 0 {
            first_line.chars().take(max_len).collect()
        } else {
            first_line.chars().take(take).collect()
        };
        format!("{truncated}...")
    }
}
fn truncate_json_str(json_str: &str, max_len: usize) -> String {
    if json_str.len() <= max_len {
        return json_str.to_string();
    }
    // Saturating sub avoids underflow panic when max_len < 3; if there
    // isn't room for an ellipsis, return as many chars as fit.
    let take = max_len.saturating_sub(3);
    if take == 0 {
        return json_str.chars().take(max_len).collect();
    }
    let truncated: String = json_str.chars().take(take).collect();
    format!("{truncated}...")
}

/// Infer a domain category from the goal for SONA trajectory grouping.
///
/// Extracts the core verb + object from the goal to create a meaningful
/// domain label. Falls back to "general" for unrecognizable patterns.
fn infer_domain(goal: &str) -> String {
    let lower = goal.to_lowercase();
    let keywords: Vec<&str> = lower.split_whitespace().take(8).collect();

    // Check for known domain indicators.
    if keywords.iter().any(|k| {
        [
            "test",
            "tests",
            "spec",
            "testing",
            "assert",
            "unit test",
            "integration",
        ]
        .contains(k)
    }) {
        return "testing".to_string();
    }
    if keywords
        .iter()
        .any(|k| ["deploy", "release", "publish", "ship"].contains(k))
    {
        return "deployment".to_string();
    }
    if keywords
        .iter()
        .any(|k| ["fix", "bug", "patch", "repair", "debug"].contains(k))
    {
        return "bugfix".to_string();
    }
    if keywords
        .iter()
        .any(|k| ["refactor", "restructure", "reorganize", "rewrite"].contains(k))
    {
        return "refactoring".to_string();
    }
    if keywords
        .iter()
        .any(|k| ["doc", "document", "readme", "guide", "explain"].contains(k))
    {
        return "documentation".to_string();
    }
    if keywords
        .iter()
        .any(|k| ["build", "create", "implement", "add", "make", "new"].contains(k))
    {
        return "development".to_string();
    }
    if keywords
        .iter()
        .any(|k| ["analyze", "review", "audit", "inspect", "check"].contains(k))
    {
        return "analysis".to_string();
    }
    if keywords
        .iter()
        .any(|k| ["config", "setup", "install", "configure", "init"].contains(k))
    {
        return "configuration".to_string();
    }

    // Fallback: first 2 meaningful words
    let meaningful: Vec<&str> = lower
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .take(2)
        .collect();
    if meaningful.len() >= 2 {
        meaningful.join("_")
    } else {
        "general".to_string()
    }
}

/// Handle compaction completion by storing the summary in the brain.
///
/// Extracts the compaction summary from the event and spawns a background
/// task to persist it via the brain daemon (RFC-047).
fn handle_compaction(
    summary: String,
    session_id: String,
    brain: Option<Arc<crate::brain::BrainConnection>>,
) {
    let content = if session_id.is_empty() {
        summary
    } else {
        format!("[session {session_id}]\n{summary}")
    };
    tokio::spawn(async move {
        if let Some(brain) = brain
            && brain.remember(&content, "compaction").await.is_none()
        {
            tracing::debug!("brain unavailable — compaction summary not stored");
        }
    });
}

/// Build a system prompt from a Directive and ExecEnv (RFC-027).
///
/// Maps [`Directive`] fields (`goal`, `original_request`, `constraints`,
/// `acceptance_criteria`) and [`ExecEnv`] fields (`workspace_context`) into
#[allow(dead_code)]
fn build_directive_system_prompt(
    directive: &Directive,
    env: &ExecEnv,
    persona_prompt: Option<&str>,
    capabilities_xml: Option<&str>,
    kernel_manifest: Option<&str>,
) -> String {
    build_system_prompt_inner(
        &directive.goal,
        &directive.original_request,
        &directive.constraints,
        &directive.acceptance_criteria,
        env.workspace_context.as_deref(),
        persona_prompt,
        capabilities_xml,
        kernel_manifest,
    )
}

/// Output protocol for the Web UI Artifact feature. Appended to every system
/// prompt so the model emits `<lobeArtifact>` tags the frontend can preview.
///
/// The `type` values are pinned to the EXACT strings the frontend matches in
/// `tagTypeToArtifactType` (web/src/types/artifact.ts). Unrecognized types
/// silently render as a plain code listing — so do not paraphrase them.
const ARTIFACT_PROTOCOL: &str = "\n\n\
    ## Artifacts\n\
    When you produce substantial, self-contained content the user will want to\n\
    view or interact with separately — a complete HTML page, an SVG graphic, a\n\
    Mermaid diagram, or an interactive React component — wrap it in an artifact\n\
    tag so the UI shows a live preview panel:\n\n\
    <lobeArtifact type=\"...\" title=\"...\" identifier=\"...\">\n\
    ...the full content...\n\
    </lobeArtifact>\n\n\
    Use exactly one of these `type` values (others are not recognised):\n\
    - `text/html`                       → an HTML document or fragment\n\
    - `image/svg+xml`                   → an SVG graphic\n\
    - `application/lobe.artifacts.mermaid` → a Mermaid diagram\n\
    - `application/lobe.artifacts.react`   → a React component (JSX/TSX)\n\n\
    - `title` — a short human title for the panel.\n\
    - `identifier` — a unique kebab-case id, e.g. `sales-dashboard`.\n\
    Put the full, runnable content INSIDE the tag (not in a separate fence).\n\n\
    Do NOT wrap non-visual code. Shell commands, Python, Rust, JSON, config, or\n\
    short snippets that are part of an explanation belong in a normal fenced\n\
    code block. Use an artifact only for content the user would open to view,\n\
    not copy-and-paste. Limit one artifact per self-contained piece.\n";
/// Shared system-prompt builder for the directive path.
///
/// Composes the static agent prelude, goal/constraints/criteria sections,
/// optional workspace context, persona, capability index, and
/// kernel manifest into a single prompt string.
#[allow(clippy::too_many_arguments)]
fn build_system_prompt_inner(
    goal: &str,
    original_request: &str,
    constraints: &[String],
    acceptance_criteria: &[String],
    workspace_context: Option<&str>,
    persona_prompt: Option<&str>,
    capabilities_xml: Option<&str>,
    kernel_manifest: Option<&str>,
) -> String {
    let mut prompt = String::from(
        "You are an autonomous agent in the Oxios operating system.\n\
         You execute Seeds — immutable specifications with goals, constraints, and\n\
         acceptance criteria.\n\n\
         ## Available Tools\n\
         You have the following tools:\n\
         - **File tools**: read, write, edit files; grep, find, ls for searching\n\
         - **Web tools**: web_search for searching the web, get_search_results for retrieving cached results\n\
         - **Exec**: run shell commands\n\
         - **Memory tools**: memory_write (store facts/preferences), memory_read (list entries), memory_search (find relevant memories) — your cross-session recall. Use memory_write proactively when the user shares preferences, facts, or corrections worth remembering.
         - **Knowledge**: knowledge — personal markdown vault for documents and notes\n\
         - **Kernel tools**: agent, project, persona, cron, security, budget, resource\n\n\
         **Important**: When the task involves fetching information from the internet,\n\
         websites, or online services, use `web_search` first — do NOT search local files.\n\
         When the task asks to \"get\", \"fetch\", \"find online\", or \"look up\" something\n\
         from the web, use `web_search`.\n",
    );
    prompt.push_str(&format!("\n## Goal\n{}\n", goal));

    // Preserve user's original wording so the agent sees exact language,
    // filenames, and nuances that may have been abstracted in the goal.
    if !original_request.is_empty() && original_request != goal {
        prompt.push_str(&format!(
            "\n## User's Original Request\n{}\n",
            original_request
        ));
    }

    if !constraints.is_empty() {
        prompt.push_str("\n## Constraints\n");
        for (i, c) in constraints.iter().enumerate() {
            prompt.push_str(&format!("{}. {}\n", i + 1, c));
        }
    }

    if !acceptance_criteria.is_empty() {
        prompt.push_str("\n## Acceptance Criteria\n");
        for (i, c) in acceptance_criteria.iter().enumerate() {
            prompt.push_str(&format!("{}. {}\n", i + 1, c));
        }
    }

    // ── Workspace Context (RFC-025) ──
    // Inject active Mounts + project instructions AFTER the goal/constraints
    // and BEFORE the persona, so the agent sees its workspace before it acts.
    if let Some(ctx) = workspace_context.filter(|s| !s.trim().is_empty()) {
        prompt.push_str("\n## Workspace Context\n");
        prompt.push_str(ctx);
        prompt.push('\n');
    }

    // Inject persona system prompt
    if let Some(pp) = persona_prompt {
        prompt.push_str("\n## Persona\n");
        prompt.push_str(pp);
        prompt.push('\n');
    }

    // Inject semantic capability index (from ToolRetriever)
    if let Some(xml) = capabilities_xml {
        prompt.push_str("\n## Available Capabilities\n");
        prompt.push_str("The following capabilities are relevant to your goal. ");
        prompt.push_str("Use the `read` tool to load SKILL.md for any program.\n\n");
        prompt.push_str(xml);
        prompt.push('\n');
    }

    // Inject kernel manifest (from CSpace)
    if let Some(manifest) = kernel_manifest {
        prompt.push('\n');
        prompt.push_str(manifest);
        prompt.push('\n');
    }

    // Execution environment guidance
    prompt.push_str(
        "\n## Execution Protocol\n\
         1. UNDERSTAND — Read the user's request carefully. If it is a simple\n\
            greeting, small talk, or a question you can answer from knowledge,\n\
            respond naturally and conversationally — no tools needed.\n\
         2. PLAN — For complex tasks, outline your approach before acting.\n\
         3. EXECUTE — Use tools only when the task actually requires them.\n\
            Prefer the simplest approach. Simple requests need no tools.\n\
         4. VERIFY — After each action, check the result: created a file? read it back.\n\
         5. REPORT — Summarize how each acceptance criterion was met, with evidence.\n\n\
         If the request is ambiguous, use the `ask_user` tool (free-text question)\n\
         or the `pi-questionnaire` tool (structured choices) to clarify before\n\
         executing — do not guess when a single question would resolve the intent.\n\n\
         ## Hard Boundaries\n\
         - NEVER modify files outside the workspace scope\n\
         - NEVER execute destructive commands without confirming scope\n\
         - NEVER claim completion without evidence — show the output, not your opinion\n\
         - NEVER add features or improvements beyond the goal's scope\n\
         - If you cannot complete the task, say so and explain WHY\n\n\
         ## Scope Guard\n\
         The goal defines your universe. Do not:\n\
         - Refactor code the goal didn't mention\n\
         - Add tests the goal didn't require\n\
         - Change configuration the goal didn't specify\n\
         - \"Improve\" anything beyond what the acceptance criteria demand\n\n\
         ## Error Handling\n\
         - If a tool fails, read the error message carefully before retrying\n\
         - If a command fails, do NOT immediately retry with --force or sudo\n\
         - If stuck after 3 attempts, report the blocker rather than continuing to fail\n\n\
         ## Shape Matching\n\
         Match your output to the task: simple task → concise response.\n\
         Do not write 50 lines when 5 would do.\n\
         Use `exec` for all command execution (git, gh, osascript, etc.).",
    );
    prompt.push_str(ARTIFACT_PROTOCOL);

    prompt
}
#[allow(dead_code)]
fn build_directive_user_prompt(directive: &Directive) -> String {
    build_user_prompt_inner(&directive.goal, &directive.acceptance_criteria)
}

/// Shared user-prompt builder for the directive path.
fn build_user_prompt_inner(goal: &str, acceptance_criteria: &[String]) -> String {
    format!(
        "Execute the following goal:\n\n{}\n\nAcceptance criteria:\n{}",
        goal,
        acceptance_criteria
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{}. {}", i + 1, c))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

impl std::fmt::Debug for AgentRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentRuntime")
            .field("model_id", &self.engine_handle.get().default_model_id())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use oxicode_sdk::{AgentTool, ToolContext, ToolError};
    use serde_json::Value;

    /// A test tool that does nothing — used to populate the registry.
    struct DummyTool {
        name: String,
    }

    #[async_trait]
    impl AgentTool for DummyTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn label(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "Test tool"
        }
        fn parameters_schema(&self) -> Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(
            &self,
            _tool_call_id: &str,
            _params: Value,
            _shutdown: Option<tokio::sync::oneshot::Receiver<()>>,
            _ctx: &ToolContext,
        ) -> Result<oxicode_sdk::AgentToolResult, ToolError> {
            Ok(oxicode_sdk::AgentToolResult::success("ok"))
        }
    }

    /// Test that requires_tools validation passes when all tools are present.
    #[test]
    fn test_requires_tools_validation_passes() {
        let registry = ToolRegistry::new();

        registry.register(DummyTool {
            name: "read".into(),
        });
        registry.register(DummyTool {
            name: "exec".into(),
        });

        let missing = registry.missing(&["read", "exec"]);

        assert!(
            missing.is_empty(),
            "Expected no missing tools, got: {:?}",
            missing
        );
    }

    /// Test that requires_tools validation fails when a tool is missing.
    #[test]
    fn test_requires_tools_validation_fails() {
        let registry = ToolRegistry::new();

        registry.register(DummyTool {
            name: "read".into(),
        });

        let missing = registry.missing(&["read", "exec", "nonexistent"]);

        assert_eq!(missing, vec!["exec", "nonexistent"]);
    }

    #[test]
    fn test_infer_domain_testing() {
        assert_eq!(infer_domain("run all unit tests for the kernel"), "testing");
    }

    #[test]
    fn test_infer_domain_deployment() {
        assert_eq!(
            infer_domain("deploy the web service to production"),
            "deployment"
        );
    }

    #[test]
    fn test_infer_domain_bugfix() {
        assert_eq!(infer_domain("fix the null pointer error in main"), "bugfix");
    }

    #[test]
    fn test_infer_domain_development() {
        assert_eq!(
            infer_domain("create a new REST API endpoint"),
            "development"
        );
    }

    #[test]
    fn test_infer_domain_analysis() {
        assert_eq!(
            infer_domain("review the code for security issues"),
            "analysis"
        );
    }

    #[test]
    fn test_infer_domain_fallback() {
        let domain = infer_domain("optimize performance metrics");
        // Should fall back to first 2 meaningful words
        assert!(!domain.is_empty());
    }
    #[test]
    fn test_system_prompt_includes_artifact_protocol() {
        let prompt = build_system_prompt_inner(
            "build a dashboard",
            "build a dashboard",
            &[],
            &[],
            None,
            None,
            None,
            None,
        );
        // The four pinned type values must appear verbatim (the frontend
        // matches these exact substrings — paraphrasing breaks the preview).
        assert!(prompt.contains("<lobeArtifact"));
        assert!(prompt.contains("text/html"));
        assert!(prompt.contains("image/svg+xml"));
        assert!(prompt.contains("application/lobe.artifacts.mermaid"));
        assert!(prompt.contains("application/lobe.artifacts.react"));
    }
}
