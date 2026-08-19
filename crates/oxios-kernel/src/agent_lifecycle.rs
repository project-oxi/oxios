//! Agent lifecycle management — fork, register, run, cleanup.
//!
//! Extracted from Orchestrator to reduce the god-object scope.
//! Handles: fork agent → register A2A → check permissions →
//! submit to scheduler → run → unregister → complete/fail.

use anyhow::{Result, bail};
use std::sync::Arc;

use tokio::time::{Duration, timeout};

use crate::a2a::{A2AProtocol, AgentCard};
use crate::access_manager::{AccessManager, Role, Subject};
use crate::event_bus::{EventBus, KernelEvent};
use crate::metrics::get_metrics;
use crate::supervisor::Supervisor;
use crate::types::{AgentId, AgentStatus};
use oxios_ouroboros::{Directive, ExecEnv, ExecutionResult};

/// Manages the full lifecycle of a single agent from fork to cleanup.
pub struct AgentLifecycleManager {
    supervisor: Arc<dyn Supervisor>,
    access_manager: Arc<parking_lot::Mutex<AccessManager>>,
    a2a: Arc<A2AProtocol>,
    event_bus: EventBus,
    /// Maximum execution time in seconds for agent tasks (0 = no limit).
    max_execution_time_secs: std::sync::atomic::AtomicU64,
    /// Default allowed tools from config.
    allowed_tools: Vec<String>,
    /// Whether agents get network access by default.
    network_access: bool,
    /// Workspace path for path sandbox.
    workspace_path: String,
    /// Workspace path for path sandbox.
    workspace_path: String,
    /// RFC-049: shared turn registry, used to bind forks to their turn key.
    turns: Arc<crate::turn_registry::TurnRegistry>,
}

impl Clone for AgentLifecycleManager {
    fn clone(&self) -> Self {
        Self {
            supervisor: self.supervisor.clone(),
            access_manager: self.access_manager.clone(),
            a2a: self.a2a.clone(),
            event_bus: self.event_bus.clone(),
            max_execution_time_secs: std::sync::atomic::AtomicU64::new(
                self.max_execution_time_secs
                    .load(std::sync::atomic::Ordering::Relaxed),
            ),
            allowed_tools: self.allowed_tools.clone(),
            network_access: self.network_access,
            workspace_path: self.workspace_path.clone(),
        }
    }
}

