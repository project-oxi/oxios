//! Infra API — Git, cron, resources, events, system.

use crate::ToolMeta;
use crate::config::OxiosConfig;
use crate::cron::{CronJob, CronJobUpdate, CronScheduler};
use crate::event_bus::{EventBus, KernelEvent};
use crate::git_layer::{GitLayer, LogEntry};
use crate::resource_monitor::{ResourceMonitor, ResourceSnapshot};
use crate::tools::{PendingAskUser, PendingPathAccess, PendingToolApprovals, known_tools};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Infrastructure system calls.
pub struct InfraApi {
    pub(crate) git_layer: Arc<GitLayer>,
    /// Vault-rooted `GitLayer` (T16) — `kb.root()` is its own repo, so
    /// `commit_file` / `log_for_file` / `restore_file` use paths computed
    /// by [`crate::git_layer::rel_path`] with no `knowledge/` prefix.
    pub(crate) knowledge_git: Arc<GitLayer>,
    pub(crate) cron_scheduler: Arc<CronScheduler>,
    pub(crate) resource_monitor: Arc<ResourceMonitor>,
    pub(crate) event_bus: EventBus,
    pub(crate) config: OxiosConfig,
    pub(crate) start_time: Instant,
    /// Hot-reloadable orchestrator config (evolution iterations, score threshold).
    pub(crate) orchestrator_config: parking_lot::RwLock<crate::config::OrchestratorConfig>,
    pub(crate) approval_config: Arc<parking_lot::RwLock<crate::approval::ApprovalConfig>>,
    /// Pending tool approval requests (HitL escalation).
    pub(crate) pending_tool_approvals: Arc<PendingToolApprovals>,
    /// Pending `ask_user` requests (RFC-027 agent-driven clarification).
    pub(crate) pending_ask_user: Arc<PendingAskUser>,
    /// Pending path-access requests (interactive Mount / temp-allow / deny cards).
    pub(crate) pending_path_access: Arc<PendingPathAccess>,
}

