//! Unified access gate — single entry point for all authorization decisions.
//!
//! Every security check in the system flows through `AccessGate`. It enforces
//! a four-layer hierarchy with short-circuit evaluation:
//!
//! ```text
//! Layer 0: CSpace (Capability)  — does the agent have the capability token?
//! Layer 1: RBAC                  — does the agent's role allow the action?
//! Layer 2: Agent Permissions     — is the tool/path in allowed lists?
//! Layer 3: ExecConfig            — is the binary allowed? No metacharacters?
//! ```
//!
//! If any layer denies, the request is rejected immediately (no further checks).
//! All decisions (allow and deny) are recorded via `AuditSink`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::access_manager::audit_sink::{AuditEvent, AuditSink};
use crate::access_manager::context::AgentContext;
use crate::access_manager::{AccessManager, Action, Subject};
use crate::capability::{ResourceRef, Rights};
use crate::config::ExecConfig;

// ─── Path Mode ──────────────────────────────────────────────────────────────

/// Path access mode for permission checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathMode {
    /// Read-only access (read, ls, grep, find).
    Read,
    /// Write access (write, edit).
    Write,
}

impl std::fmt::Display for PathMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathMode::Read => write!(f, "read"),
            PathMode::Write => write!(f, "write"),
        }
    }
}

// ─── Deny Layer ─────────────────────────────────────────────────────────────

/// Which security layer produced the deny decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyLayer {
    /// CSpace missing required capability.
    Capability,
    /// RBAC role does not allow action.
    Rbac,
    /// AgentPermissions denied (tool/path not in allowed set).
    Permission,
    /// ExecConfig denied (binary not in allowlist, metacharacters).
    ExecPolicy,
}

impl std::fmt::Display for DenyLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DenyLayer::Capability => write!(f, "CSpace"),
            DenyLayer::Rbac => write!(f, "RBAC"),
            DenyLayer::Permission => write!(f, "Permissions"),
            DenyLayer::ExecPolicy => write!(f, "ExecPolicy"),
        }
    }
}

// ─── Access Denied ──────────────────────────────────────────────────────────

/// Authorization denial — includes the layer, reason, and user-facing suggestion.
#[derive(Debug, Clone)]
pub struct AccessDenied {
    /// Agent that was denied.
    pub agent: String,
    /// Resource that was accessed.
    pub resource: String,
    /// Which security layer produced the denial.
    pub layer: DenyLayer,
    /// Machine-readable reason.
    pub reason: String,
    /// User-facing suggestion for resolution.
    pub suggestion: Option<String>,
}

impl std::fmt::Display for AccessDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {} — {}",
            self.layer,
            self.reason,
            self.suggestion.as_deref().unwrap_or("")
        )
    }
}

// ─── Check Request ──────────────────────────────────────────────────────────

/// Authorization check request — specifies what is being accessed.
#[derive(Debug)]
pub enum CheckRequest<'a> {
    /// Tool usage permission.
    Tool {
        /// Agent security context.
        context: &'a AgentContext,
        /// Name of the tool to use.
        tool_name: &'a str,
    },
    /// Path access permission.
    Path {
        /// Agent security context.
        context: &'a AgentContext,
        /// Path to access.
        path: &'a Path,
        /// Read or write mode.
        mode: PathMode,
    },
    /// Command execution permission.
    Exec {
        /// Agent security context.
        context: &'a AgentContext,
        /// Binary to execute.
        binary: &'a str,
        /// Arguments for the binary.
        args: &'a [String],
    },
    /// Network access permission.
    Network {
        /// Agent security context.
        context: &'a AgentContext,
    },
    /// Agent fork (sub-agent spawn) permission.
    Fork {
        /// Agent security context.
        context: &'a AgentContext,
    },
}

impl<'a> CheckRequest<'a> {
    /// Returns the agent context for this request.
    pub fn agent_context(&self) -> &AgentContext {
        match self {
            CheckRequest::Tool { context, .. } => context,
            CheckRequest::Path { context, .. } => context,
            CheckRequest::Exec { context, .. } => context,
            CheckRequest::Network { context } => context,
            CheckRequest::Fork { context } => context,
        }
    }

