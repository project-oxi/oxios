//! Supervisor: agent lifecycle management.
//!
//! The supervisor handles forking, executing, monitoring, and
//! terminating agent instances. It is the "init" of Oxios.
//!
//! When an agent is forked and executed, the supervisor delegates
//! the actual tool-calling loop to the [`AgentRuntime`].
//!
//! # Agent Pool & Session Persistence
//!
//! Agents are retained in an [`AgentPool`] after execution for:
//! - **Session continuation** via `Agent::continue_with()` — multi-turn
//!   conversations without re-creating the agent.
//! - **State export/import** — serialize agent conversation history to
//!   JSON for crash recovery, migration, or debugging.
//! - **Provider rate limiting** — all agents share a [`ProviderPool`] to
//!   respect per-provider RPM/concurrency limits.

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use oxicode_sdk::Agent;
use oxios_ouroboros::{Directive, ExecEnv};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::task::JoinHandle;

use crate::agent_runtime::AgentRuntime;
use crate::config::AgentLogConfig;
use crate::event_bus::EventBus;
use crate::resilience::classify;
use crate::resource_monitor::ResourceMonitor;
use crate::session_context::SessionContext;
use crate::state_store::StateStore;
use crate::types::{AgentId, AgentInfo, AgentStatus};

use oxios_ouroboros::ExecutionResult;

use crate::agent_log_db::AgentLogDb;

/// Tracks the runtime handles needed to cancel a running agent.
struct AgentHandle {
    /// Flag set on `kill()` to cooperatively signal cancellation.
    cancelled: Arc<AtomicBool>,
    /// The tokio task running the agent execution. Aborted on `kill()`.
    task: JoinHandle<()>,
}

/// Pool of live `Agent` instances, keyed by AgentId.
///
/// Retains agents after execution for:
/// - **State persistence** — `export_state()` serializes conversation history
///   to JSON for crash recovery, migration, or debugging.
/// - **State restoration** — `import_state()` restores a previous session.
#[derive(Default)]
pub struct AgentPool {
    agents: RwLock<HashMap<AgentId, Arc<Agent>>>,
}

impl AgentPool {
    /// Create an empty agent pool.
    pub fn new() -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
        }
    }

    /// Insert an agent into the pool.
    pub fn insert(&self, id: AgentId, agent: Arc<Agent>) {
        self.agents.write().insert(id, agent);
    }

    /// Get a pooled agent by ID.
    pub fn get(&self, id: &AgentId) -> Option<Arc<Agent>> {
        self.agents.read().get(id).cloned()
    }

    /// Remove an agent from the pool.
    pub fn remove(&self, id: &AgentId) -> Option<Arc<Agent>> {
        self.agents.write().remove(id)
    }

    /// Export an agent's state as JSON.
    ///
    /// Returns `None` if the agent is not in the pool or export fails.
    pub fn export_state(&self, id: &AgentId) -> Option<serde_json::Value> {
        self.agents
            .read()
            .get(id)
            .and_then(|agent| agent.export_state().ok())
    }

    /// Import agent state from JSON.
    ///
    /// Returns `false` if the agent is not in the pool or import fails.
    pub fn import_state(&self, id: &AgentId, state: serde_json::Value) -> bool {
        if let Some(agent) = self.agents.read().get(id) {
            agent.import_state(state).is_ok()
        } else {
            false
        }
    }

    /// Number of agents currently in the pool.
    pub fn len(&self) -> usize {
        self.agents.read().len()
    }

    /// Whether the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.agents.read().is_empty()
    }
}

/// Supervisor trait for managing agent lifecycles.
#[async_trait]
pub trait Supervisor: Send + Sync {
    /// Start executing an agent.
    async fn exec(&self, id: AgentId) -> Result<()>;

    /// Fork a new agent from a Directive and ExecEnv (RFC-027).
    ///
    /// Reads the goal / project_id from the unified-intent Directive.
    async fn fork_directive(&self, directive: &Directive, env: &ExecEnv) -> Result<AgentId>;

    /// Fork and execute an agent with a Directive + ExecEnv, running to completion.
    ///
    /// Dispatches to `AgentRuntime::execute_directive_with_session`.
    async fn run_with_directive(
        &self,
        id: AgentId,
        directive: &Directive,
        env: &ExecEnv,
    ) -> Result<ExecutionResult>;

