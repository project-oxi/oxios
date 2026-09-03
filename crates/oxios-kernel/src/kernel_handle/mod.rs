//! Kernel facade — domain Facades composing the System Call API.

pub mod a2a_api;
pub mod agent_api;
pub mod brain_api;
pub mod browser_api;
pub mod calendar_api;
pub mod compression_api;
pub mod email_api;
pub mod engine_api;
pub mod exec_api;
pub mod extension_api;
pub mod infra_api;
pub mod issue_api;
pub mod knowledge_lens;
pub mod marketplace_api;
pub mod mcp_api;
pub mod memo_api;
pub mod persona_api;
pub mod project_api;
#[cfg(feature = "browser")]
pub mod screenshot_api;
pub mod security_api;
pub mod state_api;
pub mod timeline_api;
pub mod token_maxing_api;

pub use crate::host_tools::HostToolsApi;
pub use a2a_api::A2aApi;
pub use agent_api::AgentApi;
pub use brain_api::BrainApi;
pub use browser_api::BrowserApi;
pub use calendar_api::CalendarApi;
pub use compression_api::CompressionApi;
pub use email_api::EmailApi;
pub use engine_api::{
    EngineApi, EngineConfigResponse, FallbackEvent, InputModality, ModelInfo, ProviderCategory,
    ProviderInfo, RoutingConfigSnapshot, RoutingStats, RoutingStatsSnapshot, RoutingUpdate,
    ValidateKeyResult,
};
pub use exec_api::ExecApi;
pub use exec_api::SharedExecConfig;
pub use extension_api::ExtensionApi;
pub use infra_api::InfraApi;
pub use issue_api::{AssignmentInfo, IssueApi, IssueQuery, IssueView, MilestoneView, NewIssue};
pub use knowledge_lens::{
    CopilotResponse, KnowledgeContext, KnowledgeLens, KnowledgeNote, MemoryNote,
};
pub use marketplace_api::MarketplaceApi;
pub use mcp_api::McpApi;
pub use memo_api::MemoApi;
pub use persona_api::PersonaApi;
pub use project_api::{ProjectApi, ProjectInfo};
#[cfg(feature = "browser")]
pub use screenshot_api::{ScreenshotEngine, ScreenshotViewport};
pub use timeline_api::TimelineApi;

pub use security_api::SecurityApi;
pub use state_api::StateApi;
pub use token_maxing_api::TokenMaxingApi;

use crate::git_layer::CommitInfo;
use crate::orchestrator::{OrchestrationResult, Orchestrator};
use crate::readiness::ReadinessGate;
use parking_lot::RwLock;
use serde::Serialize;
use std::sync::Arc;

