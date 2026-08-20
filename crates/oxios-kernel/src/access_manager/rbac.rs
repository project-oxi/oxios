//! RBAC types and manager — role-based access control with HitL approvals.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::types::AgentId;

// ─── RBAC Types ───────────────────────────────────────────────────────────────

/// Roles for role-based access control (3-tier model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    /// Basic user — can use agents, limited permissions.
    User,
    /// Superuser — can manage programs, skills, workspaces.
    Superuser,
    /// Admin — full system access, can modify RBAC.
    Admin,
}

impl Role {
    /// Returns the default policy for this role.
    pub fn default_policy(&self) -> RbacPolicy {
        match self {
            Role::Admin => RbacPolicy {
                role: Role::Admin,
                allowed_actions: vec![
                    Action::UseTool("*".into()),
                    Action::AccessPath("*".into()),
                    Action::ManageAgents,
                    Action::ManagePrograms,
                    Action::ManageWorkspaces,
                    Action::ManageRBAC,
                    Action::ViewAuditLog,
                    Action::SystemConfig,
                ]
                .into_iter()
                .collect(),
                resource_patterns: vec!["*".into()],
                max_concurrent_agents: usize::MAX,
            },
            Role::Superuser => RbacPolicy {
                role: Role::Superuser,
                allowed_actions: vec![
                    Action::UseTool("*".into()),
                    Action::AccessPath("*".into()),
                    Action::ManageAgents,
                    Action::ManagePrograms,
                    Action::ManageWorkspaces,
                    Action::ViewAuditLog,
                ]
                .into_iter()
                .collect(),
                resource_patterns: vec!["*".into()],
                max_concurrent_agents: 10,
            },
            Role::User => RbacPolicy {
                role: Role::User,
                allowed_actions: vec![
                    Action::UseTool("read".into()),
                    Action::UseTool("write".into()),
                    Action::UseTool("edit".into()),
                    Action::UseTool("bash".into()),
                    Action::UseTool("grep".into()),
                    Action::UseTool("find".into()),
                    Action::UseTool("ls".into()),
                    Action::UseTool("web_search".into()),
                    Action::UseTool("get_search_results".into()),
                    Action::AccessPath("/workspace/**".into()),
                    Action::ManageAgents,
                ]
                .into_iter()
                .collect(),
                resource_patterns: vec!["/workspace/**".into()],
                max_concurrent_agents: 2,
            },
        }
    }
}

/// Subject — who is accessing the system.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Subject {
    /// A named user.
    User(String),
    /// An agent acting on behalf of a user.
    Agent(AgentId),
    /// System-level operations (bypass RBAC).
    System,
}

impl std::fmt::Display for Subject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Subject::User(name) => write!(f, "user:{name}"),
            Subject::Agent(id) => write!(f, "agent:{id}"),
            Subject::System => write!(f, "system"),
        }
    }
}

/// Actions that can be authorized by RBAC.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum Action {
    /// Use a specific tool (name or * for all).
    UseTool(String),
    /// Access a specific path pattern (glob).
    AccessPath(String),
    /// Manage agents (fork/exec/kill).
    ManageAgents,
    /// Manage programs (install/uninstall).
    ManagePrograms,
    /// Manage workspaces (create/start/stop/remove).
    ManageWorkspaces,
    /// Modify RBAC policies and role assignments.
    ManageRBAC,
    /// View the audit log.
    ViewAuditLog,
    /// Modify system-level configuration.
    SystemConfig,
}

impl Action {
    /// Returns true if this action is considered high-risk and needs HitL approval.
    pub fn requires_approval(&self) -> bool {
        match self {
            Action::ManageRBAC | Action::SystemConfig => true,
            Action::UseTool(t) => t == "*" || t == "osascript" || t == "rm",
            _ => false,
        }
    }
}

/// RBAC policy defining what a role can do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbacPolicy {
    /// The role this policy applies to.
    pub role: Role,
    /// Set of actions this role is allowed to perform.
    pub allowed_actions: HashSet<Action>,
    /// Glob patterns for accessible resources.
    pub resource_patterns: Vec<String>,
    /// Maximum number of concurrent agents for this role.
    pub max_concurrent_agents: usize,
}

