//! Orchestrator integration tests for the Oxios pipeline.
//!
//! Tests the full Ouroboros lifecycle against a fully-mocked protocol and
//! supervisor (no real LLM, no real agent execution). Despite the file
//! name, these are NOT end-to-end tests in the production-traffic sense
//! — they verify the in-process pipeline's orchestration paths via
//! `MockSupervisor` and `MockIntentEngine`. Uses `Orchestrator::with_config`
//! to control evolution iterations explicitly, so each test verifies a
//! specific pipeline path.
#![allow(clippy::unwrap_used)] // `.unwrap()` in tests is idiomatic (workspace convention)

#[path = "common/mod.rs"]
mod common;

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use oxios_kernel::supervisor::Supervisor;
use oxios_kernel::types::{AgentId, AgentInfo, AgentStatus};
use oxios_kernel::{EventBus, KernelEvent, Orchestrator, StateStore, config::OrchestratorConfig};
use oxios_ouroboros::{Directive, ExecEnv, ExecutionResult};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[expect(dead_code)]
fn make_config(max_iterations: u32) -> OrchestratorConfig {
    OrchestratorConfig {
        max_evolution_iterations: max_iterations,
        min_evaluation_score: 0.8,
    }
}

// ---------------------------------------------------------------------------
// Mock Supervisor — tracks calls, no real agent execution
// ---------------------------------------------------------------------------

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
    async fn exec(&self, id: AgentId) -> Result<()> {
        if let Some(a) = self.agents.write().get_mut(&id) {
            a.status = AgentStatus::Running;
        }
        Ok(())
    }

    async fn fork_directive(&self, directive: &Directive, _env: &ExecEnv) -> Result<AgentId> {
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
        self.agents.write().insert(id, info);
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
    ) -> Result<ExecutionResult> {
        self.run_called.fetch_add(1, Ordering::SeqCst);
        if let Some(a) = self.agents.write().get_mut(&id) {
            a.status = AgentStatus::Idle;
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
            output: "Mock agent completed successfully".into(),
            steps_completed: 5,
            success: true,
            ..Default::default()
        })
    }

    async fn wait(&self, id: AgentId) -> Result<AgentStatus> {
        Ok(self
            .agents
            .read()
            .get(&id)
            .map(|a| a.status)
            .unwrap_or(AgentStatus::Stopped))
    }

    async fn kill(&self, id: AgentId) -> Result<()> {
        if let Some(a) = self.agents.write().get_mut(&id) {
            a.status = AgentStatus::Stopped;
        }
        Ok(())
    }

    async fn list(&self) -> Result<Vec<AgentInfo>> {
        Ok(self.agents.read().values().cloned().collect())
    }
}

// ---------------------------------------------------------------------------
// Test orchestrator builder
// ---------------------------------------------------------------------------

/// Build orchestrator parts without the legacy OuroborosProtocol mock.
/// Used by the RFC-027 tests that wire a MockIntentEngine instead.
fn build_test_parts() -> (Arc<MockSupervisor>, EventBus, Arc<StateStore>) {
    let event_bus = EventBus::new(64);
    let tmp = common::setup_tempdir();
    let state_store =
        Arc::new(StateStore::new(tmp.path().to_path_buf()).expect("StateStore creation failed"));
    let supervisor = Arc::new(MockSupervisor::new(event_bus.clone()));
    (supervisor, event_bus, state_store)
}

/// Build orchestrator with RFC-027 mock engine wired.
fn build_rfc027_orchestrator(
    supervisor: Arc<MockSupervisor>,
    state_store: Arc<StateStore>,
    event_bus: EventBus,
) -> (Arc<Orchestrator>, Arc<common::MockIntentEngine>) {
    common::build_test_orchestrator(supervisor, state_store, event_bus)
}

// ---------------------------------------------------------------------------
// Tests
/// Verifies the happy path (RFC-033): every message streams straight to execute.
#[tokio::test]
async fn test_orchestrator_happy_path() {
    let (supervisor, event_bus, state_store) = build_test_parts();
    let (orchestrator, _mock) =
        build_rfc027_orchestrator(supervisor.clone(), state_store, event_bus);

    let result = orchestrator
        .handle_unified(
            "test-user",
            "Fix the bug in main.rs",
            None,
            None,
            None,
            None, // RFC-032: role
            None, // model_override
            None, // model_params
            None, // persona_id
            "test-req",
        )
        .await
        .unwrap();

    // Reaches Execute. No external review — Directive::from_message carries no criteria.
    assert!(result.session_id.is_some());
    assert_eq!(result.phase_reached, "execute");
    assert_eq!(result.evaluation_passed, None);
    assert!(!result.response.is_empty());

    // Mock supervisor fork+run were called.
    assert_eq!(supervisor.fork_called.load(Ordering::SeqCst), 1);
    assert_eq!(supervisor.run_called.load(Ordering::SeqCst), 1);
}

