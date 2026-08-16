//! Kernel assembly — Builder pattern for wiring all Oxios components.
//!
//! This module lives in the binary crate (not oxios-kernel) because
//! it's responsible for *assembling* kernel components, not providing them.
//! The kernel library provides parts; the binary puts them together.

use anyhow::{Context, Result};
use oxicode_sdk::ModelCatalog;
use oxios_gateway::Gateway;
use oxios_kernel::{
    A2AProtocol, AgentRuntime, AuditPersistence, AuditTrail, BasicSupervisor, BrainConfig,
    BrainConnection, BudgetManager, ClawHubClient, ClawHubInstaller, CronScheduler, EngineHandle,
    EventBus, GitLayer, KernelDatabase, MarketplaceApi, McpBridge, McpServer, Orchestrator,
    OxiosConfig, OxiosEngine, PersonaManager, ProjectManager, ResourceMonitor, SkillManager,
    SkillsShClient, SkillsShInstaller, SubsystemState, Supervisor, access_manager::AccessManager,
    auth::AuthManager, config::load_config, mcp::validate_mcp_command,
};
use oxios_markdown::KnowledgeBase;
use oxios_markdown::knowledge::FileChange;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Fully assembled Oxios kernel with all components wired together.
///
/// Created via [`Kernel::builder()`]. Fields are private — access
/// through typed methods or [`Kernel::handle()`] for the KernelHandle facade.
pub struct Kernel {
    orchestrator: Arc<Orchestrator>,
    gateway: Arc<Gateway>,
    event_bus: EventBus,
    state_store: Arc<oxios_kernel::state_store::StateStore>,
    config: OxiosConfig,
    skill_manager: Arc<SkillManager>,
    supervisor: Arc<dyn Supervisor>,
    access_manager: Arc<parking_lot::Mutex<AccessManager>>,
    persona_manager: Arc<PersonaManager>,
    mcp_bridge: Arc<McpBridge>,
    /// Brain daemon connection (RFC-047). Degrades when the daemon is down.
    brain: Arc<BrainConnection>,
    auth_manager: Arc<parking_lot::Mutex<AuthManager>>,
    cron_scheduler: Arc<CronScheduler>,
    git_layer: Arc<GitLayer>,
    audit_trail: Arc<AuditTrail>,
    budget_manager: Arc<BudgetManager>,
    /// RFC-031: the shared QuotaTracker (self-tracker + recalibration).
    quota_tracker: Arc<oxios_kernel::QuotaTracker>,
    /// RFC-031: the TokenMaxer orchestrator (drain loop).
    token_maxer: Arc<oxios_kernel::TokenMaxer>,
    resource_monitor: Arc<ResourceMonitor>,
    /// Shared HitL approval registry — the SAME instance must back both the
    /// preliminary handle (AgentRuntime/tools register here) and the cached
    /// handle (HTTP resolver looks up here). Without sharing, the agent
    /// registers an approval in one map while /api/chat/tool-approval/{id}/respond
    /// resolves from another → 404 on every click.
    pending_tool_approvals: Arc<oxios_kernel::tools::PendingToolApprovals>,
    /// Shared ask_user registry (RFC-027) — same cross-handle sharing rule.
    pending_ask_user: Arc<oxios_kernel::tools::PendingAskUser>,
    /// Shared approval configuration (RFC-035) — the SAME instance must back
    /// both the preliminary handle (AgentRuntime's ApprovalGate reads mode and
    /// grants here) and the cached handle (HTTP PATCH /api/security/approval
    /// writes here). Without sharing, a mode toggle or grant writes one
    /// instance while the gate reads another → AutoRun never takes effect and
    /// every OnDemand tool (web_search, exec, write …) re-prompts forever.
    /// Same cross-handle sharing rule as `pending_tool_approvals` above.
    approval_config: Arc<parking_lot::RwLock<oxios_kernel::approval::ApprovalConfig>>,
    /// Shared path-access registry — same cross-handle sharing rule as
    /// `pending_tool_approvals`. The GatedTool registers here when an agent
    /// tries to access a path outside `allowed_paths`; the HTTP respond
    /// endpoint resolves from the same map.
    pending_path_access: Arc<oxios_kernel::tools::PendingPathAccess>,
    project_manager: Option<Arc<ProjectManager>>,
    /// Mount manager (RFC-025 path aliases). `None` when SQLite memory is off.
    /// Wired into the lazily-cached handle so `/api/mounts` and the mount tool
    /// see live data — without this the API's handle has `mounts = None` even
    /// though the orchestrator holds a separate `Arc<MountManager>`.
    mount_manager: Option<Arc<oxios_kernel::MountManager>>,
    start_time: std::time::Instant,
    /// Path to config.toml (for persistence).
    config_path: PathBuf,
    /// Cached KernelHandle — created once, reused forever.
    handle_cache: OnceLock<Arc<oxios_kernel::KernelHandle>>,
    /// A2A protocol for inter-agent communication.
    a2a_protocol: Arc<A2AProtocol>,
    /// Hot-swappable engine reference — shared between EngineApi and AgentRuntime.
    engine_handle: Arc<EngineHandle>,
    /// SQLite-backed agent history query index.
    agent_log_db: Option<Arc<oxios_kernel::agent_log_db::AgentLogDb>>,
    /// RFC-025 Phase 5: cancellation sender for the Mount auto-promotion
    /// scanner (Promo-6). Sending `true` breaks the scan loop's `select!`.
    /// `None` when the scanner is disabled. Kept on the `Kernel` so the
    /// sender stays alive (otherwise `watch::Receiver::changed()` resolves
    /// immediately on sender-drop and would abort the loop) and so a future
    /// graceful shutdown can trigger it.
    #[allow(dead_code)] // wired via `shutdown_promotion_scanner` on graceful shutdown
    promo_shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
}

impl Kernel {
    /// Create a new kernel builder with sensible defaults.
    pub fn builder() -> KernelBuilder {
        KernelBuilder {
            config_path: oxios_kernel::config::expand_home("~/.oxios/config.toml"),
        }
    }

    // ── Public accessors ────────────────────────────────────────────────