impl RbacPolicy {
    /// Checks whether this policy allows the given action.
    ///
    /// Supports wildcard matching for `UseTool("*")` (matches any tool name)
    /// and `AccessPath("*")` (matches any path).
    pub fn allows(&self, action: &Action) -> bool {
        // First, try exact match.
        if self.allowed_actions.contains(action) {
            return true;
        }

        // Then, check wildcard patterns.
        match action {
            Action::UseTool(tool_name) => {
                // Check if policy has UseTool("*") wildcard.
                self.allowed_actions
                    .iter()
                    .any(|a| matches!(a, Action::UseTool(w) if w == "*"))
                    // Also check if the specific tool name is listed.
                    || self.allowed_actions.contains(&Action::UseTool(tool_name.clone()))
            }
            Action::AccessPath(path) => {
                // Wildcard or exact match in allowed_actions.
                if self
                    .allowed_actions
                    .iter()
                    .any(|a| matches!(a, Action::AccessPath(p) if p == "*"))
                    || self
                        .allowed_actions
                        .contains(&Action::AccessPath(path.clone()))
                {
                    return true;
                }
                // Enforce resource_patterns glob match (e.g. "/workspace/**").
                // Previously this field was defined but never consulted.
                for pattern in &self.resource_patterns {
                    if pattern == "*" {
                        return true;
                    }
                    if let Ok(p) = glob::Pattern::new(pattern)
                        && p.matches(path)
                    {
                        return true;
                    }
                }
                false
            }
            // Non-parameterized actions: exact match only (already checked above).
            _ => false,
        }
    }
}

/// RBAC audit entry — records authorization decisions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbacAuditEntry {
    /// When the authorization decision was made.
    pub timestamp: DateTime<Utc>,
    /// Who performed the action.
    pub subject: Subject,
    /// What action was attempted.
    pub action: Action,
    /// Which resource was involved.
    pub resource: String,
    /// Whether the action was allowed.
    pub allowed: bool,
    /// Optional reason for the decision.
    pub reason: Option<String>,
}

impl RbacAuditEntry {
    /// Creates a new RBAC audit entry.
    pub(crate) fn new(
        subject: Subject,
        action: Action,
        resource: String,
        allowed: bool,
        reason: Option<String>,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            subject,
            action,
            resource,
            allowed,
            reason,
        }
    }
}

/// Human-in-the-loop approval request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    /// Unique identifier for this approval request.
    pub id: uuid::Uuid,
    /// Who is requesting the action.
    pub subject: Subject,
    /// What action is being requested.
    pub action: Action,
    /// Which resource is involved.
    pub resource: String,
    /// Why the action needs approval.
    pub reason: String,
    /// When the request was created.
    pub created_at: DateTime<Utc>,
}

/// Status of a HitL approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalStatus {
    /// Awaiting user decision.
    Pending,
    /// User approved the request.
    Approved,
    /// User rejected the request.
    Rejected,
    /// Request timed out.
    Expired,
}

/// RBAC Manager — manages roles, permissions, and HitL approvals.
#[derive(Debug, Clone)]
pub struct RbacManager {
    policies: HashMap<Role, RbacPolicy>,
    subject_roles: HashMap<Subject, Role>,
    audit_log: Vec<RbacAuditEntry>,
    pending_approvals: Vec<(PendingApproval, ApprovalStatus)>,
    max_audit_entries: usize,
}

impl RbacManager {
    /// Creates a new RBAC manager with default policies for all roles.
    pub fn new() -> Self {
        let mut this = Self {
            policies: HashMap::new(),
            subject_roles: HashMap::new(),
            audit_log: Vec::new(),
            pending_approvals: Vec::new(),
            max_audit_entries: 10_000,
        };
        for role in [Role::User, Role::Superuser, Role::Admin] {
            this.policies.insert(role, role.default_policy());
        }
        this
    }

    /// Assigns a role to a subject.
    pub fn assign_role(&mut self, subject: Subject, role: Role) {
        self.subject_roles.insert(subject.clone(), role);
    }

    /// Revokes the role from a subject.
    pub fn revoke_role(&mut self, subject: &Subject) {
        self.subject_roles.remove(subject);
    }

    /// Returns the role assigned to a subject, if any.
    pub fn get_role(&self, subject: &Subject) -> Option<Role> {
        self.subject_roles.get(subject).copied()
    }

