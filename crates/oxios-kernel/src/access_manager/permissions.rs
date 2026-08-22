//! Agent permissions types — per-agent permission sets and audit entries.

use chrono::{DateTime, Utc};
use glob::Pattern;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Permissions for a single agent.
///
/// Agents start with minimal permissions (least privilege).
/// Additional permissions must be explicitly granted via configuration
/// or an authorized request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPermissions {
    /// Name of the agent this permission set applies to.
    pub agent_name: String,
    /// Set of allowed tool names. Empty means no tools allowed.
    #[serde(default)]
    pub allowed_tools: HashSet<String>,
    /// Allowed path patterns (glob). Used for file operations.
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    /// Denied path patterns (glob). Always blocked, even if allowed_paths matches.
    #[serde(default)]
    pub denied_paths: Vec<String>,
    /// Whether this agent can make network requests.
    #[serde(default)]
    pub network_access: bool,
    /// Maximum execution time in seconds (0 = unlimited).
    #[serde(default)]
    pub max_execution_time_secs: u64,
    /// Maximum memory in MB (0 = unlimited).
    #[serde(default)]
    pub max_memory_mb: u64,
    /// Whether this agent can spawn sub-agents.
    #[serde(default)]
    pub can_fork: bool,
}

impl Default for AgentPermissions {
    fn default() -> Self {
        Self {
            agent_name: String::new(),
            // Always-on tier — must match the `always_on` skip list in
            // `gate.rs::check_tool` and the registration in
            // `tools::registration::register_always_on`. Without
            // web_search/get_search_results here, Layer 1+2 denies them
            // even after Layer 0 lets them through — the second half of
            // the catch-22 from RFC-017 Q3. "ls" was also missing, leaving
            // it dead-registered for default agents.
            allowed_tools: [
                "read",
                "write",
                "edit",
                "grep",
                "find",
                "ls",
                "web_search",
                "get_search_results",
                // Shell execution (legacy "bash" alias + "exec").
                "bash",
                "exec",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            allowed_paths: vec![],
            denied_paths: vec![
                "/etc/**".to_string(),
                "/root/**".to_string(),
                "/sys/**".to_string(),
                "/proc/**".to_string(),
                ".oxios/**".to_string(),
                // Vault-unification: shared ecosystem surface (`~/.oxi/vault`,
                // `~/.oxi/brain`, settings) is denied to agents by default.
                ".oxi/**".to_string(),
            ],
            network_access: false,
            max_execution_time_secs: 300,
            max_memory_mb: 512,
            can_fork: false,
        }
    }
}

/// Update struct for permission changes (partial updates).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionUpdate {
    /// Set of allowed tool names.
    #[serde(default)]
    pub allowed_tools: Option<HashSet<String>>,
    /// Allowed path patterns (glob).
    #[serde(default)]
    pub allowed_paths: Option<Vec<String>>,
    /// Denied path patterns (glob).
    #[serde(default)]
    pub denied_paths: Option<Vec<String>>,
    /// Whether this agent can make network requests.
    #[serde(default)]
    pub network_access: Option<bool>,
    /// Maximum execution time in seconds (0 = unlimited).
    #[serde(default)]
    pub max_execution_time_secs: Option<u64>,
    /// Maximum memory in MB (0 = unlimited).
    #[serde(default)]
    pub max_memory_mb: Option<u64>,
    /// Whether this agent can spawn sub-agents.
    #[serde(default)]
    pub can_fork: Option<bool>,
}

impl PermissionUpdate {
    /// Apply this update to a permission set.
    pub fn apply(&self, perms: &mut AgentPermissions) {
        if let Some(tools) = &self.allowed_tools {
            perms.allowed_tools = tools.clone();
        }
        if let Some(paths) = &self.allowed_paths {
            perms.allowed_paths = paths.clone();
        }
        if let Some(paths) = &self.denied_paths {
            perms.denied_paths = paths.clone();
        }
        if let Some(v) = self.network_access {
            perms.network_access = v;
        }
        if let Some(v) = self.max_execution_time_secs {
            perms.max_execution_time_secs = v;
        }
        if let Some(v) = self.max_memory_mb {
            perms.max_memory_mb = v;
        }
        if let Some(v) = self.can_fork {
            perms.can_fork = v;
        }
    }
}