    /// Returns a string describing the resource being accessed.
    pub fn resource(&self) -> &str {
        match self {
            CheckRequest::Tool { tool_name, .. } => tool_name,
            CheckRequest::Path { path, .. } => path.to_str().unwrap_or("<invalid-path>"),
            CheckRequest::Exec { binary, .. } => binary,
            CheckRequest::Network { .. } => "<network>",
            CheckRequest::Fork { .. } => "fork",
        }
    }
}

// ─── Shell Metacharacters ───────────────────────────────────────────────────

/// Characters blocked in structured-mode arguments.
const SHELL_METACHARS: &[char] = &[
    '|', '&', ';', '$', '`', '<', '>', '(', ')', '{', '}', '\n', '\r', '\0',
];

/// Check whether any argument contains shell metacharacters or path traversal.
fn has_metacharacters(args: &[String]) -> bool {
    for arg in args {
        if arg.contains("..") {
            return true;
        }
        if SHELL_METACHARS.iter().any(|&c| arg.contains(c)) {
            return true;
        }
    }
    false
}

/// Resolve a path to its canonical form for consistent layer matching.
///
/// Symlinks and `..` segments are resolved so that the RBAC, permission, and
/// workspace layers all see the same path — otherwise a path like
/// `/workspace/../etc/passwd` slips through prefix/glob matches.
///
/// If the path does not yet exist (e.g. a file about to be written), the
/// nearest existing ancestor is canonicalized and the remaining components are
/// re-appended. If even the ancestor cannot be canonicalized the original path
/// is returned unchanged (the workspace layer will then reject it).
fn canonicalize_for_check(path: &Path) -> PathBuf {
    if let Ok(canon) = path.canonicalize() {
        return canon;
    }
    let mut ancestor = path.to_path_buf();
    let mut tail: Vec<OsString> = Vec::new();
    while !ancestor.exists() {
        match ancestor.file_name() {
            Some(name) => {
                tail.push(name.to_os_string());
                if !ancestor.pop() {
                    break;
                }
            }
            None => break,
        }
    }
    match ancestor.canonicalize() {
        Ok(mut base) => {
            for name in tail.into_iter().rev() {
                base.push(name);
            }
            base
        }
        Err(_) => path.to_path_buf(),
    }
}

// ─── Access Gate ────────────────────────────────────────────────────────────

/// Single entry point for all authorization decisions.
///
/// Every tool execution, path access, command execution, network request,
/// and agent fork must pass through this gate.
///
/// # Example
///
/// ```no_run
/// use oxios_kernel::access_manager::{AccessGate, CheckRequest, PathMode};
///
/// // AccessGate is constructed during kernel initialization with internal
/// // parking_lot::Mutex<AccessManager>, ExecConfig, and an AuditSink.
/// // Security checks use AgentContext (provided by the kernel's agent lifecycle).
/// //
/// // gate.check(CheckRequest::Tool { context: &ctx, tool_name: "exec" })?;
/// // gate.check(CheckRequest::Path {
/// //     context: &ctx,
/// //     path: Path::new("/workspace/file.rs"),
/// //     mode: PathMode::Read,
/// // })?;
/// ```
pub struct AccessGate {
    /// Agent permission manager (includes RBAC internally).
    access: Arc<Mutex<AccessManager>>,
    /// Execution policy (allowlist, timeouts).
    exec_config: Arc<ExecConfig>,
    /// Audit event destination.
    audit: Arc<dyn AuditSink>,
}

impl AccessGate {
    /// Create a new access gate.
    pub fn new(
        access: Arc<Mutex<AccessManager>>,
        exec_config: Arc<ExecConfig>,
        audit: Arc<dyn AuditSink>,
    ) -> Self {
        Self {
            access,
            exec_config,
            audit,
        }
    }

    /// Clone the inner access manager Arc (for ExecTool fallback).
    pub fn access_clone(&self) -> Arc<Mutex<AccessManager>> {
        self.access.clone()
    }

