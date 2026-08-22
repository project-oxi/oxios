//! Access Manager — least-privilege security for agents.
//!
//! Inspired by OWASP Agentic AI security guidelines:
//! - Least privilege by default
//! - Agent identity and audit logging
//! - Sandbox boundaries (path restrictions)
//! - Tool access control (which agent can use which tools)
//!
//! Every agent starts with minimal permissions and must be explicitly granted
//! access to tools, paths, and network resources.

mod audit_sink;
mod context;
mod gate;
mod permissions;
mod rbac;

#[cfg(test)]
pub use audit_sink::NoOpAuditSink;
pub use audit_sink::{AuditEvent, AuditSink, TracingAuditSink, TrailAuditSink};
pub use context::AgentContext;
pub use gate::{
    AccessDenied, AccessGate, CheckRequest, DenyLayer, OXI_HOME_DENY_ROOTS,
    OXIOS_HOME_DENY_SUBPATHS, PathMode,
};
pub use permissions::{AgentPermissions, AuditEntry, PermissionUpdate};
pub use rbac::{
    Action, ApprovalStatus, PendingApproval, RbacAuditEntry, RbacManager, RbacPolicy, Role, Subject,
};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::types::AgentId;

/// Access Manager — least-privilege security for agents.
///
/// # Example
///
/// ```no_run
/// use oxios_kernel::access_manager::{AccessManager, AgentPermissions};
///
/// let mut access = AccessManager::new();
///
/// // Create and configure permissions for a new agent
/// let mut perms = AgentPermissions::for_new_agent("code-agent");
/// perms.allow_tool("read");
/// perms.allow_tool("write");
/// perms.allow_path("/workspace/");
/// access.set_permissions(perms);
///
/// // Check tool access
/// let can_exec = access.can_use_tool("code-agent", "exec");
/// let can_read = access.can_use_tool("code-agent", "read");
///
/// // Check path access
/// let can_read_workspace = access.can_access_path("code-agent", "/workspace/file.rs");
/// let can_read_root = access.can_access_path("code-agent", "/etc/passwd");
/// ```
/// Access Manager — least-privilege security for agents.
// NOTE: Clone is derived for ExecTool compatibility (Phase 1).
// Clone is cheap — only HashMaps of primitives, no external resources.
#[derive(Debug, Clone)]
pub struct AccessManager {
    /// Permissions for each agent.
    permissions: HashMap<String, AgentPermissions>,
    /// Audit log of all access decisions.
    audit_log: Vec<AuditEntry>,
    /// Optional path for audit log file persistence.
    #[allow(dead_code)]
    audit_log_path: Option<std::path::PathBuf>,
    /// Maximum audit log entries to retain.
    max_audit_entries: usize,
    /// RBAC manager for HitL approvals.
    pub(crate) rbac: RbacManager,
    /// Workspace paths: workspace_name -> workspace_path.
    workspace_paths: HashMap<String, PathBuf>,
    /// Agent-to-workspace assignments: agent_name -> workspace_name.
    agent_workspaces: HashMap<String, String>,
    /// Workspace-to-agents mapping: workspace_name -> set of agent_names.
    workspace_agents: HashMap<String, HashSet<String>>,
    /// Bounded channel sender for async audit log writes.
    audit_sender: Option<tokio::sync::mpsc::Sender<String>>,
    /// Handle to the background audit writer task.
    #[allow(dead_code)]
    audit_writer_handle: Option<Arc<tokio::task::JoinHandle<()>>>,
}

impl AccessManager {
    /// Creates a new access manager.
    pub fn new() -> Self {
        Self {
            permissions: HashMap::new(),
            audit_log: Vec::new(),
            audit_log_path: None,
            max_audit_entries: 10_000,
            rbac: RbacManager::new(),
            workspace_paths: HashMap::new(),
            agent_workspaces: HashMap::new(),
            workspace_agents: HashMap::new(),
            audit_sender: None,
            audit_writer_handle: None,
        }
    }

    /// Creates a new access manager with custom settings.
    ///
    /// # Arguments
    /// * `max_audit_entries` - Maximum audit log size (oldest entries are pruned)
    pub fn with_max_audit_entries(max_audit_entries: usize) -> Self {
        Self {
            permissions: HashMap::new(),
            audit_log: Vec::new(),
            audit_log_path: None,
            max_audit_entries,
            rbac: RbacManager::new(),
            workspace_paths: HashMap::new(),
            agent_workspaces: HashMap::new(),
            workspace_agents: HashMap::new(),
            audit_sender: None,
            audit_writer_handle: None,
        }
    }