impl AgentPermissions {
    /// Creates permissions for a new agent with the default restrictive set.
    pub fn for_new_agent(agent_name: &str) -> Self {
        Self {
            agent_name: agent_name.to_string(),
            ..Default::default()
        }
    }

    /// Adds a tool to the allowed set.
    pub fn allow_tool(&mut self, tool: &str) {
        self.allowed_tools.insert(tool.to_string());
    }

    /// Removes a tool from the allowed set.
    pub fn deny_tool(&mut self, tool: &str) {
        self.allowed_tools.remove(tool);
    }

    /// Adds a path pattern to the allowed set.
    pub fn allow_path(&mut self, path: &str) {
        if !self.allowed_paths.contains(&path.to_string()) {
            self.allowed_paths.push(path.to_string());
        }
    }

    /// Adds a path pattern to the denied set.
    pub fn deny_path(&mut self, path: &str) {
        if !self.denied_paths.contains(&path.to_string()) {
            self.denied_paths.push(path.to_string());
        }
    }

    /// Enables network access for this agent.
    pub fn enable_network(&mut self) {
        self.network_access = true;
    }

    /// Enables agent forking (spawning sub-agents).
    pub fn enable_forking(&mut self) {
        self.can_fork = true;
    }

    /// Checks if a path matches any denied pattern.
    pub(crate) fn is_path_denied(&self, path: &str) -> bool {
        for pattern in &self.denied_paths {
            if let Ok(p) = Pattern::new(pattern)
                && p.matches(path)
            {
                return true;
            }
        }
        false
    }

    /// Checks if a path matches any allowed pattern.
    pub(crate) fn is_path_allowed(&self, path: &str) -> bool {
        for pattern in &self.allowed_paths {
            if let Ok(p) = Pattern::new(pattern)
                && p.matches(path)
            {
                return true;
            }
        }
        false
    }
}

