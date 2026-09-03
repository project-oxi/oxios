//! Runtime extension manager — allocation, reuse, and cleanup of the
//! host-side coding services a resolved execution requested (design
//! "Runtime extensions and session lifecycle").
//!
//! The manager is the first consumer of the general extension-lifecycle
//! contract: it owns only resources allocated on behalf of a resolved
//! execution. Supervisor / `AgentLifecycleManager` keep lifecycle authority
//! over agents themselves.
//!
//! Session key: `(project binding, stable chat session id, canonicalized
//! workspace)`. A workspace change produces a different key, so shell and
//! hashline state never silently survive an authorized workspace switch.
//!
//! Extension lifetimes (pilot scope):
//!
//! | Extension | Created | Torn down |
//! |---|---|---|
//! | Hashline snapshot store | coding session start (always — the pack requires it) | session end / workspace change / process exit |
//! | Shell session | first coding turn when `CodingServiceFlags::shell` | session end / workspace change; bounded by output limits |
//! | Eval kernels | first coding turn when `CodingServiceFlags::eval` | session end / workspace change |
//!
//! LSP, TTSR, debug, and delegation have no host-side implementation yet —
//! the pack install degrades them with structured records (the SDK install
//! loop reports `ServiceUnavailable`/`ExtensionUnavailable`), so the manager
//! never allocates them and reports nothing for them here.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use oxicode_agent::runtime::{EvalKernel, PersistentShellSession, ShellSession};
use oxicode_sdk::oxicode_hashline::{InMemorySnapshotStore, SnapshotStore};

use super::types::CodingServiceFlags;

/// Identity of one coding session's extensions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExtensionKey {
    /// Active project for the chat (from `ExecEnv.project_id`).
    pub project_id: Option<uuid::Uuid>,
    /// The shared turn key (`ExecEnv.session_id` / first-request fallback).
    pub session_id: String,
    /// Canonicalized workspace root for the turn.
    pub workspace: PathBuf,
}

/// Lifecycle status of one manager-owned extension, mirrored to
/// `KernelEvent::RuntimeExtensionStatus` by the caller.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExtensionStatus {
    /// Extension slug (design event payload `runtime_extension_status`).
    pub extension: String,
    /// `started` | `reused` | `stopped` | `failed`.
    pub state: String,
    /// Optional human-readable detail (no secrets).
    pub detail: Option<String>,
}

/// The manager-owned services one coding session runs with.
#[derive(Clone)]
pub struct CodingExtensions {
    /// Hashline snapshot store (file/edit-anchor state). Always allocated —
    /// the coding pack requires the HashlineState extension.
    pub snapshot_store: Arc<dyn SnapshotStore>,
    /// Persistent shell session, when requested and started.
    pub shell_session: Option<Arc<dyn ShellSession>>,
    /// Persistent eval kernels (Python, JavaScript), when requested.
    pub eval_kernels: Vec<Arc<dyn EvalKernel>>,
    /// What the manager did this acquire/release.
    pub statuses: Vec<ExtensionStatus>,
}

impl std::fmt::Debug for CodingExtensions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodingExtensions")
            .field("snapshot_store", &self.snapshot_store)
            .field("shell_session", &self.shell_session)
            .field("eval_kernels", &self.eval_kernels.len())
            .field("statuses", &self.statuses)
            .finish()
    }
}

/// Allocates, reuses, and cleans the session-scoped coding services.
#[derive(Default)]
pub struct RuntimeExtensionManager {
    sessions: parking_lot::RwLock<HashMap<ExtensionKey, CodingExtensions>>,
}

impl RuntimeExtensionManager {
    /// Create-or-reuse the extensions for `key`.
    ///
    /// The hashline snapshot store is always allocated (the coding pack
    /// requires HashlineState); shell and eval follow `flags`. A reused key
    /// returns the SAME `Arc`s — persistent shell cwd/env and anchored-edit
    /// history survive across the turns of one session.
    pub fn acquire(&self, key: ExtensionKey, flags: CodingServiceFlags) -> CodingExtensions {
        {
            let sessions = self.sessions.read();
            if let Some(existing) = sessions.get(&key) {
                let mut statuses = Vec::with_capacity(3);
                statuses.push(ExtensionStatus {
                    extension: "hashline".into(),
                    state: "reused".into(),
                    detail: None,
                });
                if existing.shell_session.is_some() {
                    statuses.push(ExtensionStatus {
                        extension: "shell".into(),
                        state: "reused".into(),
                        detail: None,
                    });
                }
                if !existing.eval_kernels.is_empty() {
                    statuses.push(ExtensionStatus {
                        extension: "eval".into(),
                        state: "reused".into(),
                        detail: None,
                    });
                }
                let mut ext = existing.clone();
                ext.statuses = statuses;
                return ext;
            }
        }

        let mut statuses = Vec::with_capacity(3);
        let snapshot_store: Arc<dyn SnapshotStore> = Arc::new(InMemorySnapshotStore::new());
        statuses.push(ExtensionStatus {
            extension: "hashline".into(),
            state: "started".into(),
            detail: None,
        });

        let shell_session: Option<Arc<dyn ShellSession>> = if flags.shell {
            statuses.push(ExtensionStatus {
                extension: "shell".into(),
                state: "started".into(),
                detail: None,
            });
            Some(Arc::new(PersistentShellSession::new(key.workspace.clone())))
        } else {
            None
        };

        let mut eval_kernels: Vec<Arc<dyn EvalKernel>> = Vec::new();
        if flags.eval {
            eval_kernels.push(Arc::new(oxicode_agent::runtime::PythonEvalKernel::new()));
            eval_kernels.push(Arc::new(oxicode_agent::runtime::JavaScriptEvalKernel::new()));
            statuses.push(ExtensionStatus {
                extension: "eval".into(),
                state: "started".into(),
                detail: None,
            });
        }

        let ext = CodingExtensions {
            snapshot_store,
            shell_session,
            eval_kernels,
            statuses,
        };
        self.sessions.write().insert(key, ext.clone());
        ext
    }