    /// KernelHandle facade — the primary API for subcommands and plugins.
    ///
    /// Cached after first call. Use this for all kernel operations.
    pub fn handle(&self) -> Arc<oxios_kernel::KernelHandle> {
        self.handle_cache
            .get_or_init(|| {
                // KnowledgeBase — single source of truth (RFC-003)
                // Shared between KernelHandle.knowledge and KnowledgeLens.
                let knowledge = Arc::new(
                    KnowledgeBase::new(
                        std::path::PathBuf::from(&self.config.kernel.workspace).join("knowledge"),
                    )
                    .expect("KnowledgeBase init failed"),
                );
                let knowledge_lens = Arc::new(
                    oxios_kernel::KnowledgeLens::new(knowledge.clone(), Some(self.brain.clone()))
                        .expect("KnowledgeLens init failed"),
                );

                // Git auto-commit for knowledge files (async channel pattern)
                // Same pattern as KnowledgeLens — non-blocking to avoid delaying HTTP responses.
                {
                    let git = self.git_layer.clone();
                    let kb_root = knowledge.root();
                    let git_root = git.root().to_path_buf();
                    let prefix = kb_root
                        .strip_prefix(&git_root)
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| "knowledge".to_string());

                    let (git_tx, mut git_rx) =
                        tokio::sync::mpsc::channel::<(String, FileChange)>(64);

                    // Register callback — spawns a task to avoid blocking note_write()
                    knowledge.on_file_change(move |path: &str, change: FileChange| {
                        let tx = git_tx.clone();
                        let path = path.to_string();
                        tokio::spawn(async move {
                            let _ = tx.send((path, change)).await;
                        });
                    });
                    let reconcile_prefix = prefix.clone();

                    // Background consumer — commits knowledge changes to git
                    tokio::spawn(async move {
                        while let Some((path, change)) = git_rx.recv().await {
                            if !git.is_enabled() {
                                continue;
                            }
                            let rel = format!("{prefix}/{path}");
                            let msg = match &change {
                                FileChange::Created(p) => format!("knowledge: create {p}"),
                                FileChange::Updated(p) => format!("knowledge: update {p}"),
                                FileChange::Deleted(p) => format!("knowledge: delete {p}"),
                                FileChange::Moved { old, new } => {
                                    format!("knowledge: rename {old} → {new}")
                                }
                            };
                            match change {
                                FileChange::Deleted(_) => {
                                    if let Err(e) = git.remove_file(&rel, &msg) {
                                        tracing::warn!(error = %e, "knowledge git delete failed");
                                    }
                                }
                                FileChange::Moved { old, .. } => {
                                    let old_rel = format!("{prefix}/{old}");
                                    let _ = git.remove_file(&old_rel, &msg);
                                    let _ = git.commit_file(&rel, &msg);
                                }
                                _ => {
                                    if let Err(e) = git.commit_file(&rel, &msg) {
                                        tracing::warn!(error = %e, "knowledge git commit failed");
                                    }
                                }
                            }
                        }
                    });

                    // S-4: Post-crash reconciliation — commit knowledge files
                    // whose disk content diverged from git HEAD (e.g. process
                    // crashed between note_write and the async commit consumer).
                    // The I-3 dedup in commit_file_with skips files whose
                    // content matches HEAD, so this only creates commits for
                    // genuinely diverged files.
                    let reconcile_git = self.git_layer.clone();
                    if reconcile_git.is_enabled()
                        && let Ok(files) = knowledge.list_all_md_files()
                    {
                        let mut count = 0;
                        for (path, _) in &files {
                            let rel = format!("{reconcile_prefix}/{path}");
                            if let Ok(info) =
                                reconcile_git.commit_file(&rel, "knowledge: post-crash reconcile")
                                && info.hash != "(disabled)"
                            {
                                count += 1;
                            }
                        }
                        if count > 0 {
                            tracing::info!(
                                "Post-crash git reconcile: {count} diverged files re-committed"
                            );
                        }
                    }
                }

                let mut agent_api = oxios_kernel::AgentApi::new(
                    self.supervisor.clone(),
                    self.budget_manager.clone(),
                );
                agent_api.set_state_store(self.state_store.clone());

                if let Some(ref db) = self.agent_log_db {
                    agent_api.set_agent_log_db(db.clone());
                }

                let kh = oxios_kernel::KernelHandle::new(
                    oxios_kernel::StateApi::new(self.state_store.clone()),
                    agent_api,
                    oxios_kernel::SecurityApi::new(
                        self.auth_manager.clone(),
                        self.audit_trail.clone(),
                        self.access_manager.clone(),
                        self.state_store.clone(),
                    ),
                    oxios_kernel::PersonaApi::new(self.persona_manager.clone()),
                    oxios_kernel::ExtensionApi::new(Arc::clone(&self.skill_manager)),
                    oxios_kernel::McpApi::new(self.mcp_bridge.clone()),
                    oxios_kernel::InfraApi::new(
                        self.git_layer.clone(),
                        self.cron_scheduler.clone(),
                        self.resource_monitor.clone(),
                        self.event_bus.clone(),
                        self.config.clone(),
                        self.start_time,
                        self.pending_tool_approvals.clone(),
                        self.pending_ask_user.clone(),
                        self.approval_config.clone(),
                        self.pending_path_access.clone(),
                    ),
                    self.project_manager
                        .clone()
                        .map(oxios_kernel::ProjectApi::new),
                    oxios_kernel::ExecApi::new(
                        Arc::new(parking_lot::RwLock::new(self.config.exec.clone())),
                        self.access_manager.clone(),
                    ),
                    oxios_kernel::A2aApi::new(self.a2a_protocol.clone()),
                    // EngineApi — LLM providers, models, config + routing stats + engine hot-swap
                    oxios_kernel::EngineApi::new(
                        Arc::new(parking_lot::RwLock::new(self.config.clone())),
                        self.config_path.clone(),
                        Arc::new(oxios_kernel::RoutingStats::new()),
                        Arc::clone(&self.engine_handle),
                    ),
                    knowledge,
                    knowledge_lens,
                    self.build_marketplace_api(),
                    self.build_calendar_api(),
                    Arc::new(parking_lot::RwLock::new(self.build_email_api())),
                );
                // Attach the orchestrator so background loops (cron auto-start,
                // task auto-run) and the HTTP task-run handler share one
                // execution primitive via KernelHandle::run_goal.
                let kh = kh.with_orchestrator(self.orchestrator.clone());
                // RFC-025: attach MountApi to the handle the HTTP API and CLI
                // actually use. The orchestrator gets its own Arc directly; this
                // facade is what `/api/mounts` reads (`state.kernel.mounts`).
                let kh = if let Some(mm) = &self.mount_manager {
                    kh.with_mounts(oxios_kernel::MountApi::new(mm.clone()))
                } else {
                    kh
                };
                let kh = kh.with_token_maxing(oxios_kernel::TokenMaxingApi::new(
                    self.quota_tracker.clone(),
                    self.token_maxer.clone(),
                ));
                // RFC-047: brain daemon facade — the /api/brain/* surface.
                let kh = kh.with_brain(oxios_kernel::BrainApi::new(self.brain.clone()));
                // RFC-043: attach the task store so web routes, the auto-run
                // tick, and the `task` agent tool share ONE store. Boot
                // continues without it (web surface hard-requires it and
                // will fail its own expect).
                let kh = match oxios_kernel::task::TaskStore::open(
                    &(self.config.kernel.workspace.clone() + "/tasks.db"),
                ) {
                    Ok(ts) => kh.with_task_store(std::sync::Arc::new(tokio::sync::Mutex::new(ts))),
                    Err(e) => {
                        tracing::error!(error = %e, "task store init failed; tasks degraded");
                        kh
                    }
                };
                let kh = kh.with_browser(oxios_kernel::BrowserApi::from_config(&self.config));
                // oximemo (optional first-party app module; `memo` feature + [memo].enabled).
                #[cfg(feature = "memo")]
                let kh = match self.build_memo_api() {
                    Some(api) => kh.with_memo(api),
                    None => kh,
                };
                // oxiline (optional first-party app module; `timeline` feature + [timeline].enabled).
                #[cfg(feature = "timeline")]
                let kh = match self.build_timeline_api() {
                    Some(api) => kh.with_timeline(api),
                    None => kh,
                };
                let compression_service = Arc::new(oxios_kernel::CompressionService::new(
                    self.state_store.clone(),
                    self.engine_handle.clone(),
                    self.config.clone(),
                    self.event_bus.clone(),
                ));
                let kh =
                    kh.with_compression(oxios_kernel::CompressionApi::new(compression_service));

                // Unified asset store (~/.oxios/assets/).
                let assets_root = oxios_kernel::config::expand_home("~/.oxios/assets");
                match oxios_kernel::AssetStore::new(assets_root) {
                    Ok(store) => {
                        if let Err(e) = store.reconcile() {
                            tracing::warn!(error = %e, "Asset store reconcile failed");
                        }
                        let kh = kh.with_asset_store(std::sync::Arc::new(store));
                        Arc::new(kh)
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to initialize asset store");
                        Arc::new(kh)
                    }
                }
            })
            .clone()
    }

    /// Gateway reference — for channel registration and message routing.
    pub fn gateway(&self) -> Arc<Gateway> {
        self.gateway.clone()
    }

    /// Get the ProjectManager reference.
    /// Panics if SQLite is not enabled (project_manager is None).
    pub fn project_manager(&self) -> Arc<oxios_kernel::ProjectManager> {
        self.project_manager
            .clone()
            .expect("ProjectManager not available — SQLite must be enabled")
    }

    /// Get the MountManager reference, if SQLite-backed mounts are enabled.
    /// Returns `None` when the mount system is unavailable (SQLite off).
    pub fn mount_manager(&self) -> Option<Arc<oxios_kernel::MountManager>> {
        self.mount_manager.clone()
    }

    /// Build a MarketplaceApi (ClawHub + Skills.sh) from config.
    fn build_marketplace_api(&self) -> MarketplaceApi {
        let workspace = PathBuf::from(&self.config.kernel.workspace);
        let skills_dir = workspace.join("skills");
        let config = &self.config.marketplace;

        // ClawHub
        let clawhub_client = match ClawHubClient::new(config.base_url.clone()) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "Invalid marketplace.base_url, using default");
                ClawHubClient::new(Some("https://clawhub.ai".to_string()))
                    .expect("default client configuration is valid")
            }
        };
        let clawhub_installer = ClawHubInstaller::new(
            skills_dir.clone(),
            workspace.clone(),
            config.base_url.clone(),
        );

        // Skills.sh
        let ss_config = &config.skills_sh;
        let skills_sh_client =
            SkillsShClient::new(ss_config.base_url.clone(), ss_config.api_key.clone())
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "Failed to create Skills.sh client, using default");
                    SkillsShClient::new(None, None).expect("default client configuration is valid")
                });
        let skills_sh_installer = SkillsShInstaller::new(
            skills_dir,
            ss_config.base_url.clone(),
            ss_config.api_key.clone(),
        );

        MarketplaceApi::new(
            Arc::new(clawhub_installer),
            Arc::new(clawhub_client),
            Arc::new(skills_sh_installer),
            Arc::new(skills_sh_client),
        )
    }

    /// Build the calendar API facade (optional — only if `[calendar] enabled = true`).
    fn build_calendar_api(&self) -> Option<oxios_kernel::CalendarApi> {
        if !self.config.calendar.enabled {
            return None;
        }

        let workspace = PathBuf::from(&self.config.kernel.workspace);
        let calendar_dir = workspace.join("calendar").join("events");

        // CalendarEngine::new only creates the directory and loads index.json — sync-compatible.
        let engine = std::fs::create_dir_all(&calendar_dir)
            .map_err(|e| {
                tracing::warn!(error = %e, "Failed to create calendar directory");
                e
            })
            .ok()
            .and_then(|_| oxios_calendar::CalendarEngine::new_blocking(calendar_dir).ok());

        match engine {
            Some(engine) => {
                tracing::info!("Calendar system initialized");
                Some(oxios_kernel::CalendarApi::with_event_bus(
                    Arc::new(engine),
                    self.event_bus.clone(),
                ))
            }
            None => {
                tracing::warn!("Failed to initialize calendar system");
                None
            }
        }
    }

    /// Build the oximemo facade (optional first-party app module — `memo`
    /// feature + `[memo].enabled`). oxios opens the user's oximemo vault as a
    /// co-client; the vault's advisory locks keep concurrent app/oxios access
    /// safe. Only compiled when the binary's `memo` feature is on.
    #[cfg(feature = "memo")]
    fn build_memo_api(&self) -> Option<std::sync::Arc<oxios_kernel::MemoApi>> {
        if !self.config.memo.enabled {
            return None;
        }
        let vault_path = if self.config.memo.vault_path.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(&self.config.memo.vault_path))
        };
        match oxios_kernel::MemoApi::open(vault_path.as_deref(), Some(self.event_bus.clone())) {
            Ok(api) => {
                tracing::info!("oximemo module initialized (vault co-client)");
                Some(api)
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to open oximemo vault; memo module disabled");
                None
            }
        }
    }

    /// Build the oxiline facade (optional first-party app module — `timeline`
    /// feature + `[timeline].enabled`). oxios opens the user's oxiline SQLite
    /// store as a co-client (read-only context-in). Only compiled when the
    /// binary's `timeline` feature is on.
    #[cfg(feature = "timeline")]
    fn build_timeline_api(&self) -> Option<std::sync::Arc<oxios_kernel::TimelineApi>> {
        if !self.config.timeline.enabled {
            return None;
        }
        let db_path = if self.config.timeline.db_path.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(&self.config.timeline.db_path))
        };
        match oxios_kernel::TimelineApi::open(db_path.as_deref()) {
            Ok(api) => {
                tracing::info!("oxiline module initialized (timeline co-client, read-only)");
                Some(api)
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to open oxiline db; timeline module disabled");
                None
            }
        }
    }

    /// Build the email API facade (optional — only if `[email] enabled = true`).
    fn build_email_api(&self) -> Option<oxios_kernel::EmailApi> {
        if !self.config.email.enabled {
            return None;
        }

        if self.config.email.my_email.is_empty() {
            tracing::warn!("Email enabled but my_email not set — skipping");
            return None;
        }

        // Resolve SMTP password: env var → credential store
        let password: Option<String> = std::env::var("OXIOS_EMAIL_PASSWORD")
            .ok()
            .filter(|p| !p.is_empty())
            .or_else(|| std::env::var("RESEND_API_KEY").ok())
            .or_else(|| {
                // Try credential store
                oxicode_sdk::load_token(&self.config.email.secret_ref)
                    .ok()
                    .flatten()
                    .map(|t| t.access_token)
            });

        let password = match password {
            Some(p) => p,
            None => {
                tracing::warn!(
                    "Email enabled but no SMTP password found. Set OXIOS_EMAIL_PASSWORD env var or run 'oxios email setup'."
                );
                return None;
            }
        };

        match oxios_kernel::SmtpClient::from_config(&self.config.email, &password) {
            Ok(smtp) => {
                let workspace = PathBuf::from(&self.config.kernel.workspace);
                let template_dir = workspace.join("email_templates");
                let _ = std::fs::create_dir_all(&template_dir);

                tracing::info!(
                    from = %smtp.from_addr(),
                    "Email system initialized"
                );
                Some(oxios_kernel::EmailApi::new(
                    smtp,
                    template_dir,
                    self.state_store.clone(),
                    Some(self.event_bus.clone()),
                    self.config.email.rate_limit_per_hour,
                ))
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to initialize email system");
                None
            }
        }
    }

    /// Configuration reference.
    pub fn config(&self) -> &OxiosConfig {
        &self.config
    }

    /// Apply CLI overrides for the E2EE remote companion surface (RFC-044 §6.2).
    ///
    /// Wired into `cmd_serve` so `oxios serve --remote [--pairing-address host]`
    /// flips the in-memory kernel config without touching `config.toml` on
    /// disk. The daemonized path is unaffected — that one re-reads
    /// `config.toml` itself, so users must set `[remote]` there.
    ///
    /// `enabled = true` is required to start `RemoteRpcSurface`; the
    /// surface-name `"remote"` must also be present in `[surfaces].enabled`
    /// (defaulted to `[]`) for `activate_surfaces` to pick it up.
    #[cfg(feature = "remote")]
    pub fn apply_remote_overrides(&mut self, enabled: bool, pairing_address: Option<String>) {
        self.config.remote.enabled = enabled;
        if let Some(addr) = pairing_address {
            self.config.remote.pairing_address = Some(addr);
        }
        let surfaces = self.config.surfaces.get_or_insert_with(Default::default);
        if !surfaces.enabled.iter().any(|n| n == "remote") {
            surfaces.enabled.push("remote".to_string());
        }
    }
    /// Orchestrator reference — for hot-reload config propagation.
    #[allow(dead_code)]
    pub fn orchestrator(&self) -> &Arc<Orchestrator> {
        &self.orchestrator
    }

    /// Call during graceful shutdown to ensure no entries are lost.
    #[allow(dead_code)]
    pub fn flush_audit(&self) -> anyhow::Result<()> {
        self.audit_trail
            .flush_to(&*self.state_store)
            .map_err(|e| anyhow::anyhow!("audit flush failed: {e}"))
    }

    /// Shutdown kernel resources: agents, MCP, audit. All wrapped in
    /// a single timeout — on expiry, remaining work is abandoned and
    /// the process exits (the OS supervisor restarts to known-good).
    ///
    /// `flush_audit` is synchronous, so it runs on the blocking pool
    /// for the outer timeout to function (same rationale as Guardian's
    /// spawn_blocking — RFC-040 A2).
    pub async fn cleanup(&self, timeout: std::time::Duration) {
        // Promo-6: stop the Mount auto-promotion scanner first so it does
        // not race the state teardown below.
        self.shutdown_promotion_scanner();
        let handle = self.handle();

        let _ = tokio::time::timeout(timeout, async {
            // Phase 1: terminate running agents (parallel kill)
            if let Ok(agents) = handle.agents.list().await
                && !agents.is_empty()
            {
                tracing::info!(count = agents.len(), "Terminating agents...");
                let mut kill_futures = Vec::new();
                for agent in &agents {
                    let agent_id = agent.id.to_string();
                    let h = handle.clone();
                    kill_futures.push(tokio::spawn(async move {
                        if let Err(e) = h.agents.kill(&agent_id).await {
                            tracing::warn!(agent = %agent_id, error = %e, "Failed to kill agent");
                        }
                    }));
                }
                for f in kill_futures {
                    let _ = f.await;
                }
                tracing::info!(count = agents.len(), "Agents terminated");
            }

            // Phase 2: MCP shutdown (async, but may hang on unresponsive server)
            if let Err(e) = handle.mcp.shutdown_all().await {
                tracing::warn!(error = %e, "MCP shutdown error");
            }

            // Phase 3: audit flush — SYNC. Must offload to blocking pool
            // for the outer timeout to function.
            let kh = handle.clone();
            let _ = tokio::task::spawn_blocking(move || kh.flush_audit()).await;
        })
        .await;
    }

    /// RFC-025 Phase 5: signal the Mount auto-promotion scanner to stop
    /// (Promo-6). No-op when the scanner is disabled. Safe to call during
    /// graceful shutdown; the spawned task breaks its `select!` loop on the
    /// next iteration.
    pub fn shutdown_promotion_scanner(&self) {
        if let Some(tx) = &self.promo_shutdown_tx {
            let _ = tx.send(true);
        }
    }

    /// Execute a prompt with an optional session ID for multi-turn conversations.
    ///
    /// Pass `Some(session_id)` to continue an existing interview;
    /// pass `None` to start a new session.
    pub async fn execute_prompt_with_session(
        &self,
        prompt: &str,
        session_id: Option<&str>,
    ) -> Result<oxios_kernel::OrchestrationResult> {
        self.orchestrator
            .handle_unified(
                "cli",
                prompt,
                session_id,
                None,
                None,
                None,
                None,
                None, // model_params
                "cli-direct",
            )
            .await
    }

    /// Execute a prompt in chat mode (skips Ouroboros pipeline).
    /// **DEPRECATED (RFC-027):** use `execute_prompt_with_session` which
    /// routes through the unified `IntentEngine` path.
    #[allow(dead_code)]
    pub async fn execute_prompt_chat(
        &self,
        prompt: &str,
        session_id: Option<&str>,
    ) -> Result<oxios_kernel::OrchestrationResult> {
        self.orchestrator
            .handle_unified(
                "cli",
                prompt,
                session_id,
                None,
                None,
                None,
                None, // model_override
                None, // model_params
                "cli-direct",
            )
            .await
    }

    /// Register a channel with the gateway.
    pub async fn register_channel(
        &self,
        channel: Box<dyn oxios_gateway::Channel>,
    ) -> anyhow::Result<()> {
        self.gateway.register(channel).await
    }

    /// Run the gateway event loop (blocking).
    #[allow(dead_code)]
    pub async fn run_gateway(&self) -> Result<()> {
        self.gateway.run().await
    }

    /// Start the CronScheduler: restore persisted jobs, load config-defined
    /// jobs, then run the tick loop. Each fired job's goal is executed through
    /// the shared [`KernelHandle::run_goal`] primitive (direct orchestrator
    /// path — no gateway correlation, since this is a background task with no
    /// waiting HTTP client).
    ///
    /// Must be called after `handle()` is cached (so the orchestrator is
    /// attached) and after the intent engine is wired — both are true once
    /// `Kernel::build()` returns. The default 60 s tick also lands post-boot.
    ///
    /// Returns the loop's `JoinHandle` so the caller can track it for liveness,
    /// or `None` when cron is disabled in config (no task to track — a
    /// completed handle would otherwise look like a fatal critical-task exit).
    pub fn start_cron_loop(&self) -> Option<tokio::task::JoinHandle<()>> {
        if !self.config.cron.enabled {
            tracing::info!("Cron scheduler disabled in config; not starting");
            return None;
        }
        let scheduler = self.cron_scheduler.clone();
        let handle = self.handle();
        let cron_config = self.config.cron.clone();
        Some(tokio::spawn(async move {
            // Restore persisted jobs, then add config-defined ones (API jobs win).
            scheduler.restore_jobs().await;
            scheduler.load_from_config(&cron_config).await;

            let handle_for_exec = handle.clone();
            scheduler
                .start(move |job_id, goal| {
                    let handle = handle_for_exec.clone();
                    async move {
                        match handle.run_goal(&goal, None).await {
                            Ok(result) => {
                                // Success signal: no provider failure AND
                                // evaluation passed. evaluation_passed is None
                                // for goals with no acceptance criteria/review
                                // (always, for cron jobs), so the default must
                                // be true — a goal that ran without error is a
                                // success.
                                let success = result.failure_class.is_none()
                                    && result.evaluation_passed.unwrap_or(true);
                                let summary = result
                                    .output
                                    .clone()
                                    .unwrap_or_else(|| result.response.clone());
                                tracing::info!(
                                    %job_id,
                                    success,
                                    phase = %result.phase_reached,
                                    "Cron job executed"
                                );
                                (success, summary)
                            }
                            Err(e) => {
                                tracing::error!(%job_id, error = %e, "Cron job execution failed");
                                (false, format!("Error: {e}"))
                            }
                        }
                    }
                })
                .await;
        }))
    }

    // ── Initialization helpers (used by default mode only) ─────────────

    /// Initialize default skills from the share directory.
    pub async fn init_default_skills(&self, share_dir: &std::path::Path) -> Result<()> {
        let defaults_dir = share_dir.join("default-skills");
        self.skill_manager.init().await?;

        if defaults_dir.exists() {
            let count_before = self.skill_manager.list_skills().await.len();
            if let Err(e) = self.skill_manager.load_from_dir(&defaults_dir).await {
                tracing::warn!(
                    path = %defaults_dir.display(),
                    error = %e,
                    "Failed to load default skills directory"
                );
            } else {
                let count_after = self.skill_manager.list_skills().await.len();
                let installed = count_after.saturating_sub(count_before);
                if installed > 0 {
                    tracing::info!(count = installed, "Default skills installed");
                }
            }
        } else {
            tracing::debug!("No default skills directory found");
        }

        Ok(())
    }

    /// Initialize MCP servers from config.
    pub async fn init_mcp_servers(&self) -> Result<()> {
        if !self.config.mcp.servers.is_empty() {
            self.mcp_bridge.initialize_all().await?;
            tracing::info!(
                count = self.config.mcp.servers.len(),
                "MCP servers initialized"
            );
        }
        Ok(())
    }

    /// Start the guardian daemon (background integrity checks).
    ///
    /// Returns the Guardian task handle so the caller can register it
    /// with the TaskSupervisor as a safety net for clean exits.
    /// Hang detection is via the heartbeat watchdog (RFC-040 A3).
    ///
    /// `web_dist` is forwarded to the daily health check so auto-updates
    /// publish a new generation atomically (RFC-024 SP3).
    ///
    /// `heartbeat` is updated every cycle completion. The supervisor
    /// checks it every 60s; three missed cycles (900s) → abort.
    pub fn start_guardian(
        &self,
        web_dist: oxios_gateway::ActiveWebDist,
        heartbeat: Arc<AtomicU64>,
    ) -> (tokio::task::JoinHandle<()>, tokio::task::JoinHandle<()>) {
        let handle = self.handle();
        let guardian_task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;

                // All four Guardian ops (verify_chain, is_overloaded,
                // git_verify, commit_all) are synchronous. Calling them
                // on an async worker starves the runtime — and
                // tokio::time::timeout cannot preempt a blocking call.
                // spawn_blocking moves them to the blocking pool where
                // the timeout JoinHandle can actually be polled.
                let h = handle.clone();
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(180),
                    tokio::task::spawn_blocking(move || guardian_tick_sync(&h)),
                )
                .await;

                match result {
                    Ok(Ok(())) | Ok(Err(_)) => {
                        // Cycle completed (success or error from
                        // individual ops) — heartbeat alive.
                        heartbeat.store(now_secs_epoch(), Ordering::Relaxed);
                    }
                    Err(_) => {
                        // Tick hung past 180s. Do NOT update heartbeat.
                        // The detached spawn_blocking task continues on
                        // the blocking pool; the loop moves to next
                        // sleep. If hangs persist across cycles,
                        // heartbeat goes stale → watchdog aborts.
                        tracing::warn!(
                            "Guardian tick timed out after 180s — heartbeat not updated"
                        );
                    }
                }
            }
        });

        // Daily health check: web UI update, self-update check.
        // Returns the JoinHandle so callers can track it for clean shutdown
        // (audit F-14 — previously fire-and-forget).
        let health_task = self.start_daily_health_check(web_dist);

        (guardian_task, health_task)
    }

    /// Start the daily health check loop.
    ///
    /// **Eager startup check** (RFC-024 follow-up): runs `sync(Latest)` once
    /// immediately so a frequently-restarted host still gets web UI updates —
    /// the previous code slept to 03:00 before its first check, so a machine
    /// that never survived until 03:00 never updated. Throttled to once/hour
    /// via `~/.oxios/web/.last-check` so a crash loop can't hammer GitHub.
    ///
    /// **Recurring**: aligns to 03:00 local, then every 24h. Returns the
    /// `JoinHandle` so callers can track it for clean shutdown (audit F-14).
    fn start_daily_health_check(
        &self,
        web_dist: oxios_gateway::ActiveWebDist,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            // Eager startup check: catch up to the latest release immediately so
            // a host that never survives until 03:00 still gets web UI updates.
            // Throttled to once/hour via `~/.oxios/web/.last-check` so a crash
            // loop can't exhaust GitHub's unauth rate limit (60/hr). The
            // recurring 03:00 + 24h cadence below runs regardless of the throttle.
            if crate::web_dist::eager_check_allowed() {
                crate::web_dist::touch_last_check();
                if let Err(e) = daily_health_check(web_dist.clone()).await {
                    tracing::warn!(error = %e, "Startup web UI check failed");
                }
            }

            let now = chrono::Local::now();
            let mut next = now
                .date_naive()
                .and_hms_opt(3, 0, 0)
                .expect("valid time of day")
                .and_local_timezone(chrono::Local)
                .earliest()
                .unwrap_or(now + chrono::Duration::hours(24));
            if next <= now {
                next += chrono::Duration::days(1);
            }

            let delay_secs = (next - now).num_seconds().max(0) as u64;
            tracing::info!(
                next_check = %next.format("%Y-%m-%d %H:%M"),
                "Daily health check scheduled at 03:00"
            );

            tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;

            if let Err(e) = daily_health_check(web_dist.clone()).await {
                tracing::warn!(error = %e, "Daily health check failed");
            }

            let mut interval = tokio::time::interval(std::time::Duration::from_secs(86400));
            loop {
                interval.tick().await;
                if let Err(e) = daily_health_check(web_dist.clone()).await {
                    tracing::warn!(error = %e, "Daily health check failed");
                }
            }
        })
    }
}

