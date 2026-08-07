//! Approval-mode migration parity tests (RFC-035 Task 14).
//!
//! Proves that the dynamic `ExecPolicyResolver` wired through the gate
//! preserves the pre-RFC-035 behavior for every entry of the parity
//! matrix:
//!
//! 1. Structured exec with an allowed binary in Manual mode runs without
//!    approval (ExecPolicyResolver returns `Auto`).
//! 2. Shell exec in Manual mode still prompts (ExecPolicyResolver returns
//!    `OnDemand` for shell).
//! 3. Auto-run mode runs shell exec without approval (mode policy is
//!    `AutoRun + OnDemand → Allow`).
//! 4. `rm -rf /` is blocked even in Auto-run (SecurityBlacklist escalates
//!    to `Always` in Phase 3, after the dynamic resolver).
//! 5. The binary-overload fix keeps `grant_key()` returning `exec:<binary>`
//!    in allow-list mode (the `command` field must NOT leak into binary).
#![allow(clippy::unwrap_used)] // `.unwrap()` in tests is idiomatic (workspace convention)

use std::collections::HashMap;
use std::sync::Arc;

use oxios_kernel::approval::{
    ApprovalConfig, ApprovalDecision, ApprovalGate, ApprovalMode, ExecPolicyResolver,
    SecurityBlacklist, ToolCall, ToolPolicyResolver, default_blacklist_rules,
    default_tool_policy_map,
};
use parking_lot::RwLock;
use serde_json::json;

/// Build an `Arc<RwLock<Vec<String>>>` snapshot of an allowlist. Mirrors the
/// snapshot form `agent_runtime` uses to construct `ExecPolicyResolver` from
/// `ExecConfig.allowed_commands`.
fn snapshot_allowed(allowed: &[&str]) -> Arc<RwLock<Vec<String>>> {
    Arc::new(RwLock::new(allowed.iter().map(|s| s.to_string()).collect()))
}

/// Construct a gate wired with the SecurityBlacklist (Phase 3) and an
/// ExecPolicyResolver (Phase 2.5). Mirrors what `agent_runtime` builds.
fn gate_with_exec_resolver(mode: ApprovalMode, allowed: &[&str]) -> ApprovalGate {
    let policies = default_tool_policy_map();
    let config = Arc::new(RwLock::new(ApprovalConfig {
        mode,
        allow_list: vec![],
        tool_overrides: HashMap::new(),
    }));
    let exec_resolver = ExecPolicyResolver {
        allowed_commands: snapshot_allowed(allowed),
    };
    let mut dynamic = HashMap::new();
    dynamic.insert(
        "exec".to_string(),
        Box::new(exec_resolver) as Box<dyn ToolPolicyResolver>,
    );
    let blacklist = SecurityBlacklist::new(default_blacklist_rules());
    ApprovalGate::with_dynamic_resolvers(policies, config, vec![Box::new(blacklist)], dynamic)
}

/// Structured exec with an allowed binary runs without approval in Manual.
/// Pre-RFC-035 parity: this is the case the dynamic resolver exists to
/// restore.
#[test]
fn structured_allowed_binary_runs_without_approval_in_manual() {
    let g = gate_with_exec_resolver(ApprovalMode::Manual, &["curl"]);
    let args = json!({"mode": "structured", "binary": "curl"});
    let call = ToolCall {
        tool: "exec",
        binary: Some("curl"),
        args: &args,
    };
    assert!(
        matches!(g.evaluate(&call), ApprovalDecision::Allow),
        "structured exec with allowed binary must auto-run in Manual mode"
    );
}

/// Shell exec in Manual mode still prompts. Pre-RFC-035 parity: shell
/// mode never auto-runs.
#[test]
fn shell_mode_prompts_in_manual() {
    let g = gate_with_exec_resolver(ApprovalMode::Manual, &["curl"]);
    let args = json!({"mode": "shell", "command": "ls -la"});
    let call = ToolCall {
        tool: "exec",
        binary: None,
        args: &args,
    };
    assert!(
        matches!(g.evaluate(&call), ApprovalDecision::RequireApproval { .. }),
        "shell exec must prompt in Manual mode"
    );
}

/// Auto-run mode runs shell exec without approval. New RFC-035 capability
/// — pre-RFC-035 shell exec required approval in every mode.
#[test]
fn autorun_runs_shell_without_approval() {
    let g = gate_with_exec_resolver(ApprovalMode::AutoRun, &["curl"]);
    let args = json!({"mode": "shell", "command": "ls -la"});
    let call = ToolCall {
        tool: "exec",
        binary: None,
        args: &args,
    };
    assert!(
        matches!(g.evaluate(&call), ApprovalDecision::Allow),
        "AutoRun mode must allow shell exec without approval"
    );
}

/// `rm -rf /` is blocked even in Auto-run. SecurityBlacklist escalates to
/// `Always` in Phase 3 (after the dynamic resolver runs in Phase 2.5),
/// so even `AutoRun + OnDemand → Allow` never fires for blacklisted
/// commands.
#[test]
fn blacklist_rm_rf_prompts_even_in_autorun() {
    let g = gate_with_exec_resolver(ApprovalMode::AutoRun, &["curl"]);
    let args = json!({"mode": "shell", "command": "rm -rf /etc"});
    let call = ToolCall {
        tool: "exec",
        binary: None,
        args: &args,
    };
    assert!(
        matches!(g.evaluate(&call), ApprovalDecision::RequireApproval { .. }),
        "rm -rf must prompt even in AutoRun (blacklist escalation)"
    );
}

/// Binary-overload fix: `ToolCall::grant_key()` returns `exec:<binary>`
/// (or `exec:shell`) — never `exec:<full shell command>`. This is what
/// lets allow-list mode remember `exec:curl` cleanly.
#[test]
fn allow_list_grant_key_matches_exec_binary() {
    let call = ToolCall {
        tool: "exec",
        binary: Some("curl"),
        args: &json!({}),
    };
    assert_eq!(
        call.grant_key(),
        "exec:curl",
        "exec grant_key must be 'exec:<binary>'"
    );
    let shell = ToolCall {
        tool: "exec",
        binary: None,
        args: &json!({"mode": "shell", "command": "rm -rf /etc"}),
    };
    assert_eq!(
        shell.grant_key(),
        "exec:shell",
        "shell exec grant_key must be 'exec:shell', not the command"
    );
    let read = ToolCall {
        tool: "read",
        binary: None,
        args: &json!({}),
    };
    assert_eq!(read.grant_key(), "read");
}