    /// Perform a synchronous authorization check.
    ///
    /// All decisions (allow and deny) are recorded to the audit sink.
    /// Checks are evaluated in order with short-circuit: the first layer
    /// to deny stops further evaluation.
    pub fn check(&self, req: CheckRequest<'_>) -> Result<(), AccessDenied> {
        let result = match &req {
            CheckRequest::Tool { context, tool_name } => self.check_tool(context, tool_name),
            CheckRequest::Path {
                context,
                path,
                mode,
            } => self.check_path(context, path, *mode),
            CheckRequest::Exec {
                context,
                binary,
                args,
            } => self.check_exec(context, binary, args),
            CheckRequest::Network { context } => self.check_network(context),
            CheckRequest::Fork { context } => self.check_fork(context),
        };

        // Record to audit sink regardless of outcome.
        self.record_check(&req, &result);

        result
    }

    // ─── Layer Implementations ───────────────────────────────────────

    fn check_tool(&self, ctx: &AgentContext, tool: &str) -> Result<(), AccessDenied> {
        let resource = ResourceRef::KernelDomain {
            domain: tool.to_string(),
        };
        if !ctx.cspace.can(&resource, Rights::EXECUTE) {
            // The always-on tier — registered unconditionally for every
            // agent by `tools::registration::register_always_on` (file ops
            // + web search). CSpace capability is advisory for these tools:
            // they are part of the baseline agent contract documented in
            // ARCHITECTURE.md §"Tier 1" and enumerated in
            // `OxiosKernelBridge::tool_names`, so Layer 0 must not gate
            // them. Without web_search/get_search_results in this list,
            // every default agent hits a catch-22: the tool is registered
            // (so the LLM calls it) but no capability template grants the
            // matching EXECUTE right, so Layer 0 hard-denies. See RFC-017
            // Q3 ("the bug where web_search was denied despite being
            // always-on is fixed separately — adding it to the gate's
            // skip list"). This is that fix.
            //
            // CSpace-driven tools (exec, memory_*, knowledge, browse*,
            // a2a, persona, ...) still require an explicit EXECUTE
            // capability in the agent's Seed and are denied here when
            // absent. RFC-017 covers the per-session escalation flow
            // (GatedTool → PendingToolApprovals → user dialog) for that
            // case.
            let always_on = [
                "read",
                "write",
                "edit",
                "grep",
                "find",
                "ls",
                "web_search",
                "get_search_results",
            ];
            if !always_on.contains(&tool) {
                return Err(AccessDenied {
                    agent: ctx.agent_name.clone(),
                    resource: tool.to_string(),
                    layer: DenyLayer::Capability,
                    reason: format!("CSpace lacks EXECUTE capability for tool '{tool}'"),
                    suggestion: Some(format!("Add the '{tool}' capability to the agent's Seed.")),
                });
            }
        }

        // Layer 1+2: RBAC + Permissions (AccessManager)
        let mut access = self.access.lock();
        if !access.can_use_tool(&ctx.agent_name, tool) {
            return Err(AccessDenied {
                agent: ctx.agent_name.clone(),
                resource: tool.to_string(),
                layer: DenyLayer::Permission,
                reason: format!(
                    "'{}' is not in agent '{}'s allowed_tools",
                    tool, ctx.agent_name
                ),
                suggestion: Some(format!(
                    "Request permission for the '{}' tool on agent '{}' from your administrator.",
                    tool, ctx.agent_name
                )),
            });
        }

        Ok(())
    }