/// Synchronous Guardian tick — runs on the blocking pool via spawn_blocking.
///
/// All four operations (verify_chain, is_overloaded, git_verify, commit_all)
/// are synchronous blocking calls. Extracted from the old inline loop body
/// so they can be offloaded from the async worker thread (RFC-040 A2).
fn guardian_tick_sync(handle: &oxios_kernel::KernelHandle) {
    use oxicode_sdk::AuditAction;

    if let Ok(valid) = handle.security.verify_chain()
        && !valid
    {
        handle.security.audit(
            "guardian",
            AuditAction::Other {
                detail: "AUDIT CHAIN BROKEN".into(),
            },
            "guardian",
        );
    }

    if handle.infra.is_overloaded() {
        let snap = handle.infra.resource_snapshot();
        handle.security.audit(
            "guardian",
            AuditAction::Other {
                detail: format!("OVERLOADED: cpu={:.1}%", snap.cpu_percent),
            },
            "guardian",
        );
    }

    if let Ok(valid) = handle.infra.git_verify()
        && !valid
    {
        handle.security.audit(
            "guardian",
            AuditAction::Other {
                detail: "GIT REPOSITORY CORRUPTED".into(),
            },
            "guardian",
        );
    }

    let _ = handle.commit_all("guardian: periodic checkpoint");
}