/// Oxios kernel System Call API — composed of domain Facades.
///
/// Each Facade groups related system calls:
/// - [`StateApi`]     — data persistence, sessions
/// - [`AgentApi`]     — agent lifecycle, budgets, memory
/// - [`SecurityApi`]  — auth, audit trail, RBAC, approvals
/// - [`PersonaApi`]   — multi-persona management
/// - [`ExtensionApi`] — programs, skills, host tools
/// - [`McpApi`]       — MCP server bridge
/// - [`ProjectApi`]  — Project management, memory linking
/// - [`ExecApi`]      — execution config, access management
/// - [`A2aApi`]       — agent-to-agent communication
/// - [`EngineApi`]    — LLM engine providers, models, config
/// - [`KnowledgeBase`] — markdown note management (kernel-free, via oxios-markdown)
pub struct KernelHandle {
    /// State management: save/load/sessions.
    pub state: StateApi,
    /// Agent management: lifecycle/budgets/memory.
    pub agents: AgentApi,
    /// Security: auth/audit/RBAC/approvals.
    pub security: SecurityApi,
    /// Persona management.
    pub persona: PersonaApi,
    /// Extensions: programs/skills/host tools.
    pub extensions: ExtensionApi,
    /// MCP server bridge.
    pub mcp: McpApi,
    /// Infrastructure: Git/scheduler/cron/resources/events/system.
    pub infra: InfraApi,
    /// Project management: work context (RFC-011).
    pub projects: Option<ProjectApi>,
    /// Brain daemon (RFC-047): oxibrain connection facade. `None` only on the
    /// incomplete preliminary handle; the cached handle attaches it via
    /// [`KernelHandle::with_brain`](Self::with_brain).
    pub brain: Option<BrainApi>,
    /// Execution: config + access management.
    pub exec: ExecApi,
    /// Headless browser (RFC: browser-migration). `None` unless `[browser].enabled`.
    pub browser: Option<BrowserApi>,
    /// Agent-to-agent communication.
    pub a2a: A2aApi,
    /// Engine: LLM providers, models, config.
    pub engine: Arc<EngineApi>,
    /// Knowledge base: markdown notes (direct access, no kernel dependency).
    pub knowledge: Arc<oxios_markdown::KnowledgeBase>,
    /// Semantic knowledge overlay (HNSW index + agent recall).
    pub knowledge_lens: Arc<KnowledgeLens>,
    /// Marketplace API — ClawHub search, install, update.
    pub marketplace_api: MarketplaceApi,
    /// Calendar events — create, update, delete, list, search, freebusy.
    pub calendar: Option<CalendarApi>,
    /// oximemo integration (opt-in first-party app module; `memo` feature).
    /// Live runtime slot (mirrors `email`): `None` when disabled or the vault
    /// hasn't opened; swapped in by `POST /api/memo/enable` with no restart.
    /// oxios is a co-client of the vault — never its owner. Shared via `Arc`
    /// so the agent tool can delegate to the facade (which publishes events).
    pub memo: Arc<RwLock<Option<Arc<MemoApi>>>>,
    /// oxiline integration (opt-in first-party app module; `timeline` feature).
    /// Live runtime slot (mirrors `memo`/`email`): read-only context-in for
    /// agents (current activity, today's plan compliance, recent records). oxios
    /// is a co-client of the store — never its owner.
    pub timeline: Arc<RwLock<Option<Arc<TimelineApi>>>>,
    /// Email — send HTML emails via SMTP, template management.
    pub email: Arc<RwLock<Option<EmailApi>>>,
    /// Token-maxing (RFC-031): the shared QuotaTracker facade. `None` only on
    /// the incomplete preliminary handle; the cached handle attaches it.
    pub token_maxing: Option<TokenMaxingApi>,
    /// Context compression: LLM session summaries (optional).
    pub compression: Option<CompressionApi>,
    /// Host Integrations (RFC-041): host-CLI discovery, OAuth, provisioning.
    pub host_tools: HostToolsApi,
    /// RFC-024 SP4: subsystem readiness gate.
    pub readiness: Arc<ReadinessGate>,
    /// Per-session streaming sink registry (P1 chat transparency).
    ///
    /// The agent runtime callback looks up the sink by `session_id` (which
    /// it already has via `transparency_session`) and pushes live text
    /// deltas. The gateway registers a strong sender before invoking
    /// the orchestrator and drops it after the collector completes; the
    /// `Weak` entries auto-clean.
    pub streaming_sinks: Arc<crate::streaming_sink::StreamingSinkRegistry>,
    /// RFC-049: in-flight turns, addressable for cancellation. Shares its key
    /// space with `streaming_sinks`.
    pub turns: Arc<crate::turn_registry::TurnRegistry>,
    /// The Ouroboros orchestrator — the "brain". Attached by the kernel
    /// assembler so background loops (cron auto-start, task auto-run) and
    /// the HTTP task-run handler share ONE execution primitive (`run_goal`)
    /// instead of each rebuilding the orchestration call. `None` on the
    /// preliminary handle before full assembly.
    pub orchestrator: Option<Arc<Orchestrator>>,
    /// Unified asset store — central binary storage with metadata index.
    /// Attached by the kernel assembler. `None` on the preliminary handle.
    pub asset_store: Option<Arc<crate::asset_store::AssetStore>>,
    /// Automation store — SQLite-backed automation lifecycle. Attached by
    /// the kernel assembler so the web routes, the auto-run tick, and the
    /// `task` agent tool share ONE store. `None` on the preliminary handle.
    pub automation_store: Option<Arc<tokio::sync::Mutex<crate::automation::AutomationStore>>>,
    /// Structured tool-result side channel: tools insert payloads keyed by
    /// `tool_call_id`; the agent-runtime completion callback takes them when
    /// publishing `KernelEvent::ToolExecutionFinished`. Consumers today are
    /// `web_search`/`get_search_results`, `todo`, and `issue`.
    pub tool_results: Arc<crate::tools::StructuredResultBus>,
    /// Per-project issue tracking + milestones. Shared across handles by the
    /// assembler so the store cache and the ownership `flock` guards are
    /// common — two instances would not see each other's claims.
    pub issues: Arc<issue_api::IssueApi>,
    /// Session-scoped todo plans. Shared across handles by the assembler so
    /// a plan created on one turn is found again on the next.
    pub todos: Arc<crate::tools::TodoRegistry>,
    /// Session-scoped coding-recipe runtime extensions (hashline snapshot
    /// store, persistent shell, eval kernels). Shared across handles by the
    /// assembler so one chat's shell/anchor state survives across turns
    /// regardless of which handle serves them.
    pub runtime_extensions: Arc<crate::execution_recipe::extensions::RuntimeExtensionManager>,
}