    /// Wait for an agent to complete and return its final status.
    async fn wait(&self, id: AgentId) -> Result<AgentStatus>;

    /// Terminate an agent.
    async fn kill(&self, id: AgentId) -> Result<()>;

    /// List all known agents.
    async fn list(&self) -> Result<Vec<AgentInfo>>;
}

/// Basic in-memory supervisor implementation with AgentRuntime integration.
pub struct BasicSupervisor {
    agents: RwLock<HashMap<AgentId, AgentInfo>>,
    /// Per-agent cancellation tokens and join handles for task abortion.
    handles: RwLock<HashMap<AgentId, AgentHandle>>,
    /// Pool of live Agent instances for session continuation.
    agent_pool: AgentPool,
    event_bus: EventBus,
    runtime: Arc<AgentRuntime>,
    resource_monitor: Option<Arc<ResourceMonitor>>,
    /// Session context for proactive recall timing (RFC-020).
    /// Shared across all agent executions within this supervisor's lifetime
    /// so that RecallTiming can track message count and topic changes.
    /// Uses tokio::sync::RwLock (not parking_lot) so the guard is Send,
    /// allowing it to be held across .await in tokio::spawn.
    session_context: Arc<tokio::sync::RwLock<SessionContext>>,
    /// Filesystem state store for agent persistence (JSON files).
    state_store: Option<Arc<StateStore>>,
    /// SQLite-backed agent history query index.
    agent_log_db: Option<Arc<AgentLogDb>>,
    /// Agent log retention configuration.
    agent_log_config: AgentLogConfig,
}

impl BasicSupervisor {
    /// Creates a new supervisor with the given event bus and agent runtime.
    pub fn new(event_bus: EventBus, runtime: AgentRuntime) -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
            handles: RwLock::new(HashMap::new()),
            agent_pool: AgentPool::new(),
            event_bus,
            runtime: Arc::new(runtime),
            resource_monitor: None,
            session_context: Arc::new(tokio::sync::RwLock::new(SessionContext::new())),
            state_store: None,
            agent_log_db: None,
            agent_log_config: AgentLogConfig::default(),
        }
    }

    /// Attach a filesystem state store for agent history persistence.
    pub fn set_state_store(&mut self, store: Arc<StateStore>) {
        self.state_store = Some(store);
    }

    /// Attach a SQLite-backed agent history log database.
    pub fn set_agent_log_db(&mut self, db: Arc<AgentLogDb>) {
        self.agent_log_db = Some(db);
    }

    /// Set agent log retention configuration.
    pub fn set_agent_log_config(&mut self, config: AgentLogConfig) {
        self.agent_log_config = config;
    }

    /// Attach a resource monitor for agent count tracking.
    pub fn set_resource_monitor(&mut self, rm: Arc<ResourceMonitor>) {
        self.resource_monitor = Some(rm);
    }

    /// Update the resource monitor with current active agent count.
    fn update_agent_count(&self) {
        if let Some(ref rm) = self.resource_monitor {
            let count = self.agents.read().len();
            rm.set_active_agents(count);
        }
    }

    /// Access the agent pool for session continuation.
    pub fn pool(&self) -> &AgentPool {
        &self.agent_pool
    }
}

#[async_trait]
impl Supervisor for BasicSupervisor {
    async fn fork_directive(&self, directive: &Directive, env: &ExecEnv) -> Result<AgentId> {
        let id = AgentId::new_v4();
        let info = AgentInfo {
            id,
            name: directive.goal.clone(),
            status: AgentStatus::Starting,
            created_at: Utc::now(),
            project_id: env.project_id,
            started_at: None,
            completed_at: None,
            error: None,
            steps_completed: 0,
            steps_total: None,
            tool_calls: vec![],
            tokens_input: 0,
            tokens_output: 0,
            cost_usd: 0.0,
            model_id: String::new(),
            session_id: None,
        };

        {
            let mut agents = self.agents.write();
            agents.insert(id, info);
        }

        self.update_agent_count();

        let _ = self
            .event_bus
            .publish(crate::event_bus::KernelEvent::AgentCreated {
                id,
                name: directive.goal.clone(),
                session_id: env.session_id.clone(),
            });

        tracing::info!(agent_id = %id, "Forked new agent from directive");
        Ok(id)
    }