/// Current time as Unix epoch seconds.
fn now_secs_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Daily health check: sync the web UI to the latest GitHub release.
///
/// Delegates to [`crate::web_dist::sync`], which compares the active
/// dist's `version.json` against `releases/latest` and atomically
/// publishes a new generation when they differ (RFC-024 SP3). The
/// compare/download/publish logic lives in `web_dist.rs` and is shared
/// with the eager startup check and `oxios update --web-only`.
async fn daily_health_check(web_dist: oxios_gateway::ActiveWebDist) -> anyhow::Result<()> {
    use crate::web_dist::{SyncOutcome, SyncTarget};
    match crate::web_dist::sync(&web_dist, SyncTarget::Latest).await {
        SyncOutcome::UpToDate { active, target } => {
            tracing::debug!(
                current = %active,
                latest = %target,
                "Daily health check: web UI up to date"
            );
        }
        SyncOutcome::Updated { to } => {
            tracing::info!(version = %to, "Daily health check: web UI updated");
        }
        SyncOutcome::Unstamped => {
            tracing::debug!("Daily health check: active dist is unstamped, skipping download");
        }
        SyncOutcome::Failed { reason } => {
            anyhow::bail!(reason);
        }
    }
    Ok(())
}

/// Builder for assembling the Oxios kernel.
pub struct KernelBuilder {
    config_path: PathBuf,
}

impl KernelBuilder {
    /// Set the config file path.
    pub fn config_path(mut self, path: PathBuf) -> Self {
        self.config_path = path;
        self
    }

    /// Assemble all kernel components and wire them together.
    pub async fn build(self) -> Result<Kernel> {
        let config_path = self.config_path;

        let config = if config_path.exists() {
            tracing::info!(path = %config_path.display(), "Loading config");
            load_config(&config_path)?
        } else {
            tracing::info!("No config file found, using defaults");
            OxiosConfig::default()
        };

        // RFC-018: Apply consolidation preset if not "custom".
        // This overwrites individual consolidation fields with preset values.
        // NOTE: `config.memory.consolidation` is no longer read by the kernel
        // after the RFC-047 brain migration (memory owned by the daemon);
        // this preset application is intentionally removed with it.

        let event_bus = EventBus::new(config.kernel.event_bus_capacity);

        // RFC-015 P1: shared streaming-sink registry. The gateway registers
        // a strong sender per active chat session, the runtime callback
        // looks it up by session_id to push live text deltas. The SAME Arc
        // is attached to KernelHandle (for runtime lookup) and to the
        // Gateway (for registration).
        let streaming_sinks = Arc::new(oxios_kernel::streaming_sink::StreamingSinkRegistry::new());
        let state_store = Arc::new(oxios_kernel::state_store::StateStore::new(PathBuf::from(
            &config.kernel.workspace,
        ))?);

        // Model comes from config, not hardcoded default
        let model_id = &config.engine.default_model;

        if let Some(router_cfg) = &config.engine.router
            && let Err(err) = router_cfg.validate()
        {
            anyhow::bail!("Invalid [engine.router] configuration: {err}");
        }

        fn attach_hooks(
            engine_builder: oxios_kernel::OxiosEngineBuilder,
            hooks: &[oxios_kernel::HookSpec],
        ) -> oxios_kernel::OxiosEngineBuilder {
            if hooks.is_empty() {
                engine_builder
            } else {
                engine_builder.with_hook_specs(hooks.to_vec())
            }
        }

        let catalog = match OxiosEngine::init_file_catalog().await {
            Ok(c) => {
                tracing::info!("Model catalog initialized (dynamic models.dev data)");
                Some(c)
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Failed to initialize model catalog; resolving via static registry"
                );
                None
            }
        };
        let engine = if let Some(ref router_cfg) = config.engine.router {
            if router_cfg.enabled {
                let effective_model = format!("router/{}", router_cfg.default_profile);
                let mut engine_builder = OxiosEngine::builder()
                    .default_model(&effective_model)
                    .with_router(router_cfg.clone());
                if let Some(ref c) = catalog {
                    engine_builder = engine_builder.with_catalog(c.clone());
                }
                Arc::new(attach_hooks(engine_builder, &config.engine.hooks).build())
            } else {
                build_default_engine(&config, model_id, &catalog)
            }
        } else if config.engine.routing_enabled {
            // Legacy routing_enabled path (backward compat)
            let mut engine_builder = OxiosEngine::builder().default_model(model_id);
            if let Some(ref c) = catalog {
                engine_builder = engine_builder.with_catalog(c.clone());
            }
            let (engine, _routing_control) =
                attach_hooks(engine_builder, &config.engine.hooks).build_with_routing();
            Arc::new(engine)
        } else {
            build_default_engine(&config, model_id, &catalog)
        };

