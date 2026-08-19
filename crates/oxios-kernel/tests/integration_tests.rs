//! Integration tests for the Oxios kernel.
//!
//! Tests the main kernel components using mock implementations:
//! - Orchestrator with mock OuroborosProtocol and mock Supervisor
//! - StateStore markdown/JSON read/write
//! - EventBus publish/subscribe
//! - Agent lifecycle and orchestrator integration
#![allow(clippy::unwrap_used)] // `.unwrap()` in tests is idiomatic (workspace convention)

#[path = "common/mod.rs"]
mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;

use oxios_kernel::access_manager::{AccessManager, AgentPermissions};
use oxios_kernel::event_bus::{EventBus, KernelEvent};
use oxios_kernel::state_store::StateStore;
use oxios_kernel::supervisor::Supervisor;
use oxios_ouroboros::{Directive, ExecEnv, ExecutionResult};

use oxios_kernel::types::{AgentId, AgentInfo, AgentStatus};

// ---------------------------------------------------------------------------
// Mock Supervisor
// ---------------------------------------------------------------------------

/// Mock supervisor that tracks agent creation without actually running agents.
struct MockSupervisor {
    agents: parking_lot::RwLock<HashMap<AgentId, AgentInfo>>,
    fork_called: AtomicUsize,
    run_called: AtomicUsize,
    event_bus: EventBus,
}

impl MockSupervisor {
    fn new(event_bus: EventBus) -> Self {
        Self {
            agents: parking_lot::RwLock::new(HashMap::new()),
            fork_called: AtomicUsize::new(0),
            run_called: AtomicUsize::new(0),
            event_bus,
        }
    }
}

#[async_trait]
impl Supervisor for MockSupervisor {
    async fn exec(&self, id: AgentId) -> anyhow::Result<()> {
        let mut agents = self.agents.write();
        match agents.get_mut(&id) {
            Some(a) => a.status = AgentStatus::Running,
            None => anyhow::bail!("Agent {id} not found"),
        }
        Ok(())
    }

