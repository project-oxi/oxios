//! Integration tests for RFC-029 recovery coordinator (P2).
//!
//! Verifies the end-to-end recovery behavior:
//! - L1 same-model backoff retry on Transient failure.
//! - L2 model/provider swap when L1 is exhausted — records a FallbackEvent.
//! - QuotaExhausted skips L1 (goes straight to provider swap).
//! - AttemptBudget bounds total attempts.
#![allow(clippy::unwrap_used)] // `.unwrap()` in tests is idiomatic (workspace convention)

#[path = "common/mod.rs"]
mod common;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use oxios_kernel::a2a::A2AProtocol;
use oxios_kernel::access_manager::AccessManager;
use oxios_kernel::agent_lifecycle::AgentLifecycleManager;
use oxios_kernel::event_bus::EventBus;
use oxios_kernel::resilience::{RecoveryCoordinator, ResilienceConfig};
use oxios_kernel::supervisor::Supervisor;
use oxios_kernel::{AgentId, AgentInfo, AgentStatus, RoutingStats};
use oxios_ouroboros::{Directive, ExecEnv, ExecutionResult, FailureClass};
use parking_lot::RwLock;

/// A mock supervisor that simulates provider failures for recovery testing.
///
/// - Returns a failing `ExecutionResult` (with a configurable
///   `failure_class`) until `model_override` appears in the env.
/// - Once `model_override` is `Some`, returns success.
///
/// This mirrors what the real supervisor does after RFC-029 P0: a
/// provider error surfaces as `Ok(ExecutionResult { success: false,
/// failure_class: Some(..) })`.
struct FailingUntilOverrideSupervisor {
    /// The failure class to emit before a model_override is seen.
    fail_class: FailureClass,
    /// How many times run_with_directive was called (for assertions).
    run_count: AtomicUsize,
    agents: RwLock<HashMap<AgentId, AgentInfo>>,
}

impl FailingUntilOverrideSupervisor {
    fn new(fail_class: FailureClass) -> Self {
        Self {
            fail_class,
            run_count: AtomicUsize::new(0),
            agents: RwLock::new(HashMap::new()),
        }
    }