    /// Drop one session key's extensions. The shell is cancelled synchronously
    /// (kills any running command); its async `reset` runs best-effort in a
    /// spawned task so a missing runtime context cannot block cleanup.
    pub fn release(&self, key: &ExtensionKey) {
        if let Some(ext) = self.sessions.write().remove(key)
            && let Some(shell) = &ext.shell_session
        {
            shell.cancel();
            // Best-effort async reset; a missing runtime context (sync
            // tests, teardown paths) skips it — dropping the session's
            // last Arc tears the shell process down with it.
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let shell = shell.clone();
                handle.spawn(async move {
                    let _ = shell.reset().await;
                });
            }
        }
        // Snapshot store + eval kernels drop with the Arcs.
    }

    /// Release every extension set bound to `session_id` — session end or an
    /// authorized workspace switch (the new workspace forms a new key).
    pub fn release_session(&self, session_id: &str) {
        let keys: Vec<ExtensionKey> = {
            let sessions = self.sessions.read();
            sessions
                .keys()
                .filter(|k| k.session_id == session_id)
                .cloned()
                .collect()
        };
        for key in keys {
            self.release(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(workspace: &str) -> ExtensionKey {
        ExtensionKey {
            project_id: None,
            session_id: "chat-1".into(),
            workspace: PathBuf::from(workspace),
        }
    }

    fn flags(shell: bool, eval: bool) -> CodingServiceFlags {
        CodingServiceFlags { shell, eval }
    }

    #[test]
    fn same_key_reuses_the_same_store() {
        let mgr = RuntimeExtensionManager::default();
        let first = mgr.acquire(key("/tmp/w"), flags(true, true));
        assert_eq!(first.statuses[0].state, "started");
        let second = mgr.acquire(key("/tmp/w"), flags(true, true));
        assert_eq!(second.statuses[0].state, "reused");
        assert_eq!(second.statuses[1].state, "reused");
        assert_eq!(second.statuses[2].state, "reused");
        // Same store arc: the anchored-edit history survives across turns.
        let a: Arc<dyn SnapshotStore> = first.snapshot_store.clone();
        let b: Arc<dyn SnapshotStore> = second.snapshot_store.clone();
        assert!(Arc::ptr_eq(&a, &b));
        let sa = first.shell_session.clone().expect("shell");
        let sb = second.shell_session.clone().expect("shell");
        assert!(Arc::ptr_eq(&sa, &sb));
    }

    #[test]
    fn different_workspace_allocates_fresh_state() {
        let mgr = RuntimeExtensionManager::default();
        let a = mgr.acquire(key("/tmp/w1"), flags(false, false));
        let b = mgr.acquire(key("/tmp/w2"), flags(false, false));
        let x: Arc<dyn SnapshotStore> = a.snapshot_store.clone();
        let y: Arc<dyn SnapshotStore> = b.snapshot_store.clone();
        assert!(!Arc::ptr_eq(&x, &y));
        assert!(a.shell_session.is_none());
        assert_eq!(b.statuses[0].state, "started");
    }

    #[test]
    fn release_then_acquire_starts_fresh() {
        let mgr = RuntimeExtensionManager::default();
        let _ = mgr.acquire(key("/tmp/w"), flags(true, false));
        mgr.release(&key("/tmp/w"));
        let again = mgr.acquire(key("/tmp/w"), flags(true, false));
        assert!(again.statuses.iter().all(|s| s.state == "started"));
    }

    #[test]
    fn flags_off_skip_services_without_degradation_here() {
        let mgr = RuntimeExtensionManager::default();
        let ext = mgr.acquire(key("/tmp/w"), flags(false, false));
        assert!(ext.shell_session.is_none());
        assert!(ext.eval_kernels.is_empty());
        // Only hashline runs; the SDK install loop reports the pack-level
        // degradations for shell/eval/lsp/debug/ttsr/delegation.
        assert_eq!(ext.statuses.len(), 1);
        assert_eq!(ext.statuses[0].extension, "hashline");
    }

    #[test]
    fn release_session_clears_every_workspace_for_that_chat() {
        let mgr = RuntimeExtensionManager::default();
        let _ = mgr.acquire(key("/tmp/w1"), flags(false, false));
        let _ = mgr.acquire(key("/tmp/w2"), flags(false, false));
        let mut other = key("/tmp/w3");
        other.session_id = "chat-2".into();
        let _ = mgr.acquire(other.clone(), flags(false, false));
        mgr.release_session("chat-1");
        assert!(mgr.sessions.read().get(&key("/tmp/w1")).is_none());
        assert!(mgr.sessions.read().get(&key("/tmp/w2")).is_none());
        assert!(mgr.sessions.read().get(&other).is_some());
    }
}