impl InfraApi {
    /// Create a new InfraApi.
    // Facade construction gathers the independent infrastructure services.
    //
    // `approval_config` is an Arc the caller shares across ALL KernelHandle
    // instances — the preliminary handle (AgentRuntime's ApprovalGate reads
    // here) and the cached handle (HTTP PATCH /api/security/approval writes
    // here). Passing it in (rather than deriving a fresh Arc from `config`
    // inside `new`) mirrors the `pending_tool_approvals` sharing rule: without
    // it, a mode toggle writes one instance while the gate reads another, so
    // AutoRun never takes effect and grants never stick.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        git_layer: Arc<GitLayer>,
        knowledge_git: Arc<GitLayer>,
        cron_scheduler: Arc<CronScheduler>,
        resource_monitor: Arc<ResourceMonitor>,
        event_bus: EventBus,
        config: OxiosConfig,
        start_time: Instant,
        pending_tool_approvals: Arc<PendingToolApprovals>,
        pending_ask_user: Arc<PendingAskUser>,
        approval_config: Arc<parking_lot::RwLock<crate::approval::ApprovalConfig>>,
        pending_path_access: Arc<PendingPathAccess>,
    ) -> Self {
        Self {
            git_layer,
            knowledge_git,
            cron_scheduler,
            resource_monitor,
            event_bus,
            config,
            start_time,
            orchestrator_config: parking_lot::RwLock::new(
                crate::config::OrchestratorConfig::default(),
            ),
            approval_config,
            pending_tool_approvals,
            pending_ask_user,
            pending_path_access,
        }
    }
    /// Get a reference to the GitLayer.
    pub fn git(&self) -> &GitLayer {
        &self.git_layer
    }

    /// Get a reference to the vault-rooted knowledge `GitLayer` (T16).
    /// Distinct from [`Self::git`] which tracks workspace artifacts
    /// (audit trail, agent work). The knowledge layer commits to the
    /// vault itself, so callers MUST route paths through
    /// [`crate::git_layer::rel_path`] to stay relative to the vault root.
    pub fn knowledge_git(&self) -> &GitLayer {
        &self.knowledge_git
    }

    pub fn approval_config_handle(
        &self,
    ) -> Arc<parking_lot::RwLock<crate::approval::ApprovalConfig>> {
        Arc::clone(&self.approval_config)
    }
    pub fn approval_config(&self) -> crate::approval::ApprovalConfig {
        self.approval_config.read().clone()
    }

    pub async fn set_approval_config(
        &self,
        config: crate::approval::ApprovalConfig,
    ) -> anyhow::Result<crate::approval::ApprovalConfig> {
        *self.approval_config.write() = config.clone();
        Ok(config)
    }

    pub async fn add_grant(&self, key: String) -> anyhow::Result<crate::approval::ApprovalConfig> {
        let mut config = self.approval_config();
        if !config.allow_list.contains(&key) {
            config.allow_list.push(key);
        }
        self.set_approval_config(config.clone()).await
    }

    pub async fn remove_grant(&self, key: &str) -> anyhow::Result<crate::approval::ApprovalConfig> {
        let mut config = self.approval_config();
        config.allow_list.retain(|item| item != key);
        self.set_approval_config(config.clone()).await
    }

    /// Get commit log.
    pub fn git_log(&self, max: usize) -> anyhow::Result<Vec<LogEntry>> {
        self.git_layer.log(max)
    }

    /// Tag current state.
    pub fn git_tag(&self, name: &str, message: &str) -> anyhow::Result<()> {
        self.git_layer.tag(name, message)
    }

    /// Restore file from commit.
    pub fn git_restore(&self, path: &str, hash: &str) -> anyhow::Result<()> {
        self.git_layer.restore_file(path, hash)
    }

    /// Verify git repository integrity.
    pub fn git_verify(&self) -> anyhow::Result<bool> {
        self.git_layer.verify()
    }

    /// List git tags.
    pub fn git_tags(&self) -> anyhow::Result<Vec<String>> {
        self.git_layer.list_tags()
    }

    /// Delete a tag by name.
    pub fn git_delete_tag(&self, name: &str) -> anyhow::Result<()> {
        self.git_layer.delete_tag(name)
    }

    /// Add a cron job.
    pub async fn add_cron(&self, job: CronJob) -> anyhow::Result<uuid::Uuid> {
        self.cron_scheduler.add_job(job).await
    }

    /// Get a cron job by ID.
    pub fn get_cron(&self, id: uuid::Uuid) -> Option<CronJob> {
        self.cron_scheduler.get_job(id)
    }

    /// Update a cron job.
    pub async fn update_cron(&self, id: uuid::Uuid, update: CronJobUpdate) -> anyhow::Result<()> {
        self.cron_scheduler.update_job(id, update).await
    }

    /// Remove a cron job by ID.
    pub async fn remove_cron(&self, id: uuid::Uuid) -> anyhow::Result<()> {
        self.cron_scheduler.remove_job(id).await
    }

    /// Trigger a cron job manually.
    pub fn trigger_cron(&self, id: uuid::Uuid) -> anyhow::Result<CronJob> {
        self.cron_scheduler.trigger_job(id)
    }

    /// Mark cron job completed.
    pub async fn complete_cron(&self, id: uuid::Uuid, success: bool, summary: String) {
        self.cron_scheduler
            .mark_job_completed(id, success, summary)
            .await
    }

    /// List all cron jobs.
    pub fn list_crons(&self) -> Vec<CronJob> {
        self.cron_scheduler.list_jobs()
    }

    /// Get resource snapshot.
    pub fn resource_snapshot(&self) -> ResourceSnapshot {
        self.resource_monitor.snapshot()
    }

    /// Get resource history snapshots.
    pub fn resource_history(&self, last_n: usize) -> Vec<ResourceSnapshot> {
        self.resource_monitor.history(last_n)
    }

    /// Check if system is overloaded.
    pub fn is_overloaded(&self) -> bool {
        self.resource_monitor.is_overloaded()
    }

    /// Subscribe to kernel events.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<KernelEvent> {
        self.event_bus.subscribe()
    }

    /// Publish a kernel event.
    pub fn publish(&self, event: KernelEvent) -> anyhow::Result<()> {
        self.event_bus
            .publish(event)
            .map_err(|e| anyhow::anyhow!("broadcast error: {e}"))
    }

    /// Get config reference.
    pub fn config(&self) -> &OxiosConfig {
        &self.config
    }

    /// Resource monitor reference — for hot-reload config propagation.
    pub fn resource_monitor(&self) -> &Arc<ResourceMonitor> {
        &self.resource_monitor
    }

    /// Hot-reload orchestrator config (stored in InfraApi for propagation).
    pub fn update_orchestrator_config(&self, config: crate::config::OrchestratorConfig) {
        *self.orchestrator_config.write() = config;
    }

    /// Get system uptime.
    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Access the pending tool approvals registry.
    pub fn pending_tool_approvals(&self) -> Arc<PendingToolApprovals> {
        self.pending_tool_approvals.clone()
    }
    /// Access the pending path-access registry.
    pub fn pending_path_access(&self) -> Arc<PendingPathAccess> {
        self.pending_path_access.clone()
    }

    /// Access the pending `ask_user` registry (RFC-027 agent-driven clarification).
    pub fn pending_ask_user(&self) -> Arc<PendingAskUser> {
        Arc::clone(&self.pending_ask_user)
    }

    /// Clone the underlying event bus so tools can publish `KernelEvent`s
    /// without routing through the wrapping `publish` helper.
    pub fn event_bus_clone(&self) -> EventBus {
        self.event_bus.clone()
    }

    /// List all known tool metadata (static catalog).
    pub fn list_available_tools(&self) -> &'static [ToolMeta] {
        known_tools()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::{ApprovalConfig, ApprovalMode};
    use crate::state_store::StateStore;

    fn minimal_infra(approval_config: Arc<parking_lot::RwLock<ApprovalConfig>>) -> InfraApi {
        let dir = tempfile::tempdir().unwrap();
        InfraApi::new(
            Arc::new(GitLayer::new(dir.path().join("git"), false).unwrap()),
            // T16: knowledge_git — same Arc is fine for the unit test; the
            // contract under test is approval_config Arc sharing, not git
            // topology.
            Arc::new(GitLayer::new(dir.path().join("git"), false).unwrap()),
            Arc::new(CronScheduler::new(
                Arc::new(StateStore::new(dir.path().join("state")).unwrap()),
                60,
            )),
            Arc::new(ResourceMonitor::new(60, 60)),
            EventBus::new(64),
            OxiosConfig::default(),
            std::time::Instant::now(),
            Arc::new(PendingToolApprovals::new()),
            Arc::new(PendingAskUser::new()),
            approval_config,
            Arc::new(PendingPathAccess::new()),
        )
    }

    // Regression: InfraApi::new must store the PASSED approval_config Arc, not
    // derive a fresh one from `config.security.approval`. Without sharing, the
    // cached handle (HTTP PATCH /api/security/approval) and the preliminary
    // handle (AgentRuntime's ApprovalGate) hold independent Arcs — a mode
    // toggle writes one while the gate reads another, so AutoRun never takes
    // effect and every OnDemand tool (web_search, exec, write …) re-prompts.
    #[test]
    fn new_stores_shared_approval_config_arc() {
        let shared = Arc::new(parking_lot::RwLock::new(ApprovalConfig::default()));
        let infra = minimal_infra(Arc::clone(&shared));
        assert!(
            Arc::ptr_eq(&infra.approval_config_handle(), &shared),
            "InfraApi must store the passed approval_config Arc, not a fresh clone"
        );
    }

    // Two InfraApi instances built from the same shared Arc must see each
    // other's mutations — the cross-handle visibility contract.
    #[tokio::test]
    async fn shared_arc_visible_across_instances() {
        let shared = Arc::new(parking_lot::RwLock::new(ApprovalConfig::default()));
        let a = minimal_infra(Arc::clone(&shared));
        let b = minimal_infra(shared);

        // Simulates HTTP PATCH { mode: "auto-run" } on the cached handle.
        let mut cfg = a.approval_config();
        cfg.mode = ApprovalMode::AutoRun;
        a.set_approval_config(cfg).await.unwrap();

        // The AgentRuntime's gate (reading the preliminary handle) must see it.
        assert_eq!(b.approval_config().mode, ApprovalMode::AutoRun);
    }
}
