//! Gated tool registry — intercepts all tool executions through AccessGate.
//!
//! Instead of wrapping individual tools (which requires access to tool internals),
//! this module provides a registry-level proxy that checks permissions before
//! delegating to the real tool. This means:
//!
//! - No changes to individual tool code
//! - New tools are automatically protected
//! - oxicode-sdk crate tools (ReadTool, WriteTool, etc.) are covered without modification
//!
//! RFC-035: After the structural `AccessGate` (CSpace / RBAC / Permissions /
//! ExecConfig) passes, `GatedTool` consults the [`ApprovalGate`] (Step 2.5).
//! On [`ApprovalDecision::Allow`] the call delegates; on
//! [`ApprovalDecision::RequireApproval`] the tool requests a user decision via
//! the kernel event bus, blocking until resolved (or until the 120s timeout).
//! This is the unified mechanism that supersedes the bespoke exec-only shell
//! approval.

use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use oxicode_sdk::{AgentTool, AgentToolResult, ToolContext};
use serde_json::Value;

use crate::access_manager::{
    AccessDenied, AccessGate, AgentContext, CheckRequest, DenyLayer, PathMode,
};
use crate::approval::{ApprovalDecision, ApprovalGate, ToolCall};
use crate::event_bus::EventBus;
use crate::tools::{PathAccessResult, PendingPathAccess, PendingToolApprovals, ToolApprovalResult};

/// Default timeout for awaiting a user approval response.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);
/// How often a blocked tool call re-checks whether a policy change now lets it
/// auto-run (e.g. the user switched to AutoRun, or added a grant, while the
/// approval card is still showing). Short enough to feel instant; the per-poll
/// gate evaluation is a HashMap lookup + blacklist scan, so the cost is nil.
const APPROVAL_REEVAL_INTERVAL: Duration = Duration::from_millis(500);
/// Default timeout for a path-access card. Same window as tool approval.
const PATH_ACCESS_TIMEOUT: Duration = Duration::from_secs(120);
/// Re-eval interval for path-access cards — mirrors APPROVAL_REEVAL_INTERVAL.
const PATH_ACCESS_REEVAL_INTERVAL: Duration = Duration::from_millis(500);

// ─── Path Extraction ────────────────────────────────────────────────────────

/// Tool names that perform file operations and need path checking.
const FILE_TOOLS: &[&str] = &["read", "write", "edit", "ls", "find", "grep"];

/// Extract the target path from tool parameters.
fn extract_path_from_params(tool_name: &str, params: &Value) -> Option<String> {
    if !FILE_TOOLS.contains(&tool_name) {
        return None;
    }

    // Most file tools use "path" parameter
    params
        .get("path")
        .and_then(|v| v.as_str())
        .map(String::from)
}

/// Determine path access mode from tool name.
fn path_mode_for_tool(tool_name: &str) -> PathMode {
    match tool_name {
        "write" | "edit" => PathMode::Write,
        _ => PathMode::Read,
    }
}

/// Format an access denied error for tool execution.
fn format_denied(denied: &AccessDenied) -> String {
    let layer_tag = match denied.layer {
        DenyLayer::Capability => "[CSpace]",
        DenyLayer::Rbac => "[RBAC]",
        DenyLayer::Permission => "[Permissions]",
        DenyLayer::ExecPolicy => "[ExecPolicy]",
    };
    format!(
        "🔒 Access denied: {} — {} {}",
        denied.reason,
        denied.suggestion.as_deref().unwrap_or(""),
        layer_tag
    )
}

// ─── Gated Tool ─────────────────────────────────────────────────────────────