    /// Configure an audit log file path for persistence.
    ///
    /// Spawns a background tokio task that reads from a bounded channel
    /// and appends audit entries to the file. Uses a bounded channel of
    /// capacity 1000 to provide backpressure.
    pub fn with_audit_log_path(mut self, path: std::path::PathBuf) -> Self {
        self.audit_log_path = Some(path.clone());

        // Create the bounded channel (capacity 1000)
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(1000);
        self.audit_sender = Some(tx);

        // Spawn the background writer task using the current tokio runtime
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let audit_path = path;
            let audit_handle = handle.spawn(async move {
                while let Some(line) = rx.recv().await {
                    #[cfg(unix)]
                    let result = {
                        use std::os::unix::fs::OpenOptionsExt;
                        std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .mode(0o600)
                            .open(&audit_path)
                    };
                    #[cfg(not(unix))]
                    let result = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&audit_path);
                    if let Ok(mut f) = result {
                        use std::io::Write;
                        let _ = writeln!(f, "{line}");
                    }
                }
            });
            self.audit_writer_handle = Some(Arc::new(audit_handle));
        }

        self
    }

    /// Checks if an agent is allowed to use a specific tool.
    ///
    /// Logs the access decision to the audit log.
    pub fn can_use_tool(&mut self, agent_name: &str, tool: &str) -> bool {
        let allowed = match self.permissions.get(agent_name) {
            Some(perms) => perms.allowed_tools.contains(tool),
            None => {
                tracing::warn!(agent = %agent_name, tool = %tool, "Agent not found in access manager, denying");
                false
            }
        };

        let reason = if allowed {
            None
        } else {
            Some("tool not in allowed set".to_string())
        };

        self.log_access(agent_name, "use_tool", tool, allowed, reason);

        allowed
    }

    /// Checks if an agent is allowed to access a specific path.
    ///
    /// Enforces both allowed_paths and denied_paths rules.
    /// A path is only allowed if it matches an allowed pattern AND
    /// does not match any denied pattern.
    ///
    /// Logs the access decision to the audit log.
    pub fn can_access_path(&mut self, agent_name: &str, path: &str) -> bool {
        let allowed = match self.permissions.get(agent_name) {
            Some(perms) => {
                // First check denials (they take precedence).
                if perms.is_path_denied(path) {
                    false
                } else {
                    perms.is_path_allowed(path)
                }
            }
            None => {
                tracing::warn!(agent = %agent_name, path = %path, "Agent not found, denying path access");
                false
            }
        };

        let reason = if allowed {
            None
        } else {
            Some("path not in allowed set or is denied".to_string())
        };

        self.log_access(agent_name, "access_path", path, allowed, reason);

        allowed
    }

    /// Checks if an agent is allowed to make network requests.
    ///
    /// Logs the access decision to the audit log.
    pub fn can_access_network(&mut self, agent_name: &str) -> bool {
        let allowed = match self.permissions.get(agent_name) {
            Some(perms) => perms.network_access,
            None => false,
        };

        let reason = if allowed {
            None
        } else {
            Some("network access not enabled".to_string())
        };

        self.log_access(agent_name, "network_request", "<network>", allowed, reason);

        allowed
    }

    /// Checks if an agent can execute for the given duration (in seconds).
    ///
    /// Returns true if unlimited (max_execution_time_secs = 0) or
    /// if the requested duration is within the limit.
    pub fn can_execute_for(&self, agent_name: &str, duration_secs: u64) -> bool {
        match self.permissions.get(agent_name) {
            Some(perms) => {
                perms.max_execution_time_secs == 0 || duration_secs <= perms.max_execution_time_secs
            }
            None => false,
        }
    }

    /// Checks if an agent can use the specified amount of memory (in MB).
    ///
    /// Returns true if unlimited (max_memory_mb = 0) or
    /// if the requested memory is within the limit.
    pub fn can_use_memory(&self, agent_name: &str, memory_mb: u64) -> bool {
        match self.permissions.get(agent_name) {
            Some(perms) => perms.max_memory_mb == 0 || memory_mb <= perms.max_memory_mb,
            None => false,
        }
    }

    /// Checks if an agent can fork (spawn sub-agents).
    pub fn can_fork(&self, agent_name: &str) -> bool {
        match self.permissions.get(agent_name) {
            Some(perms) => perms.can_fork,
            None => false,
        }
    }

    /// Gets the permission set for an agent.
    ///
    /// Returns None if no permissions are defined for the agent.
    pub fn get_permissions(&self, agent_name: &str) -> Option<&AgentPermissions> {
        self.permissions.get(agent_name)
    }

    /// Gets the permission set for an agent, creating a default one if needed.
    ///
    /// Useful for dynamically creating permissions on first access.
    pub fn get_or_create_permissions(&mut self, agent_name: &str) -> &mut AgentPermissions {
        self.permissions
            .entry(agent_name.to_string())
            .or_insert_with(|| AgentPermissions::for_new_agent(agent_name))
    }

    /// Sets the permissions for an agent.
    ///
    /// Overwrites any existing permissions for this agent.
    pub fn set_permissions(&mut self, permissions: AgentPermissions) {
        let agent_name = permissions.agent_name.clone();
        self.permissions.insert(agent_name, permissions);
    }

    /// Updates permissions for an agent using a partial update.
    ///
    /// Creates default permissions if the agent doesn't exist.
    /// Only updates fields that are Some() in the update.
    pub fn update_permissions(
        &mut self,
        agent_name: &str,
        update: PermissionUpdate,
    ) -> anyhow::Result<()> {
        let perms = self
            .permissions
            .entry(agent_name.to_string())
            .or_insert_with(|| AgentPermissions::for_new_agent(agent_name));
        update.apply(perms);
        Ok(())
    }

    /// Removes permissions for an agent.
    ///
    /// After removal, all access by this agent will be denied.
    pub fn remove_permissions(&mut self, agent_name: &str) {
        self.permissions.remove(agent_name);
        tracing::info!(agent = %agent_name, "Agent permissions removed");
    }

    /// Lists all agents with defined permissions.
    pub fn list_agents(&self) -> Vec<String> {
        self.permissions.keys().cloned().collect()
    }

    /// Gets the full audit log.
    ///
    /// Returns entries in chronological order (oldest first).
    pub fn audit_log(&self) -> &[AuditEntry] {
        &self.audit_log
    }

    /// Gets recent audit log entries.
    ///
    /// # Arguments
    /// * `limit` - Maximum number of entries to return (from the end)
    pub fn audit_log_recent(&self, limit: usize) -> Vec<AuditEntry> {
        let start = self.audit_log.len().saturating_sub(limit);
        self.audit_log[start..].to_vec()
    }

    /// Gets audit log entries for a specific agent.
    pub fn audit_log_for_agent(&self, agent_name: &str) -> Vec<AuditEntry> {
        self.audit_log
            .iter()
            .filter(|e| e.agent_name == agent_name)
            .cloned()
            .collect()
    }

    /// Searches audit log for denied actions.
    pub fn denied_actions(&self) -> Vec<&AuditEntry> {
        self.audit_log.iter().filter(|e| !e.allowed).collect()
    }

    /// Returns a reference to the RBAC manager (for HitL approvals).
    pub fn rbac_manager(&self) -> &RbacManager {
        &self.rbac
    }

    /// Returns a mutable reference to the RBAC manager (for HitL approvals).
    pub fn rbac_manager_mut(&mut self) -> &mut RbacManager {
        &mut self.rbac
    }

    // ─── Workspace Sandbox Integration ────────────────────────────────────

    /// Registers a workspace path.
    ///
    /// This is used to report which paths belong to each workspace.
    ///
    /// # Arguments
    /// * `workspace_name` - Name of the workspace
    /// * `workspace_path` - Absolute path to the workspace directory
    pub fn register_workspace_path(&mut self, workspace_name: &str, workspace_path: PathBuf) {
        self.workspace_paths
            .insert(workspace_name.to_string(), workspace_path);
        tracing::debug!(workspace = %workspace_name, "Workspace path registered");
    }

    /// Assigns an agent to a specific workspace.
    ///
    /// After assignment, the agent is sandboxed to that workspace.
    ///
    /// # Arguments
    /// * `agent_name` - Name of the agent to assign
    /// * `workspace_name` - Name of the workspace to assign the agent to
    ///
    /// # Returns
    /// `true` if assignment succeeded, `false` if the workspace doesn't exist
    pub fn assign_workspace(&mut self, agent_name: &str, workspace_name: &str) -> bool {
        if !self.workspace_paths.contains_key(workspace_name) {
            tracing::warn!(agent = %agent_name, workspace = %workspace_name, "Cannot assign agent to non-existent workspace");
            return false;
        }

        // Remove from previous workspace if any
        if let Some(prev_workspace) = self.agent_workspaces.get(agent_name)
            && let Some(agents) = self.workspace_agents.get_mut(prev_workspace)
        {
            agents.remove(agent_name);
        }

        // Assign to new workspace
        self.agent_workspaces
            .insert(agent_name.to_string(), workspace_name.to_string());
        self.workspace_agents
            .entry(workspace_name.to_string())
            .or_default()
            .insert(agent_name.to_string());

        tracing::info!(agent = %agent_name, workspace = %workspace_name, "Agent assigned to workspace");
        true
    }

    /// Gets the workspace name that an agent is assigned to.
    ///
    /// # Returns
    /// `Some(workspace_name)` if assigned, `None` if the agent is not assigned to any workspace
    pub fn get_workspace_for_agent(&self, agent_name: &str) -> Option<String> {
        self.agent_workspaces.get(agent_name).cloned()
    }

    /// Gets the path for a specific workspace.
    ///
    /// # Returns
    /// `Some(path)` if the workspace exists, `None` otherwise
    pub fn get_workspace_path(&self, workspace_name: &str) -> Option<&PathBuf> {
        self.workspace_paths.get(workspace_name)
    }

    /// Lists all registered workspaces.
    pub fn list_workspaces(&self) -> Vec<String> {
        self.workspace_paths.keys().cloned().collect()
    }

    /// Lists all agents assigned to a specific workspace.
    pub fn list_agents_in_workspace(&self, workspace_name: &str) -> Vec<String> {
        self.workspace_agents
            .get(workspace_name)
            .map(|agents| agents.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Checks if an agent can access a specific workspace.
    ///
    /// An agent can access a workspace if it is assigned to it.
    ///
    /// # Arguments
    /// * `agent_name` - Name of the agent
    /// * `workspace_name` - Name of the workspace to check
    ///
    /// # Returns
    /// `true` if the agent is assigned to the workspace, `false` otherwise
    pub fn can_access_workspace(&self, agent_name: &str, workspace_name: &str) -> bool {
        self.agent_workspaces
            .get(agent_name)
            .map(|w| w == workspace_name)
            .unwrap_or(false)
    }

    /// Checks if a path is within a workspace's directory.
    ///
    /// This performs a canonical path comparison to ensure the path is
    /// a descendant of the workspace directory.
    ///
    /// # Arguments
    /// * `workspace_name` - Name of the workspace
    /// * `path` - Path to check (absolute or relative)
    ///
    /// # Returns
    /// `true` if the path is within the workspace, `false` otherwise
    pub fn is_path_in_workspace(&self, workspace_name: &str, path: &str) -> bool {
        let workspace = match self.workspace_paths.get(workspace_name) {
            Some(w) => w,
            None => return false,
        };

        // Resolve the path to an absolute canonical path
        let requested_path = match Path::new(path).canonicalize() {
            Ok(p) => p,
            Err(_) => {
                // If we can't canonicalize, try as relative to workspace
                let candidate = workspace.join(path);
                match candidate.canonicalize() {
                    Ok(p) => p,
                    Err(_) => return false,
                }
            }
        };

        // Check if the canonical path starts with the workspace
        let workspace_canonical = match workspace.canonicalize() {
            Ok(w) => w,
            Err(_) => return false,
        };

        requested_path.starts_with(&workspace_canonical)
    }

    /// Full sandbox check: RBAC → path allowed? → within workspace?
    ///
    /// This is the main method for enforcing sandbox boundaries. It checks:
    /// 1. RBAC - does the agent's role allow the action?
    /// 2. Path permissions - is the path in the agent's allowed_paths?
    /// 3. Workspace boundary - is the path within the assigned workspace?
    ///
    /// If any check fails, the access is denied and logged as "sandbox violation"
    /// if the path would be valid but outside the workspace boundary.
    ///
    /// # Arguments
    /// * `agent_name` - Name of the agent
    /// * `path` - Path to access
    /// * `workspace` - Workspace context (if agent is assigned to one)
    ///
    /// # Returns
    /// `true` if all checks pass, `false` otherwise
    /// Check whether an agent can access a path within its workspace.
    ///
    /// Verifies RBAC permissions, path allow/deny lists, and workspace boundary.
    ///
    /// # Arguments
    /// * `agent_id` - The agent's unique identifier
    /// * `agent_name` - Human-readable agent name for permission lookup
    /// * `path` - The path to check access for
    /// * `workspace` - Optional explicit workspace name
    pub fn can_access_path_in_workspace(
        &mut self,
        agent_id: &AgentId,
        agent_name: &str,
        path: &str,
        workspace: Option<&str>,
    ) -> bool {
        // RBAC check using the actual agent_id
        let subject = Subject::Agent(*agent_id);
        let action = Action::AccessPath(path.to_string());
        let rbac_allowed = self.rbac.check_permission(&subject, &action, path);

        // Check path permissions (allowed_paths vs denied_paths)
        let path_allowed = self.can_access_path(agent_name, path);

        // Check workspace boundary
        let workspace_allowed = if let Some(workspace_name) = workspace {
            let is_in_workspace = self.is_path_in_workspace(workspace_name, path);

            if !is_in_workspace {
                // Log as sandbox violation
                self.log_access(
                    agent_name,
                    "sandbox_violation",
                    path,
                    false,
                    Some(format!(
                        "Path '{path}' is outside workspace '{workspace_name}' boundary"
                    )),
                );
            }

            is_in_workspace
        } else {
            // No workspace context - check if agent has any workspace assignment
            if let Some(assigned_workspace) = self.agent_workspaces.get(agent_name) {
                let is_in_workspace = self.is_path_in_workspace(assigned_workspace, path);

                if !is_in_workspace {
                    self.log_access(
                        agent_name,
                        "sandbox_violation",
                        path,
                        false,
                        Some(format!(
                            "Path '{path}' is outside assigned workspace '{assigned_workspace}' boundary"
                        )),
                    );
                }

                is_in_workspace
            } else {
                // Agent has no workspace assignment - default to allowing path check only
                true
            }
        };

        // All three checks must pass
        rbac_allowed && path_allowed && workspace_allowed
    }

    /// Unassigns an agent from its workspace (if any).
    ///
    /// The agent will no longer be sandboxed to any workspace.
    pub fn unassign_workspace(&mut self, agent_name: &str) -> Option<String> {
        if let Some(workspace_name) = self.agent_workspaces.remove(agent_name) {
            if let Some(agents) = self.workspace_agents.get_mut(&workspace_name) {
                agents.remove(agent_name);
            }
            tracing::info!(agent = %agent_name, workspace = %workspace_name, "Agent unassigned from workspace");
            Some(workspace_name)
        } else {
            None
        }
    }

    /// Removes a workspace and unassigns all agents from it.
    ///
    /// All agents assigned to this workspace will have their assignments cleared.
    pub fn remove_workspace(&mut self, workspace_name: &str) {
        // Unassign all agents from this workspace
        if let Some(agents) = self.workspace_agents.remove(workspace_name) {
            for agent_name in agents {
                self.agent_workspaces.remove(&agent_name);
            }
        }

        // Remove the workspace path
        self.workspace_paths.remove(workspace_name);

        tracing::info!(workspace = %workspace_name, "Workspace removed from access manager");
    }

    /// Clears the audit log.
    ///
    /// The clear itself is recorded as an audit entry first, so the deletion is
    /// visible in the trail rather than silently wiping history.
    pub fn clear_audit_log(&mut self) {
        let count = self.audit_log.len();
        self.log_access(
            "system",
            "audit_clear",
            &format!("{count} entries"),
            true,
            Some("audit log cleared by request".to_string()),
        );
        self.audit_log.clear();
        tracing::info!(cleared = count, "Audit log cleared");
    }

    /// Logs an access decision to the audit log.
    ///
    /// Automatically prunes old entries if max_audit_entries is exceeded.
    /// Persists to file if audit_log_path is configured.
    pub(crate) fn log_access(
        &mut self,
        agent_name: &str,
        action: &str,
        resource: &str,
        allowed: bool,
        reason: Option<String>,
    ) {
        let entry = AuditEntry::new(agent_name, action, resource, allowed, reason.clone());

        self.audit_log.push(entry.clone());

        // Prune if needed.
        if self.audit_log.len() > self.max_audit_entries {
            let prune_count = self.audit_log.len() - self.max_audit_entries;
            self.audit_log.drain(0..prune_count);
        }

        // Persist to file (fire-and-forget).
        self.persist_audit_entry(&entry);

        // Trace denied actions at warn level.
        if !allowed {
            tracing::warn!(
                agent = %agent_name,
                action = %action,
                resource = %resource,
                reason = ?reason,
                "Access denied"
            );
        }
    }

    /// Persists an audit entry to the configured log file.
    ///
    /// Sends the serialized entry through the bounded channel to the
    /// background writer task. If the channel is full, a warning is logged
    /// and the entry is dropped (better than blocking the caller).
    fn persist_audit_entry(&self, entry: &AuditEntry) {
        if self.audit_log_path.is_none() {
            return;
        }
        let line = match serde_json::to_string(entry) {
            Ok(s) => s,
            Err(_) => return,
        };
        if let Some(sender) = &self.audit_sender {
            match sender.try_send(line) {
                Ok(()) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!("Audit log channel full — dropping entry");
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    tracing::warn!("Audit log channel closed — dropping entry");
                }
            }
        }
    }

    /// Validates a permission set for correctness.
    ///
    /// Returns a list of warnings about the permissions.
    pub fn validate_permissions(&self, perms: &AgentPermissions) -> Vec<String> {
        let mut warnings = Vec::new();

        if perms.allowed_tools.is_empty() {
            warnings.push("Agent has no allowed tools".to_string());
        }

        if perms.allowed_paths.is_empty() {
            warnings.push(
                "Agent has no path restrictions (paths granted by ensure_permissions)".to_string(),
            );
        }

        if perms.network_access {
            warnings.push("Agent has network access enabled".to_string());
        }

        if perms.can_fork {
            warnings.push("Agent can fork sub-agents".to_string());
        }

        if perms.max_execution_time_secs == 0 {
            warnings.push("Agent has no execution time limit".to_string());
        }

        if perms.max_memory_mb == 0 {
            warnings.push("Agent has no memory limit".to_string());
        }

        warnings
    }
}