/// An entry in the security audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// When the action occurred.
    pub timestamp: DateTime<Utc>,
    /// Agent that performed the action.
    pub agent_name: String,
    /// The action attempted (e.g., "use_tool", "access_path", "network_request").
    pub action: String,
    /// The resource involved (e.g., "bash", "/workspace/file.txt").
    pub resource: String,
    /// Whether the action was allowed.
    pub allowed: bool,
    /// Reason for the decision (e.g., "path not in allowed list").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl AuditEntry {
    /// Creates a new audit entry.
    pub fn new(
        agent_name: &str,
        action: &str,
        resource: &str,
        allowed: bool,
        reason: Option<String>,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            agent_name: agent_name.to_string(),
            action: action.to_string(),
            resource: resource.to_string(),
            allowed,
            reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_permissions_has_basic_tools() {
        let perms = AgentPermissions::default();
        assert!(perms.allowed_tools.contains("read"));
        assert!(perms.allowed_tools.contains("write"));
        assert!(perms.allowed_tools.contains("bash"));
        assert!(perms.allowed_tools.contains("exec"));
        assert!(!perms.network_access);
        assert!(!perms.can_fork);
        assert_eq!(perms.max_execution_time_secs, 300);
        assert_eq!(perms.max_memory_mb, 512);
    }

    #[test]
    fn test_default_permissions_denies_sensitive_paths() {
        let perms = AgentPermissions::default();
        assert!(perms.is_path_denied("/etc/passwd"));
        assert!(perms.is_path_denied("/root/.ssh/id_rsa"));
        assert!(perms.is_path_denied("/proc/self/environ"));
        assert!(perms.is_path_denied("/sys/kernel/addr"));
        assert!(perms.is_path_denied(".oxios/config.toml"));
    }

    #[test]
    fn test_default_permissions_denies_oxi_vault() {
        // Default deny list MUST block the ecosystem `.oxi/**` surface so
        // agents cannot touch shared config/state (vault, brain, settings,
        // sessions) through their tool surface. Without this, an agent
        // could pivot through `read`/`bash` into vault files or
        // `~/.oxi/settings.json` and exfiltrate credentials — defeating
        // AccessManager's per-tool sandbox.
        let perms = AgentPermissions::default();
        assert!(
            perms.denied_paths.iter().any(|p| p == ".oxi/**"),
            "default denied_paths must include `.oxi/**`; got {:?}",
            perms.denied_paths,
        );
        assert!(perms.is_path_denied(".oxi/vault/notes/foo.md"));
        assert!(perms.is_path_denied(".oxi/settings.json"));
        assert!(perms.is_path_denied(".oxi/brain/index.sqlite"));
    }

    #[test]
    fn test_default_permissions_allows_workspace() {
        let perms = AgentPermissions::default();
        // Default has empty allowed_paths — paths are granted by ensure_permissions
        assert!(!perms.is_path_allowed("/workspace/src/main.rs"));
        assert!(!perms.is_path_allowed("/tmp/evil"));
        // With explicit pattern, it should match
        let mut perms = AgentPermissions::for_new_agent("test");
        perms.allow_path("/workspace/**");
        assert!(perms.is_path_allowed("/workspace/src/main.rs"));
        assert!(perms.is_path_allowed("/workspace/README.md"));
    }

    #[test]
    fn test_for_new_agent_sets_name() {
        let perms = AgentPermissions::for_new_agent("test-agent");
        assert_eq!(perms.agent_name, "test-agent");
        assert!(perms.allowed_tools.contains("read"));
    }

    #[test]
    fn test_allow_and_deny_tool() {
        let mut perms = AgentPermissions::for_new_agent("a");
        perms.allow_tool("custom_tool");
        assert!(perms.allowed_tools.contains("custom_tool"));

        perms.deny_tool("bash");
        assert!(!perms.allowed_tools.contains("bash"));

        // denying non-existent tool is a no-op
        perms.deny_tool("nonexistent");
    }

    #[test]
    fn test_allow_and_deny_path_deduplication() {
        let mut perms = AgentPermissions::for_new_agent("a");
        perms.allow_path("/data/**");
        perms.allow_path("/data/**"); // duplicate
        assert_eq!(
            perms
                .allowed_paths
                .iter()
                .filter(|p| **p == "/data/**")
                .count(),
            1
        );

        perms.deny_path("/secret/**");
        perms.deny_path("/secret/**"); // duplicate
        assert_eq!(
            perms
                .denied_paths
                .iter()
                .filter(|p| **p == "/secret/**")
                .count(),
            1
        );
    }

    #[test]
    fn test_enable_network_and_forking() {
        let mut perms = AgentPermissions::for_new_agent("a");
        assert!(!perms.network_access);
        assert!(!perms.can_fork);

        perms.enable_network();
        assert!(perms.network_access);

        perms.enable_forking();
        assert!(perms.can_fork);
    }

    #[test]
    fn test_denied_overrides_allowed() {
        let mut perms = AgentPermissions::for_new_agent("a");
        perms.allowed_paths = vec!["/workspace/**".to_string()];
        perms.denied_paths = vec!["/workspace/secret/**".to_string()];

        assert!(perms.is_path_allowed("/workspace/secret/key.pem"));
        assert!(perms.is_path_denied("/workspace/secret/key.pem"));
        // Both match — denied takes precedence at the gate level
    }

    #[test]
    fn test_invalid_glob_pattern() {
        let mut perms = AgentPermissions::for_new_agent("a");
        perms.allowed_paths = vec!["[invalid".to_string()];
        // Invalid glob should not panic, just not match
        assert!(!perms.is_path_allowed("/anything"));
    }

    #[test]
    fn test_permission_update_partial() {
        let mut perms = AgentPermissions::for_new_agent("a");
        let original_tools = perms.allowed_tools.clone();

        let update = PermissionUpdate {
            network_access: Some(true),
            max_execution_time_secs: Some(600),
            ..Default::default()
        };
        update.apply(&mut perms);

        assert!(perms.network_access);
        assert_eq!(perms.max_execution_time_secs, 600);
        // Untouched fields remain the same
        assert_eq!(perms.allowed_tools, original_tools);
        assert!(!perms.can_fork);
    }

    #[test]
    fn test_permission_update_full_replace() {
        let mut perms = AgentPermissions::for_new_agent("a");

        let update = PermissionUpdate {
            allowed_tools: Some(HashSet::from(["read".to_string()])),
            allowed_paths: Some(vec!["/safe/**".to_string()]),
            denied_paths: Some(vec![]),
            network_access: Some(true),
            max_execution_time_secs: Some(0),
            max_memory_mb: Some(1024),
            can_fork: Some(true),
        };
        update.apply(&mut perms);

        assert_eq!(perms.allowed_tools.len(), 1);
        assert!(perms.allowed_tools.contains("read"));
        assert_eq!(perms.allowed_paths, vec!["/safe/**"]);
        assert!(perms.denied_paths.is_empty());
        assert!(perms.network_access);
        assert!(perms.can_fork);
        assert_eq!(perms.max_memory_mb, 1024);
    }

    #[test]
    fn test_audit_entry_new_allowed() {
        let entry = AuditEntry::new("agent-1", "use_tool", "bash", true, None);
        assert_eq!(entry.agent_name, "agent-1");
        assert_eq!(entry.action, "use_tool");
        assert_eq!(entry.resource, "bash");
        assert!(entry.allowed);
        assert!(entry.reason.is_none());
    }

    #[test]
    fn test_audit_entry_new_denied_with_reason() {
        let entry = AuditEntry::new(
            "rogue-agent",
            "access_path",
            "/etc/shadow",
            false,
            Some("path not in allowed list".to_string()),
        );
        assert!(!entry.allowed);
        assert_eq!(entry.reason.as_deref(), Some("path not in allowed list"));
    }

    #[test]
    fn test_permissions_serialization_roundtrip() {
        let mut perms = AgentPermissions::for_new_agent("serializer");
        perms.enable_network();
        perms.allow_tool("curl");

        let json = serde_json::to_string(&perms).unwrap();
        let restored: AgentPermissions = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.agent_name, "serializer");
        assert!(restored.network_access);
        assert!(restored.allowed_tools.contains("curl"));
    }

    #[test]
    fn test_audit_entry_serialization_roundtrip() {
        let entry = AuditEntry::new(
            "test",
            "network_request",
            "https://example.com",
            false,
            Some("network not allowed".to_string()),
        );
        let json = serde_json::to_string(&entry).unwrap();
        let restored: AuditEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.agent_name, entry.agent_name);
        assert_eq!(restored.action, entry.action);
        assert_eq!(restored.allowed, entry.allowed);
        assert_eq!(restored.reason, entry.reason);
    }

    #[test]
    fn test_permission_update_default_is_noop() {
        let mut perms = AgentPermissions::for_new_agent("a");
        let snapshot = perms.clone();

        let update = PermissionUpdate::default();
        update.apply(&mut perms);

        assert_eq!(perms.agent_name, snapshot.agent_name);
        assert_eq!(perms.allowed_tools, snapshot.allowed_tools);
        assert_eq!(perms.allowed_paths, snapshot.allowed_paths);
        assert_eq!(perms.denied_paths, snapshot.denied_paths);
        assert_eq!(perms.network_access, snapshot.network_access);
        assert_eq!(
            perms.max_execution_time_secs,
            snapshot.max_execution_time_secs
        );
        assert_eq!(perms.max_memory_mb, snapshot.max_memory_mb);
        assert_eq!(perms.can_fork, snapshot.can_fork);
    }
}
