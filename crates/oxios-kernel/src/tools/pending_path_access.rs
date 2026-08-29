//! Pending path-access requests — interactive escalation when an agent tries
//! to read/write a path outside its `allowed_paths`.
//!
//! Mirrors [`PendingToolApprovals`] but carries path-specific context
//! (path, agent_name, mode) so the resolve endpoint can add a temporary
//! `allowed_paths` entry or record the denial before unblocking the GatedTool.
//!
//! Pattern: identical to `pending_tool_approvals.rs` (RFC-017 / RFC-035).

use parking_lot::Mutex;
use std::collections::HashMap;
use tokio::sync::oneshot;
use uuid::Uuid;

/// Result of a path-access request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathAccessResult {
    /// Path was granted (temp-allowed) — re-check and proceed.
    Allowed,
    /// User declined — return the original denial error to the agent.
    Denied,
}

struct PathAccessEntry {
    /// The tool that tried to access the path (read, write …).
    tool_name: String,
    /// The denied absolute path.
    path: String,
    /// Access mode: "read" or "write".
    #[allow(dead_code)]
    mode: String,
    /// The agent whose `allowed_paths` must be updated.
    agent_name: String,
    sender: oneshot::Sender<PathAccessResult>,
}

/// Thread-safe registry of in-flight path-access requests.
#[derive(Default)]
pub struct PendingPathAccess {
    inner: Mutex<HashMap<Uuid, PathAccessEntry>>,
}

impl PendingPathAccess {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new pending path-access request.
    /// Returns the request ID and a receiver to await the user's decision.
    pub fn register(
        &self,
        tool_name: String,
        path: String,
        mode: String,
        agent_name: String,
    ) -> (Uuid, oneshot::Receiver<PathAccessResult>) {
        let id = Uuid::new_v4();
        let (tx, rx) = oneshot::channel();
        self.inner.lock().insert(
            id,
            PathAccessEntry {
                tool_name,
                path,
                mode,
                agent_name,
                sender: tx,
            },
        );
        (id, rx)
    }

    /// Resolve a pending request with the user's decision.
    /// Returns `(tool_name, path, agent_name)` if the entry existed.
    pub fn resolve(&self, id: Uuid, result: PathAccessResult) -> Option<(String, String, String)> {
        let entry = self.inner.lock().remove(&id)?;
        let _ = entry.sender.send(result);
        Some((entry.tool_name, entry.path, entry.agent_name))
    }

    /// Look up the path context for a pending request without resolving it.
    /// Returns `(agent_name, path)` — used by the respond endpoint to know
    /// which agent's `allowed_paths` to update before resolving.
    pub fn path_context(&self, id: Uuid) -> Option<(String, String)> {
        let guard = self.inner.lock();
        let entry = guard.get(&id)?;
        Some((entry.agent_name.clone(), entry.path.clone()))
    }

    /// Cancel all pending entries (e.g., on shutdown).
    pub fn cancel_all(&self) {
        let mut guard = self.inner.lock();
        for (_, entry) in guard.drain() {
            let _ = entry.sender.send(PathAccessResult::Denied);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_resolve_round_trip() {
        let registry = PendingPathAccess::new();
        let (id, rx) = registry.register(
            "read".to_string(),
            "/secret/file".to_string(),
            "read".to_string(),
            "agent-1".to_string(),
        );
        assert_eq!(
            registry.path_context(id),
            Some(("agent-1".to_string(), "/secret/file".to_string()))
        );
        let resolved = registry.resolve(id, PathAccessResult::Allowed);
        assert_eq!(
            resolved,
            Some((
                "read".to_string(),
                "/secret/file".to_string(),
                "agent-1".to_string()
            ))
        );
        let result = rx.blocking_recv().expect("oneshot did not deliver");
        assert_eq!(result, PathAccessResult::Allowed);
    }

    #[test]
    fn resolve_missing_returns_none() {
        let registry = PendingPathAccess::new();
        let missing = Uuid::new_v4();
        assert!(
            registry
                .resolve(missing, PathAccessResult::Denied)
                .is_none()
        );
    }
}