    fn check_path(
        &self,
        ctx: &AgentContext,
        path: &Path,
        mode: PathMode,
    ) -> Result<(), AccessDenied> {
        // Resolve relative paths to absolute using CWD, then canonicalize so
        // that `..`, symlink prefixes, and case differences are resolved
        // consistently across the RBAC, permission, and workspace layers.
        // Without this, `/workspace/../etc/passwd` would pass a `/workspace/`
        // prefix check. Agents run in the workspace directory.
        let resolved = if path.is_relative() {
            std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join(path)
        } else {
            path.to_path_buf()
        };
        let resolved = canonicalize_for_check(&resolved);
        let path_str = resolved.to_string_lossy();

        // Layer 0: CSpace (file system access)
        let resource = ResourceRef::KernelDomain {
            domain: "fs".to_string(),
        };
        let required = match mode {
            PathMode::Read => Rights::READ,
            PathMode::Write => Rights::WRITE,
        };
        if !ctx.cspace.can(&resource, required) {
            // File system CSpace check is advisory — most agents need file access.
            // We don't block on CSpace for fs domain, but log it.
            tracing::debug!(
                agent = %ctx.agent_name,
                mode = %mode,
                "CSpace does not contain fs capability, proceeding (advisory)"
            );
        }

        // Layer 1: RBAC check — use the resolved path for matching.
        let mut access = self.access.lock();
        let rbac_subject = Subject::Agent(ctx.agent_id);
        let rbac_action = Action::AccessPath(path_str.to_string());
        if !access
            .rbac_manager_mut()
            .check_permission(&rbac_subject, &rbac_action, &path_str)
        {
            return Err(AccessDenied {
                agent: ctx.agent_name.clone(),
                resource: path_str.to_string(),
                layer: DenyLayer::Rbac,
                reason: "RBAC policy denies access to this path".into(),
                suggestion: Some("Review the RBAC policy.".into()),
            });
        }

        // Layer 2: Path permissions (allowed_paths / denied_paths)
        if !access.can_access_path(&ctx.agent_name, &path_str) {
            return Err(AccessDenied {
                agent: ctx.agent_name.clone(),
                resource: path_str.to_string(),
                layer: DenyLayer::Permission,
                reason: format!("Path '{path_str}' is not in allowed_paths or is in denied_paths"),
                suggestion: Some("Review the allowed_paths / denied_paths settings.".into()),
            });
        }

        // Layer 2 (continued): Workspace sandbox
        if let Some(ws) = access.get_workspace_for_agent(&ctx.agent_name)
            && !access.is_path_in_workspace(&ws, &path_str)
        {
            // Record sandbox violation separately
            self.audit.record(AuditEvent::SandboxViolation {
                timestamp: chrono::Utc::now(),
                agent: ctx.agent_name.clone(),
                path: path_str.to_string(),
                workspace: ws.clone(),
            });
            return Err(AccessDenied {
                agent: ctx.agent_name.clone(),
                resource: path_str.to_string(),
                layer: DenyLayer::Permission,
                reason: format!("Path '{path_str}' is outside the '{ws}' workspace boundary"),
                suggestion: None,
            });
        }

        Ok(())
    }

    fn check_exec(
        &self,
        ctx: &AgentContext,
        binary: &str,
        args: &[String],
    ) -> Result<(), AccessDenied> {
        // Layer 0: CSpace (exec capability)
        let resource = ResourceRef::Exec {
            mode: "structured".to_string(),
        };
        if !ctx.cspace.can(&resource, Rights::EXECUTE) {
            // Also try shell mode CSpace
            let shell_resource = ResourceRef::Exec {
                mode: "shell".to_string(),
            };
            if !ctx.cspace.can(&shell_resource, Rights::EXECUTE)
                && !ctx.cspace.can(&resource, Rights::EXECUTE)
            {
                return Err(AccessDenied {
                    agent: ctx.agent_name.clone(),
                    resource: binary.to_string(),
                    layer: DenyLayer::Capability,
                    reason: "CSpace lacks Exec capability".into(),
                    suggestion: Some("Add the Exec capability to the Seed.".into()),
                });
            }
        }

        // Layer 1+2: Permissions — agent must be allowed the 'exec' tool.
        // Per-binary control is handled by Layer 3 (ExecConfig allowlist), so a
        // single permission check avoids double audit-log entries.
        let mut access = self.access.lock();
        if !access.can_use_tool(&ctx.agent_name, "exec") {
            return Err(AccessDenied {
                agent: ctx.agent_name.clone(),
                resource: binary.to_string(),
                layer: DenyLayer::Permission,
                reason: format!("Agent lacks permission to execute '{binary}'"),
                suggestion: None,
            });
        }

        // Layer 3: ExecConfig — binary allowlist
        if !self.exec_config.is_binary_allowed(binary) {
            return Err(AccessDenied {
                agent: ctx.agent_name.clone(),
                resource: binary.to_string(),
                layer: DenyLayer::ExecPolicy,
                reason: format!("Binary '{binary}' is not in the allowlist"),
                suggestion: Some("Add it to exec.allowed_commands.".into()),
            });
        }

        // Layer 3: ExecConfig — metacharacter blocking
        if has_metacharacters(args) {
            return Err(AccessDenied {
                agent: ctx.agent_name.clone(),
                resource: binary.to_string(),
                layer: DenyLayer::ExecPolicy,
                reason: "Arguments contain shell metacharacters or path traversal patterns".into(),
                suggestion: None,
            });
        }

        Ok(())
    }

