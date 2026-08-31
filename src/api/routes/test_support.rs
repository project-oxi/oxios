//! Test-only fixture shared by the route contract tests.
//!
//! Builds a real `AppState` over temp directories: an in-memory project
//! database, a real state store for sessions, and an empty access manager.
//! The web surface (router + middleware) is intentionally NOT built here —
//! route tests construct the sub-router they exercise.

#![allow(clippy::unwrap_used)] // test fixture: setup `.unwrap()`s are idiomatic

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;

use crate::api::bridge::{WebBridge, WebBridgeHandle};
use crate::api::middleware::RateLimiter;
use crate::api::server::AppState;
use oxicode_sdk::AuditTrail;
use oxios_gateway::{ActiveWebDist, Gateway, ReliabilityLayer};
use oxios_kernel::a2a::A2AProtocol;
use oxios_kernel::access_manager::AccessManager;
use oxios_kernel::agent_lifecycle::AgentLifecycleManager;
use oxios_kernel::approval::ApprovalConfig;
use oxios_kernel::auth::AuthManager;
use oxios_kernel::automation::AutomationStore;
use oxios_kernel::budget::BudgetManager;
use oxios_kernel::event_bus::EventBus;
use oxios_kernel::kernel_db::KernelDatabase;
use oxios_kernel::kernel_handle::{
    A2aApi, AgentApi, EngineApi, ExecApi, ExtensionApi, InfraApi, KnowledgeLens, MarketplaceApi,
    McpApi, PersonaApi, ProjectApi, RoutingStats, SecurityApi, StateApi,
};
use oxios_kernel::mcp::McpBridge;
use oxios_kernel::orchestrator::Orchestrator;
use oxios_kernel::persona::PersonaManager;
use oxios_kernel::project::ProjectManager;
use oxios_kernel::resource_monitor::ResourceMonitor;
use oxios_kernel::skill::SkillManager;
use oxios_kernel::state_store::{Session, StateStore};
use oxios_kernel::supervisor::NoOpSupervisor;
use oxios_kernel::tools::{PendingAskUser, PendingPathAccess, PendingToolApprovals};
use oxios_kernel::{ClawHubClient, ClawHubInstaller, OxiosConfig, OxiosEngine, ReadinessGate};
use oxios_kernel::{ExecConfig, GitLayer, SkillsShClient, SkillsShInstaller};

/// Loopback client address for handler-level tests (extractor value).
pub(crate) fn loopback_peer() -> SocketAddr {
    "127.0.0.1:51000".parse().unwrap()
}

/// Build a kernel handle over temp dirs with a real `ProjectManager`
/// backed by an in-memory database. Returns the handle, the project
/// manager, the shared state store, and the config path.
fn kernel_handle(
    tmp: &Path,
) -> (
    oxios_kernel::KernelHandle,
    Arc<ProjectManager>,
    Arc<StateStore>,
    PathBuf,
) {
    let event_bus = EventBus::new(64);
    let state_store = Arc::new(StateStore::new(tmp.join("state")).unwrap());
    let access_manager = Arc::new(parking_lot::Mutex::new(AccessManager::new()));
    let config = OxiosConfig::default();
    let config_path = tmp.join("config.toml");

    let a2a = Arc::new(A2AProtocol::new(event_bus.clone()));
    let db = Arc::new(KernelDatabase::open_in_memory().unwrap());
    let project_manager = Arc::new(ProjectManager::new(db, None).unwrap());

    let knowledge = Arc::new(oxios_markdown::KnowledgeBase::new(tmp.join("knowledge")).unwrap());
    let knowledge_lens = Arc::new(KnowledgeLens::new(knowledge.clone(), None).unwrap());

    let skills_dir = tmp.join("skills");
    let marketplace = MarketplaceApi::new(
        Arc::new(ClawHubInstaller::new(
            skills_dir.clone(),
            tmp.to_path_buf(),
            Some("https://clawhub.ai".to_string()),
        )),
        Arc::new(ClawHubClient::new(Some("https://clawhub.ai".to_string())).unwrap()),
        Arc::new(SkillsShInstaller::new(
            skills_dir,
            Some("https://skills.sh".to_string()),
            None,
        )),
        Arc::new(SkillsShClient::new(Some("https://skills.sh".to_string()), None).unwrap()),
    );

    let engine = Arc::new(EngineApi::new(
        Arc::new(parking_lot::RwLock::new(config.clone())),
        config_path.clone(),
        Arc::new(RoutingStats::new()),
        Arc::new(oxios_kernel::EngineHandle::new(Arc::new(OxiosEngine::new(
            "anthropic/claude-sonnet-4-20250514",
        )))),
    ));

    let handle = oxios_kernel::KernelHandle::new(
        StateApi::new(state_store.clone()),
        AgentApi::new(Arc::new(NoOpSupervisor), Arc::new(BudgetManager::new())),
        SecurityApi::new(
            Arc::new(parking_lot::Mutex::new(AuthManager::new())),
            Arc::new(AuditTrail::new(100)),
            access_manager.clone(),
            state_store.clone(),
        ),
        PersonaApi::new(Arc::new(PersonaManager::new())),
        ExtensionApi::new(Arc::new(SkillManager::new(
            tmp.join("skills"),
            tmp.join("share/skills"),
        ))),
        McpApi::new(Arc::new(McpBridge::new())),
        InfraApi::new(
            Arc::new(GitLayer::new(tmp.join("git"), false).unwrap()),
            Arc::new(GitLayer::new(tmp.join("kb_git"), false).unwrap()),
            Arc::new(ResourceMonitor::new(60, 100)),
            event_bus.clone(),
            config.clone(),
            Instant::now(),
            Arc::new(PendingToolApprovals::new()),
            Arc::new(PendingAskUser::new()),
            Arc::new(parking_lot::RwLock::new(ApprovalConfig::default())),
            Arc::new(PendingPathAccess::new()),
        ),
        Some(ProjectApi::new(project_manager.clone())),
        ExecApi::new(
            Arc::new(parking_lot::RwLock::new(ExecConfig::default())),
            access_manager,
        ),
        A2aApi::new(a2a),
        engine,
        knowledge,
        knowledge_lens,
        marketplace,
        None,                        // calendar
        Arc::new(RwLock::new(None)), // email
    );

    (handle, project_manager, state_store, config_path)
}