    async fn exec(&self, id: AgentId) -> Result<()> {
        {
            let mut agents = self.agents.write();
            match agents.get_mut(&id) {
                Some(agent) => {
                    agent.status = AgentStatus::Running;
                }
                None => anyhow::bail!("Agent {id} not found"),
            }
        }

        self.update_agent_count();

        let _ = self
            .event_bus
            .publish(crate::event_bus::KernelEvent::AgentStarted {
                id,
                session_id: None,
            });
        tracing::info!(agent_id = %id, "Agent execution started");

        Ok(())
    }

    async fn run_with_directive(
        &self,
        id: AgentId,
        directive: &Directive,
        env: &ExecEnv,
    ) -> Result<ExecutionResult> {
        // Mark as running.
        {
            let mut agents = self.agents.write();
            match agents.get_mut(&id) {
                Some(agent) => {
                    agent.status = AgentStatus::Running;
                    agent.started_at = Some(Utc::now());
                }
                None => anyhow::bail!("Agent {id} not found"),
            }
        }

        let _ = self
            .event_bus
            .publish(crate::event_bus::KernelEvent::AgentStarted {
                id,
                session_id: env.session_id.clone(),
            });

        tracing::info!(agent_id = %id, "Running agent task from directive");

        // Spawn the execution as a tokio task so we can track and abort it.
        let cancelled = Arc::new(AtomicBool::new(false));
        let runtime = Arc::clone(&self.runtime);
        let directive = directive.clone();
        let session_id = env.session_id.clone();
        let env = env.clone();

        // Share the session context so RecallTiming persists across directives.
        // Uses tokio::sync::RwLock so the guard is Send-safe across .await.
        let session_ctx = self.session_context.clone();

        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<Result<ExecutionResult>>();
        let cancelled_done = cancelled.clone();
        let handle: JoinHandle<()> = tokio::spawn(async move {
            // Check for cancellation before starting.
            let result = if cancelled_done.load(Ordering::Relaxed) {
                Ok(ExecutionResult {
                    output: "Agent cancelled before execution".into(),
                    steps_completed: 0,
                    success: false,
                    tool_calls: vec![],
                    tokens_input: 0,
                    tokens_output: 0,
                    model_id: String::new(),
                    failure_class: None, // cancellation, not a provider failure
                    restore_state: None,
                    reasoning_text: String::new(),
                    reasoning_segments: Vec::new(),
                })
            } else {
                // Execute with a lightweight SessionContext copy: project IDs
                // are read-only during execution (recall timing moved to the
                // oxibrain daemon — RFC-047), so no write-back is needed.
                let mut temp_ctx = {
                    let ctx = session_ctx.read().await;
                    let mut c = crate::session_context::SessionContext::new();
                    c.primary_project_id = ctx.primary_project_id;
                    c.secondary_project_ids = ctx.secondary_project_ids.clone();
                    c
                };
                runtime
                    .execute_directive(id, &directive, &env, &mut temp_ctx)
                    .await
            };
            // Receiver gone (run_with_directive returned early) → ignore error.
            let _ = done_tx.send(result);
        });

        // Store the handle so kill() can abort the task.
        {
            let mut handles = self.handles.write();
            handles.insert(
                id,
                AgentHandle {
                    cancelled,
                    task: handle,
                },
            );
        }

        // Await completion via the oneshot channel. If kill() aborts the task
        // (or it panics), done_tx is dropped and this returns Err — treat as
        // cancellation.
        let result = match done_rx.await {
            Ok(res) => res,
            Err(_) => {
                let mut handles = self.handles.write();
                handles.remove(&id);
                Ok(ExecutionResult {
                    output: "Agent task aborted".into(),
                    steps_completed: 0,
                    success: false,
                    tool_calls: vec![],
                    tokens_input: 0,
                    tokens_output: 0,
                    model_id: String::new(),
                    failure_class: None, // abort (kill/panic), not a provider failure
                    restore_state: None,
                    reasoning_text: String::new(),
                    reasoning_segments: Vec::new(),
                })
            }
        };

        // Natural completion — remove the handle.
        {
            let mut handles = self.handles.write();
            handles.remove(&id);
        }

        match result {
            Ok(result) => {
                tracing::info!(
                    agent_id = %id,
                    success = result.success,
                    steps = result.steps_completed,
                    "Agent task completed (directive)"
                );

                {
                    let mut agents = self.agents.write();
                    if let Some(agent) = agents.get_mut(&id) {
                        agent.status = if result.success {
                            AgentStatus::Completed
                        } else {
                            AgentStatus::Failed
                        };
                        agent.completed_at = Some(Utc::now());
                        agent.steps_completed = result.steps_completed;
                        agent.tool_calls = result
                            .tool_calls
                            .iter()
                            .map(|tc| crate::types::ToolCallRecord {
                                tool: tc.tool.clone(),
                                input: tc.input.clone(),
                                output: tc.output.clone(),
                                duration_ms: tc.duration_ms,
                                is_error: tc.is_error,
                                tool_call_id: tc.tool_call_id.clone(),
                                timestamp: tc.timestamp,
                            })
                            .collect();
                        agent.tokens_input = result.tokens_input;
                        agent.tokens_output = result.tokens_output;
                        agent.model_id = result.model_id.clone();
                        agent.cost_usd = if !result.model_id.is_empty() {
                            crate::kernel_handle::engine_api::estimate_cost(
                                &result.model_id,
                                result.tokens_input,
                                result.tokens_output,
                            )
                        } else {
                            0.0
                        };
                        if !result.success {
                            agent.error = Some(result.output.clone());
                        }
                    }
                }

                let _ = self
                    .event_bus
                    .publish(crate::event_bus::KernelEvent::AgentStopped {
                        id,
                        success: result.success,
                        session_id: session_id.clone(),
                    });
                self.update_agent_count();

                // Persist to agent history log (async, non-blocking)
                self.persist_agent(id).await;

                Ok(result)
            }
            Err(e) => {
                tracing::error!(agent_id = %id, error = %e, "Agent task failed (directive)");

                {
                    let mut agents = self.agents.write();
                    if let Some(agent) = agents.get_mut(&id) {
                        agent.status = AgentStatus::Failed;
                        agent.completed_at = Some(Utc::now());
                        agent.error = Some(e.to_string());
                    }
                }

                let _ = self
                    .event_bus
                    .publish(crate::event_bus::KernelEvent::AgentFailed {
                        id,
                        error: e.to_string(),
                        session_id: session_id.clone(),
                    });
                self.update_agent_count();

                // Persist to agent history log (async, non-blocking)
                self.persist_agent(id).await;

                Ok(ExecutionResult {
                    output: format!("Agent failed: {e}"),
                    steps_completed: 0,
                    success: false,
                    tool_calls: vec![],
                    tokens_input: 0,
                    tokens_output: 0,
                    model_id: String::new(),
                    reasoning_text: String::new(),
                    reasoning_segments: Vec::new(),
                    // (P2 RecoveryCoordinator, gateway user-facing
                    // messages) can see whether this is a transient
                    // retry, a quota/auth that needs provider swap,
                    // context overflow, etc. Conservative: Unknown
                    // when no pattern matches.
                    failure_class: Some(classify(&e)),
                    restore_state: e
                        .downcast_ref::<crate::resilience::AgentRunError>()
                        .and_then(|err| err.restore_state.clone()),
                })
            }
        }
    }