impl AgentLifecycleManager {
    /// Create a new lifecycle manager.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        supervisor: Arc<dyn Supervisor>,
        access_manager: Arc<parking_lot::Mutex<AccessManager>>,
        a2a: Arc<A2AProtocol>,
        event_bus: EventBus,
        max_execution_time_secs: u64,
        allowed_tools: Vec<String>,
        network_access: bool,
        workspace_path: String,
    ) -> Self {
        Self {
            supervisor,
            access_manager,
            a2a,
            event_bus,
            max_execution_time_secs: std::sync::atomic::AtomicU64::new(max_execution_time_secs),
            allowed_tools,
            network_access,
            workspace_path,
        }
    }

    /// Hot-reload max execution time without restart.
    pub fn set_max_execution_time(&self, secs: u64) {
        self.max_execution_time_secs
            .store(secs, std::sync::atomic::Ordering::Relaxed);
        tracing::info!(
            max_execution_time_secs = secs,
            "Lifecycle config hot-reloaded"
        );
    }

    /// Fork an agent, register it in A2A and access control, submit to
    /// scheduler, run the directive + exec env, then clean up (RFC-027).
    pub async fn execute_directive(
        &self,
        directive: &Directive,
        env: &ExecEnv,
    ) -> Result<ExecutionResult> {
        // 1. Fork
        let agent_id = self.supervisor.fork_directive(directive, env).await?;
        let agent_name = format!("agent-{agent_id}");
        tracing::info!(agent_id = %agent_id, "Agent forked from directive");

        // 2. Register A2A card
        let card = self.build_agent_card_directive(agent_id, &agent_name, directive);
        if let Err(e) = self.a2a.registry().register_agent(card).await {
            tracing::warn!(agent_id = %agent_id, error = %e, "Failed to register A2A card");
        }

        // 2b. Deliver any pending A2A messages to this agent
        if let Err(e) = self.a2a.deliver_pending_messages(agent_id).await {
            tracing::debug!(agent_id = %agent_id, error = %e, "No pending A2A messages");
        }

        // 3. Ensure access permissions
        self.ensure_permissions(&agent_name);

        get_metrics().agents_forked.inc();

        // 5. Run — always cleanup even on failure
        let max_secs = self
            .max_execution_time_secs
            .load(std::sync::atomic::Ordering::Relaxed);
        let result = if max_secs > 0 {
            let exec_timeout = Duration::from_secs(max_secs);
            match timeout(
                exec_timeout,
                self.supervisor.run_with_directive(agent_id, directive, env),
            )
            .await
            {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => {
                    tracing::warn!(agent_id = %agent_id, error = %e, "Agent execution failed, cleaning up");
                    self.cleanup_on_failure(agent_id).await;
                    return Err(e);
                }
                Err(_) => {
                    let secs = exec_timeout.as_secs();
                    tracing::warn!(
                        agent_id = %agent_id,
                        secs,
                        "Agent execution timed out after {}s",
                        secs
                    );
                    // Abort the detached execution body. Previously the
                    // timeout only dropped the awaiting future while the
                    // spawned task kept running — leaking tokens/resources.
                    self.cleanup_on_failure(agent_id).await;
                    bail!("Agent execution timed out after {secs} seconds");
                }
            }
        } else {
            match self
                .supervisor
                .run_with_directive(agent_id, directive, env)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(agent_id = %agent_id, error = %e, "Agent execution failed, cleaning up");
                    self.cleanup_on_failure(agent_id).await;
                    return Err(e);
                }
            }
        };

        // 6. Cleanup on success
        self.cleanup(agent_id, &result).await;

        Ok(result)
    }

    /// Execute a directive with feedback from a previous failed attempt (RFC-027).
    ///
    /// Injects the previous result's output and the review gaps into the
    /// directive's constraints so the agent sees what went wrong.
    pub async fn execute_with_feedback(
        &self,
        directive: &Directive,
        env: &ExecEnv,
        prev_result: &ExecutionResult,
        gaps: &[String],
    ) -> Result<ExecutionResult> {
        // Augment the directive with feedback from the previous attempt.
        let mut augmented = directive.clone();
        let feedback = format!(
            "## Previous attempt failed\n{}\n\n## Unmet criteria\n{}\n\n\
             Review the above output and fix the unmet criteria.",
            prev_result.output,
            gaps.iter()
                .enumerate()
                .map(|(i, g)| format!("{}. {g}", i + 1))
                .collect::<Vec<_>>()
                .join("\n")
        );
        augmented.constraints.push(feedback);

        self.execute_directive(&augmented, env).await
    }

    /// Kill an agent and clean up all registered state.
    pub async fn terminate(&self, agent_id: AgentId) -> Result<()> {
        if let Err(e) = self.supervisor.kill(agent_id).await {
            tracing::warn!(agent_id = %agent_id, error = %e, "Failed to kill agent");
        }
        if let Err(e) = self.a2a.registry().unregister_agent(agent_id).await {
            tracing::warn!(agent_id = %agent_id, error = %e, "Failed to unregister A2A card");
        }
        let _ = self.event_bus.publish(KernelEvent::AgentStopped {
            id: agent_id,
            success: false,
        });
        Ok(())
    }

    /// Build an A2A agent card from a Directive (RFC-027).
    ///
    /// Reads the goal from a Directive and advertises `execute-directive`
    /// so A2A consumers know the agent follows the directive path.
    fn build_agent_card_directive(
        &self,
        agent_id: AgentId,
        agent_name: &str,
        directive: &Directive,
    ) -> AgentCard {
        let goal_lower = directive.goal.to_lowercase();

        let mut card = AgentCard::new(
            agent_id,
            agent_name,
            format!("Agent executing directive: {}", directive.goal),
        )
        .with_capability("execute-directive")
        .with_status(AgentStatus::Starting);

        // Infer capabilities from goal.
        if goal_lower.contains("review") || goal_lower.contains("code") {
            card = card.with_capability("code-review");
        }
        if goal_lower.contains("test") {
            card = card.with_capability("testing");
        }
        if goal_lower.contains("refactor") || goal_lower.contains("improve") {
            card = card.with_capability("refactoring");
        }
        if goal_lower.contains("write")
            || goal_lower.contains("create")
            || goal_lower.contains("implement")
        {
            card = card.with_capability("code-generation");
        }
        if goal_lower.contains("debug") || goal_lower.contains("fix") {
            card = card.with_capability("debugging");
        }

        card
    }

    /// Ensure default tool permissions exist for an agent.
    ///
    /// Applies config.toml `[security]` settings:
    /// - `allowed_tools` → agent's tool set
    /// - `network_access` → network permission
    /// - workspace path → path sandbox
    /// - RBAC `Superuser` role → allows all tools and paths
    fn ensure_permissions(&self, agent_name: &str) {
        let mut access = self.access_manager.lock();
        let perms = access.get_or_create_permissions(agent_name);

        // Grant all tools from config
        for tool in &self.allowed_tools {
            if !perms.allowed_tools.contains(tool.as_str()) {
                perms.allow_tool(tool);
            }
        }

        // Add workspace path to allowed paths
        let ws_pattern = format!("{}/**", self.workspace_path.trim_end_matches('/'));
        if !perms.allowed_paths.iter().any(|p| p == &ws_pattern) {
            perms.allow_path(&ws_pattern);
        }
        // Also allow /tmp for agent temp files
        if !perms.allowed_paths.iter().any(|p| p == "/tmp/**") {
            perms.allow_path("/tmp/**");
        }

        // Apply network access from config
        if self.network_access {
            perms.enable_network();
        }

        // Assign Superuser RBAC role so AccessGate passes
        // (config.toml already defines which tools are allowed)
        let subject = Subject::Agent(
            agent_name
                .strip_prefix("agent-")
                .and_then(|s| s.parse().ok())
                .unwrap_or_default(),
        );
        access
            .rbac_manager_mut()
            .assign_role(subject, Role::Superuser);
    }

    /// Unregister A2A card on completion.
    async fn cleanup(&self, agent_id: AgentId, _result: &ExecutionResult) {
        if let Err(e) = self.a2a.registry().unregister_agent(agent_id).await {
            tracing::warn!(agent_id = %agent_id, error = %e, "Failed to unregister A2A card");
        }
    }

    /// Cleanup when agent execution fails (no ExecutionResult available).
    async fn cleanup_on_failure(&self, agent_id: AgentId) {
        if let Err(e) = self.a2a.registry().unregister_agent(agent_id).await {
            tracing::warn!(agent_id = %agent_id, error = %e, "Failed to unregister A2A card");
        }
    }
}