/// An `AppState` over temp storage plus the pieces tests manipulate.
pub(crate) struct TestApp {
    pub state: Arc<AppState>,
    pub project_manager: Arc<ProjectManager>,
    pub state_store: Arc<StateStore>,
}

/// Build the fixture without any project or session.
pub(crate) fn test_app(tmp: &Path) -> TestApp {
    let (handle, project_manager, state_store, config_path) = kernel_handle(tmp);

    let mut config = OxiosConfig::default();
    config.security.auth_enabled = false;

    let readiness = Arc::new(ReadinessGate::new(0));
    readiness.set_state_store(oxios_kernel::readiness::SubsystemState::Ready);
    readiness.set_engine(oxios_kernel::readiness::SubsystemState::Ready);

    let bridge = WebBridge::new(256, Arc::new(ReliabilityLayer::new(Default::default())));
    let state = Arc::new(AppState {
        base_url: "http://127.0.0.1:0".to_string(),
        kernel: Arc::new(handle),
        bridge: WebBridgeHandle::from_bridge(&bridge),
        config: Arc::new(RwLock::new(config)),
        config_path: config_path.clone(),
        start_time: Instant::now(),
        rate_limiter: RateLimiter::new(0),
        web_dist: ActiveWebDist::new(None),
        readiness,
        gateway: Arc::new(Gateway::new(Arc::new(dummy_orchestrator(tmp)))),
        automation_store: Arc::new(tokio::sync::Mutex::new(
            AutomationStore::in_memory().unwrap(),
        )),
    });

    TestApp {
        state,
        project_manager,
        state_store,
    }
}

/// Minimal orchestrator for the `AppState.gateway` field — never invoked by
/// these route tests.
fn dummy_orchestrator(tmp: &Path) -> oxios_kernel::Orchestrator {
    let event_bus = EventBus::new(64);
    let state_store = Arc::new(StateStore::new(tmp.join("state-gw")).unwrap());
    let lifecycle = AgentLifecycleManager::new(
        Arc::new(NoOpSupervisor),
        Arc::new(parking_lot::Mutex::new(AccessManager::new())),
        Arc::new(A2AProtocol::new(event_bus.clone())),
        event_bus.clone(),
        300,
        vec![],
        true,
        "/tmp/oxios-test-workspace".to_string(),
        Arc::new(oxios_kernel::turn_registry::TurnRegistry::new()),
    );
    Orchestrator::new(event_bus, state_store, lifecycle)
}

/// Create a project with the given roots and bind a fresh session to it.
/// Returns the session id.
pub(crate) async fn session_for_project(app: &TestApp, name: &str, roots: Vec<PathBuf>) -> String {
    let project = app.project_manager.create(name, roots, "").unwrap();
    let mut session = Session::new("tester");
    session.project_id = Some(project.id.to_string());
    app.state_store.save_session(&session).await.unwrap();
    session.id.0
}

/// Create a session with no project binding.
pub(crate) async fn scopeless_session(app: &TestApp) -> String {
    let session = Session::new("tester");
    app.state_store.save_session(&session).await.unwrap();
    session.id.0
}