    fn run_count(&self) -> usize {
        self.run_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Supervisor for FailingUntilOverrideSupervisor {
    async fn exec(&self, _id: AgentId) -> anyhow::Result<()> {
        Ok(())
    }

    async fn fork_directive(
        &self,
        _directive: &Directive,
        _env: &ExecEnv,
    ) -> anyhow::Result<AgentId> {
        Ok(AgentId::new_v4())
    }

    async fn run_with_directive(
        &self,
        _id: AgentId,
        _directive: &Directive,
        env: &ExecEnv,
    ) -> anyhow::Result<ExecutionResult> {
        self.run_count.fetch_add(1, Ordering::SeqCst);

        // Simulate success once a model_override is present (the
        // coordinator sets this when swapping to a fallback model).
        if env.model_override.is_some() {
            return Ok(ExecutionResult {
                output: "recovered via fallback".into(),
                steps_completed: 1,
                success: true,
                tokens_input: 10,
                tokens_output: 5,
                model_id: env.model_override.clone().unwrap_or_default(),
                ..Default::default()
            });
        }

        // Simulate a provider failure with the configured class.
        Ok(ExecutionResult {
            output: "HTTP error 429: rate limited".into(),
            steps_completed: 0,
            success: false,
            model_id: "anthropic/claude-sonnet-4".into(),
            failure_class: Some(self.fail_class),
            ..Default::default()
        })
    }

    async fn wait(&self, id: AgentId) -> anyhow::Result<AgentStatus> {
        Ok(self
            .agents
            .read()
            .get(&id)
            .map(|a| a.status)
            .unwrap_or(AgentStatus::Stopped))
    }

    async fn kill(&self, _id: AgentId) -> anyhow::Result<()> {
        Ok(())
    }

    async fn list(&self) -> anyhow::Result<Vec<AgentInfo>> {
        Ok(self.agents.read().values().cloned().collect())
    }
}

fn build_parts(
    fail_class: FailureClass,
) -> (
    Arc<FailingUntilOverrideSupervisor>,
    AgentLifecycleManager,
    Arc<RoutingStats>,
) {
    let event_bus = EventBus::new(64);
    let tmp = common::setup_tempdir();
    let _state_store = Arc::new(oxios_kernel::StateStore::new(tmp.path().to_path_buf()).unwrap());
    let supervisor = Arc::new(FailingUntilOverrideSupervisor::new(fail_class));
    let access_manager = Arc::new(parking_lot::Mutex::new(AccessManager::new()));
    let a2a = Arc::new(A2AProtocol::new(event_bus.clone()));
    let lifecycle = AgentLifecycleManager::new(
        supervisor.clone(),
        access_manager,
        a2a,
        event_bus,
        0, // no execution timeout
        vec![],
        false,
        tmp.path().to_str().unwrap().to_string(),
    );
    let routing_stats = Arc::new(RoutingStats::new());
    (supervisor, lifecycle, routing_stats)
}

fn dummy_directive() -> Directive {
    Directive {
        goal: "test goal".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn transient_failure_recovers_via_fallback_model_swap() {
    // Primary model fails with 429 (Transient); the coordinator should
    // exhaust L1 same-model retries, then swap to the fallback model
    // (which succeeds in the mock) and record a FallbackEvent.
    let (supervisor, lifecycle, routing_stats) = build_parts(FailureClass::Transient);

    let config = ResilienceConfig {
        enabled: true,
        max_same_model_retries: 2,
        backoff_base_ms: 1, // fast for tests
        backoff_max_ms: 2,
        max_total_attempts: 8,
    };
    let coordinator = RecoveryCoordinator::new(Arc::clone(&routing_stats), config);
    coordinator.set_fallback_models(vec!["openai/gpt-4o".into()]);

    let directive = dummy_directive();
    let env = ExecEnv::default();

    let result = coordinator
        .execute(&lifecycle, &directive, &env)
        .await
        .unwrap();

    // Recovered via the fallback.
    assert!(result.success, "expected recovery via fallback model");
    assert_eq!(result.model_id, "openai/gpt-4o");

    // The supervisor was called: 1 (L0) + 2 (L1 retries) + 1 (L2 fallback) = 4.
    assert!(
        supervisor.run_count() >= 3,
        "expected at least L0 + L1 retries + L2 attempt, got {}",
        supervisor.run_count()
    );

    // A fallback event was recorded (wires the dead record_fallback).
    let fallbacks = routing_stats.fallback_history(10);
    assert_eq!(fallbacks.len(), 1, "expected exactly one fallback event");
    assert_eq!(fallbacks[0].from_model, "anthropic/claude-sonnet-4");
    assert_eq!(fallbacks[0].to_model, "openai/gpt-4o");
    assert_eq!(fallbacks[0].reason, "transient");
    assert!(fallbacks[0].success);
}

#[tokio::test]
async fn quota_exhausted_skips_same_model_retry_goes_straight_to_swap() {
    // QuotaExhausted requires a provider swap — the coordinator must NOT
    // waste L1 same-model retries (which would just hit the same quota).
    let (supervisor, lifecycle, routing_stats) = build_parts(FailureClass::QuotaExhausted);

    let config = ResilienceConfig {
        enabled: true,
        max_same_model_retries: 5, // generous; should be skipped entirely
        backoff_base_ms: 1,
        backoff_max_ms: 2,
        max_total_attempts: 8,
    };
    let coordinator = RecoveryCoordinator::new(Arc::clone(&routing_stats), config);
    // Cross-provider fallback so the swap is allowed.
    coordinator.set_fallback_models(vec!["openai/gpt-4o".into()]);

    let directive = dummy_directive();
    let env = ExecEnv::default();

    let result = coordinator
        .execute(&lifecycle, &directive, &env)
        .await
        .unwrap();

    assert!(result.success);
    // Only 2 calls: L0 (fails) + L2 fallback (succeeds). No L1 retries.
    assert_eq!(
        supervisor.run_count(),
        2,
        "QuotaExhausted must skip L1 same-model retries"
    );
}

#[tokio::test]
async fn no_fallback_models_returns_best_failure() {
    // With no fallback configured, the ladder degrades to terminal.
    let (_supervisor, lifecycle, routing_stats) = build_parts(FailureClass::Transient);

    let config = ResilienceConfig {
        enabled: true,
        max_same_model_retries: 1,
        backoff_base_ms: 1,
        backoff_max_ms: 2,
        max_total_attempts: 8,
    };
    let coordinator = RecoveryCoordinator::new(routing_stats, config);
    // No fallback models set.

    let directive = dummy_directive();
    let env = ExecEnv::default();

    let result = coordinator
        .execute(&lifecycle, &directive, &env)
        .await
        .unwrap();

    assert!(!result.success);
    assert_eq!(result.failure_class, Some(FailureClass::Transient));
}

#[tokio::test]
async fn disabled_config_is_passthrough() {
    // When disabled, the coordinator must not retry — just one call.
    let (supervisor, lifecycle, routing_stats) = build_parts(FailureClass::Transient);

    let config = ResilienceConfig {
        enabled: false,
        ..Default::default()
    };
    let coordinator = RecoveryCoordinator::new(routing_stats, config);
    coordinator.set_fallback_models(vec!["openai/gpt-4o".into()]);

    let directive = dummy_directive();
    let env = ExecEnv::default();

    let result = coordinator
        .execute(&lifecycle, &directive, &env)
        .await
        .unwrap();

    assert!(!result.success); // passthrough, no recovery
    assert_eq!(
        supervisor.run_count(),
        1,
        "disabled = single passthrough call"
    );
}

#[tokio::test]
async fn same_provider_fallback_skipped_for_quota() {
    // QuotaExhausted + a fallback on the SAME provider must be skipped
    // (waiting on the same provider won't fix quota). With no eligible
    // fallback, the result stays failed.
    let (supervisor, lifecycle, routing_stats) = build_parts(FailureClass::QuotaExhausted);

    let config = ResilienceConfig {
        enabled: true,
        max_same_model_retries: 0,
        backoff_base_ms: 1,
        backoff_max_ms: 2,
        max_total_attempts: 8,
    };
    let coordinator = RecoveryCoordinator::new(routing_stats, config);
    // Same provider as the failing primary.
    coordinator.set_fallback_models(vec!["anthropic/claude-haiku".into()]);

    let directive = dummy_directive();
    let env = ExecEnv::default();

    let result = coordinator
        .execute(&lifecycle, &directive, &env)
        .await
        .unwrap();

    assert!(
        !result.success,
        "same-provider fallback must not recover quota"
    );
    // Only L0; the L2 candidate was skipped (same provider).
    assert_eq!(supervisor.run_count(), 1);
}
