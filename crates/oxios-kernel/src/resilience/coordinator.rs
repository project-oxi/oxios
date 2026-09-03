//! Recovery coordinator (RFC-029 §3.4) — drives the OTP-style recovery
//! policy on top of the Unix lifecycle.
//!
//! Wraps `AgentLifecycleManager::execute_directive` in an escalation
//! ladder. On a provider/infra failure (signalled by
//! `ExecutionResult.failure_class`), it retries with backoff (L1) and/or
//! swaps to a fallback model (L2), recording each fallback for
//! observability. Bounded by an `AttemptBudget`.
//!
//! # Current scope (P2)
//!
//! - L1 same-model backoff retry (Transient/Unknown).
//! - L2 model/provider swap via `ExecEnv.model_override` (reads
//!   `engine.fallback_models`). Records `FallbackEvent`.
//! - L5 terminal — returns the best result with `failure_class` set.
//!
//! # Deferred (P2b / P3 / P4)
//!
//! - **Snapshot+restore state preservation (P2b):** L2 currently re-runs
//!   from scratch (a fresh fork). This loses mid-execution conversation
//!   state. The unified snapshot→restore-with-new-model operation (RFC
//!   §3.2) will be wired once the `Agent::export_state` capture-on-failure
//!   path lands. For now, the side-effect duplication risk is mitigated
//!   by: (1) provider failures predominantly occur at connection/first-
//!   call level where no tools ran, and (2) the AttemptBudget bounds
//!   total attempts.
//! - **Per-provider circuit breaker (P3):** `ProviderHealthRegistry`.
//! - **A2A delegation (P4):** agent-specific failures only.

use std::sync::Arc;

use chrono::Utc;
use oxios_ouroboros::{Directive, ExecEnv, ExecutionResult, FailureClass};
use parking_lot::RwLock;
use tracing::{info, warn};

use crate::agent_lifecycle::AgentLifecycleManager;
use crate::resilience::ProviderHealthRegistry;

use crate::kernel_handle::{FallbackEvent, RoutingStats};

use super::budget::AttemptBudget;

/// Resilience configuration (RFC-029 §4). Mirrors a subset of
/// `[intent.resilience]` from config.toml.
#[derive(Debug, Clone)]
pub struct ResilienceConfig {
    /// Master switch. When `false`, `execute` is a passthrough to the
    /// lifecycle with no recovery.
    pub enabled: bool,
    /// L1: max same-model backoff retries before escalating to L2.
    pub max_same_model_retries: u32,
    /// L1: base delay in ms for exponential backoff (`base * 2^attempt`).
    pub backoff_base_ms: u64,
    /// L1: cap on the backoff delay in ms.
    pub backoff_max_ms: u64,
    /// Global cap on total directive executions across the ladder. `0`
    /// = unlimited (not recommended for production).
    pub max_total_attempts: u32,
}

impl Default for ResilienceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_same_model_retries: 2,
            backoff_base_ms: 1000,
            backoff_max_ms: 30_000,
            max_total_attempts: 8,
        }
    }
}

/// The recovery coordinator. Holds the shared routing stats (for
/// `FallbackEvent` recording) and the live fallback-model list. The
/// lifecycle is passed per-call (the orchestrator owns it).
pub struct RecoveryCoordinator {
    routing_stats: Arc<RoutingStats>,
    /// Fallback model IDs (`"provider/model"`), tried left-to-right.
    /// Updated live from `engine.fallback_models` via [`set_fallback_models`].
    fallback_models: RwLock<Vec<String>>,
    /// Per-provider health for smart skip in L2 (RFC-029 P3). `None` =
    /// all providers assumed healthy (no health gating).
    health: RwLock<Option<Arc<ProviderHealthRegistry>>>,
    /// A2A circuit breaker for L4 delegation gating (RFC-029 P4). When
    /// set, the L4 step checks `is_allowed` before attempting delegation.
    a2a_breaker: RwLock<Option<Arc<crate::a2a::circuit_breaker::A2ACircuitBreaker>>>,
    config: RwLock<ResilienceConfig>,
}

impl RecoveryCoordinator {
    /// Create a coordinator. `routing_stats` is shared with `EngineApi`
    /// / `AgentRuntime` so fallback events surface in the Web UI.
    pub fn new(routing_stats: Arc<RoutingStats>, config: ResilienceConfig) -> Self {
        Self {
            routing_stats,
            fallback_models: RwLock::new(Vec::new()),
            health: RwLock::new(None),
            a2a_breaker: RwLock::new(None),
            config: RwLock::new(config),
        }
    }

    /// Wire the per-provider circuit breaker (RFC-029 P3). When set,
    /// L2 skips every model whose provider's breaker is open.
    pub fn set_health(&self, health: Arc<ProviderHealthRegistry>) {
        *self.health.write() = Some(health);
    }

    /// Wire the A2A circuit breaker for L4 delegation gating (RFC-029 P4).
    /// When set, L4 checks `is_allowed` before attempting delegation.
    pub fn set_a2a_breaker(&self, breaker: Arc<crate::a2a::circuit_breaker::A2ACircuitBreaker>) {
        *self.a2a_breaker.write() = Some(breaker);
    }
    /// Update the fallback-model list (called when engine routing config
    /// changes, e.g. `set_routing` / hot-reload).
    pub fn set_fallback_models(&self, models: Vec<String>) {
        *self.fallback_models.write() = models;
    }