        fn build_default_engine(
            config: &OxiosConfig,
            model_id: &str,
            catalog: &Option<Arc<dyn ModelCatalog>>,
        ) -> Arc<OxiosEngine> {
            let primary_provider = model_id
                .split_once('/')
                .map(|(p, _)| p)
                .unwrap_or("anthropic");
            let mut engine_builder = oxios_kernel::OxiosEngine::builder().default_model(model_id);
            if let Some(key) = config.engine.api_key.as_deref() {
                engine_builder = engine_builder.api_key(primary_provider, key);
            }
            if let Some(c) = catalog {
                engine_builder = engine_builder.with_catalog(c.clone());
            }
            Arc::new(attach_hooks(engine_builder, &config.engine.hooks).build())
        }
        // Boot-time validation: resolve the engine's effective default model
        // so a broken config fails fast (daemon refuses to start). When the
        // router is enabled, the effective id is `router/<default_profile>`
        // and a synthetic model was registered for that profile by Task 2;
        // otherwise it matches `config.engine.default_model`. Using the raw
        // config field here would diverge from the router path (and fail on
        // an empty config default). `model.provider` is reused below to seed
        // the agent API key — for the router it yields "router" and the
        // CredentialStore gracefully falls back to `config.engine.api_key`.
        let effective_model_id = engine.default_model_id();
        let model = engine
            .resolve_model(effective_model_id)
            .context(format!("Failed to resolve model: {effective_model_id}"))?;

        // EngineHandle — hot-swappable engine reference. Created here so both
        // the OuroborosEngine (interview/crystallize/review) and the AgentRuntime
        // (execute) resolve the *live* default model through it — the single
        // source of truth that makes the phases agree and honors hot-swaps.
        let engine_handle = Arc::new(EngineHandle::new(engine));

        // Boot-time fail-fast for the provider too: this also warms the
        // EngineHandle provider cache.
        engine_handle
            .resolve_default()
            .context("Boot model/provider resolution failed")?;

        let resolver: Arc<dyn oxios_ouroboros::ModelResolver> = engine_handle.clone();
        let intent_engine: Arc<oxios_ouroboros::IntentEngine> =
            Arc::new(oxios_ouroboros::IntentEngine::with_lightweight(
                resolver.clone(),
                config.intent.lightweight_model.clone(),
            ));

        let mut access_manager = AccessManager::new();
        if let Some(ref audit_path) = config.security.audit_log_path {
            let expanded = oxios_kernel::config::expand_home(audit_path);
            access_manager = access_manager.with_audit_log_path(expanded.clone());
            tracing::info!(path = %expanded.display(), "Audit log file persistence enabled");
        }
        let access_manager = Arc::new(parking_lot::Mutex::new(access_manager));

        let persona_manager = Arc::new(PersonaManager::new().with_state_store(state_store.clone()));
        // RFC-039: 디스크에서 페르소나 로드 → config 적용 → 활성 결정 → intent 시드.
        // 손상은 silent fallback 하지 않고 tracing log 에 남김.
        if let Err(e) = persona_manager.load_from_state_store(&state_store).await {
            tracing::warn!(error = %e, "persona load from state store failed; using in-memory defaults");
        }
        persona_manager.apply_config(&config.persona);
        if let Some(p) = persona_manager.first_enabled() {
            intent_engine.set_persona_prompt(Some(p.system_prompt.clone()));
            tracing::info!(persona = %p.name, "Active persona set on engines");
        }

        // RFC-039: re-seed intent engine on every persona switch.
        // Set on the manager so all callers (HTTP, tool, gateway) re-seed.
        let ie_for_persona = intent_engine.clone();
        persona_manager.set_reseed_callback(Some(Arc::new(move |prompt| {
            ie_for_persona.set_persona_prompt(prompt);
        })));

        let a2a_protocol = Arc::new(A2AProtocol::new(event_bus.clone()));

        let git_layer = Arc::new(GitLayer::new(
            PathBuf::from(&config.kernel.workspace),
            config.git.auto_commit,
        )?);

        let skills_dir = PathBuf::from(&config.kernel.workspace).join("skills");
        let bundled_dir = PathBuf::from(&config.kernel.workspace).join("share/skills");
        let skill_manager = Arc::new(SkillManager::new(skills_dir, bundled_dir));

        let mcp_bridge = Arc::new(init_mcp_bridge(&config).await?);

        // ── Pre-create all kernel service objects ──
        // These are needed before KernelHandle creation (for AgentRuntime).
        // Order doesn't matter — they're independent.

        // Brain daemon connection (RFC-047) — degraded when unavailable.
        let brain = Arc::new(if config.brain.enabled {
            let socket_path = if config.brain.socket_path.is_empty() {
                let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
                home.join(".oxi").join("brain").join("oxibrain.sock")
            } else {
                oxios_kernel::config::expand_home(&config.brain.socket_path)
            };
            BrainConnection::connect(BrainConfig::new(socket_path, config.brain.space.clone()))
                .await
        } else {
            // Fully degraded: connect against a socket that will never exist.
            BrainConnection::connect(BrainConfig::new(
                PathBuf::from("/nonexistent/oxibrain.sock"),
                config.brain.space.clone(),
            ))
            .await
        });
        // Publish the initial daemon availability to the metrics gauge.
        oxios_kernel::metrics::get_metrics()
            .oxibrain_available
            .set(if brain.is_available() { 1.0 } else { 0.0 });

        // KernelDatabase — shared SQLite connection for mount/project tables.
        // Forward-only migration: the legacy `memory.db` is preserved untouched
        // (spec §9); the kernel's own tables move to `kernel.db`. One-time copy
        // of existing mount/project rows from the old shared `memory.db`.
        let kernel_db = Arc::new(KernelDatabase::open(
            PathBuf::from(&config.kernel.workspace).join("kernel.db"),
        )?);
        {
            let legacy = PathBuf::from(&config.kernel.workspace).join("memory.db");
            if let Err(e) = kernel_db.migrate_legacy_mount_project(&legacy) {
                tracing::warn!(
                    error = %e,
                    "mount/project migration from memory.db failed (non-fatal)"
                );
            }
        }

        // ProjectManager (RFC-011) + MountManager (RFC-025) share the db.
        let project_manager =
            match oxios_kernel::ProjectManager::new(kernel_db.clone(), Some(event_bus.clone())) {
                Ok(pm) => {
                    tracing::info!("ProjectManager initialized");
                    Some(Arc::new(pm))
                }
                Err(e) => {
                    tracing::warn!(error = %e, "ProjectManager init failed (non-fatal)");
                    None
                }
            };

        // MountManager (RFC-025) using the same SQLite database.
        let mount_manager =
            match oxios_kernel::MountManager::new(kernel_db.clone(), Some(event_bus.clone())) {
                Ok(mm) => {
                    // RFC-025: one-time migration — promote legacy
                    // Project paths into Mounts. Idempotent: Projects
                    // that already reference Mounts are skipped.
                    if let Some(ref pm) = project_manager {
                        migrate_projects_to_mounts(&mm, pm);
                    }
                    tracing::info!("MountManager initialized");
                    Some(Arc::new(mm))
                }
                Err(e) => {
                    tracing::warn!(error = %e, "MountManager init failed (non-fatal)");
                    None
                }
            };

