//! Shared test helpers for oxios-kernel integration and e2e tests (RFC-027).
//!
//! Provides a mock IntentEngine that returns predictable responses without
//! making real LLM calls.

#![allow(dead_code)]

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use oxios_ouroboros::{Directive, ExecutionResult, IntentEngineOps, Verdict};

/// Mock IntentEngine that returns configurable review verdicts (RFC-033).
///
/// RFC-033 removed `assess`/`crystallize`, so the mock only configures
/// `review` — the sole surviving external intent call.
pub struct MockIntentEngine {
    pub review_response: RwLock<Verdict>,
}

impl MockIntentEngine {
    pub fn new() -> Self {
        Self {
            review_response: RwLock::new(Verdict {
                passed: true,
                score: 1.0,
                notes: vec!["Mock review passed".into()],
                gaps: vec![],
            }),
        }
    }

    pub fn with_review(self, verdict: Verdict) -> Self {
        *self.review_response.write() = verdict;
        self
    }

    pub fn into_arc(self) -> Arc<dyn IntentEngineOps> {
        Arc::new(self)
    }
}

impl Default for MockIntentEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl IntentEngineOps for MockIntentEngine {
    async fn review(
        &self,
        _directive: &Directive,
        _result: &ExecutionResult,
    ) -> anyhow::Result<Verdict> {
        Ok(self.review_response.read().clone())
    }
}

/// Build a test orchestrator wired with a MockIntentEngine.
pub fn build_test_orchestrator(
    supervisor: Arc<dyn oxios_kernel::supervisor::Supervisor>,
    state_store: Arc<oxios_kernel::state_store::StateStore>,
    event_bus: oxios_kernel::event_bus::EventBus,
) -> (Arc<oxios_kernel::Orchestrator>, Arc<MockIntentEngine>) {
    let access_manager = Arc::new(parking_lot::Mutex::new(
        oxios_kernel::access_manager::AccessManager::new(),
    ));
    let a2a = Arc::new(oxios_kernel::a2a::A2AProtocol::new(event_bus.clone()));
    let lifecycle = oxios_kernel::agent_lifecycle::AgentLifecycleManager::new(
        supervisor,
        access_manager,
        a2a,
        event_bus.clone(),
        300,
        vec![],
        true,
        "/tmp/oxios-test-workspace".to_string(),
        Arc::new(oxios_kernel::turn_registry::TurnRegistry::new()),
    );

    let mock = Arc::new(MockIntentEngine::new());
    let orchestrator = oxios_kernel::Orchestrator::new(event_bus, state_store, lifecycle);
    orchestrator.set_intent_engine(mock.clone() as Arc<dyn IntentEngineOps>);

    (Arc::new(orchestrator), mock)
}

/// Helper: build a Verdict that fails.
pub fn failing_verdict(gaps: Vec<String>) -> Verdict {
    Verdict {
        passed: false,
        score: 0.3,
        notes: vec!["Mock review failed".into()],
        gaps,
    }
}

/// Helper: build a Verdict that passes.
pub fn passing_verdict() -> Verdict {
    Verdict {
        passed: true,
        score: 1.0,
        notes: vec!["Mock review passed".into()],
        gaps: vec![],
    }
}

/// Create a fresh temporary directory for tests that need a real filesystem
/// scratch space. Auto-deleted on drop. Replaces 12+ copies of
/// `tempfile::tempdir().unwrap()` scattered across integration_tests.rs,
/// resilience_test.rs, and e2e_test.rs.
pub fn setup_tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("test tempdir creation should never fail")
}