    async fn fork_directive(
        &self,
        directive: &Directive,
        _env: &ExecEnv,
    ) -> anyhow::Result<AgentId> {
        self.fork_called.fetch_add(1, Ordering::SeqCst);
        let id = AgentId::new_v4();
        let info = AgentInfo {
            id,
            name: directive.goal.clone(),
            status: AgentStatus::Starting,
            created_at: chrono::Utc::now(),
            project_id: None,
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
        let _ = self.event_bus.publish(KernelEvent::AgentCreated {
            id,
            name: directive.goal.clone(),
            session_id: None,
        });
        Ok(id)
    }

    async fn run_with_directive(
        &self,
        id: AgentId,
        _directive: &Directive,
        _env: &ExecEnv,
    ) -> anyhow::Result<ExecutionResult> {
        self.run_called.fetch_add(1, Ordering::SeqCst);
        {
            let mut agents = self.agents.write();
            if let Some(a) = agents.get_mut(&id) {
                a.status = AgentStatus::Idle;
            }
        }
        let _ = self.event_bus.publish(KernelEvent::AgentStarted {
            id,
            session_id: None,
        });
        let _ = self.event_bus.publish(KernelEvent::AgentStopped {
            id,
            success: true,
            session_id: None,
        });
        Ok(ExecutionResult {
            output: "Mock agent completed".into(),
            steps_completed: 3,
            success: true,
            ..Default::default()
        })
    }

    async fn wait(&self, id: AgentId) -> anyhow::Result<AgentStatus> {
        let agents = self.agents.read();
        match agents.get(&id) {
            Some(a) => Ok(a.status),
            None => anyhow::bail!("Agent {id} not found"),
        }
    }

    async fn kill(&self, id: AgentId) -> anyhow::Result<()> {
        let mut agents = self.agents.write();
        if let Some(a) = agents.get_mut(&id) {
            a.status = AgentStatus::Stopped;
        }
        Ok(())
    }

    async fn list(&self) -> anyhow::Result<Vec<AgentInfo>> {
        let agents = self.agents.read();
        Ok(agents.values().cloned().collect())
    }
}

// ===========================================================================
// Tests
// ===========================================================================

// --- EventBus tests ---

#[tokio::test]
async fn test_event_bus_publish_subscribe() {
    let bus = EventBus::new(16);
    let mut rx = bus.subscribe();

    bus.publish(KernelEvent::AgentCreated {
        id: uuid::Uuid::new_v4(),
        name: "test-agent".into(),
        session_id: None,
    })
    .unwrap();

    let event = rx.recv().await.unwrap();
    match event {
        KernelEvent::AgentCreated { .. } => {}
        other => panic!("Expected AgentCreated, got {other:?}"),
    }
}

#[tokio::test]
async fn test_event_bus_multiple_subscribers() {
    let bus = EventBus::new(16);
    let mut rx1 = bus.subscribe();
    let mut rx2 = bus.subscribe();

    bus.publish(KernelEvent::AgentCreated {
        id: uuid::Uuid::new_v4(),
        name: "test-agent".into(),
        session_id: None,
    })
    .unwrap();

    let e1 = rx1.recv().await.unwrap();
    let e2 = rx2.recv().await.unwrap();

    assert!(matches!(e1, KernelEvent::AgentCreated { .. }));
    assert!(matches!(e2, KernelEvent::AgentCreated { .. }));
}

#[tokio::test]
async fn test_event_bus_no_subscribers_ok() {
    let bus = EventBus::new(16);
    // Should not panic when publishing with no subscribers.
    bus.publish(KernelEvent::AgentCreated {
        id: uuid::Uuid::new_v4(),
        name: "test-agent".into(),
        session_id: None,
    })
    .unwrap();
}

// --- StateStore tests ---

#[tokio::test]
async fn test_state_store_save_load_markdown() {
    let tmp = common::setup_tempdir();
    let store = StateStore::new(tmp.path().to_path_buf()).unwrap();

    store
        .save_markdown("memory", "test-note", "Hello, world!")
        .await
        .unwrap();

    let loaded = store.load_markdown("memory", "test-note").await.unwrap();
    assert_eq!(loaded, Some("Hello, world!".to_string()));
}

#[tokio::test]
async fn test_state_store_load_nonexistent() {
    let tmp = common::setup_tempdir();
    let store = StateStore::new(tmp.path().to_path_buf()).unwrap();

    let loaded = store.load_markdown("memory", "nope").await.unwrap();
    assert_eq!(loaded, None);
}

#[tokio::test]
async fn test_state_store_list_category() {
    let tmp = common::setup_tempdir();
    let store = StateStore::new(tmp.path().to_path_buf()).unwrap();

    store
        .save_markdown("notes", "alpha", "alpha content")
        .await
        .unwrap();
    store
        .save_markdown("notes", "beta", "beta content")
        .await
        .unwrap();

    let names = store.list_category("notes").await.unwrap();
    assert_eq!(names, vec!["alpha", "beta"]);
}

#[tokio::test]
async fn test_state_store_save_load_json() {
    let tmp = common::setup_tempdir();
    let store = StateStore::new(tmp.path().to_path_buf()).unwrap();

    let data = serde_json::json!({
        "name": "test",
        "value": 42
    });

    store.save_json("config", "test", &data).await.unwrap();
    let loaded: Option<serde_json::Value> = store.load_json("config", "test").await.unwrap();
    assert_eq!(loaded, Some(data));
}

#[tokio::test]
async fn test_state_store_path_traversal_blocked() {
    let tmp = common::setup_tempdir();
    let store = StateStore::new(tmp.path().to_path_buf()).unwrap();

    // Category traversal should be blocked.
    let result = store.save_markdown("../etc", "shadow", "hacked").await;
    assert!(result.is_err());

    // Name traversal should be blocked.
    let result = store.save_markdown("memory", "../shadow", "hacked").await;
    assert!(result.is_err());

    // Slash in category is allowed (sub-directory categories like memory/knowledge).
    let result = store.save_markdown("foo/bar", "test", "content").await;
    assert!(result.is_ok());

    // Backslash should be blocked.
    let result = store.save_markdown("foo\\bar", "test", "content").await;
    assert!(result.is_err());

    // Empty category should be blocked.
    let result = store.save_markdown("", "test", "content").await;
    assert!(result.is_err());

    // Leading slash should be blocked.
    let result = store.save_markdown("/foo", "test", "content").await;
    assert!(result.is_err());

    // Trailing slash should be blocked.
    let result = store.save_markdown("foo/", "test", "content").await;
    assert!(result.is_err());

    // Double slash should be blocked.
    let result = store.save_markdown("foo//bar", "test", "content").await;
    assert!(result.is_err());
}

// --- Orchestrator tests ---

#[tokio::test]
async fn test_orchestrator_happy_path() {
    let supervisor = Arc::new(MockSupervisor::new(EventBus::new(64)));
    let event_bus = EventBus::new(64);
    let tmp = common::setup_tempdir();
    let state_store = Arc::new(StateStore::new(tmp.path().to_path_buf()).unwrap());

    let (orchestrator, _mock) =
        common::build_test_orchestrator(supervisor.clone(), state_store, event_bus);

    let result = orchestrator
        .handle_unified(
            "test-user",
            "Do something useful",
            None,
            None,
            None,
            None, // RFC-032: role
            None, // model_override
            None, // model_params
            "test-req",
        )
        .await
        .unwrap();

    assert!(result.session_id.is_some());
    assert_eq!(result.phase_reached, "execute");
    assert_eq!(result.evaluation_passed, None);
    assert!(!result.response.is_empty());

    // Verify supervisor was called.
    assert_eq!(supervisor.fork_called.load(Ordering::SeqCst), 1);
    assert_eq!(supervisor.run_called.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_orchestrator_events_published() {
    let event_bus = EventBus::new(64);
    let mut rx = event_bus.subscribe();
    let tmp = common::setup_tempdir();
    let state_store = Arc::new(StateStore::new(tmp.path().to_path_buf()).unwrap());

    let supervisor = Arc::new(MockSupervisor::new(event_bus.clone()));
    let (orchestrator, _mock) =
        common::build_test_orchestrator(supervisor, state_store, event_bus.clone());

    // Run orchestration in background.
    let handle = tokio::spawn(async move {
        orchestrator
            .handle_unified(
                "test-user",
                "Check events",
                None,
                None,
                None,
                None, // RFC-032: role
                None, // model_override
                None, // model_params
                "test-req",
            )
            .await
    });

    // Collect events with timeout.
    let mut agent_events = 0;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);

    loop {
        let evt = tokio::select! {
            evt = rx.recv() => evt.unwrap(),
            _ = tokio::time::sleep_until(deadline) => break,
        };
        match evt {
            KernelEvent::AgentCreated { .. }
            | KernelEvent::AgentStarted { .. }
            | KernelEvent::AgentStopped { .. } => {
                agent_events += 1;
            }
            _ => {}
        }
        if agent_events >= 2 {
            break;
        }
    }

    // Should have at least AgentCreated + AgentStarted + AgentStopped events.
    assert!(
        agent_events >= 2,
        "Expected at least 2 agent events, got {agent_events}"
    );

    // Ensure the orchestration completed.
    let _ = handle.await.unwrap();
}

// ===========================================================================
// Orchestrator Integration
// ===========================================================================

use std::sync::atomic::AtomicU32;

/// Mock supervisor with agent tracking for orchestrator integration tests.
struct TrackingSupervisor {
    agents: parking_lot::RwLock<HashMap<AgentId, AgentInfo>>,
    event_bus: EventBus,
    runs: AtomicU32,
}

impl TrackingSupervisor {
    fn new(event_bus: EventBus) -> Self {
        Self {
            agents: parking_lot::RwLock::new(HashMap::new()),
            event_bus,
            runs: AtomicU32::new(0),
        }
    }
}

#[async_trait]
impl Supervisor for TrackingSupervisor {
    async fn exec(&self, id: AgentId) -> anyhow::Result<()> {
        let mut agents = self.agents.write();
        if let Some(a) = agents.get_mut(&id) {
            a.status = AgentStatus::Running;
        }
        Ok(())
    }

    async fn fork_directive(
        &self,
        directive: &Directive,
        _env: &ExecEnv,
    ) -> anyhow::Result<AgentId> {
        let id = AgentId::new_v4();
        let info = AgentInfo {
            id,
            name: directive.goal.clone(),
            status: AgentStatus::Starting,
            created_at: chrono::Utc::now(),
            project_id: None,
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
        let _ = self.event_bus.publish(KernelEvent::AgentCreated {
            id,
            name: directive.goal.clone(),
            session_id: None,
        });
        Ok(id)
    }

    async fn run_with_directive(
        &self,
        id: AgentId,
        _directive: &Directive,
        _env: &ExecEnv,
    ) -> anyhow::Result<ExecutionResult> {
        self.runs.fetch_add(1, Ordering::SeqCst);

        {
            let mut agents = self.agents.write();
            if let Some(a) = agents.get_mut(&id) {
                a.status = AgentStatus::Idle;
            }
        }
        let _ = self.event_bus.publish(KernelEvent::AgentStarted {
            id,
            session_id: None,
        });
        let _ = self.event_bus.publish(KernelEvent::AgentStopped {
            id,
            success: true,
            session_id: None,
        });
        Ok(ExecutionResult {
            output: "Task completed".into(),
            steps_completed: 1,
            success: true,
            ..Default::default()
        })
    }

    async fn wait(&self, id: AgentId) -> anyhow::Result<AgentStatus> {
        let agents = self.agents.read();
        match agents.get(&id) {
            Some(a) => Ok(a.status),
            None => anyhow::bail!("Agent {id} not found"),
        }
    }

    async fn kill(&self, id: AgentId) -> anyhow::Result<()> {
        let mut agents = self.agents.write();
        if let Some(a) = agents.get_mut(&id) {
            a.status = AgentStatus::Stopped;
        }
        Ok(())
    }

    async fn list(&self) -> anyhow::Result<Vec<AgentInfo>> {
        let agents = self.agents.read();
        Ok(agents.values().cloned().collect())
    }
}

#[tokio::test]
async fn test_orchestrator_routes_to_supervisor() {
    let event_bus = EventBus::new(64);
    let tmp = common::setup_tempdir();
    let state_store = Arc::new(StateStore::new(tmp.path().to_path_buf()).unwrap());

    let supervisor = Arc::new(TrackingSupervisor::new(event_bus.clone()));

    let (orchestrator, _mock) =
        common::build_test_orchestrator(supervisor, state_store, event_bus.clone());

    let result = orchestrator
        .handle_unified(
            "test-user",
            "Build a simple thing",
            None,
            None,
            None,
            None, // RFC-032: role
            None, // model_override
            None, // model_params
            "test-req",
        )
        .await
        .unwrap();

    assert!(result.session_id.is_some());
    assert_eq!(result.phase_reached, "execute");
}

// ===========================================================================
// Access Manager Enforcing Permissions
// ===========================================================================

#[tokio::test]
async fn test_access_manager_blocks_dangerous_tools() {
    let mut access = AccessManager::new();

    // Create a restrictive permission set (no dangerous tools).
    let mut perms = AgentPermissions::for_new_agent("safe-agent");
    perms.allowed_tools.clear(); // Start with nothing.
    perms.allow_tool("read"); // Only safe tools.
    perms.allow_tool("grep");
    access.set_permissions(perms);

    // Verify dangerous tools are blocked.
    assert!(!access.can_use_tool("safe-agent", "bash")); // bash not allowed.
    assert!(!access.can_use_tool("safe-agent", "rm")); // rm not allowed.
    assert!(!access.can_use_tool("safe-agent", "sudo")); // sudo not allowed.
    assert!(access.can_use_tool("safe-agent", "read")); // read is allowed.
    assert!(access.can_use_tool("safe-agent", "grep")); // grep is allowed.

    // Unknown agent gets no tools.
    assert!(!access.can_use_tool("unknown-agent", "read"));
}

#[tokio::test]
async fn test_access_manager_enforces_path_restrictions() {
    let mut access = AccessManager::new();

    let mut perms = AgentPermissions::for_new_agent("file-agent");
    perms.allowed_paths = vec![
        "/workspace/**".to_string(),
        "/home/user/projects/**".to_string(),
    ];
    perms.denied_paths = vec![
        "/workspace/secrets/**".to_string(),
        "/workspace/.oxios/**".to_string(),
    ];
    access.set_permissions(perms);

    // Allowed paths.
    assert!(access.can_access_path("file-agent", "/workspace/file.txt"));
    assert!(access.can_access_path("file-agent", "/workspace/subdir/code.rs"));
    assert!(access.can_access_path("file-agent", "/home/user/projects/app/main.rs"));

    // Blocked: outside allowed paths.
    assert!(!access.can_access_path("file-agent", "/etc/passwd"));
    assert!(!access.can_access_path("file-agent", "/root/.ssh/id_rsa"));

    // Blocked: denied pattern matches.
    assert!(!access.can_access_path("file-agent", "/workspace/secrets/api-key.txt"));
    assert!(!access.can_access_path("file-agent", "/workspace/.oxios/config.toml"));
}

#[tokio::test]
async fn test_access_manager_audit_log_on_denied_access() {
    let mut access = AccessManager::new();

    let perms = AgentPermissions::for_new_agent("audited-agent");
    access.set_permissions(perms);

    // Attempt denied operations.
    access.can_use_tool("audited-agent", "network"); // not in allowed set.
    access.can_access_path("audited-agent", "/etc/shadow"); // path not allowed.

    let log = access.audit_log();
    assert_eq!(log.len(), 2);

    // Both should be marked as denied.
    assert!(!log[0].allowed);
    assert!(!log[1].allowed);

    // Check reason is recorded.
    assert!(log[0].reason.is_some());
    assert!(log[1].reason.is_some());

    // Check denied_actions filter.
    let denied = access.denied_actions();
    assert_eq!(denied.len(), 2);
}

#[tokio::test]
async fn test_access_manager_network_and_fork_permissions() {
    let mut access = AccessManager::new();

    // Agent with network but no fork.
    let mut perms = AgentPermissions::for_new_agent("web-agent");
    perms.enable_network();
    // can_fork stays false by default.
    access.set_permissions(perms);

    assert!(access.can_access_network("web-agent"));
    assert!(!access.can_fork("web-agent"));

    // Agent with fork but no network.
    let mut perms2 = AgentPermissions::for_new_agent("fork-agent");
    perms2.enable_forking();
    // network_access stays false.
    access.set_permissions(perms2);

    assert!(!access.can_access_network("fork-agent"));
    assert!(access.can_fork("fork-agent"));

    // Agent with execution time and memory limits.
    let mut perms3 = AgentPermissions::for_new_agent("limited-agent");
    perms3.max_execution_time_secs = 60;
    perms3.max_memory_mb = 256;
    access.set_permissions(perms3);

    assert!(access.can_execute_for("limited-agent", 30));
    assert!(access.can_execute_for("limited-agent", 60));
    assert!(!access.can_execute_for("limited-agent", 61));
    assert!(access.can_use_memory("limited-agent", 128));
    assert!(!access.can_use_memory("limited-agent", 257));
}

#[tokio::test]
async fn test_access_manager_permission_lifecycle() {
    let mut access = AccessManager::new();

    // Create permissions.
    access.set_permissions(AgentPermissions::for_new_agent("lifecycle-agent"));

    // Check agent is listed.
    assert!(
        access
            .list_agents()
            .contains(&"lifecycle-agent".to_string())
    );

    // Grant a tool.
    let perms = access.get_or_create_permissions("lifecycle-agent");
    perms.allow_tool("custom-tool");

    // Verify tool is now allowed.
    assert!(access.can_use_tool("lifecycle-agent", "custom-tool"));

    // Remove permissions.
    access.remove_permissions("lifecycle-agent");

    // Agent no longer listed.
    assert!(
        !access
            .list_agents()
            .contains(&"lifecycle-agent".to_string())
    );

    // All access denied for removed agent.
    assert!(!access.can_use_tool("lifecycle-agent", "custom-tool"));
    assert!(!access.can_access_path("lifecycle-agent", "/workspace/test.txt"));
    assert!(!access.can_access_network("lifecycle-agent"));
}