        // ── RFC-025 Phase 5: Mount auto-promotion background scanner ──
        // Scans session history on a cadence and promotes paths that cross
        // the frequency threshold into Mounts. Cheap (one filesystem walk
        // per scan) and debounced by the threshold.
        //
        // Promo-6: a `watch` channel provides a cancellation point so a
        // future graceful shutdown can break the loop. The `Sender` is
        // stored on the returned `Kernel` to keep it alive.
        let promo_shutdown_tx = {
            let mounts_cfg = &config.mounts;
            if !mounts_cfg.auto_promote_enabled {
                None
            } else if let Some(ref mm) = mount_manager {
                let mm = mm.clone();
                let ss = state_store.clone();
                // Promo-11: respect the configured toggle instead of a
                // hardcoded `true`.
                let promo_config = oxios_kernel::PromotionConfig {
                    enabled: mounts_cfg.auto_promote_enabled,
                    threshold: mounts_cfg.auto_promote_threshold,
                    window_days: mounts_cfg.auto_promote_window_days,
                };
                let interval_secs = mounts_cfg.auto_promote_interval_secs;
                // Promo-6: cancellation channel.
                let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
                let window_days = mounts_cfg.auto_promote_window_days;
                tokio::spawn(async move {
                    let mut ticker =
                        tokio::time::interval(std::time::Duration::from_secs(interval_secs));
                    // The first tick completes immediately — run a scan right
                    // after startup, then wait the full interval thereafter.
                    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    loop {
                        // Promo-6: break out on shutdown.
                        tokio::select! {
                            _ = ticker.tick() => {}
                            res = shutdown_rx.changed() => {
                                if res.is_err() || *shutdown_rx.borrow() {
                                    tracing::info!(
                                        "Mount auto-promotion scanner shutting down"
                                    );
                                    break;
                                }
                            }
                        }

                        // Promo-1: only load sessions updated within the
                        // promotion window, bounding memory to the ones that
                        // can actually contribute a touch.
                        let cutoff = chrono::Utc::now() - chrono::Duration::days(window_days);
                        let sessions = match ss.load_sessions_for_promotion(cutoff).await {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::warn!(error = %e, "Mount promotion scan failed");
                                continue;
                            }
                        };

                        // Promo-5: `promote_frequent_paths` does blocking
                        // filesystem I/O (canonicalize + marker walks). Run it
                        // on a blocking thread so it never stalls the async
                        // runtime.
                        let mm = mm.clone();
                        let promo_config = promo_config.clone();
                        match tokio::task::spawn_blocking(move || {
                            mm.promote_frequent_paths(&sessions, &promo_config)
                        })
                        .await
                        {
                            Ok(created) if !created.is_empty() => {
                                tracing::info!(
                                    promoted = created.len(),
                                    "RFC-025: auto-promoted frequent paths to Mounts"
                                );
                            }
                            Ok(_) => {}
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    "Mount promotion scan task panicked"
                                );
                            }
                        }
                    }
                });
                tracing::info!("Mount auto-promotion scanner spawned");
                Some(shutdown_tx)
            } else {
                None
            }
        };

        let budget_manager = Arc::new(BudgetManager::new());
        // RFC-031 v2: the shared QuotaTracker. One live instance per
        // kernel. v2 derives eligibility from live quota snapshots;
        // the v1 `[[token-maxing.providers]]` opt-in block is now
        // optional and no longer required. We keep a forward-looking
        // warning so users with existing v1 configs know they can
        // remove the block safely.
        if !config.token_maxing.providers.is_empty() {
            tracing::warn!(
                count = config.token_maxing.providers.len(),
                "[[token-maxing.providers]] is no longer required by RFC-031 v2; \
                 token-maxing now derives eligibility from live quota API \
                 responses. The block was preserved for back-compat but can be \
                 removed."
            );
        }
        let quota_tracker = Arc::new(oxios_kernel::QuotaTracker::new(config.token_maxing.clone()));

        // RFC-031 Phase 2: recalibration tick. Where a provider exposes a
        // usage/balance endpoint, periodically snap the self-tracked counter
        // to real state, erasing drift from a key shared with another app.
        {
            let interval = config.token_maxing.recalibration_interval_secs;
            let api_key = config.engine.api_key.clone();
            if interval > 0 {
                tokio::spawn(recalibration_tick(
                    Arc::clone(&quota_tracker),
                    interval,
                    api_key,
                ));
            }
        }

        let auth_manager = AuthManager::new();
        // API key auth is now via engine.api_key or ~/.oxicode/auth.json
        // No more security.api_keys_path
        let auth_manager = Arc::new(parking_lot::Mutex::new(auth_manager));

        let audit_trail = Arc::new(AuditTrail::new(config.audit.max_entries));

        let mut cron_scheduler =
            CronScheduler::new(state_store.clone(), config.cron.tick_interval_secs);
        cron_scheduler.set_git_layer(git_layer.clone());
        let cron_scheduler = Arc::new(cron_scheduler);

        let resource_monitor = Arc::new(ResourceMonitor::new(
            config.resource_monitor.interval_secs,
            config.resource_monitor.history_max,
        ));

        oxios_kernel::event_bus::attach_audit_trail(&event_bus, audit_trail.clone());

        // Restore persisted audit entries.
        if let Ok(entries) = state_store.load()
            && !entries.is_empty()
        {
            tracing::info!(count = entries.len(), "Restored audit trail entries");
            audit_trail.restore_from(entries);
        }

        // Routing stats — shared between EngineApi and AgentRuntime
        let routing_stats = Arc::new(oxios_kernel::RoutingStats::new());

        // EngineHandle was created earlier (before OuroborosEngine) so both the
        // Ouroboros phases and AgentRuntime resolve the live default model
        // through the same handle. EngineApi writes (set_model / set_api_key)
        // rebuild and swap it; AgentRuntime reads the latest on each execute().
        // ── Gateway APIs — Arc-wrapped for sharing with Gateway and KernelHandle ──
        let engine_api = Arc::new(oxios_kernel::EngineApi::new(
            Arc::new(parking_lot::RwLock::new(config.clone())),
            config_path.clone(),
            Arc::clone(&routing_stats),
            Arc::clone(&engine_handle),
        ));
        let persona_api = Arc::new(oxios_kernel::PersonaApi::new(Arc::clone(&persona_manager)));

        // Shared KnowledgeBase — single source of truth (RFC-003)
        let knowledge_base = Arc::new(
            KnowledgeBase::new(PathBuf::from(&config.kernel.workspace).join("knowledge"))
                .expect("KnowledgeBase init failed"),
        );

        // Shared HitL registries — created ONCE so the preliminary handle
        // (used by AgentRuntime to register tool approvals) and the cached
        // handle (used by the HTTP API to resolve them) see the same map.
        // Without this, exec_tool registers in one PendingToolApprovals
        // instance while /api/chat/tool-approval/{id}/respond resolves from
        // another → 404 on every click, no matter how fast.
        let pending_tool_approvals = Arc::new(oxios_kernel::tools::PendingToolApprovals::new());
        let pending_ask_user = Arc::new(oxios_kernel::tools::PendingAskUser::new());
        let approval_config = Arc::new(parking_lot::RwLock::new(config.security.approval.clone()));
        let pending_path_access = Arc::new(oxios_kernel::tools::PendingPathAccess::new());

        // Build AgentApi (placeholder supervisor — the real one needs
        // AgentRuntime which needs this handle. AgentApi.supervisor is only
        // used for list/kill, not during tool registration.)
        let agent_api = oxios_kernel::AgentApi::new(
            Arc::new(oxios_kernel::supervisor::NoOpSupervisor),
            budget_manager.clone(),
        );

        // ── KernelHandle — the syscall table for agent OS control ──
        // Created inline here because AgentRuntime needs it.
        // Will be cached again in the Kernel instance.
        let kernel_handle: Arc<oxios_kernel::KernelHandle> = {
            let kh = oxios_kernel::KernelHandle::new(
                oxios_kernel::StateApi::new(state_store.clone()),
                agent_api,
                oxios_kernel::SecurityApi::new(
                    auth_manager.clone(),
                    audit_trail.clone(),
                    access_manager.clone(),
                    state_store.clone(),
                ),
                oxios_kernel::PersonaApi::new(Arc::clone(&persona_manager)),
                oxios_kernel::ExtensionApi::new(Arc::clone(&skill_manager)),
                oxios_kernel::McpApi::new(mcp_bridge.clone()),
                oxios_kernel::InfraApi::new(
                    git_layer.clone(),
                    cron_scheduler.clone(),
                    resource_monitor.clone(),
                    event_bus.clone(),
                    config.clone(),
                    std::time::Instant::now(),
                    pending_tool_approvals.clone(),
                    pending_ask_user.clone(),
                    approval_config.clone(),
                    pending_path_access.clone(),
                ),
                project_manager.clone().map(oxios_kernel::ProjectApi::new),
                oxios_kernel::ExecApi::new(
                    Arc::new(parking_lot::RwLock::new(config.exec.clone())),
                    access_manager.clone(),
                ),
                oxios_kernel::A2aApi::new(a2a_protocol.clone()),
                // EngineApi — routing stats shared between EngineApi and AgentRuntime + engine hot-swap
                oxios_kernel::EngineApi::new(
                    Arc::new(parking_lot::RwLock::new(config.clone())),
                    config_path.clone(),
                    Arc::clone(&routing_stats),
                    Arc::clone(&engine_handle),
                ),
                // KnowledgeBase — single source of truth (RFC-003), shared
                knowledge_base.clone(),
                // KnowledgeLens — semantic overlay, shares same KnowledgeBase
                Arc::new(
                    oxios_kernel::KnowledgeLens::new(knowledge_base.clone(), Some(brain.clone()))
                        .expect("KnowledgeLens init failed"),
                ),
                build_marketplace_api_value(&config),
                None,                                     // calendar (initialized later)
                Arc::new(parking_lot::RwLock::new(None)), // email (initialized later)
            );

            // RFC-015 P1: attach the streaming-sink registry so the runtime
            // callback's per-session `TextChunk` lookup finds the gateway's
            // collector sender. Wired before `Arc::new(kh)` so we can use
            // the consuming builder.
            let kh = kh.with_streaming_sinks(streaming_sinks.clone());
            // Attach the Mount facade (RFC-025). Set before Arc so the handle
            // carries it from construction.
            let kh = if let Some(mm) = mount_manager.clone() {
                kh.with_mounts(oxios_kernel::MountApi::new(mm))
            } else {
                kh
            };
            let kh = kh.with_browser(oxios_kernel::BrowserApi::from_config(&config));
            let compression_svc = Arc::new(oxios_kernel::CompressionService::new(
                state_store.clone(),
                engine_handle.clone(),
                config.clone(),
                event_bus.clone(),
            ));
            let kh = kh.with_compression(oxios_kernel::CompressionApi::new(compression_svc));

            // Unified asset store (~/.oxios/assets/).
            let assets_root = oxios_kernel::config::expand_home("~/.oxios/assets");
            let kh = match oxios_kernel::AssetStore::new(assets_root) {
                Ok(store) => {
                    if let Err(e) = store.reconcile() {
                        tracing::warn!(error = %e, "Asset store reconcile failed");
                    }
                    kh.with_asset_store(Arc::new(store))
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to initialize asset store");
                    kh
                }
            };
            Arc::new(kh)
        };

        // Knowledge dream (RFC-022)
        if config.memory.knowledge_dream.enabled {
            let kb = kernel_handle.knowledge.clone();
            let kd_config = config.memory.knowledge_dream.clone();
            match oxios_kernel::knowledge_dream::KnowledgeDream::new(
                kb,
                git_layer.clone(),
                engine_handle.clone(),
                kd_config,
            ) {
                Ok(kd) => {
                    Arc::new(kd).spawn();
                    tracing::info!("Knowledge dream spawned for background note curation");
                }
                Err(e) => {
                    // Non-fatal: the dream is a background feature. A bad
                    // curation model disables it with a clear log rather than
                    // crashing the daemon or silently failing every cycle.
                    tracing::error!(
                        error = %e,
                        "Knowledge dream disabled — invalid model config, skipping background curation"
                    );
                }
            }
        }

        // Build ToolRetriever for semantic capability discovery.
        let tool_retriever = build_tool_retriever(&skill_manager).await;

        let agent_runtime = AgentRuntime::new(
            Arc::clone(&engine_handle),
            kernel_handle.clone(),
            Some(Arc::clone(&routing_stats)),
        )
        .with_persona_manager(Arc::clone(&persona_manager))
        .with_tool_retriever(Arc::new(tool_retriever))
        .with_sona(Arc::new(oxios_kernel::SonaEngine::new(
            oxios_kernel::SonaMode::Balanced,
            Arc::new(oxios_kernel::TfIdfEmbeddingProvider),
        )))
        .with_config({
            // Resolve API key from CredentialStore based on the model's provider.
            let provider_name = model.provider.as_str();
            let config_api_key = config.engine.api_key.as_deref();
            let api_key = oxios_kernel::CredentialStore::resolve(provider_name, config_api_key)
                .map(|(key, _)| key);

            oxios_kernel::agent_runtime::AgentRuntimeConfig {
                api_key,
                provider_options: config.engine.provider_options.clone(),
                ..Default::default()
            }
        })
        .with_persistence_hook(Arc::new(oxios_kernel::PersistenceHook::new(
            Some(brain.clone()),
            knowledge_base.clone(),
            Arc::clone(&engine_handle),
            state_store.clone(),
            event_bus.clone(),
        )));

        let mut basic_supervisor = BasicSupervisor::new(event_bus.clone(), agent_runtime);

        // Wire agent history persistence
        basic_supervisor.set_state_store(state_store.clone());
        basic_supervisor.set_agent_log_config(config.agent_log.clone());

        // Wire SQLite agent log index if available
        let (agent_log_db,) = {
            let db_path = if config.agent_log.db_path.is_empty() {
                dirs::home_dir()
                    .unwrap_or_default()
                    .join(".oxios/state/agent_log.db")
            } else {
                let p = std::path::PathBuf::from(&config.agent_log.db_path);
                if p.is_absolute() {
                    p
                } else {
                    dirs::home_dir().unwrap_or_default().join(".oxios").join(&p)
                }
            };

            // Ensure parent dir exists
            if let Some(parent) = db_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            match oxios_kernel::agent_log_db::AgentLogDb::open(&db_path) {
                Ok(db) => {
                    let db = Arc::new(db);
                    basic_supervisor.set_agent_log_db(db.clone());
                    tracing::info!(path = %db_path.display(), "Agent history SQLite log initialized");
                    (Some(db),)
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        path = %db_path.display(),
                        "Failed to open agent history SQLite DB, falling back to filesystem-only"
                    );
                    (None,)
                }
            }
        };

        let supervisor: Arc<dyn Supervisor> = Arc::new(basic_supervisor);

        let lifecycle = oxios_kernel::AgentLifecycleManager::new(
            supervisor.clone(),
            access_manager.clone(),
            a2a_protocol.clone(),
            event_bus.clone(),
            config.security.max_execution_time_secs,
            config.security.allowed_tools.clone(),
            config.security.network_access,
            config.kernel.workspace.clone(),
        );

        // Register the A2A dispatch handler.
        // When a TaskDelegation arrives, the handler spawns an agent via
        // the lifecycle manager and returns the execution result.
        let dispatch_lifecycle = lifecycle.clone();
        a2a_protocol
            .set_delegation_handler(Arc::new(move |_from, _to, task| {
                let lc = dispatch_lifecycle.clone();
                Box::pin(async move {
                    let directive = oxios_ouroboros::Directive {
                        goal: task.description.clone(),
                        ..Default::default()
                    };
                    let env = oxios_ouroboros::ExecEnv::default();
                    match lc.execute_directive(&directive, &env).await {
                        Ok(result) => Ok(serde_json::json!({
                            "output": result.output,
                            "success": result.success,
                            "steps": result.steps_completed,
                        })),
                        Err(e) => Ok(serde_json::json!({
                            "error": e.to_string(),
                            "success": false,
                        })),
                    }
                })
            }))
            .await;

        // RFC-031 Phase 3: the TokenMaxer orchestrator. Clone the lifecycle
        // manager (it impls Clone) before the orchestrator consumes it; the
        // maxer drains eligible subscription providers over a window.
        let maxer_lifecycle = lifecycle.clone();
        let planner = oxios_kernel::WorkPlanner::new(
            Arc::clone(&skill_manager),
            project_manager.clone(),
            mount_manager.clone(),
        );
        let token_maxer = Arc::new(oxios_kernel::TokenMaxer::new(
            maxer_lifecycle,
            Arc::clone(&quota_tracker),
            planner,
            state_store.clone(),
        ));

        let mut orchestrator = Orchestrator::with_config(
            event_bus.clone(),
            state_store.clone(),
            lifecycle,
            config.orchestrator.clone(),
        );
        orchestrator.set_intent_engine(intent_engine.clone());
        orchestrator.set_intent_config(config.intent.clone());
        orchestrator.set_git_layer(git_layer.clone());
        orchestrator.set_a2a(a2a_protocol.clone());
        if let Some(pm) = project_manager.clone() {
            orchestrator.set_project_manager(pm);
        }
        if let Some(mm) = mount_manager.clone() {
            orchestrator.set_mount_manager(mm);
        }
        // RFC-029: wire the recovery coordinator (L1 backoff / L2 model
        // swap). Shares RoutingStats with EngineApi/AgentRuntime so
        // fallback events surface in the Web UI. Reads the configured
        // fallback-model list (live-updatable via set_routing).
        {
            let coordinator = Arc::new(oxios_kernel::resilience::RecoveryCoordinator::new(
                Arc::clone(&routing_stats),
                oxios_kernel::resilience::ResilienceConfig::default(),
            ));
            coordinator.set_fallback_models(config.engine.fallback_models.clone());
            orchestrator.set_recovery(coordinator);
        }

        let orchestrator = Arc::new(orchestrator);

        // RFC-015 P1: attach the streaming-sink registry shared with the
        // KernelHandle so the runtime callback can find the gateway's
        // collector sender for live text deltas.
        let gateway = Gateway::with_apis(orchestrator.clone(), engine_api, persona_api)
            .with_streaming_sinks(streaming_sinks);
        let gateway = Arc::new(gateway);

        // Initialize metrics and observability singletons.
        oxios_kernel::register_builtin_metrics();
        oxios_kernel::observability::init();

        let kernel = Kernel {
            orchestrator,
            gateway,
            event_bus: event_bus.clone(),
            state_store: state_store.clone(),
            config,
            skill_manager,
            supervisor,
            access_manager,
            persona_manager,
            mcp_bridge,
            brain,
            auth_manager,
            cron_scheduler,
            git_layer,
            audit_trail,
            budget_manager,
            quota_tracker,
            token_maxer,
            resource_monitor,
            pending_tool_approvals,
            pending_ask_user,
            approval_config,
            pending_path_access,
            project_manager,
            mount_manager,
            start_time: std::time::Instant::now(),
            config_path,
            // Do NOT pre-seed with the cycle-breaking preliminary handle
            // (`kernel_handle`, built above solely to construct AgentRuntime):
            // its AgentApi is intentionally incomplete (NoOpSupervisor, no
            // agent_log_db / state_store). Caching it made every control-plane
            // agent query silently return empty even though the real
            // supervisor kept persisting rows. The fully-wired handle is
            // assembled lazily by `handle()` and cached just below.
            handle_cache: std::sync::OnceLock::new(),
            a2a_protocol,
            engine_handle,
            agent_log_db,
            promo_shutdown_tx,
        };

        // Eagerly assemble the fully-wired KernelHandle (real supervisor +
        // SQLite agent log + state store) and cache it, so the control plane
        // — HTTP API, CLI — never observes the incomplete preliminary handle.
        // Runs `handle()`'s `get_or_init` exactly once.
        let handle = kernel.handle();

        // RFC-024 SP4: mark state store ready and start the 30 s readiness
        // deadline on the *cached* handle's gate. The engine state is
        // finalized by the caller (main.rs cmd_serve) once it knows whether
        // the configured model has an API key — at which point the gate is
        // set to `Ready` or `Degraded`. The deadline forcibly promotes any
        // still-Warming subsystem to `Degraded` so a missing API key cannot
        // lock the gate forever.
        handle.readiness.set_state_store(SubsystemState::Ready);
        let deadline_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() + 30)
            .unwrap_or(0);
        handle.readiness.set_deadline_secs(deadline_secs);

        Ok(kernel)
    }
}