    async fn wait(&self, id: AgentId) -> Result<AgentStatus> {
        let agents = self.agents.read();
        match agents.get(&id) {
            Some(info) => Ok(info.status),
            None => anyhow::bail!("Agent {id} not found"),
        }
    }

    async fn kill(&self, id: AgentId) -> Result<()> {
        // Cancel and abort the running task, if any.
        {
            let mut handles = self.handles.write();
            if let Some(agent_handle) = handles.remove(&id) {
                agent_handle.cancelled.store(true, Ordering::Relaxed);
                agent_handle.task.abort();
                tracing::info!(agent_id = %id, "Agent task aborted");
            }
        }

        {
            let mut agents = self.agents.write();
            if let Some(agent) = agents.get_mut(&id) {
                agent.status = AgentStatus::Stopped;
                agent.completed_at = Some(Utc::now());
            } else {
                anyhow::bail!("Agent {id} not found");
            }
        }

        let _ = self
            .event_bus
            .publish(crate::event_bus::KernelEvent::AgentStopped {
                id,
                success: false,
                session_id: None,
            });
        self.update_agent_count();

        // Persist to agent history log (async, non-blocking)
        self.persist_agent(id).await;

        tracing::info!(agent_id = %id, "Agent killed");
        Ok(())
    }

    async fn list(&self) -> Result<Vec<AgentInfo>> {
        let agents = self.agents.read();
        Ok(agents.values().cloned().collect())
    }
}