    /// Checks whether a subject has permission for the given action on a resource.
    pub fn check_permission(&mut self, subject: &Subject, action: &Action, resource: &str) -> bool {
        if matches!(subject, Subject::System) {
            // System subject bypasses role checks, but the decision is still
            // recorded in the audit trail so bypasses are visible (never silent).
            self.audit_log.push(RbacAuditEntry::new(
                subject.clone(),
                action.clone(),
                resource.to_string(),
                true,
                Some("system subject bypass".to_string()),
            ));
            if self.audit_log.len() > self.max_audit_entries {
                self.audit_log
                    .drain(0..self.audit_log.len() - self.max_audit_entries);
            }
            return true;
        }
        let role = match self.subject_roles.get(subject) {
            Some(r) => *r,
            None => return false,
        };
        let policy = match self.policies.get(&role) {
            Some(p) => p,
            None => return false,
        };
        let allowed = policy.allows(action);
        self.audit_log.push(RbacAuditEntry::new(
            subject.clone(),
            action.clone(),
            resource.to_string(),
            allowed,
            if allowed {
                None
            } else {
                Some(format!("role {role:?} does not allow {action:?}"))
            },
        ));
        if self.audit_log.len() > self.max_audit_entries {
            self.audit_log
                .drain(0..self.audit_log.len() - self.max_audit_entries);
        }
        allowed
    }

    /// Creates a new approval request for a high-risk action.
    pub fn request_approval(
        &mut self,
        subject: Subject,
        action: Action,
        resource: String,
        reason: String,
    ) -> uuid::Uuid {
        let id = uuid::Uuid::new_v4();
        self.pending_approvals.push((
            PendingApproval {
                id,
                subject,
                action,
                resource,
                reason,
                created_at: Utc::now(),
            },
            ApprovalStatus::Pending,
        ));
        id
    }

    /// Approves a pending approval request.
    pub fn approve(&mut self, id: uuid::Uuid) -> bool {
        if let Some((_, s)) = self
            .pending_approvals
            .iter_mut()
            .find(|(p, s)| p.id == id && *s == ApprovalStatus::Pending)
        {
            *s = ApprovalStatus::Approved;
            return true;
        }
        false
    }

    /// Rejects a pending approval request.
    pub fn reject(&mut self, id: uuid::Uuid) -> bool {
        if let Some((_, s)) = self
            .pending_approvals
            .iter_mut()
            .find(|(p, s)| p.id == id && *s == ApprovalStatus::Pending)
        {
            *s = ApprovalStatus::Rejected;
            return true;
        }
        false
    }

    /// Returns all currently pending approval requests.
    pub fn pending_approvals(&self) -> Vec<&PendingApproval> {
        self.pending_approvals
            .iter()
            .filter(|(_, s)| matches!(s, ApprovalStatus::Pending))
            .map(|(p, _)| p)
            .collect()
    }

    /// Returns all approval requests (pending + history) with their status.
    pub fn all_approvals(&self) -> &[(PendingApproval, ApprovalStatus)] {
        &self.pending_approvals
    }

    /// Returns the RBAC audit log.
    pub fn audit_log(&self) -> &[RbacAuditEntry] {
        &self.audit_log
    }
}