/// RFC-031 v2: background recalibration loop. Periodically fetches
/// real provider quota and caches the live snapshot in
/// [`QuotaTracker`]. v2 departure from v1: probes **every provider
/// for which a [`QuotaFetcher`] is registered** (via
/// [`crate::api::quota::all_fetchers`]) AND has credentials
/// configured. This is what makes the user's bug fix work: zai is
/// registered in the engine (so the credential store has a key)
/// but missing from `[[token-maxing.providers]]`. The v1 tick
/// filtered on config eligibility and never fetched zai.
async fn recalibration_tick(
    tracker: Arc<oxios_kernel::QuotaTracker>,
    interval_secs: u64,
    api_key: Option<String>,
) {
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
    // The first tick fires immediately on construction — skip it so we don't
    // fan out HTTP on boot, then settle into the configured cadence.
    ticker.tick().await;
    loop {
        ticker.tick().await;
        let cfg = tracker.config();
        if !cfg.enabled {
            continue;
        }
        drop(cfg);

        // v2: probe every registered fetcher. `all_fetchers()` is the
        // canonical list of providers with a known quota endpoint
        // (zai, openai, minimax today).
        let fetchers = crate::api::quota::all_fetchers();
        let mut creds: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for f in &fetchers {
            let provider = f.provider();
            if let Some((key, _)) =
                oxios_kernel::CredentialStore::resolve(provider, api_key.as_deref())
            {
                creds.insert(provider.to_string(), key);
            }
        }
        if creds.is_empty() {
            continue;
        }
        let snaps = crate::api::quota::fetch_all(&creds).await;
        for snap in &snaps {
            // v2: also cache the live snapshot so QuotaTracker::availability
            // can use it for auto-discovery.
            let live = crate::api::quota::to_live_snapshot(snap);
            tracker.update_live_snapshot(live);
            if snap.error.is_some() {
                tracker.apply_recalibration(
                    &snap.provider,
                    None,
                    None,
                    None,
                    oxios_kernel::RecalibrationOutcome::FetchFailed,
                );
                continue;
            }
            let rw = snap
                .rate_windows
                .iter()
                .find(|w| w.remaining_percent.is_some());
            let (rem, resets) = match rw {
                Some(w) => (w.remaining_percent, w.resets_at),
                None => (None, None),
            };
            tracker.apply_recalibration(
                &snap.provider,
                rem,
                resets,
                snap.token_limit
                    .and_then(|l| if l > 0.0 { Some(l as u64) } else { None }),
                oxios_kernel::RecalibrationOutcome::Ok,
            );
        }
    }
}