/// A tool wrapper that checks permissions before execution.
///
/// Wraps any `AgentTool` and performs access control before delegating
/// to the inner tool's `execute` method.
///
/// When constructed via [`GatedTool::with_approval`], the wrapper also
/// consults the [`ApprovalGate`] (RFC-035) after the structural `AccessGate`
/// and surfaces `RequireApproval` decisions as user prompts on the kernel
/// event bus.
pub struct GatedTool<T: AgentTool> {
    inner: T,
    gate: Arc<AccessGate>,
    context: AgentContext,
    /// RFC-035 approval gate. `None` ⇒ no approval logic, executable as soon
    /// as `AccessGate` allows.
    approval_gate: Option<Arc<ApprovalGate>>,
    /// Kernel event bus used to publish `KernelEvent::ApprovalRequested`
    /// and `KernelEvent::PathAccessRequested`.
    event_bus: Option<EventBus>,
    /// Registry of pending tool-approval decisions (RFC-035).
    pending_approvals: Option<Arc<PendingToolApprovals>>,
    /// Registry of pending path-access requests. When an agent tries to
    /// read/write outside its `allowed_paths`, the denial is surfaced as
    /// an interactive card (create Mount / temp-allow / deny) instead of
    /// a hard error. `None` ⇒ headless, returns the error immediately.
    pending_path_access: Option<Arc<PendingPathAccess>>,
}

impl<T: AgentTool> GatedTool<T> {
    /// Create a new gated tool with no approval pipeline.
    ///
    /// Existing call sites that don't yet pass an approval gate still compile
    /// — the approval pipeline is opted-in via [`GatedTool::with_approval`].
    pub fn new(inner: T, gate: Arc<AccessGate>, context: AgentContext) -> Self {
        Self {
            inner,
            gate,
            context,
            approval_gate: None,
            event_bus: None,
            pending_approvals: None,
            pending_path_access: None,
        }
    }

    /// Construct a gated tool with the RFC-035 approval pipeline wired.
    ///
    /// All three opt-in arguments may be `None` (legacy callers, tests,
    /// headless paths), but production paths always supply them.
    pub fn with_approval(
        inner: T,
        gate: Arc<AccessGate>,
        context: AgentContext,
        approval_gate: Option<Arc<ApprovalGate>>,
        event_bus: Option<EventBus>,
        pending_approvals: Option<Arc<PendingToolApprovals>>,
        pending_path_access: Option<Arc<PendingPathAccess>>,
    ) -> Self {
        Self {
            inner,
            gate,
            context,
            approval_gate,
            event_bus,
            pending_approvals,
            pending_path_access,
        }
    }
}

impl<T: AgentTool> std::fmt::Debug for GatedTool<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatedTool")
            .field("name", &self.inner.name())
            .finish()
    }
}