impl BasicSupervisor {
    /// Persist a terminated agent to both filesystem JSON and SQLite.
    /// Non-blocking: spawns a tokio task for the actual persistence.
    async fn persist_agent(&self, id: AgentId) {
        // Snapshot the agent info from the in-memory map
        let info = {
            let agents = self.agents.read();
            agents.get(&id).cloned()
        };

        let Some(info) = info else { return };

        // 1. Filesystem JSON (source of truth)
        if let Some(ref store) = self.state_store {
            let store = store.clone();
            let info = info.clone();
            let max_entries = self.agent_log_config.max_entries;
            let ttl_hours = self.agent_log_config.ttl_hours;
            let batch_size = self.agent_log_config.prune_batch_size;
            tokio::spawn(async move {
                let _ = store
                    .save_json("agents", &id.to_string(), &info)
                    .await
                    .inspect_err(|e| tracing::warn!(agent_id = %id, error = %e, "Failed to persist agent to filesystem"));

                // Prune old records (async, best-effort)
                if max_entries > 0 || ttl_hours > 0 {
                    let _ = store
                        .prune_agents_by_config(max_entries, ttl_hours, batch_size)
                        .await
                        .inspect_err(|e| tracing::warn!(error = %e, "Failed to prune agent log"));
                }
            });
        }

        // 2. SQLite (query index)
        if let Some(ref db) = self.agent_log_db {
            let db = db.clone();
            let info = info.clone();
            let config = self.agent_log_config.clone();
            tokio::spawn(async move {
                let _ = db
                    .upsert_agent(&info)
                    .inspect_err(|e| tracing::warn!(agent_id = %id, error = %e, "Failed to upsert agent to SQLite"));

                // Prune old records
                let _ = db
                    .prune(&config)
                    .inspect_err(|e| tracing::warn!(error = %e, "Failed to prune agent SQLite"));
            });
        }
    }
}

/// A no-op supervisor used during KernelBuilder::build() to break the
/// KernelHandle → AgentRuntime → Supervisor → KernelHandle cycle.
///
/// AgentApi.supervisor is only used for list/kill operations, not during
/// tool registration, so this placeholder is safe during build time.
pub struct NoOpSupervisor;

#[async_trait::async_trait]
impl Supervisor for NoOpSupervisor {
    async fn exec(&self, _id: AgentId) -> Result<()> {
        Err(anyhow::anyhow!(
            "NoOpSupervisor: exec not available during build"
        ))
    }
    async fn fork_directive(&self, _directive: &Directive, _env: &ExecEnv) -> Result<AgentId> {
        Err(anyhow::anyhow!(
            "NoOpSupervisor: fork_directive not available during build"
        ))
    }
    async fn run_with_directive(
        &self,
        _id: AgentId,
        _directive: &Directive,
        _env: &ExecEnv,
    ) -> Result<ExecutionResult> {
        Err(anyhow::anyhow!(
            "NoOpSupervisor: run_with_directive not available during build"
        ))
    }
    async fn wait(&self, _id: AgentId) -> Result<AgentStatus> {
        Err(anyhow::anyhow!(
            "NoOpSupervisor: wait not available during build"
        ))
    }
    async fn kill(&self, _id: AgentId) -> Result<()> {
        Err(anyhow::anyhow!(
            "NoOpSupervisor: kill not available during build"
        ))
    }
    async fn list(&self) -> Result<Vec<AgentInfo>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_bus::EventBus;
    use crate::types::AgentStatus;