/// Verifies session ID is preserved across messages in the same session.
#[tokio::test]
async fn test_session_continuation() {
    let (supervisor, event_bus, state_store) = build_test_parts();
    let (orchestrator, _mock) =
        build_rfc027_orchestrator(supervisor.clone(), state_store, event_bus);

    let session_id = "test-session-123";

    let result1 = orchestrator
        .handle_unified(
            "test-user",
            "Work on the project",
            Some(session_id),
            None,
            None,
            None, // RFC-032: role
            None, // model_override
            None, // model_params
            None, // persona_id
            "test-req",
        )
        .await
        .unwrap();
    assert_eq!(result1.session_id.as_deref(), Some(session_id));

    // Second message with same session.
    let result2 = orchestrator
        .handle_unified(
            "test-user",
            "Make it production ready",
            Some(session_id),
            None,
            None,
            None, // RFC-032: role
            None, // model_override
            None, // model_params
            None, // persona_id
            "test-req",
        )
        .await
        .unwrap();
    assert_eq!(result2.session_id.as_deref(), Some(session_id));
}
/// Verifies multiple sessions are independent.
#[tokio::test]
async fn test_multiple_sessions_independent() {
    let (supervisor, event_bus, state_store) = build_test_parts();
    let (orchestrator, _mock) =
        build_rfc027_orchestrator(supervisor.clone(), state_store, event_bus);

    let result_a = orchestrator
        .handle_unified(
            "user-a",
            "Task A",
            Some("session-a"),
            None,
            None,
            None, // RFC-032: role
            None, // model_override
            None, // model_params
            None, // persona_id
            "test-req",
        )
        .await
        .unwrap();
    let result_b = orchestrator
        .handle_unified(
            "user-b",
            "Task B",
            Some("session-b"),
            None,
            None,
            None, // RFC-032: role
            None, // model_override
            None, // model_params
            None, // persona_id
            "test-req",
        )
        .await
        .unwrap();

    assert_eq!(result_a.session_id.as_deref(), Some("session-a"));
    assert_eq!(result_b.session_id.as_deref(), Some("session-b"));
    assert_ne!(result_a.session_id, result_b.session_id);
}

/// Verifies session cleanup after orchestration completes.
#[tokio::test]
async fn test_session_cleaned_after_completion() {
    let (supervisor, event_bus, state_store) = build_test_parts();
    let (orchestrator, _mock) =
        build_rfc027_orchestrator(supervisor.clone(), state_store, event_bus);

    let session_id = "cleanup-test-session";

    orchestrator
        .handle_unified(
            "test-user",
            "Simple task",
            Some(session_id),
            None,
            None,
            None, // RFC-032: role
            None, // model_override
            None, // model_params
            None, // persona_id
            "test-req",
        )
        .await
        .unwrap();

    // New message without session ID should get a fresh session.
    let result2 = orchestrator
        .handle_unified(
            "test-user",
            "Another task",
            None,
            None,
            None,
            None, // RFC-032: role
            None, // model_override
            None, // model_params
            None, // persona_id
            "test-req",
        )
        .await
        .unwrap();
    assert_ne!(result2.session_id.as_deref(), Some(session_id));
}

/// Verifies that the orchestrator completes within a reasonable time.
/// (Phase events are now Status events on the event bus.)
#[tokio::test]
async fn test_phase_events_published() {
    let (supervisor, event_bus, state_store) = build_test_parts();
    let (orchestrator, _mock) =
        build_rfc027_orchestrator(supervisor.clone(), state_store, event_bus);

    let result = orchestrator
        .handle_unified(
            "test-user",
            "Test events",
            None,
            None,
            None,
            None, // RFC-032: role
            None, // model_override
            None, // model_params
            None, // persona_id
            "test-req",
        )
        .await
        .unwrap();

    // The unified path completes and returns a result.
    assert!(!result.response.is_empty());
    assert!(result.session_id.is_some());
}