impl Default for AccessManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- AgentPermissions tests ---

    #[test]
    fn test_default_permissions() {
        let perms = AgentPermissions::default();
        assert!(perms.allowed_tools.contains("bash"));
        assert!(!perms.network_access);
        assert!(!perms.can_fork);
        assert_eq!(perms.max_execution_time_secs, 300);
        assert_eq!(perms.max_memory_mb, 512);
    }

    #[test]
    fn test_for_new_agent() {
        let perms = AgentPermissions::for_new_agent("my-agent");
        assert_eq!(perms.agent_name, "my-agent");
        assert!(perms.allowed_tools.contains("bash"));
    }

    #[test]
    fn test_allow_deny_tool() {
        let mut perms = AgentPermissions::for_new_agent("test");
        assert!(perms.allowed_tools.contains("bash"));

        perms.deny_tool("bash");
        assert!(!perms.allowed_tools.contains("bash"));

        perms.allow_tool("custom");
        assert!(perms.allowed_tools.contains("custom"));
    }

    #[test]
    fn test_allow_deny_path() {
        let mut perms = AgentPermissions::for_new_agent("test");

        perms.allow_path("/workspace/**");
        assert!(perms.allowed_paths.contains(&"/workspace/**".to_string()));

        perms.deny_path("/workspace/.secret/**");
        assert!(
            perms
                .denied_paths
                .contains(&"/workspace/.secret/**".to_string())
        );
    }

    #[test]
    fn test_enable_network() {
        let mut perms = AgentPermissions::for_new_agent("test");
        assert!(!perms.network_access);

        perms.enable_network();
        assert!(perms.network_access);
    }

    #[test]
    fn test_enable_forking() {
        let mut perms = AgentPermissions::for_new_agent("test");
        assert!(!perms.can_fork);

        perms.enable_forking();
        assert!(perms.can_fork);
    }

    #[test]
    fn test_path_matching_allowed() {
        let mut perms = AgentPermissions::for_new_agent("test");
        perms.allowed_paths = vec!["/workspace/**".to_string(), "/home/*/docs/**".to_string()];
        perms.denied_paths = vec!["/workspace/.oxios/**".to_string()];

        // Matches allowed.
        assert!(perms.is_path_allowed("/workspace/project/file.rs"));
        assert!(perms.is_path_allowed("/home/user/docs/readme.md"));

        // Does not match any allowed pattern.
        assert!(!perms.is_path_allowed("/etc/passwd"));
        assert!(!perms.is_path_allowed("/home/user/secret.txt"));
    }

    #[test]
    fn test_path_matching_denied() {
        let mut perms = AgentPermissions::for_new_agent("test");
        perms.allowed_paths = vec!["/workspace/**".to_string()];
        perms.denied_paths = vec!["/workspace/.oxios/**".to_string()];

        // Denied takes precedence over allowed.
        assert!(perms.is_path_denied("/workspace/.oxios/config.toml"));
        assert!(!perms.is_path_denied("/workspace/project/file.rs"));

        // Even though it matches allowed, denied blocks it.
        let _access = AccessManager::new();
        let mut perms2 = perms.clone();
        perms2.agent_name = "test".to_string();
        // The is_path_denied check happens first in can_access_path.
        // If allowed_paths matches but denied_paths also matches, it's blocked.
        // We test this via the full can_access_path method.
    }

    #[test]
    fn test_path_denied_pattern_matching() {
        let mut perms = AgentPermissions::for_new_agent("test");
        perms.denied_paths = vec!["/etc/**".to_string(), "**/secrets/*".to_string()];

        assert!(perms.is_path_denied("/etc/passwd"));
        assert!(perms.is_path_denied("/etc/shadow"));
        assert!(!perms.is_path_denied("/workspace/file"));
    }

    // --- AccessManager tool access tests ---

    #[test]
    fn test_can_use_tool_allowed() {
        let mut access = AccessManager::new();

        let mut perms = AgentPermissions::for_new_agent("code-agent");
        perms.allow_tool("bash");
        perms.allow_tool("read");
        access.set_permissions(perms);

        assert!(access.can_use_tool("code-agent", "bash"));
        assert!(access.can_use_tool("code-agent", "read"));
    }

    #[test]
    fn test_can_use_tool_denied() {
        let mut access = AccessManager::new();

        let mut perms = AgentPermissions::for_new_agent("code-agent");
        perms.allow_tool("read");
        perms.deny_tool("bash"); // Explicitly denied.
        access.set_permissions(perms);

        assert!(!access.can_use_tool("code-agent", "bash")); // denied
        assert!(!access.can_use_tool("code-agent", "spawn")); // not in list
        assert!(!access.can_use_tool("unknown-agent", "bash")); // unknown agent
    }

    #[test]
    fn test_unknown_agent_denied_all_tools() {
        let mut access = AccessManager::new();

        // No permissions set for unknown-agent.
        assert!(!access.can_use_tool("unknown-agent", "read"));
        assert!(!access.can_access_path("unknown-agent", "/workspace/test.txt"));
        assert!(!access.can_access_network("unknown-agent"));
        assert!(!access.can_fork("unknown-agent"));
    }

    // --- AccessManager path access tests ---

    #[test]
    fn test_can_access_path_allowed() {
        let mut access = AccessManager::new();

        let mut perms = AgentPermissions::for_new_agent("file-agent");
        perms.allow_path("/workspace/**");
        access.set_permissions(perms);

        assert!(access.can_access_path("file-agent", "/workspace/project/file.rs"));
        assert!(!access.can_access_path("file-agent", "/etc/passwd"));
    }

    #[test]
    fn test_can_access_path_denied_takes_precedence() {
        let mut access = AccessManager::new();

        let mut perms = AgentPermissions::for_new_agent("test");
        perms.allowed_paths = vec!["/workspace/**".to_string()];
        perms.denied_paths = vec!["/workspace/.oxios/**".to_string()];
        access.set_permissions(perms);

        // Allowed but also denied → blocked.
        assert!(!access.can_access_path("test", "/workspace/.oxios/config.toml"));

        // Just allowed → allowed.
        assert!(access.can_access_path("test", "/workspace/project/file.rs"));
    }

    // --- AccessManager network access tests ---

    #[test]
    fn test_can_access_network() {
        let mut access = AccessManager::new();

        let mut perms = AgentPermissions::for_new_agent("net-agent");
        perms.enable_network();
        access.set_permissions(perms);

        assert!(access.can_access_network("net-agent"));
        assert!(!access.can_access_network("no-net-agent"));
    }

    // --- Execution limits tests ---

    #[test]
    fn test_can_execute_for() {
        let mut access = AccessManager::new();

        let mut perms = AgentPermissions::for_new_agent("test");
        perms.max_execution_time_secs = 300;
        access.set_permissions(perms);

        assert!(access.can_execute_for("test", 100));
        assert!(access.can_execute_for("test", 300));
        assert!(!access.can_execute_for("test", 301));
    }

    #[test]
    fn test_unlimited_execution_time() {
        let mut access = AccessManager::new();

        let mut perms = AgentPermissions::for_new_agent("test");
        perms.max_execution_time_secs = 0; // unlimited
        access.set_permissions(perms);

        assert!(access.can_execute_for("test", 100_000));
    }

    #[test]
    fn test_can_use_memory() {
        let mut access = AccessManager::new();

        let mut perms = AgentPermissions::for_new_agent("test");
        perms.max_memory_mb = 512;
        access.set_permissions(perms);

        assert!(access.can_use_memory("test", 256));
        assert!(access.can_use_memory("test", 512));
        assert!(!access.can_use_memory("test", 513));
    }

    #[test]
    fn test_unlimited_memory() {
        let mut access = AccessManager::new();

        let mut perms = AgentPermissions::for_new_agent("test");
        perms.max_memory_mb = 0;
        access.set_permissions(perms);

        assert!(access.can_use_memory("test", 1_000_000));
    }

    // --- Fork tests ---

    #[test]
    fn test_can_fork() {
        let mut access = AccessManager::new();

        let mut perms = AgentPermissions::for_new_agent("test");
        perms.enable_forking();
        access.set_permissions(perms);

        assert!(access.can_fork("test"));
        assert!(!access.can_fork("no-fork-agent"));
    }

    // --- Permission management tests ---

    #[test]
    fn test_set_and_get_permissions() {
        let mut access = AccessManager::new();

        let perms = AgentPermissions::for_new_agent("test-agent");
        access.set_permissions(perms);

        let retrieved = access.get_permissions("test-agent");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().agent_name, "test-agent");
    }

    #[test]
    fn test_get_nonexistent_permissions() {
        let access = AccessManager::new();
        assert!(access.get_permissions("ghost").is_none());
    }

    #[test]
    fn test_get_or_create_permissions() {
        let mut access = AccessManager::new();

        // First access creates default.
        let perms = access.get_or_create_permissions("new-agent");
        assert_eq!(perms.agent_name, "new-agent");

        // Second access returns same instance.
        let perms2 = access.get_or_create_permissions("new-agent");
        assert_eq!(perms2.agent_name, "new-agent");
    }

    #[test]
    fn test_remove_permissions() {
        let mut access = AccessManager::new();

        let perms = AgentPermissions::for_new_agent("to-remove");
        access.set_permissions(perms);

        assert!(access.get_permissions("to-remove").is_some());

        access.remove_permissions("to-remove");

        assert!(access.get_permissions("to-remove").is_none());
        // All access should now be denied.
        assert!(!access.can_use_tool("to-remove", "bash"));
    }

    #[test]
    fn test_list_agents() {
        let mut access = AccessManager::new();

        access.set_permissions(AgentPermissions::for_new_agent("agent-1"));
        access.set_permissions(AgentPermissions::for_new_agent("agent-2"));

        let agents = access.list_agents();
        assert_eq!(agents.len(), 2);
        assert!(agents.contains(&"agent-1".to_string()));
        assert!(agents.contains(&"agent-2".to_string()));
    }

    // --- Audit log tests ---

    #[test]
    fn test_audit_log_records_access() {
        let mut access = AccessManager::new();

        let perms = AgentPermissions::for_new_agent("test-agent");
        access.set_permissions(perms);

        access.can_use_tool("test-agent", "bash"); // allowed
        access.can_use_tool("test-agent", "network"); // denied

        let log = access.audit_log();
        assert_eq!(log.len(), 2);
        assert!(log[0].allowed);
        assert!(!log[1].allowed);
        assert_eq!(log[0].agent_name, "test-agent");
        assert_eq!(log[0].action, "use_tool");
        assert_eq!(log[0].resource, "bash");
    }

    #[test]
    fn test_audit_log_recent() {
        let mut access = AccessManager::new();

        let perms = AgentPermissions::for_new_agent("test");
        access.set_permissions(perms);

        for i in 0..10 {
            access.can_use_tool("test", &format!("tool-{}", i));
        }

        let recent = access.audit_log_recent(3);
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn test_audit_log_for_agent() {
        let mut access = AccessManager::new();

        access.set_permissions(AgentPermissions::for_new_agent("agent-a"));
        access.set_permissions(AgentPermissions::for_new_agent("agent-b"));

        access.can_use_tool("agent-a", "tool1");
        access.can_use_tool("agent-b", "tool2");
        access.can_use_tool("agent-a", "tool3");

        let log_a = access.audit_log_for_agent("agent-a");
        assert_eq!(log_a.len(), 2);
    }

    #[test]
    fn test_denied_actions() {
        let mut access = AccessManager::new();

        let perms = AgentPermissions::for_new_agent("test");
        access.set_permissions(perms);

        access.can_use_tool("test", "bash"); // allowed
        access.can_use_tool("test", "dangerous"); // denied
        access.can_access_path("test", "/etc/shadow"); // denied

        let denied = access.denied_actions();
        assert_eq!(denied.len(), 2);
    }

    #[test]
    fn test_clear_audit_log() {
        let mut access = AccessManager::new();

        let perms = AgentPermissions::for_new_agent("test");
        access.set_permissions(perms);

        for _ in 0..5 {
            access.can_use_tool("test", "tool");
        }

        assert_eq!(access.audit_log().len(), 5);

        access.clear_audit_log();

        assert!(access.audit_log().is_empty());
    }

    // --- Max audit entries pruning ---

    #[test]
    fn test_audit_log_prunes_old_entries() {
        let mut access = AccessManager::with_max_audit_entries(5);

        let perms = AgentPermissions::for_new_agent("test");
        access.set_permissions(perms);

        // Add 10 entries.
        for i in 0..10 {
            access.can_use_tool("test", &format!("tool-{}", i));
        }

        // Should be pruned to max_audit_entries.
        assert_eq!(access.audit_log().len(), 5);
    }

    // --- Validate permissions tests ---

    #[test]
    fn test_validate_permissions_no_tools() {
        let mut access = AccessManager::new();
        let mut perms = AgentPermissions::for_new_agent("test");
        perms.allowed_tools.clear();
        access.set_permissions(perms.clone());

        let warnings = access.validate_permissions(&perms);
        assert!(warnings.iter().any(|w| w.contains("no allowed tools")));
    }

    #[test]
    fn test_validate_permissions_no_path_restrictions() {
        let mut perms = AgentPermissions::for_new_agent("test");
        perms.allowed_paths.clear();

        let access = AccessManager::new();
        let warnings = access.validate_permissions(&perms);
        assert!(warnings.iter().any(|w| w.contains("no path restrictions")));
    }

    #[test]
    fn test_validate_permissions_warnings() {
        let mut access = AccessManager::new();
        let mut perms = AgentPermissions::for_new_agent("test");
        perms.network_access = true;
        perms.can_fork = true;
        perms.max_execution_time_secs = 0;
        perms.max_memory_mb = 0;
        access.set_permissions(perms.clone());

        let warnings = access.validate_permissions(&perms);
        assert!(warnings.iter().any(|w| w.contains("network access")));
        assert!(warnings.iter().any(|w| w.contains("fork sub-agents")));
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("no execution time limit"))
        );
        assert!(warnings.iter().any(|w| w.contains("no memory limit")));
    }

    // --- AuditEntry timestamp ---

    #[test]
    fn test_audit_entry_has_timestamp() {
        let entry = AuditEntry::new("agent", "action", "resource", true, None);
        // timestamp should be set (not default DateTime).
        assert!(entry.timestamp.timestamp() > 0);
    }

    // --- Workspace Sandbox tests ---

    #[test]
    fn test_register_workspace_path() {
        let mut access = AccessManager::new();
        access.register_workspace_path("my-workspace", PathBuf::from("/workspace/my-workspace"));

        assert_eq!(access.list_workspaces(), vec!["my-workspace"]);
        assert_eq!(
            access.get_workspace_path("my-workspace"),
            Some(&PathBuf::from("/workspace/my-workspace"))
        );
    }

    #[test]
    fn test_assign_agent_to_workspace() {
        let mut access = AccessManager::new();
        access.register_workspace_path("project-alpha", PathBuf::from("/workspace/alpha"));

        // Assign agent to workspace
        assert!(access.assign_workspace("agent-1", "project-alpha"));

        // Check agent is assigned
        assert_eq!(
            access.get_workspace_for_agent("agent-1"),
            Some("project-alpha".to_string())
        );
        assert!(access.can_access_workspace("agent-1", "project-alpha"));
        assert!(!access.can_access_workspace("agent-1", "other-workspace"));
    }

    #[test]
    fn test_assign_agent_to_nonexistent_workspace_fails() {
        let mut access = AccessManager::new();

        // Cannot assign to non-existent workspace
        assert!(!access.assign_workspace("agent-1", "nonexistent"));
        assert_eq!(access.get_workspace_for_agent("agent-1"), None);
    }

    #[test]
    fn test_reassign_agent_to_different_workspace() {
        let mut access = AccessManager::new();
        access.register_workspace_path("workspace-a", PathBuf::from("/workspace/a"));
        access.register_workspace_path("workspace-b", PathBuf::from("/workspace/b"));

        // Assign to first workspace
        access.assign_workspace("agent-1", "workspace-a");
        assert_eq!(
            access.get_workspace_for_agent("agent-1"),
            Some("workspace-a".to_string())
        );

        // Reassign to second workspace
        access.assign_workspace("agent-1", "workspace-b");
        assert_eq!(
            access.get_workspace_for_agent("agent-1"),
            Some("workspace-b".to_string())
        );

        // Agent should not be in first workspace anymore
        assert!(!access.can_access_workspace("agent-1", "workspace-a"));
    }

    #[test]
    fn test_unassign_agent_from_workspace() {
        let mut access = AccessManager::new();
        access.register_workspace_path("my-workspace", PathBuf::from("/workspace/my"));

        access.assign_workspace("agent-1", "my-workspace");
        assert!(access.get_workspace_for_agent("agent-1").is_some());

        let removed = access.unassign_workspace("agent-1");
        assert_eq!(removed, Some("my-workspace".to_string()));
        assert!(access.get_workspace_for_agent("agent-1").is_none());
    }

    #[test]
    fn test_list_agents_in_workspace() {
        let mut access = AccessManager::new();
        access.register_workspace_path("my-workspace", PathBuf::from("/workspace/my"));

        access.assign_workspace("agent-1", "my-workspace");
        access.assign_workspace("agent-2", "my-workspace");
        access.assign_workspace("agent-3", "other-workspace");

        let agents = access.list_agents_in_workspace("my-workspace");
        assert_eq!(agents.len(), 2);
        assert!(agents.contains(&"agent-1".to_string()));
        assert!(agents.contains(&"agent-2".to_string()));
        assert!(!agents.contains(&"agent-3".to_string()));
    }

    #[test]
    fn test_remove_workspace_unassigns_all_agents() {
        let mut access = AccessManager::new();
        access.register_workspace_path("my-workspace", PathBuf::from("/workspace/my"));

        access.assign_workspace("agent-1", "my-workspace");
        access.assign_workspace("agent-2", "my-workspace");

        access.remove_workspace("my-workspace");

        assert!(access.list_workspaces().is_empty());
        assert!(access.get_workspace_for_agent("agent-1").is_none());
        assert!(access.get_workspace_for_agent("agent-2").is_none());
    }

    #[test]
    fn test_is_path_in_workspace() {
        let mut access = AccessManager::new();

        // Use /tmp for testing - it should exist on most systems
        let workspace = PathBuf::from("/tmp/oxios-test-workspace");

        // Create temp directories BEFORE registering (so canonicalize works)
        std::fs::create_dir_all(&workspace).ok();
        std::fs::create_dir_all(workspace.join("subdir")).ok();

        // Now register the workspace
        access.register_workspace_path("my-workspace", workspace.clone());

        // Path inside workspace
        let inside_path = workspace.join("file.txt");
        std::fs::write(&inside_path, "test").ok(); // Create the file too

        assert!(
            access.is_path_in_workspace("my-workspace", inside_path.to_str().unwrap()),
            "Path {:?} should be inside workspace",
            inside_path
        );

        let nested_path = workspace.join("subdir/nested.txt");
        std::fs::write(&nested_path, "test").ok();
        assert!(
            access.is_path_in_workspace("my-workspace", nested_path.to_str().unwrap()),
            "Path {:?} should be inside workspace",
            nested_path
        );

        // Path outside workspace (use /tmp directly without our subdirectory)
        assert!(!access.is_path_in_workspace("my-workspace", "/tmp/other-workspace/file.txt"));

        // Non-existent workspace
        assert!(!access.is_path_in_workspace("nonexistent", "/tmp/test"));

        // Cleanup
        std::fs::remove_dir_all(workspace).ok();
    }
}