impl Default for RbacManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_policies_exist() {
        let mgr = RbacManager::new();
        assert!(mgr.policies.contains_key(&Role::User));
        assert!(mgr.policies.contains_key(&Role::Superuser));
        assert!(mgr.policies.contains_key(&Role::Admin));
    }

    #[test]
    fn test_role_assignment() {
        let mut mgr = RbacManager::new();
        let subject = Subject::User("alice".into());
        mgr.assign_role(subject.clone(), Role::Admin);
        assert_eq!(mgr.get_role(&subject), Some(Role::Admin));

        mgr.revoke_role(&subject);
        assert_eq!(mgr.get_role(&subject), None);
    }

    #[test]
    fn test_system_bypasses_rbac() {
        let mut mgr = RbacManager::new();
        let subject = Subject::System;
        assert!(mgr.check_permission(&subject, &Action::ManageRBAC, "test"));
    }

    #[test]
    fn test_unknown_subject_denied() {
        let mut mgr = RbacManager::new();
        let subject = Subject::User("nobody".into());
        assert!(!mgr.check_permission(&subject, &Action::UseTool("read".into()), "test"));
    }

    #[test]
    fn test_user_allowed_specific_tools() {
        let mut mgr = RbacManager::new();
        let subject = Subject::User("bob".into());
        mgr.assign_role(subject.clone(), Role::User);

        assert!(mgr.check_permission(&subject, &Action::UseTool("read".into()), "test"));
        assert!(mgr.check_permission(&subject, &Action::UseTool("write".into()), "test"));
        assert!(mgr.check_permission(&subject, &Action::UseTool("bash".into()), "test"));
    }

    #[test]
    fn test_user_denied_admin_tools() {
        let mut mgr = RbacManager::new();
        let subject = Subject::User("bob".into());
        mgr.assign_role(subject.clone(), Role::User);

        assert!(!mgr.check_permission(&subject, &Action::ManageRBAC, "test"));
        assert!(!mgr.check_permission(&subject, &Action::SystemConfig, "test"));
    }

    #[test]
    fn test_admin_wildcard_allows_all_tools() {
        let mut mgr = RbacManager::new();
        let subject = Subject::User("admin".into());
        mgr.assign_role(subject.clone(), Role::Admin);

        // Admin should be able to use ANY tool via wildcard.
        assert!(mgr.check_permission(&subject, &Action::UseTool("any_tool".into()), "test"));
        assert!(mgr.check_permission(&subject, &Action::UseTool("custom_thing".into()), "test"));
        assert!(mgr.check_permission(&subject, &Action::UseTool("dangerous".into()), "test"));
    }

    #[test]
    fn test_superuser_wildcard_allows_all_tools() {
        let mut mgr = RbacManager::new();
        let subject = Subject::User("super".into());
        mgr.assign_role(subject.clone(), Role::Superuser);

        assert!(mgr.check_permission(&subject, &Action::UseTool("custom".into()), "test"));
        assert!(mgr.check_permission(&subject, &Action::UseTool("anything".into()), "test"));
    }

    #[test]
    fn test_admin_all_paths_wildcard() {
        let mut mgr = RbacManager::new();
        let subject = Subject::User("admin".into());
        mgr.assign_role(subject.clone(), Role::Admin);

        assert!(mgr.check_permission(&subject, &Action::AccessPath("/any/path".into()), "test"));
        assert!(mgr.check_permission(&subject, &Action::AccessPath("/secret/data".into()), "test"));
    }

    #[test]
    fn test_policy_allows_exact_match() {
        let policy = Role::User.default_policy();
        assert!(policy.allows(&Action::UseTool("read".into())));
        assert!(policy.allows(&Action::UseTool("bash".into())));
        assert!(!policy.allows(&Action::UseTool("unknown_tool".into())));
    }

    #[test]
    fn user_default_policy_allows_web_tools() {
        // Task 7: `Role::User` must declare `ls`, `web_search`, and
        // `get_search_results` in its default `allowed_actions` so
        // that the user role can run the baseline web/FS tool set
        // without RBAC-level denial (Layer 1). Layer 0 is enforced
        // separately through the typed resource mapping.
        let policy = Role::User.default_policy();
        for t in ["ls", "web_search", "get_search_results"] {
            assert!(
                policy.allowed_actions.contains(&Action::UseTool(t.into())),
                "User default policy must allow tool {t:?}; got {:?}",
                policy.allowed_actions
            );
        }
    }

    #[test]
    fn test_policy_allows_wildcard() {
        let policy = Role::Admin.default_policy();
        assert!(policy.allows(&Action::UseTool("literally_anything".into())));
        assert!(policy.allows(&Action::AccessPath("/some/random/path".into())));
    }

    #[test]
    fn test_approval_request_lifecycle() {
        let mut mgr = RbacManager::new();
        let id = mgr.request_approval(
            Subject::User("alice".into()),
            Action::ManageRBAC,
            "rbac".into(),
            "need admin".into(),
        );

        let pending = mgr.pending_approvals();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id);

        assert!(mgr.approve(id));
        assert!(mgr.pending_approvals().is_empty());

        // Already approved
        assert!(!mgr.approve(id));
    }

    #[test]
    fn test_approval_rejection() {
        let mut mgr = RbacManager::new();
        let id = mgr.request_approval(
            Subject::User("alice".into()),
            Action::SystemConfig,
            "config".into(),
            "need config".into(),
        );

        assert!(mgr.reject(id));
        assert!(mgr.pending_approvals().is_empty());
    }

    #[test]
    fn test_approval_nonexistent() {
        let mut mgr = RbacManager::new();
        assert!(!mgr.approve(uuid::Uuid::new_v4()));
        assert!(!mgr.reject(uuid::Uuid::new_v4()));
    }

    #[test]
    fn test_audit_log_recorded() {
        let mut mgr = RbacManager::new();
        let subject = Subject::User("alice".into());
        mgr.assign_role(subject.clone(), Role::User);

        mgr.check_permission(&subject, &Action::UseTool("read".into()), "test");
        assert!(!mgr.audit_log().is_empty());

        let entry = &mgr.audit_log()[0];
        assert!(entry.allowed);
    }

    #[test]
    fn test_audit_log_denied_recorded() {
        let mut mgr = RbacManager::new();
        let subject = Subject::User("alice".into());
        mgr.assign_role(subject.clone(), Role::User);

        mgr.check_permission(&subject, &Action::ManageRBAC, "test");
        let denied_entries: Vec<_> = mgr.audit_log().iter().filter(|e| !e.allowed).collect();
        assert_eq!(denied_entries.len(), 1);
    }
}