#[async_trait]
impl<T: AgentTool + 'static> AgentTool for GatedTool<T> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn label(&self) -> &str {
        self.inner.label()
    }

    fn description(&self) -> &'static str {
        "Execute commands and access system resources. Permissions enforced by AccessGate."
    }

    fn parameters_schema(&self) -> Value {
        self.inner.parameters_schema()
    }

    async fn execute(
        &self,
        tool_call_id: &str,
        params: Value,
        signal: Option<tokio::sync::oneshot::Receiver<()>>,
        ctx: &ToolContext,
    ) -> Result<AgentToolResult, oxicode_sdk::ToolError> {
        let tool_name = self.inner.name();

        // Step 1: Check tool access permission (CSpace / RBAC / Permissions / ExecConfig).
        let check = CheckRequest::Tool {
            context: &self.context,
            tool_name,
        };
        if let Err(denied) = self.gate.check(check) {
            tracing::warn!(
                agent = %denied.agent,
                tool = %tool_name,
                layer = ?denied.layer,
                "GatedTool: tool access denied"
            );
            // Return Err (not Ok(error-result)): oxicode-agent's loop only sets
            // `is_error` on ToolExecutionEnd for Err results or after_tool_call
            // hook overrides — an Ok soft-error streams as `is_error: false`,
            // so the web UI renders denials as successful tool calls. Err keeps
            // the same denial text for the LLM (the loop converts it back to an
            // error AgentToolResult) while flagging the call as failed.
            return Err(format_denied(&denied));
        }

        // Step 2: For file tools, check path access permission. On denial,
        // surface an interactive path-access card (create Mount / temp-allow /
        // deny) when the event bus and path-access registry are wired;
        // otherwise return the hard error (headless / test path).
        if let Some(path) = extract_path_from_params(tool_name, &params) {
            let mode = path_mode_for_tool(tool_name);
            let path_check = CheckRequest::Path {
                context: &self.context,
                path: Path::new(&path),
                mode,
            };
            if let Err(denied) = self.gate.check(path_check) {
                tracing::warn!(
                    agent = %denied.agent,
                    path = %path,
                    tool = %tool_name,
                    layer = ?denied.layer,
                    "GatedTool: path access denied"
                );
                match (self.event_bus.as_ref(), self.pending_path_access.as_ref()) {
                    (Some(bus), Some(registry)) => {
                        let mode_str = match mode {
                            PathMode::Read => "read",
                            PathMode::Write => "write",
                        };
                        let (request_id, rx) = registry.register(
                            tool_name.to_string(),
                            path.clone(),
                            mode_str.to_string(),
                            self.context.agent_name.clone(),
                        );
                        let _ = bus.publish(crate::event_bus::KernelEvent::PathAccessRequested {
                            id: request_id,
                            tool_name: tool_name.to_string(),
                            path: path.chars().take(200).collect(),
                            mode: mode_str.to_string(),
                            agent_name: self.context.agent_name.clone(),
                            reason: denied.reason.clone(),
                            session_id: None,
                        });
                        tracing::info!(
                            request_id = %request_id,
                            path = %path,
                            tool = %tool_name,
                            "path access requested — awaiting user decision"
                        );
                        let deadline = tokio::time::Instant::now() + PATH_ACCESS_TIMEOUT;
                        tokio::pin!(rx);
                        let allowed = loop {
                            tokio::select! {
                                biased;
                                res = &mut rx => match res {
                                    Ok(PathAccessResult::Allowed) => break true,
                                    _ => break false,
                                },
                                _ = tokio::time::sleep_until(deadline) => {
                                    let _ = registry
                                        .resolve(request_id, PathAccessResult::Denied);
                                    break false;
                                }
                                _ = tokio::time::sleep(PATH_ACCESS_REEVAL_INTERVAL) => {
                                    if self.gate.check(CheckRequest::Path {
                                        context: &self.context,
                                        path: Path::new(&path),
                                        mode,
                                    }).is_ok() {
                                        let _ = registry
                                            .resolve(request_id, PathAccessResult::Allowed);
                                        break true;
                                    }
                                }
                            }
                        };
                        if !allowed {
                            // Err — see the Step 1 note on is_error propagation.
                            return Err(format!("🔒 Path access denied: {}", denied.reason));
                        }
                        // Path now granted — fall through to Step 2.5.
                    }
                    _ => {
                        // Headless: no event bus / registry — hard deny.
                        // Err — see the Step 1 note on is_error propagation.
                        return Err(format!("🔒 Path access denied: {}", denied.reason));
                    }
                }
            }
        }

        // Step 2.5 (RFC-035): ApprovalGate evaluation — decides whether the
        // call auto-runs or surfaces a human-in-the-loop approval card.
        if let Some(approval_gate) = &self.approval_gate {
            // `binary` is the binary field ONLY (RFC-035 Task 14). The
            // `command` field is the full shell string and would pollute
            // `grant_key()` (which uses `binary`) in allow-list mode. The
            // approval card's display `resource` derives a richer string
            // from `command` separately (below), but `ToolCall.binary`
            // must stay binary-only so `exec:<binary>` / `exec:shell`
            // grant keys are correct.
            let binary = if tool_name == "exec" {
                params.get("binary").and_then(|v| v.as_str())
            } else {
                None
            };
            let call = ToolCall {
                tool: tool_name,
                binary,
                args: &params,
            };
            match approval_gate.evaluate(&call) {
                ApprovalDecision::Allow => {
                    // fall through to Step 3
                }
                ApprovalDecision::RequireApproval { reason } => {
                    match (self.event_bus.as_ref(), self.pending_approvals.as_ref()) {
                        (Some(bus), Some(pending)) => {
                            let approvals = pending.clone();
                            let (approval_id, rx) =
                                approvals.register(tool_name.to_string(), call.grant_key());
                            let action = format!("tool:{tool_name}");
                            // Resource shown on the approval card. Prefer the
                            // shell `command` (most informative), fall back to
                            // the structured `binary`, then the bare tool name.
                            // Independent of `binary` because shell exec has
                            // `binary = None` after the Task 14 overload fix.
                            let resource = params
                                .get("command")
                                .and_then(|v| v.as_str())
                                .map(String::from)
                                .or_else(|| binary.map(String::from))
                                .unwrap_or_else(|| tool_name.to_string());
                            // Publish using the exact KernelEvent::ApprovalRequested
                            // field shape (event_bus.rs:83-96). The frontend uses
                            // this to render the approval card.
                            let _ = bus.publish(crate::event_bus::KernelEvent::ApprovalRequested {
                                id: approval_id,
                                tool_name: tool_name.to_string(),
                                action,
                                resource: resource.chars().take(200).collect(),
                                reason: reason.clone(),
                                session_id: None,
                            });
                            // Await the user's decision, but periodically
                            // re-evaluate the gate. Without re-evaluation a
                            // policy change made while the card is shown
                            // (switching to AutoRun, or granting the tool)
                            // would strand the call — the oneshot only fires
                            // on an explicit click, so the agent would freeze
                            // until the 120s timeout. Polling the SAME gate
                            // that issued the card means the live policy —
                            // dynamic resolvers and the security blacklist
                            // included — is applied exactly as a fresh call.
                            let deadline = tokio::time::Instant::now() + APPROVAL_TIMEOUT;
                            tokio::pin!(rx);
                            let approved = loop {
                                tokio::select! {
                                    biased;
                                    // Explicit user decision wins immediately.
                                    res = &mut rx => match res {
                                        Ok(ToolApprovalResult::Approved) => {
                                            tracing::info!(
                                                approval_id = %approval_id,
                                                tool = %tool_name,
                                                "tool call approved by user"
                                            );
                                            break true;
                                        }
                                        _ => {
                                            let _ = approvals
                                                .resolve(approval_id, ToolApprovalResult::Denied);
                                            break false;
                                        }
                                    },
                                    // Hard timeout — deny and surface an error.
                                    _ = tokio::time::sleep_until(deadline) => {
                                        let _ = approvals
                                            .resolve(approval_id, ToolApprovalResult::Denied);
                                        break false;
                                    }
                                    // Re-evaluate under the live config; if the
                                    // call would now auto-run, approve ourselves.
                                    _ = tokio::time::sleep(APPROVAL_REEVAL_INTERVAL) => {
                                        if matches!(
                                            approval_gate.evaluate(&call),
                                            ApprovalDecision::Allow
                                        ) {
                                            tracing::info!(
                                                approval_id = %approval_id,
                                                tool = %tool_name,
                                                "tool call auto-approved after policy change"
                                            );
                                            let _ = approvals
                                                .resolve(approval_id, ToolApprovalResult::Approved);
                                            break true;
                                        }
                                        // Policy still requires approval — poll again.
                                    }
                                }
                            };
                            if !approved {
                                // Err — see the Step 1 note on is_error propagation.
                                return Err(format!(
                                    "Tool execution was denied or timed out ({}s).",
                                    APPROVAL_TIMEOUT.as_secs()
                                ));
                            }
                            // fall through to Step 3
                        }
                        _ => {
                            tracing::warn!(
                                tool = %tool_name,
                                "ApprovalGate requires approval but no event bus / pending approvals \
                                 wired — allowing (headless path, tool would deadlock)"
                            );
                            // fall through to Step 3
                        }
                    }
                }
            }
        }

        // Step 3: Access and (RFC-035) approval both granted — delegate.
        self.inner.execute(tool_call_id, params, signal, ctx).await
    }
}