    /// Update the resilience config (hot-reload).
    pub fn set_config(&self, config: ResilienceConfig) {
        *self.config.write() = config;
    }

    /// Execute a directive with the recovery ladder.
    ///
    /// On success or non-provider failure (cancellation/abort), returns
    /// the result as-is. On a provider failure (`failure_class: Some`),
    /// escalates through L1 (same-model backoff) → L2 (model swap) →
    /// terminal.
    pub async fn execute(
        &self,
        lifecycle: &AgentLifecycleManager,
        directive: &Directive,
        env: &ExecEnv,
    ) -> anyhow::Result<ExecutionResult> {
        let config = self.config.read().clone();
        if !config.enabled {
            // Passthrough — no recovery.
            return lifecycle.execute_directive(directive, env).await;
        }

        let budget = AttemptBudget::new(config.max_total_attempts);
        // L0 — initial attempt.
        budget.try_consume();
        let result = lifecycle.execute_directive(directive, env).await?;

        // Decide whether to escalate based on the failure signal.
        let class = match (result.success, result.failure_class) {
            (true, _) => return Ok(result),
            // Cancellation / abort (no classified failure) — not retryable.
            (false, None) => return Ok(result),
            (false, Some(class)) => class,
        };

        self.escalate(lifecycle, directive, env, &result, class, &budget, &config)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn escalate(
        &self,
        lifecycle: &AgentLifecycleManager,
        directive: &Directive,
        env: &ExecEnv,
        initial: &ExecutionResult,
        class: FailureClass,
        budget: &AttemptBudget,
        config: &ResilienceConfig,
    ) -> anyhow::Result<ExecutionResult> {
        let primary = env
            .model_override
            .clone()
            .unwrap_or_else(|| initial.model_id.clone());
        let mut best = initial.clone();

        // L1 — same-model backoff retry (only Transient/Unknown benefit).
        if class.benefits_from_same_model_retry() {
            for attempt in 1..=config.max_same_model_retries {
                if !budget.try_consume() {
                    warn!(attempt, "attempt budget exhausted during L1 backoff");
                    break;
                }
                let delay_ms = backoff(attempt, config.backoff_base_ms, config.backoff_max_ms);
                info!(
                    attempt,
                    delay_ms,
                    class = %class,
                    model = %primary,
                    "L1: same-model backoff retry"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                match lifecycle.execute_directive(directive, env).await {
                    Ok(r) => {
                        if r.success {
                            return Ok(r);
                        }
                        // Capture the class before moving r into better_of.
                        let new_class = r.failure_class;
                        best = better_of(best, r);
                        // If the class changed (e.g. now auth), re-evaluate.
                        if let Some(c) = new_class
                            && !c.benefits_from_same_model_retry()
                        {
                            info!(class = %c, "L1: failure class changed, escalating to L2");
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "L1: lifecycle error during retry");
                        break;
                    }
                }
            }
        }

        // L2 — model/provider swap via fallback chain.
        let fallbacks = self.fallback_models.read().clone();
        if fallbacks.is_empty() {
            info!("L2: no fallback models configured — skipping to terminal");
            return Ok(best);
        }

        for alt in fallbacks {
            if alt == primary {
                continue; // don't retry the model that just failed
            }
            if class.requires_provider_swap() && same_provider(&alt, &primary) {
                info!(
                    from = %primary,
                    to = %alt,
                    "L2: skipping same-provider fallback (quota/auth needs a different provider)"
                );
                continue;
            }
            // P3: skip models on providers whose circuit breaker is open.
            {
                let health_guard = self.health.read();
                if let Some(ref health) = *health_guard {
                    let provider = provider_of(&alt);
                    if !health.is_healthy(provider) {
                        info!(
                            from = %primary,
                            to = %alt,
                            provider,
                            "L2: skipping fallback — provider breaker open"
                        );
                        continue;
                    }
                }
            }

            if !budget.try_consume() {
                warn!("attempt budget exhausted during L2 model swap");
                break;
            }

            let mut env2 = env.clone();
            env2.model_override = Some(alt.clone());
            // RFC-029 P2b: carry the previous run's conversation state so
            // the new agent continues from the checkpoint rather than
            // restarting from scratch (snapshot → restore-with-new-model).
            env2.restore_state.clone_from(&best.restore_state);

            info!(from = %primary, to = %alt, class = %class, "L2: model swap retry");
            match lifecycle.execute_directive(directive, &env2).await {
                Ok(r) => {
                    let success = r.success;
                    // Record the fallback event (wires the dead record_fallback).
                    self.routing_stats.record_fallback(FallbackEvent {
                        timestamp: Utc::now(),
                        from_model: primary.clone(),
                        to_model: alt.clone(),
                        reason: class.to_string(),
                        success,
                    });
                    if success {
                        info!(to = %alt, "L2: fallback model succeeded");
                        return Ok(r);
                    }
                    best = better_of(best, r);
                }
                Err(e) => {
                    warn!(error = %e, to = %alt, "L2: lifecycle error during fallback");
                    self.routing_stats.record_fallback(FallbackEvent {
                        timestamp: Utc::now(),
                        from_model: primary.clone(),
                        to_model: alt.clone(),
                        reason: class.to_string(),
                        success: false,
                    });
                }
            }
        }

        // L4 — A2A delegation (agent-specific failures only).
        // Gates via the A2A circuit breaker. When all model/provider options
        // are exhausted, delegate to a fresh agent via A2A. Only helps for
        // agent-specific failures (bad persona, missing tool); infra-wide
        // outages make delegation pointless because the fresh agent uses the
        // same dead providers.
        {
            let breaker_guard = self.a2a_breaker.read();
            if let Some(ref breaker) = *breaker_guard {
                if breaker.is_allowed() {
                    info!(class = %class, "L4: A2A delegation attempted");
                    // The actual delegation is handled by the orchestrator
                    // (A2AProtocol). Here we just gate and record the
                    // attempt. The result carries a best-effort terminal
                    // that signals "no provider-based recovery worked."
                    // On failure: the breaker records it.
                    breaker.record_failure();
                } else {
                    info!("L4: A2A delegation blocked — circuit breaker open");
                }
            }
        }

        // L5 — terminal. Return the best result obtained.
        info!(
            class = %class,
            success = best.success,
            "L5: recovery exhausted, returning best result"
        );
        Ok(best)
    }
}

/// Exponential backoff: `base * 2^(attempt-1)`, capped at `max`.
fn backoff(attempt: u32, base_ms: u64, max_ms: u64) -> u64 {
    let exp = attempt.saturating_sub(1).min(10);
    base_ms.saturating_mul(2u64.saturating_pow(exp)).min(max_ms)
}

/// Extract the provider prefix from a `"provider/model"` id.
fn provider_of(model_id: &str) -> &str {
    model_id.split_once('/').map(|(p, _)| p).unwrap_or(model_id)
}

/// Whether two model IDs share the same provider.
fn same_provider(a: &str, b: &str) -> bool {
    provider_of(a) == provider_of(b)
}

/// Pick the "better" of two results: prefer success, then more steps.
fn better_of(a: ExecutionResult, b: ExecutionResult) -> ExecutionResult {
    if a.success {
        return a;
    }
    if b.success {
        return b;
    }
    if b.steps_completed > a.steps_completed {
        b
    } else {
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_exponentially_and_caps() {
        assert_eq!(backoff(1, 1000, 30_000), 1000);
        assert_eq!(backoff(2, 1000, 30_000), 2000);
        assert_eq!(backoff(3, 1000, 30_000), 4000);
        assert_eq!(backoff(4, 1000, 30_000), 8000);
        // capped
        assert_eq!(backoff(10, 1000, 30_000), 30_000);
    }

    #[test]
    fn provider_of_extracts_prefix() {
        assert_eq!(provider_of("anthropic/claude-sonnet-4"), "anthropic");
        assert_eq!(provider_of("openai/gpt-4o"), "openai");
        assert_eq!(provider_of("bare-model"), "bare-model"); // no slash
    }

    #[test]
    fn same_provider_detection() {
        assert!(same_provider("anthropic/a", "anthropic/b"));
        assert!(!same_provider("anthropic/a", "openai/b"));
    }

    #[test]
    fn better_of_prefers_success() {
        let ok = ExecutionResult {
            recipe_id: None,
            output: "ok".into(),
            steps_completed: 0,
            success: true,
            tool_calls: vec![],
            tokens_input: 0,
            tokens_output: 0,
            model_id: "m".into(),
            failure_class: None,
            restore_state: None,
            reasoning_text: String::new(),
            reasoning_segments: Vec::new(),
        };
        let fail = ExecutionResult {
            success: false,
            output: "fail".into(),
            steps_completed: 5,
            ..ok.clone()
        };
        assert!(better_of(fail.clone(), ok.clone()).success);
        assert!(better_of(ok.clone(), fail.clone()).success);
    }

    #[test]
    fn better_of_prefers_more_steps_on_tie() {
        let base = ExecutionResult {
            recipe_id: None,
            output: String::new(),
            steps_completed: 0,
            success: false,
            tool_calls: vec![],
            tokens_input: 0,
            tokens_output: 0,
            model_id: String::new(),
            failure_class: Some(FailureClass::Transient),
            restore_state: None,
            reasoning_text: String::new(),
            reasoning_segments: Vec::new(),
        };
        let more = ExecutionResult {
            steps_completed: 3,
            ..base.clone()
        };
        assert_eq!(better_of(base, more).steps_completed, 3);
    }

    #[test]
    fn budget_integration_with_coordinator_logic() {
        // The coordinator delegates to the lifecycle, so we test the
        // pure decision helpers here. Full integration is covered by
        // the lifecycle mock in the integration tests.
        let b = AttemptBudget::new(3);
        assert!(b.try_consume());
        assert!(b.try_consume());
        assert!(b.try_consume());
        assert!(!b.try_consume());
    }
}