/// Initialize the MCP bridge from config and environment variables.
async fn init_mcp_bridge(config: &OxiosConfig) -> Result<McpBridge> {
    let bridge = McpBridge::new();

    for (name, def) in &config.mcp.servers {
        // SECURITY (audit F-1): reject dangerous commands at registration
        // time so a bad config surfaces at boot, not at first spawn. The
        // spawn chokepoint (McpClient::initialize) re-checks regardless.
        if let Err(reason) = validate_mcp_command(&def.command) {
            tracing::warn!(server = %name, command = %def.command, %reason,
                "Skipping MCP server from config (unsafe command)");
            continue;
        }
        let mut server = McpServer::new(name, &def.command);
        server.args = def.args.clone();
        server.env = def.env.clone();
        server.enabled = def.enabled;
        bridge.register_server(server);
        tracing::debug!(server = %name, command = %def.command, "Registered MCP server from config");
    }

    for (key, value) in std::env::vars() {
        if let Some(name) = key.strip_prefix("OXIOS_MCP_") {
            let name = name.trim_end_matches("_COMMAND");
            if name.is_empty() || config.mcp.servers.contains_key(name) {
                continue;
            }
            // SECURITY (audit F-1): env-injected commands get the same
            // blocklist check as config commands.
            if let Err(reason) = validate_mcp_command(&value) {
                tracing::warn!(server = %name, command = %value, %reason,
                    "Skipping MCP server from environment (unsafe command)");
                continue;
            }
            let mut server = McpServer::new(name, &value);
            if let Ok(args_str) = std::env::var(format!("OXIOS_MCP_{name}_ARGS")) {
                server.args = args_str.split_whitespace().map(String::from).collect();
            }
            if let Ok(env_str) = std::env::var(format!("OXIOS_MCP_{name}_ENV")) {
                for pair in env_str.split(',') {
                    if let Some((k, v)) = pair.split_once('=') {
                        server
                            .env
                            .insert(k.trim().to_string(), v.trim().to_string());
                    }
                }
            }
            bridge.register_server(server);
            tracing::debug!(server = %name, "Registered MCP server from environment");
        }
    }

    Ok(bridge)
}

/// Build a ToolRetriever with all OS tools and installed skills indexed.
async fn build_tool_retriever(sm: &SkillManager) -> oxios_kernel::tools::retrieval::ToolRetriever {
    use oxios_kernel::embedding::TfIdfEmbeddingProvider;
    use oxios_kernel::tools::retrieval::ToolEntry;

    let embedder = Arc::new(TfIdfEmbeddingProvider);
    let mut retriever = oxios_kernel::tools::retrieval::ToolRetriever::new(embedder);

    // Index built-in OS tools.
    let builtin_tools = vec![
        (
            "exec",
            "os-tool",
            "Execute shell commands or structured binaries in workspace",
        ),
        ("read", "os-tool", "Read file contents"),
        ("write", "os-tool", "Write content to files"),
        ("edit", "os-tool", "Make precise text edits in files"),
        ("grep", "os-tool", "Search file contents with regex"),
        ("find", "os-tool", "Find files by name or pattern"),
        ("ls", "os-tool", "List directory contents"),
        ("web_search", "os-tool", "Search the web for information"),
        ("memory_read", "os-tool", "Recall persistent memories"),
        ("memory_write", "os-tool", "Store persistent memories"),
        ("memory_search", "os-tool", "Semantic search over memories"),
        (
            "knowledge",
            "os-service",
            "Personal markdown vault — save, read, search documents and notes",
        ),
        (
            "browse",
            "os-tool",
            "Render web pages and read content (markdown/html) — use after web_search for page bodies",
        ),
    ];

    for (name, category, desc) in builtin_tools {
        retriever
            .index_tool(ToolEntry {
                name: name.to_string(),
                category: category.to_string(),
                description: desc.to_string(),
                skill_path: None,
                command: None,
            })
            .await;
    }

    // Index installed skills.
    let skills = sm.list_skills().await;
    for entry in &skills {
        let desc = entry.skill.description.clone();
        retriever
            .index_tool(ToolEntry {
                name: format!("skill:{}", entry.skill.name),
                category: "skill".to_string(),
                description: desc,
                skill_path: Some(format!("skills/{}/SKILL.md", entry.skill.name)),
                command: None,
            })
            .await;
    }

    tracing::info!(count = retriever.len(), "ToolRetriever indexed");
    retriever
}

/// Build a MarketplaceApi from the Kernel instance (used after Kernel construction).
fn build_marketplace_api_value(config: &OxiosConfig) -> MarketplaceApi {
    let workspace = PathBuf::from(&config.kernel.workspace);
    let skills_dir = workspace.join("skills");

    // ClawHub
    let clawhub_client =
        ClawHubClient::new(config.marketplace.base_url.clone()).unwrap_or_else(|_| {
            tracing::warn!("Invalid marketplace.base_url, using default");
            ClawHubClient::new(Some("https://clawhub.ai".to_string()))
                .expect("default client configuration is valid")
        });
    let clawhub_installer = ClawHubInstaller::new(
        skills_dir.clone(),
        workspace.clone(),
        config.marketplace.base_url.clone(),
    );

    // Skills.sh
    let ss = &config.marketplace.skills_sh;
    let skills_sh_client = SkillsShClient::new(ss.base_url.clone(), ss.api_key.clone())
        .expect("valid skills.sh client configuration");
    let skills_sh_installer =
        SkillsShInstaller::new(skills_dir, ss.base_url.clone(), ss.api_key.clone());

    MarketplaceApi::new(
        Arc::new(clawhub_installer),
        Arc::new(clawhub_client),
        Arc::new(skills_sh_installer),
        Arc::new(skills_sh_client),
    )
}

/// RFC-025 one-time migration: promote legacy Project paths into Mounts.
///
/// For each Project that has `paths` but no `mount_ids`, resolve a Mount for
/// every legacy path — reusing an existing Mount that already covers the path
/// (path-prefix match) when one exists, otherwise creating one named after the
/// Project — link them via `mount_ids`, then clear the legacy `paths` field.
///
/// Idempotent: Projects already referencing Mounts are skipped, and the
/// path-coverage check prevents duplicate Mounts for paths a user registered
/// under a different name.
fn migrate_projects_to_mounts(
    mount_manager: &oxios_kernel::MountManager,
    project_manager: &ProjectManager,
) {
    let projects = project_manager.list_projects();
    let mut migrated = 0usize;

    for project in projects {
        // Skip Projects that already reference Mounts (idempotent).
        if !project.mount_ids.is_empty() {
            continue;
        }
        // Skip Projects without paths — nothing to lift into a Mount.
        if project.paths.is_empty() {
            continue;
        }

        // Partition legacy paths: reuse any existing Mount that already covers
        // a path (path-prefix match), collecting only the uncovered ones for a
        // new Mount. This avoids duplicating a Mount the user registered for
        // the same path under a different name.
        let mut mount_ids: Vec<oxios_kernel::MountId> = Vec::new();
        let mut uncovered: Vec<PathBuf> = Vec::new();
        for path in &project.paths {
            match mount_manager.covering_mount_id(path) {
                Some(mid) => {
                    if !mount_ids.contains(&mid) {
                        mount_ids.push(mid);
                    }
                }
                None => uncovered.push(path.clone()),
            }
        }

        // Create one Mount for any uncovered paths, named after the Project
        // (suffixed to avoid colliding with a manually-created Mount).
        if !uncovered.is_empty() {
            let name = unique_mount_name(mount_manager, &project.name);
            match mount_manager.create_mount(
                name,
                uncovered,
                oxios_kernel::MountSource::AutoDetected,
            ) {
                Ok(mount) => mount_ids.push(mount.id),
                Err(e) => {
                    tracing::warn!(
                        project = %project.name,
                        error = %e,
                        "failed to create Mount during migration; leaving Project paths in place"
                    );
                    continue;
                }
            }
        }

        // Link the resolved Mounts and clear the legacy `paths` field so the
        // runtime legacy fallbacks never re-activate for this Project.
        if let Err(e) = project_manager.update_project_bundle(project.id, Some(mount_ids), None) {
            tracing::warn!(
                project = %project.name,
                error = %e,
                "link failed; orphan Mounts may remain"
            );
            continue;
        }
        if let Err(e) = project_manager.clear_legacy_paths(project.id) {
            tracing::warn!(
                project = %project.name,
                error = %e,
                "Mounts linked but failed to clear legacy paths"
            );
        }
        migrated += 1;
    }

    if migrated > 0 {
        tracing::info!(
            migrated = migrated,
            "RFC-025: migrated legacy Project paths into Mounts"
        );
    }
}

/// Pick a Mount name based on `base` that is not already taken, suffixing
/// `-2`, `-3`, … as needed so the migration never fails on a name collision.
fn unique_mount_name(mount_manager: &oxios_kernel::MountManager, base: &str) -> String {
    if mount_manager.get_mount_by_name(base).is_none() {
        return base.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if mount_manager.get_mount_by_name(&candidate).is_none() {
            return candidate;
        }
        n += 1;
    }
}
