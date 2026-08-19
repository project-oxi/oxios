//! Turn registry — makes an in-flight chat turn addressable for cancellation.
//!
//! Keyed by the SAME string the streaming sink uses: the chat session id, or
//! the request id for a session's first message (`gateway.rs` computes
//! `session_id.unwrap_or(request_id)`; the orchestrator mirrors it into
//! `ExecEnv.session_id`). Never introduce a second identity for a turn.
//!
//! Two halves must be reachable to actually stop a turn:
//!   * the gateway dispatch future, woken through [`TurnToken::cancelled`], and
//!   * the supervisor-spawned agent task, killed by the [`bind_agent`] binding.
//!
//!
//! Deliberately dependency-free (`Notify` + `AtomicBool`): `oxios-kernel` does
//! not depend on `tokio-util`, and adding it for one token is not warranted.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use tokio::sync::Notify;

/// Shared cancellation state for one turn.
#[derive(Debug)]
struct TurnEntry {
    cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
    agent_id: Option<uuid::Uuid>,
}

/// Handle held by the dispatch task for the lifetime of one turn.
#[derive(Debug, Clone)]
pub struct TurnToken {
    cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl TurnToken {
    /// Whether this turn has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Resolves as soon as the turn is cancelled. Safe to `select!` on: the
    /// flag is checked before awaiting, so a cancel that lands before the
    /// waiter is registered is not missed.
    pub async fn cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }
            let waiter = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            waiter.await;
        }
    }
}

/// Registry of cancellable in-flight turns, keyed by turn key.
#[derive(Debug, Default)]
pub struct TurnRegistry {
    inner: Mutex<HashMap<String, TurnEntry>>,
}

impl TurnRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a turn. Replaces any stale entry for the same key so a new turn
    /// never inherits a previous turn's cancel flag.
    pub fn open(&self, key: &str) -> TurnToken {
        let cancelled = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(Notify::new());
        self.inner.lock().insert(
            key.to_string(),
            TurnEntry {
                cancelled: cancelled.clone(),
                notify: notify.clone(),
                agent_id: None,
            },
        );
        TurnToken { cancelled, notify }
    }

    /// Bind the forked agent to the turn so cancellation can kill it.
    ///
    /// Returns:
    /// * `true` when the entry exists AND was already cancelled at bind time
    ///   — the caller MUST kill the just-forked agent, since `cancel` ran
    ///   concurrently with `fork_directive` and could not see the agent id.
    /// * `false` when the entry exists and is still live (normal path), or
    ///   when no entry exists for the key (the turn already ended; nothing
    ///   to cancel, but nothing to bind either).
    pub fn bind_agent(&self, key: &str, agent_id: uuid::Uuid) -> bool {
        let mut guard = self.inner.lock();
        if let Some(entry) = guard.get_mut(key) {
            entry.agent_id = Some(agent_id);
            entry.cancelled.load(Ordering::Acquire)
        } else {
            false
        }
    }

    /// Cancel a turn. Returns the bound agent id so the caller can kill it
    /// (the registry deliberately does not own a supervisor reference).
    /// `None` when no turn is in flight for this key.
    pub fn cancel(&self, key: &str) -> Option<uuid::Uuid> {
        let guard = self.inner.lock();
        let entry = guard.get(key)?;
        entry.cancelled.store(true, Ordering::Release);
        entry.notify.notify_waiters();
        entry.agent_id
    }

    /// End a turn. Idempotent.
    pub fn close(&self, key: &str) {
        self.inner.lock().remove(key);
    }

    /// Whether a live turn exists for this key. Diagnostic only.
    pub fn contains(&self, key: &str) -> bool {
        self.inner.lock().contains_key(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancel_returns_bound_agent_and_wakes_token() {
        let reg = TurnRegistry::new();
        let token = reg.open("sess-1");
        let agent = uuid::Uuid::new_v4();
        reg.bind_agent("sess-1", agent);

        assert!(!token.is_cancelled());
        assert_eq!(reg.cancel("sess-1"), Some(agent));
        assert!(token.is_cancelled());
        // The waiter resolves immediately once cancelled.
        tokio::time::timeout(std::time::Duration::from_millis(50), token.cancelled())
            .await
            .expect("cancelled() must resolve after cancel()");
    }

    #[tokio::test]
    async fn cancel_unknown_key_is_none_and_close_is_idempotent() {
        let reg = TurnRegistry::new();
        assert_eq!(reg.cancel("nope"), None);
        reg.open("sess-2");
        reg.close("sess-2");
        reg.close("sess-2");
        assert_eq!(reg.cancel("sess-2"), None);
    }

    #[tokio::test]
    async fn reopening_a_key_clears_the_previous_cancel_flag() {
        let reg = TurnRegistry::new();
        let first = reg.open("sess-3");
        reg.cancel("sess-3");
        assert!(first.is_cancelled());
        reg.close("sess-3");

        let second = reg.open("sess-3");
        assert!(
            !second.is_cancelled(),
            "a new turn must not inherit the old cancel"
        );
    }

    #[tokio::test]
    async fn bind_agent_returns_false_for_live_turn() {
        let reg = TurnRegistry::new();
        let _token = reg.open("sess-4");
        let was_cancelled = reg.bind_agent("sess-4", uuid::Uuid::new_v4());
        assert!(
            !was_cancelled,
            "live turn must report not-cancelled at bind"
        );
    }

    #[tokio::test]
    async fn bind_agent_returns_true_when_cancel_landed_during_fork() {
        let reg = TurnRegistry::new();
        let _token = reg.open("sess-5");
        reg.cancel("sess-5");
        // The fork landed AFTER cancel — caller MUST kill the agent.
        let was_cancelled = reg.bind_agent("sess-5", uuid::Uuid::new_v4());
        assert!(
            was_cancelled,
            "bind after cancel must surface the cancel so the caller can kill the forked agent"
        );
    }

    #[test]
    fn bind_agent_returns_false_after_close() {
        let reg = TurnRegistry::new();
        reg.open("sess-6");
        reg.close("sess-6");
        let was_cancelled = reg.bind_agent("sess-6", uuid::Uuid::new_v4());
        assert!(!was_cancelled, "no entry means nothing to bind or cancel");
    }
}