    fn check_network(&self, ctx: &AgentContext) -> Result<(), AccessDenied> {
        let mut access = self.access.lock();
        if !access.can_access_network(&ctx.agent_name) {
            return Err(AccessDenied {
                agent: ctx.agent_name.clone(),
                resource: "<network>".into(),
                layer: DenyLayer::Permission,
                reason: "Network access is disabled".into(),
                suggestion: Some("Set permissions.network_access to true.".into()),
            });
        }
        Ok(())
    }

    fn check_fork(&self, ctx: &AgentContext) -> Result<(), AccessDenied> {
        // Layer 0: CSpace
        let resource = ResourceRef::KernelDomain {
            domain: "agent".to_string(),
        };
        if !ctx.cspace.can(&resource, Rights::EXECUTE) {
            return Err(AccessDenied {
                agent: ctx.agent_name.clone(),
                resource: "fork".into(),
                layer: DenyLayer::Capability,
                reason: "CSpace lacks agent-management capability".into(),
                suggestion: None,
            });
        }

        // Layer 2: Permissions
        let access = self.access.lock();
        if !access.can_fork(&ctx.agent_name) {
            return Err(AccessDenied {
                agent: ctx.agent_name.clone(),
                resource: "fork".into(),
                layer: DenyLayer::Permission,
                reason: "Agent lacks fork permission".into(),
                suggestion: Some("Set permissions.can_fork to true.".into()),
            });
        }
        Ok(())
    }

    // ─── Audit Recording ─────────────────────────────────────────────

    fn record_check(&self, req: &CheckRequest<'_>, result: &Result<(), AccessDenied>) {
        let event = match result {
            Ok(()) => self.allowed_event(req),
            Err(denied) => self.denied_event(req, denied),
        };
        self.audit.record(event);
    }

    fn allowed_event(&self, req: &CheckRequest<'_>) -> AuditEvent {
        let ctx = req.agent_context();
        let ts = chrono::Utc::now();
        match req {
            CheckRequest::Tool { tool_name, .. } => AuditEvent::ToolAccess {
                timestamp: ts,
                agent: ctx.agent_name.clone(),
                tool: tool_name.to_string(),
                allowed: true,
                layer: None,
                reason: None,
            },
            CheckRequest::Path { path, mode, .. } => AuditEvent::PathAccess {
                timestamp: ts,
                agent: ctx.agent_name.clone(),
                path: path.to_string_lossy().to_string(),
                mode: mode.to_string(),
                allowed: true,
                layer: None,
                reason: None,
            },
            CheckRequest::Exec { binary, .. } => AuditEvent::ExecAccess {
                timestamp: ts,
                agent: ctx.agent_name.clone(),
                binary: binary.to_string(),
                allowed: true,
                layer: None,
                reason: None,
            },
            CheckRequest::Network { .. } => AuditEvent::ToolAccess {
                timestamp: ts,
                agent: ctx.agent_name.clone(),
                tool: "network".into(),
                allowed: true,
                layer: None,
                reason: None,
            },
            CheckRequest::Fork { .. } => AuditEvent::ToolAccess {
                timestamp: ts,
                agent: ctx.agent_name.clone(),
                tool: "fork".into(),
                allowed: true,
                layer: None,
                reason: None,
            },
        }
    }