    // Note: MockProvider no longer needed — OxiosEngine handles provider resolution.
    // The engine resolves models internally, so tests just use OxiosEngine::new().

    /// Helper to create a real BasicSupervisor wired to a real EventBus.
    async fn make_supervisor() -> BasicSupervisor {
        let event_bus = EventBus::new(64);

        // Build a mock KernelHandle with temp dirs.
        let tmp = std::env::temp_dir().join(format!("oxios-test-{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&tmp);

        let state_store_2 =
            Arc::new(crate::state_store::StateStore::new(tmp.join("state")).expect("state store"));
        let state_store = state_store_2.clone();

        let kernel_handle = Arc::new(crate::KernelHandle::new(
            crate::kernel_handle::StateApi::new(state_store),
            crate::kernel_handle::AgentApi::new(
                Arc::new(crate::supervisor::NoOpSupervisor),
                Arc::new(crate::budget::BudgetManager::new()),
            ),
            crate::kernel_handle::SecurityApi::new(
                Arc::new(parking_lot::Mutex::new(crate::auth::AuthManager::new())),
                Arc::new(oxicode_sdk::observability::AuditTrail::new(100)),
                Arc::new(parking_lot::Mutex::new(
                    crate::access_manager::AccessManager::new(),
                )),
                Arc::new(
                    crate::state_store::StateStore::new(tmp.join("state2")).expect("state store 2"),
                ),
            ),
            crate::kernel_handle::PersonaApi::new(Arc::new(crate::persona::PersonaManager::new())),
            crate::kernel_handle::ExtensionApi::new(Arc::new(crate::skill::SkillManager::new(
                tmp.join("skills"),
                tmp.join("share/skills"),
            ))),
            crate::kernel_handle::McpApi::new(Arc::new(crate::mcp::McpBridge::new())),
            crate::kernel_handle::InfraApi::new(
                Arc::new(
                    crate::git_layer::GitLayer::new(tmp.join("git"), false).expect("git layer"),
                ),
                // T16: knowledge_git — distinct layer for vault-rooted commits.
                // In tests we point it at a sibling temp dir so repo init does
                // not race with the workspace git layer above.
                Arc::new(
                    crate::git_layer::GitLayer::new(tmp.join("kb_git"), false)
                        .expect("knowledge git layer"),
                ),
                Arc::new(crate::cron::CronScheduler::new(
                    Arc::new(
                        crate::state_store::StateStore::new(tmp.join("cron")).expect("cron state"),
                    ),
                    60,
                )),
                Arc::new(crate::resource_monitor::ResourceMonitor::new(60, 100)),
                EventBus::new(64),
                crate::config::OxiosConfig::default(),
                std::time::Instant::now(),
                std::sync::Arc::new(crate::tools::PendingToolApprovals::new()),
                std::sync::Arc::new(crate::tools::PendingAskUser::new()),
                std::sync::Arc::new(parking_lot::RwLock::new(
                    crate::approval::ApprovalConfig::default(),
                )),
                std::sync::Arc::new(crate::tools::PendingPathAccess::new()),
            ),
            None,
            crate::kernel_handle::ExecApi::new(
                Arc::new(parking_lot::RwLock::new(
                    crate::config::ExecConfig::default(),
                )),
                Arc::new(parking_lot::Mutex::new(
                    crate::access_manager::AccessManager::new(),
                )),
            ),
            crate::kernel_handle::A2aApi::new(Arc::new(crate::a2a::A2AProtocol::new(
                EventBus::new(64),
            ))),
            crate::kernel_handle::EngineApi::new(
                Arc::new(parking_lot::RwLock::new(
                    crate::config::OxiosConfig::default(),
                )),
                tmp.join("config.toml"),
                Arc::new(crate::kernel_handle::RoutingStats::new()),
                Arc::new(crate::engine::EngineHandle::new(Arc::new(
                    crate::OxiosEngine::new("anthropic/claude-sonnet-4-20250514"),
                ))),
            ),
            Arc::new(oxios_markdown::KnowledgeBase::new(tmp.join("knowledge")).unwrap()),
            Arc::new(
                crate::kernel_handle::KnowledgeLens::new(
                    Arc::new(oxios_markdown::KnowledgeBase::new(tmp.join("knowledge")).unwrap()),
                    None,
                )
                .unwrap(),
            ),
            crate::kernel_handle::MarketplaceApi::new(
                Arc::new(crate::skill::clawhub::ClawHubInstaller::new(
                    tmp.join("skills"),
                    tmp.join("state"),
                    None,
                )),
                Arc::new(
                    crate::skill::clawhub::ClawHubClient::new(None).expect("valid ClawHub client"),
                ),
                Arc::new(crate::skill::skills_sh::SkillsShInstaller::new(
                    tmp.join("skills"),
                    None,
                    None,
                )),
                Arc::new(
                    crate::skill::skills_sh::SkillsShClient::new(None, None)
                        .expect("valid Skills.sh client"),
                ),
            ),
            None,                                     // calendar (not configured in test)
            Arc::new(parking_lot::RwLock::new(None)), // email (not configured in test)
        ));

        let engine = crate::OxiosEngine::new("mock/model");
        let engine_handle = Arc::new(crate::engine::EngineHandle::new(Arc::new(engine)));
        let runtime = AgentRuntime::new(engine_handle, kernel_handle, None);
        BasicSupervisor::new(event_bus, runtime)
    }

    /// Helper to create a minimal (Directive, ExecEnv) pair for testing.
    fn make_directive(goal: &str) -> (Directive, ExecEnv) {
        (Directive::from_message(goal), ExecEnv::default())
    }

    #[tokio::test]
    async fn test_fork_creates_agent() {
        let supervisor = make_supervisor().await;
        let (directive, env) = make_directive("Test agent");

        let id = supervisor.fork_directive(&directive, &env).await.unwrap();

        let agents = supervisor.list().await.unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, id);
        assert_eq!(agents[0].name, "Test agent");
        assert_eq!(agents[0].status, AgentStatus::Starting);
    }

    #[tokio::test]
    async fn test_exec_updates_status_to_running() {
        let supervisor = make_supervisor().await;
        let (directive, env) = make_directive("Running agent");

        let id = supervisor.fork_directive(&directive, &env).await.unwrap();
        assert_eq!(supervisor.wait(id).await.unwrap(), AgentStatus::Starting);

        supervisor.exec(id).await.unwrap();
        assert_eq!(supervisor.wait(id).await.unwrap(), AgentStatus::Running);
    }

    #[tokio::test]
    async fn test_kill_sets_stopped() {
        let supervisor = make_supervisor().await;
        let (directive, env) = make_directive("Doomed agent");

        let id = supervisor.fork_directive(&directive, &env).await.unwrap();
        supervisor.exec(id).await.unwrap();
        assert_eq!(supervisor.wait(id).await.unwrap(), AgentStatus::Running);

        supervisor.kill(id).await.unwrap();
        assert_eq!(supervisor.wait(id).await.unwrap(), AgentStatus::Stopped);
    }

    #[tokio::test]
    async fn test_kill_unknown_agent_returns_error() {
        let supervisor = make_supervisor().await;
        let unknown_id = uuid::Uuid::new_v4();

        let result = supervisor.kill(unknown_id).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_list_returns_all_agents() {
        let supervisor = make_supervisor().await;

        let (d1, e1) = make_directive("Agent 1");
        let id1 = supervisor.fork_directive(&d1, &e1).await.unwrap();
        let (d2, e2) = make_directive("Agent 2");
        let id2 = supervisor.fork_directive(&d2, &e2).await.unwrap();
        let (d3, e3) = make_directive("Agent 3");
        let id3 = supervisor.fork_directive(&d3, &e3).await.unwrap();

        let agents = supervisor.list().await.unwrap();
        assert_eq!(agents.len(), 3);

        let ids: std::collections::HashSet<AgentId> = agents.iter().map(|a| a.id).collect();
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
        assert!(ids.contains(&id3));
    }

    #[tokio::test]
    async fn test_exec_unknown_agent_returns_error() {
        let supervisor = make_supervisor().await;
        let unknown_id = uuid::Uuid::new_v4();

        let result = supervisor.exec(unknown_id).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_wait_unknown_agent_returns_error() {
        let supervisor = make_supervisor().await;
        let unknown_id = uuid::Uuid::new_v4();

        let result = supervisor.wait(unknown_id).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }
}