impl KernelHandle {
    /// Create a new KernelHandle from 14 domain Facades.
    ///
    /// Each Facade is assembled independently in `kernel.rs` and passed here.
    /// This enables testing individual Facades without the full kernel.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: StateApi,
        agents: AgentApi,
        security: SecurityApi,
        persona: PersonaApi,
        extensions: ExtensionApi,
        mcp: McpApi,
        infra: InfraApi,
        projects: Option<ProjectApi>,
        exec: ExecApi,
        a2a: A2aApi,
        engine: Arc<EngineApi>,
        knowledge: Arc<oxios_markdown::KnowledgeBase>,
        knowledge_lens: Arc<KnowledgeLens>,
        marketplace_api: MarketplaceApi,
        calendar: Option<CalendarApi>,
        email: Arc<RwLock<Option<EmailApi>>>,
    ) -> Self {
        Self {
            state,
            agents,
            security,
            persona,
            extensions,
            mcp,
            infra,
            projects,
            brain: None,
            exec,
            browser: None,
            a2a,
            engine,
            knowledge,
            knowledge_lens,
            marketplace_api,
            calendar,
            email,
            memo: Arc::new(RwLock::new(None)),
            timeline: Arc::new(RwLock::new(None)),
            token_maxing: None,
            compression: None,
            host_tools: HostToolsApi::new(),
            streaming_sinks: Arc::new(crate::streaming_sink::StreamingSinkRegistry::new()),
            // Structured tool-result side channel (tools → runtime callback).
            // Per-handle by default; the assembler may inject a shared
            // instance across handles via `with_tool_results`.
            tool_results: Arc::new(crate::tools::StructuredResultBus::new()),
            // Session-scoped coding-recipe runtime extensions (hashline
            // snapshot store, persistent shell, eval kernels). Per-handle by
            // default; the assembler may inject a shared instance across
            // handles via `with_runtime_extensions`.
            runtime_extensions: Arc::new(
                crate::execution_recipe::extensions::RuntimeExtensionManager::default(),
            ),
            // Per-handle by default; the assembler injects a shared instance
            // via `with_issues` so ownership guards are observed everywhere.
            issues: Arc::new(issue_api::IssueApi::new()),
            todos: Arc::new(crate::tools::TodoRegistry::new()),
            turns: Arc::new(crate::turn_registry::TurnRegistry::new()),
            // RFC-024 SP4: default Warming/no-deadline. The Kernel
            readiness: Arc::new(ReadinessGate::new(0)),
            orchestrator: None,
            asset_store: None,
            automation_store: None,
        }
    }

    /// Attach the brain facade (RFC-047). Called by the kernel
    /// assembler at boot after the `BrainSession` is built.
    pub fn with_brain(mut self, brain: BrainApi) -> Self {
        self.brain = Some(brain);
        self
    }

    /// Attach the oximemo facade (first-party app module). Called by the kernel
    /// assembler at boot when `[memo].enabled`, and by `POST /api/memo/enable`
    /// for a live swap (no restart). Replaces any prior facade in the slot.
    pub fn with_memo(self, memo: Arc<MemoApi>) -> Self {
        *self.memo.write() = Some(memo);
        self
    }
    /// Attach the oxiline facade (first-party app module). Called by the kernel
    /// assembler at boot when `[timeline].enabled`, and by
    /// `POST /api/timeline/enable` for a live swap (no restart).
    pub fn with_timeline(self, timeline: Arc<TimelineApi>) -> Self {
        *self.timeline.write() = Some(timeline);
        self
    }
    /// Attach the unified AssetStore. Called by the kernel assembler.
    pub fn with_asset_store(mut self, store: Arc<crate::asset_store::AssetStore>) -> Self {
        self.asset_store = Some(store);
        self
    }

    /// Set the AssetStore in place (post-construction wiring).
    pub fn set_asset_store(&mut self, store: Arc<crate::asset_store::AssetStore>) {
        self.asset_store = Some(store);
    }

    /// Attach the TokenMaxing facade (RFC-031). Called by the kernel
    /// assembler after constructing the shared `QuotaTracker`.
    pub fn with_token_maxing(mut self, api: TokenMaxingApi) -> Self {
        self.token_maxing = Some(api);
        self
    }

    /// Set the TokenMaxing facade in place (post-construction wiring).
    pub fn set_token_maxing(&mut self, api: TokenMaxingApi) {
        self.token_maxing = Some(api);
    }

    /// Attach the CompressionApi facade. Called by the kernel assembler.
    pub fn with_compression(mut self, api: CompressionApi) -> Self {
        self.compression = Some(api);
        self
    }

    /// Attach the browser facade (RFC: browser-migration).
    ///
    /// Called by the kernel assembler when `[browser].enabled` is set.
    /// Without the `browser` feature the facade exists but never
    pub fn with_browser(mut self, api: Option<BrowserApi>) -> Self {
        self.browser = api;
        self
    }

    /// Attach the shared streaming-sink registry. Called by the kernel
    /// assembler to make the runtime callback's per-session `TextChunk`
    /// lookup find the gateway's collector sender. The same `Arc` must be
    /// passed to the gateway via `Gateway::with_streaming_sinks`.
    pub fn with_streaming_sinks(
        mut self,
        registry: Arc<crate::streaming_sink::StreamingSinkRegistry>,
    ) -> Self {
        self.streaming_sinks = registry;
        self
    }

    /// Attach the shared turn registry (the gateway holds the same `Arc`).
    pub fn with_turns(mut self, registry: Arc<crate::turn_registry::TurnRegistry>) -> Self {
        self.turns = registry;
        self
    }

    /// Attach the orchestrator (the "brain"). Called by the kernel assembler
    /// so background loops and the HTTP task-run handler share one execution
    /// primitive via [`Self::run_goal`].
    pub fn with_orchestrator(mut self, orchestrator: Arc<Orchestrator>) -> Self {
        self.orchestrator = Some(orchestrator);
        self
    }

    /// Share one [`crate::tools::StructuredResultBus`] across handles.
    ///
    /// Called by the kernel assembler so the preliminary handle (AgentRuntime)
    /// and the cached control-plane handle observe the same structured
    /// tool payloads.
    pub fn with_tool_results(mut self, bus: Arc<crate::tools::StructuredResultBus>) -> Self {
        self.tool_results = bus;
        self
    }

    /// Share one [`issue_api::IssueApi`] across handles.
    ///
    /// Required for correctness, not just efficiency: the facade holds the
    /// `flock` guards backing issue assignment, so separate instances would
    /// each believe the other's claims belong to a dead session.
    pub fn with_issues(mut self, issues: Arc<issue_api::IssueApi>) -> Self {
        self.issues = issues;
        self
    }

    /// Share one [`crate::tools::TodoRegistry`] across handles, so a plan
    /// created on one turn is found again on the next.
    pub fn with_todos(mut self, todos: Arc<crate::tools::TodoRegistry>) -> Self {
        self.todos = todos;
        self
    }

    /// Share one [`RuntimeExtensionManager`] across handles, so a coding
    /// chat's shell/anchor state survives across turns regardless of which
    /// handle serves them.
    pub fn with_runtime_extensions(
        mut self,
        extensions: Arc<crate::execution_recipe::extensions::RuntimeExtensionManager>,
    ) -> Self {
        self.runtime_extensions = extensions;
        self
    }

    /// Attach the automation store.
    pub fn with_automation_store(
        mut self,
        store: Arc<tokio::sync::Mutex<crate::automation::AutomationStore>>,
    ) -> Self {
        self.automation_store = Some(store);
        self
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Convenience methods (cross-Facades orchestration)
    // ═══════════════════════════════════════════════════════════════════════
    /// Execute a goal through the Ouroboros pipeline.
    ///
    /// The single shared execution primitive for autonomous/scheduled runs.
    /// Used by:
    /// - the CronScheduler auto-start loop (background cron job execution),
    /// - the automation auto-run tick loop (scheduled/heartbeat automations),
    /// - `POST /api/automations/:id/run` (manual automation execution).
    ///
    /// This calls `orchestrator.handle_unified` directly — the same in-process
    /// path the CLI uses (`execute_prompt_with_session`). It does NOT route
    /// through the gateway, so there is no HTTP response correlation: callers
    /// simply `await` the [`OrchestrationResult`].
    ///
    /// Returns an error if the orchestrator was not attached (preliminary
    /// handle) — the cached handle from `Kernel::handle()` always has it.
    pub async fn run_goal(
        &self,
        goal: &str,
        session_id: Option<&str>,
    ) -> anyhow::Result<OrchestrationResult> {
        let orch = self
            .orchestrator
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("orchestrator not wired on this handle"))?;
        let request_id = format!("run-goal-{}", uuid::Uuid::new_v4());
        orch.handle_unified(
            "system",
            goal,
            session_id,
            None,
            None,
            None,
            None,
            None,
            None, // brain_space
            &request_id,
            None,
        )
        .await
    }

    /// Save data and commit to git (State + Infra).
    ///
    /// The state save is the source of truth and is fully propagated. The git
    /// commit is best-effort observability: if it fails (full disk, lock
    /// contention, missing committer identity) we log a warning rather than
    /// failing the save — the data is already persisted on disk and failing
    /// here would mislead callers into thinking the save itself failed.
    pub async fn save_and_commit<T: Serialize>(
        &self,
        category: &str,
        name: &str,
        data: &T,
    ) -> anyhow::Result<()> {
        self.state.save(category, name, data).await?;
        let git = self.infra.git();
        if git.is_enabled() {
            let rel_path = format!("{category}/{name}.json");
            if let Err(e) = git.commit_file(&rel_path, &format!("save {category}/{name}")) {
                tracing::warn!(
                    error = %e, rel_path = %rel_path,
                    "save_and_commit: git commit failed (data was still saved)"
                );
            }
        }
        Ok(())
    }

    /// Save markdown and commit to git (State + Infra).
    ///
    /// See [`Self::save_and_commit`] for the git-failure policy.
    pub async fn save_markdown_and_commit(
        &self,
        category: &str,
        name: &str,
        content: &str,
    ) -> anyhow::Result<()> {
        self.state.save_markdown(category, name, content).await?;
        let git = self.infra.git();
        if git.is_enabled() {
            let rel_path = format!("{category}/{name}.md");
            if let Err(e) = git.commit_file(&rel_path, &format!("save {category}/{name}")) {
                tracing::warn!(
                    error = %e, rel_path = %rel_path,
                    "save_markdown_and_commit: git commit failed (data was still saved)"
                );
            }
        }
        Ok(())
    }

    /// Delete a file and commit the removal to git (State + Infra).
    ///
    /// See [`Self::save_and_commit`] for the git-failure policy.
    pub async fn delete_and_commit(&self, category: &str, name: &str) -> anyhow::Result<bool> {
        let deleted = self.state.delete(category, name).await?;
        if deleted {
            let git = self.infra.git();
            if git.is_enabled() {
                let rel_path = format!("{category}/{name}.json");
                if let Err(e) = git.remove_file(&rel_path, &format!("delete {category}/{name}")) {
                    tracing::warn!(
                        error = %e, rel_path = %rel_path,
                        "delete_and_commit: git remove failed (file was still deleted)"
                    );
                }
            }
        }
        Ok(deleted)
    }

    /// Commit all current changes to git.
    pub fn commit_all(&self, message: &str) -> anyhow::Result<Option<CommitInfo>> {
        self.state.commit_all(self.infra.git(), message)
    }

    /// Flush audit trail and commit to git (Security + Infra).
    pub fn flush_audit(&self) -> anyhow::Result<()> {
        self.security.flush(self.infra.git())
    }

    /// Load JSON from state store.
    pub async fn load_json<T: serde::de::DeserializeOwned>(
        &self,
        category: &str,
        name: &str,
    ) -> anyhow::Result<Option<T>> {
        self.state.load(category, name).await
    }

    /// Get kernel start time.
    pub fn start_time(&self) -> std::time::Instant {
        self.infra.start_time
    }

    /// Marketplace API — ClawHub search, install, update.
    pub fn marketplace_api(&self) -> &MarketplaceApi {
        &self.marketplace_api
    }
}