    fn denied_event(&self, req: &CheckRequest<'_>, denied: &AccessDenied) -> AuditEvent {
        let ctx = req.agent_context();
        let ts = chrono::Utc::now();
        let layer = Some(denied.layer.to_string());
        let reason = Some(denied.reason.clone());

        match req {
            CheckRequest::Tool { .. } => AuditEvent::ToolAccess {
                timestamp: ts,
                agent: ctx.agent_name.clone(),
                tool: denied.resource.clone(),
                allowed: false,
                layer,
                reason,
            },
            CheckRequest::Path { path, mode, .. } => AuditEvent::PathAccess {
                timestamp: ts,
                agent: ctx.agent_name.clone(),
                path: path.to_string_lossy().to_string(),
                mode: mode.to_string(),
                allowed: false,
                layer,
                reason,
            },
            CheckRequest::Exec { .. } => AuditEvent::ExecAccess {
                timestamp: ts,
                agent: ctx.agent_name.clone(),
                binary: denied.resource.clone(),
                allowed: false,
                layer,
                reason,
            },
            CheckRequest::Network { .. } => AuditEvent::ToolAccess {
                timestamp: ts,
                agent: ctx.agent_name.clone(),
                tool: "network".into(),
                allowed: false,
                layer,
                reason,
            },
            CheckRequest::Fork { .. } => AuditEvent::ToolAccess {
                timestamp: ts,
                agent: ctx.agent_name.clone(),
                tool: "fork".into(),
                allowed: false,
                layer,
                reason,
            },
        }
    }
}