/// Wrap a tool with access control.
///
/// Convenience function for creating `GatedTool` instances.
pub fn gate_tool<T: AgentTool + 'static>(
    tool: T,
    gate: Arc<AccessGate>,
    context: AgentContext,
) -> GatedTool<T> {
    GatedTool::new(tool, gate, context)
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access_manager::{AccessManager, AgentPermissions, NoOpAuditSink, Role, Subject};
    use crate::config::ExecConfig;
    use oxicode_sdk::ReadTool;
    use parking_lot::Mutex;

    fn make_gate_for_test() -> Arc<AccessGate> {
        let mut access = AccessManager::new();
        let perms = AgentPermissions::for_new_agent("test-agent");
        access.set_permissions(perms);

        let subject = Subject::Agent(uuid::Uuid::new_v4());
        access
            .rbac_manager_mut()
            .assign_role(subject, Role::Superuser);

        Arc::new(AccessGate::new(
            Arc::new(Mutex::new(access)),
            Arc::new(ExecConfig::default()),
            Arc::new(NoOpAuditSink),
        ))
    }

    #[test]
    fn test_gated_tool_preserves_name() {
        let gate = make_gate_for_test();
        let ctx = AgentContext::test_fixture("test-agent");
        let tool = GatedTool::new(ReadTool::new(), gate, ctx);
        assert_eq!(tool.name(), "read");
    }

    #[test]
    fn test_extract_path_read_tool() {
        let params = serde_json::json!({"path": "/workspace/file.rs"});
        assert_eq!(
            extract_path_from_params("read", &params),
            Some("/workspace/file.rs".to_string())
        );
    }

    #[test]
    fn test_extract_path_exec_tool() {
        let params = serde_json::json!({"command": "echo hello"});
        assert_eq!(extract_path_from_params("exec", &params), None);
    }

    #[test]
    fn test_path_mode_for_tool() {
        assert_eq!(path_mode_for_tool("write"), PathMode::Write);
        assert_eq!(path_mode_for_tool("edit"), PathMode::Write);
        assert_eq!(path_mode_for_tool("read"), PathMode::Read);
        assert_eq!(path_mode_for_tool("ls"), PathMode::Read);
    }

    /// A stub CSpace-driven tool ("exec") whose inner execute always
    /// succeeds — the gate decision is the only variable under test.
    struct StubExecTool;

    #[async_trait]
    impl AgentTool for StubExecTool {
        fn name(&self) -> &str {
            "exec"
        }

        fn label(&self) -> &str {
            "exec"
        }

        fn description(&self) -> &'static str {
            "stub exec tool for gate tests"
        }

        fn parameters_schema(&self) -> Value {
            serde_json::json!({})
        }

        async fn execute(
            &self,
            _tool_call_id: &str,
            _params: Value,
            _signal: Option<tokio::sync::oneshot::Receiver<()>>,
            _ctx: &ToolContext,
        ) -> Result<AgentToolResult, oxicode_sdk::ToolError> {
            Ok(AgentToolResult::success("executed"))
        }
    }

    /// A CSpace denial must surface as `Err` — oxicode-agent's loop derives
    /// `ToolExecutionEnd.is_error` only from Err results (an Ok soft-error
    /// streams as success), and the web UI renders the failure state off that
    /// flag. Regression guard for the neutral-denial-card bug.
    #[tokio::test]
    async fn cspace_denial_returns_err_with_denial_text() {
        let gate = make_gate_for_test();
        // Empty CSpace: "exec" is CSpace-driven, so Layer 0 must deny.
        let ctx = AgentContext::test_fixture_with_cspace(
            "test-agent",
            crate::capability::CSpace::new(uuid::Uuid::new_v4()),
        );
        let tool = GatedTool::new(StubExecTool, gate, ctx);

        let res = tool
            .execute(
                "c1",
                serde_json::json!({"command": "ls"}),
                None,
                &ToolContext::default(),
            )
            .await;

        assert!(res.is_err(), "CSpace denial must be Err, got Ok: {res:?}");
        let msg = res.expect_err("checked is_err above");
        assert!(msg.contains("🔒"), "denial text missing lock marker: {msg}");
        assert!(
            msg.contains("[CSpace]"),
            "denial text missing layer tag: {msg}"
        );
    }

    #[test]
    fn test_format_denied() {
        let denied = AccessDenied {
            agent: "test".into(),
            resource: "exec".into(),
            layer: DenyLayer::ExecPolicy,
            reason: "not in allowlist".into(),
            suggestion: Some("add to config".into()),
        };
        let s = format_denied(&denied);
        assert!(s.contains("🔒"));
        assert!(s.contains("[ExecPolicy]"));
    }
}