impl std::fmt::Debug for AccessGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessGate").finish()
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access_manager::AgentPermissions;
    use crate::access_manager::audit_sink::NoOpAuditSink;
    use crate::config::AllowlistMode;

    /// Helper: build an AccessGate with a configured agent.
    fn make_gate() -> (AccessGate, AgentContext) {
        let mut access = AccessManager::new();

        // Create the context first to get a stable agent_id
        let ctx = AgentContext::test_fixture("test-agent");

        // Set up permissions for test agent
        let mut perms = AgentPermissions::for_new_agent("test-agent");
        perms.allow_path("/workspace/**");
        perms.allow_path("/tmp/**");
        access.set_permissions(perms);

        // Assign RBAC role using the same agent_id as the context
        let subject = Subject::Agent(ctx.agent_id);
        access
            .rbac_manager_mut()
            .assign_role(subject, crate::access_manager::Role::Superuser);

        let gate = AccessGate::new(
            Arc::new(Mutex::new(access)),
            Arc::new(ExecConfig {
                allowlist_mode: AllowlistMode::Permissive, // Allow all for general tests
                ..Default::default()
            }),
            Arc::new(NoOpAuditSink),
        );

        (gate, ctx)
    }

    /// Helper: build an AccessGate with Enforced mode and specific allowed commands.
    fn make_enforced_gate(allowed_commands: Vec<&str>) -> (AccessGate, AgentContext) {
        let mut access = AccessManager::new();
        let ctx = AgentContext::test_fixture("test-agent");

        let perms = AgentPermissions::for_new_agent("test-agent");
        access.set_permissions(perms);

        let subject = Subject::Agent(ctx.agent_id);
        access
            .rbac_manager_mut()
            .assign_role(subject, crate::access_manager::Role::Superuser);

        let config = ExecConfig {
            allowlist_mode: AllowlistMode::Enforced,
            allowed_commands: allowed_commands.into_iter().map(String::from).collect(),
            ..Default::default()
        };

        let gate = AccessGate::new(
            Arc::new(Mutex::new(access)),
            Arc::new(config),
            Arc::new(NoOpAuditSink),
        );

        (gate, ctx)
    }

    // ─── Tool checks ────────────────────────────────────────────────

    #[test]
    fn test_tool_access_allowed() {
        let (gate, ctx) = make_gate();
        let result = gate.check(CheckRequest::Tool {
            context: &ctx,
            tool_name: "bash",
        });
        assert!(result.is_ok(), "bash should be allowed: {:?}", result);
    }

    #[test]
    fn test_tool_access_web_search_always_on() {
        // Regression: web_search + get_search_results are registered
        // unconditionally for every agent (register_always_on) and must
        // pass Layer 0 even when the agent's CSpace carries no matching
        // capability. Before this fix, the test_fixture CSpace (which
        // grants no web_search cap) caused a hard deny — the
        // triple-deadlock described in RFC-017 Q3.
        let (gate, ctx) = make_gate();
        let result = gate.check(CheckRequest::Tool {
            context: &ctx,
            tool_name: "web_search",
        });
        assert!(result.is_ok(), "web_search is always-on: {:?}", result);

        let result = gate.check(CheckRequest::Tool {
            context: &ctx,
            tool_name: "get_search_results",
        });
        assert!(
            result.is_ok(),
            "get_search_results is always-on: {:?}",
            result
        );
    }

    #[test]
    fn test_tool_access_unknown_agent_denied() {
        let gate = AccessGate::new(
            Arc::new(Mutex::new(AccessManager::new())), // empty — no permissions
            Arc::new(ExecConfig::default()),
            Arc::new(NoOpAuditSink),
        );
        let ctx = AgentContext::test_fixture("unknown");

        let result = gate.check(CheckRequest::Tool {
            context: &ctx,
            tool_name: "exec",
        });
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.layer, DenyLayer::Permission);
    }

    // ─── Exec checks ────────────────────────────────────────────────

    #[test]
    fn test_exec_allowed_permissive() {
        let (gate, ctx) = make_gate();
        let result = gate.check(CheckRequest::Exec {
            context: &ctx,
            binary: "echo",
            args: &["hello".to_string()],
        });
        assert!(result.is_ok(), "echo should be allowed in permissive mode");
    }

    #[test]
    fn test_exec_denied_enforced() {
        let (gate, ctx) = make_enforced_gate(vec!["git"]);
        let result = gate.check(CheckRequest::Exec {
            context: &ctx,
            binary: "rm",
            args: &[],
        });
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().layer, DenyLayer::ExecPolicy);
    }

    #[test]
    fn test_exec_metacharacters_denied() {
        let (gate, ctx) = make_enforced_gate(vec!["echo"]);
        let result = gate.check(CheckRequest::Exec {
            context: &ctx,
            binary: "echo",
            args: &["foo; rm -rf /".to_string()],
        });
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().layer, DenyLayer::ExecPolicy);
    }

    #[test]
    fn test_exec_path_traversal_denied() {
        let (gate, ctx) = make_enforced_gate(vec!["cat"]);
        let result = gate.check(CheckRequest::Exec {
            context: &ctx,
            binary: "cat",
            args: &["../etc/passwd".to_string()],
        });
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().layer, DenyLayer::ExecPolicy);
    }

    #[test]
    fn test_exec_enforced_allowed() {
        let (gate, ctx) = make_enforced_gate(vec!["echo", "git"]);
        let result = gate.check(CheckRequest::Exec {
            context: &ctx,
            binary: "echo",
            args: &["hello".to_string(), "world".to_string()],
        });
        assert!(result.is_ok(), "listed binary should be allowed");
    }

    // ─── Path checks ────────────────────────────────────────────────

    #[test]
    fn test_path_read_allowed() {
        let (gate, ctx) = make_gate();
        let result = gate.check(CheckRequest::Path {
            context: &ctx,
            path: Path::new("/workspace/project/file.rs"),
            mode: PathMode::Read,
        });
        assert!(result.is_ok(), "workspace path should be readable");
    }

    // ─── Network checks ─────────────────────────────────────────────

    #[test]
    fn test_network_denied_by_default() {
        let (gate, ctx) = make_gate();
        let result = gate.check(CheckRequest::Network { context: &ctx });
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().layer, DenyLayer::Permission);
    }

    // ─── Fork checks ────────────────────────────────────────────────

    #[test]
    fn test_fork_denied_by_default() {
        let (gate, ctx) = make_gate();
        let result = gate.check(CheckRequest::Fork { context: &ctx });
        // Default AgentPermissions has can_fork = false
        // But we need CSpace to have agent domain first
        // With an empty CSpace (test_fixture), CSpace check will fail
        assert!(result.is_err());
    }

    // ─── Deny layer display ─────────────────────────────────────────

    #[test]
    fn test_deny_layer_display() {
        assert_eq!(format!("{}", DenyLayer::Capability), "CSpace");
        assert_eq!(format!("{}", DenyLayer::Rbac), "RBAC");
        assert_eq!(format!("{}", DenyLayer::Permission), "Permissions");
        assert_eq!(format!("{}", DenyLayer::ExecPolicy), "ExecPolicy");
    }

    // ─── Metacharacter detection ─────────────────────────────────────

    #[test]
    fn test_no_metacharacters_in_clean_args() {
        assert!(!has_metacharacters(&["hello".into(), "world".into()]));
    }

    #[test]
    fn test_metacharacters_semicolon() {
        assert!(has_metacharacters(&["foo;bar".into()]));
    }

    #[test]
    fn test_metacharacters_pipe() {
        assert!(has_metacharacters(&["a | b".into()]));
    }

    #[test]
    fn test_metacharacters_dollar() {
        assert!(has_metacharacters(&["$(whoami)".into()]));
    }

    #[test]
    fn test_metacharacters_path_traversal() {
        assert!(has_metacharacters(&["../etc/passwd".into()]));
    }

    // ─── AccessDenied Display ────────────────────────────────────────

    #[test]
    fn test_access_denied_display() {
        let denied = AccessDenied {
            agent: "test".into(),
            resource: "exec".into(),
            layer: DenyLayer::ExecPolicy,
            reason: "not in allowlist".into(),
            suggestion: Some("add to config".into()),
        };
        let s = format!("{}", denied);
        assert!(s.contains("[ExecPolicy]"));
        assert!(s.contains("not in allowlist"));
    }

    // ─── Foundation package capabilities (RFC-048 §4) ────────────────

    /// Helper: a Foundation `shell.execute` package mapped through the
    /// reviewed requirement table.
    fn shell_package() -> crate::foundation::packages::ImportedPackage {
        crate::foundation::packages::ImportedPackage {
            id: "oxi.shell-helper".into(),
            version: "0.1.0".into(),
            source: "local://./oxi.shell-helper".into(),
            digest: "0".repeat(64),
            trust: crate::foundation::packages::SourceTrust::Unsigned,
            targets: vec!["oxios".into()],
            capabilities: vec![crate::foundation::packages::requirement_to_resource(
                crate::foundation::packages::AbstractRequirement::ShellExecute,
            )],
            persona: None,
        }
    }

    #[test]
    fn foundation_shell_requirement_denied_when_cspace_lacks_exec() {
        // Agent CSpace carries no Exec capability: the package requires
        // `shell.execute`, but a package never bypasses Layer 0 — the
        // mapped capability must already be in the agent's resolved
        // CSpace for the gate to admit it.
        let cspace = crate::capability::CSpace::new(crate::types::AgentId::new_v4());
        let ctx = AgentContext::test_fixture_with_cspace("pkg-agent", cspace);
        let (gate, _) = make_gate();
        let err = gate
            .check(CheckRequest::Exec {
                context: &ctx,
                binary: "ls",
                args: &[],
            })
            .unwrap_err();
        assert_eq!(err.layer, DenyLayer::Capability);
    }

    #[test]
    fn foundation_cspace_capability_still_denied_by_permissions() {
        // The package's `shell.execute` maps into the CSpace (Layer 0
        // passes), but the agent holds no `exec` tool grant — Layer 2
        // denies. A verified digest is never an authorization decision.
        let template = crate::foundation::packages::apply_to_template(
            crate::capability::template::CapabilityTemplate::worker(),
            &shell_package(),
        );
        let ctx = AgentContext::test_fixture_with_cspace("pkg-agent", template.build());
        // AccessManager has no permissions for "pkg-agent" (make_gate's
        // fixture is for "test-agent").
        let (gate, _) = make_gate();
        let err = gate
            .check(CheckRequest::Exec {
                context: &ctx,
                binary: "ls",
                args: &[],
            })
            .unwrap_err();
        assert_eq!(err.layer, DenyLayer::Permission);
    }
}
