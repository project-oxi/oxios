//! Unified execution tool for Oxios agents.
//!
//! Provides two execution modes:
//! - **shell** — Execute a raw command string via `bash -c <cmd>`.
//!   Intended for general-purpose workspace commands (compilation, tests, etc.).
//!
//! - **structured** — Execute a binary with explicit args, subject to allowlist
//!   enforcement and shell-metacharacter blocking.
//!   Intended for host-sensitive operations (git, gh, osascript, open) that
//!   need stricter control.
//!
//! ## Security model
//!
//! `shell` mode: runs through `bash -c` — the command string is passed as-is.
//! Access control is enforced upstream by `AccessManager` (RBAC, path sandboxing).
//!
//! `structured` mode: binary must be in the allowlist (from `ExecConfig`),
//! and all arguments are validated against shell metacharacters (`;`, `|`, `$`,
//! backtick, `<`, `>`, etc.) and path traversal (`..`).

use async_trait::async_trait;
use std::sync::Arc;

use oxicode_sdk::{AgentTool, AgentToolResult, ToolContext};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::access_manager::AccessManager;
use crate::access_manager::AgentContext;

// ─── Shell metacharacter blocklist ──────────

/// Characters that are rejected in structured-mode arguments.
const SHELL_METACHARS: &[char] = &[
    '|', '&', ';', '$', '`', '<', '>', '(', ')', '{', '}', '\n', '\r', '\0',
];

// ─── ExecResult ────────────────────────────────────────────────────────────

/// Result of a command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    /// Standard output captured from the process.
    pub stdout: String,
    /// Standard error captured from the process.
    pub stderr: String,
    /// Process exit code (0 = success, -1 = signal / timeout).
    pub exit_code: i32,
    /// Wall-clock execution duration in milliseconds.
    pub duration_ms: u64,
}

// ─── ExecTool ──────────────────────────────────────────────────────────────

/// Unified execution tool for agents.
///
/// Wraps both shell-string and structured binary+args execution behind a
/// single `AgentTool` implementation that uses a `mode` parameter to
/// dispatch to the appropriate method.
///
/// Access control is enforced based on `agent_name`:
/// - **shell_exec**: audit logging (cannot sandbox arbitrary shell).
/// - **structured_exec**: pre-flight permission check via `AccessManager`.
pub struct ExecTool {
    config: crate::kernel_handle::SharedExecConfig,
    access: Arc<Mutex<AccessManager>>,
    context: Option<AgentContext>,
}

impl ExecTool {
    /// Create a new `ExecTool` with an `AgentContext` (production path).
    pub fn new(
        config: crate::kernel_handle::SharedExecConfig,
        access: Arc<Mutex<AccessManager>>,
        context: AgentContext,
    ) -> Self {
        Self {
            config,
            access,
            context: Some(context),
        }
    }

    /// Create an `ExecTool` from a [`KernelHandle`] with an agent context.
    ///
    /// Note (RFC-035): human-in-the-loop approval for shell exec now lives
    /// in `GatedTool::execute` (Step 2.5) — the bespoke Phase D approval
    /// fields previously set here have moved up.
    pub fn from_kernel_with_context(
        kernel: &crate::kernel_handle::KernelHandle,
        context: AgentContext,
    ) -> Self {
        Self::new(
            Arc::new(parking_lot::RwLock::new(kernel.exec.config_snapshot())),
            kernel.exec.access_manager().clone(),
            context,
        )
    }

    /// Create an `ExecTool` from a [`KernelHandle`] (legacy, no context).
    pub fn from_kernel(kernel: &crate::kernel_handle::KernelHandle) -> Self {
        Self {
            config: Arc::new(parking_lot::RwLock::new(kernel.exec.config_snapshot())),
            access: kernel.exec.access_manager().clone(),
            context: None,
        }
    }

    /// Prefer `new()` with `AgentContext` for full security.
    pub fn for_agent(
        config: crate::kernel_handle::SharedExecConfig,
        access: Arc<Mutex<AccessManager>>,
        _agent_name: String,
    ) -> Self {
        Self {
            config,
            access,
            context: None,
        }
    }

    /// Legacy constructor without agent context (for backward compatibility).
    ///
    /// **Warning:** This bypasses the `AgentContext` path.
    /// Use only for migration or testing.
    pub fn new_unrestricted(
        config: crate::kernel_handle::SharedExecConfig,
        access: Arc<Mutex<AccessManager>>,
    ) -> Self {
        Self {
            config,
            access,
            context: None,
        }
    }

    /// Returns the agent name if a context is attached.
    fn agent_name(&self) -> Option<&str> {
        self.context.as_ref().map(|c| c.agent_name.as_str())
    }

    /// The chat-scoped brain binding of the attached agent context.
    ///
    /// `None` = no brain connected to this chat: every path to the
    /// `oxibrain` CLI must be denied (design §4.3).
    fn brain_space(&self) -> Option<&str> {
        self.context.as_ref().and_then(|c| c.brain_space.as_deref())
    }

    /// Execute a raw command string via `bash -c <cmd>`.
    ///
    /// Primary shell execution path.
    /// The entire command string is forwarded to `bash -c`, so pipelines,
    /// redirects, and compound commands all work.
    ///
    /// If a `shutdown` signal is provided and fires before the command
    /// completes, the child process is killed and an error is returned.
    pub async fn shell_exec(
        &self,
        command: &str,
        timeout_ms: u64,
        shutdown: Option<oneshot::Receiver<()>>,
    ) -> Result<ExecResult, String> {
        // Check if shell mode is allowed
        let cfg = self.config.read().clone();
        if !cfg.allow_shell_mode {
            return Err(
                "shell_exec: shell mode is disabled by configuration (allow_shell_mode = false). \
                 Use mode='structured' instead, or set allow_shell_mode=true in config.toml"
                    .to_string(),
            );
        }

        if command.trim().is_empty() {
            return Err("shell_exec: command must not be empty".to_string());
        }

        // Audit + access check.
        if let Some(name) = self.agent_name() {
            let mut access = self.access.lock();
            if !access.can_use_tool(name, "bash") {
                return Err(format!(
                    "shell_exec: agent '{name}' is not allowed to execute 'bash'"
                ));
            }
            tracing::info!(
                agent = %name,
                mode = "shell",
                command = %command.chars().take(200).collect::<String>(),
                "ExecTool: executing shell command (shell mode enabled)",
            );
        } else {
            tracing::warn!(
                mode = "shell",
                command = %command.chars().take(200).collect::<String>(),
                "ExecTool: shell mode executing without agent context",
            );
        }

        // Brain gating: `oxibrain` is the only channel to the memory store,
        // and an unbound chat has no store to talk to (design §4.3).
        if command.contains("oxibrain") && self.brain_space().is_none() {
            return Err(
                "shell_exec: 'oxibrain' is denied — no brain connected to this chat.".into(),
            );
        }

        let effective_timeout = timeout_ms.clamp(1_000, cfg.max_timeout_secs * 1_000);

        let start = std::time::Instant::now();

        // Spawn the child process so we can kill it on shutdown.
        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-c")
            .arg(command)
            .env_clear()
            .env("HOME", std::env::var("HOME").unwrap_or_default())
            .env("USER", std::env::var("USER").unwrap_or_default())
            .env("LOGNAME", std::env::var("LOGNAME").unwrap_or_default())
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env(
                "LANG",
                std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".to_string()),
            )
            .env("TERM", "dumb");
        // Chat-scoped brain binding: `oxibrain` discovers its space via env.
        if let Some(space) = self.brain_space() {
            cmd.env("OXIOS_BRAIN_SPACE", space);
        }
        let mut child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("shell spawn error: {e}"))?;

        // Take stdout/stderr handles before the select! so we can read them
        // after wait() completes (wait_with_output consumes the child).
        let stdout_handle = child.stdout.take();
        let stderr_handle = child.stderr.take();

        // Build a shutdown-aware future that either waits for the signal or
        // stays pending forever (so select! only fires when a signal exists).
        let shutdown_fut = async {
            if let Some(rx) = shutdown {
                let _ = rx.await;
            } else {
                std::future::pending::<()>().await;
            }
        };

        let result = tokio::select! {
            status = tokio::time::timeout(
                std::time::Duration::from_millis(effective_timeout),
                child.wait(),
            ) => {
                match status {
                    Ok(Ok(status)) => {
                        let stdout = read_handle(stdout_handle).await;
                        let stderr = read_stderr_handle(stderr_handle).await;
                        Ok(ExecResult {
                            stdout,
                            stderr,
                            exit_code: status.code().unwrap_or(-1),
                            duration_ms: start.elapsed().as_millis() as u64,
                        })
                    }
                    Ok(Err(e)) => Err(format!("shell execution error: {e}")),
                    Err(_) => {
                        // Timeout elapsed — kill the child to avoid orphans.
                        let _ = child.kill().await;
                        let _ = child.wait().await; // reap to avoid zombies
                        Err(format!(
                            "shell command timed out after {effective_timeout}ms"
                        ))
                    }
                }
            }
            _ = shutdown_fut => {
                // Shutdown signal received — kill the child process.
                let _ = child.kill().await;
                let _ = child.wait().await; // reap to avoid zombies
                Err("Execution cancelled by shutdown signal".to_string())
            }
        };

        result
    }

    /// Execute a binary with explicit args, enforcing allowlist + metachar blocking.
    ///
    /// Primary structured execution path.
    /// Security checks:
    /// 1. Binary must be a bare name (no `/` or `..`).
    /// 2. Binary must be in the allowlist (or allowlist is empty = dev mode).
    /// 3. Arguments must not contain shell metacharacters or path traversal.
    ///
    /// If a `shutdown` signal is provided and fires before the command
    /// completes, the child process is killed and an error is returned.
    pub async fn structured_exec(
        &self,
        binary: &str,
        args: Vec<String>,
        timeout_ms: u64,
        shutdown: Option<oneshot::Receiver<()>>,
    ) -> Result<ExecResult, String> {
        // --- Access control ---
        if let Some(name) = self.agent_name() {
            let mut access = self.access.lock();
            if !access.can_use_tool(name, binary) {
                return Err(format!(
                    "structured_exec: agent '{name}' is not allowed to execute '{binary}'"
                ));
            }
        }

        // --- Binary validation ---

        // Log execution for audit trail
        tracing::debug!(mode = "structured", binary = %binary, args = ?args, "ExecTool executing");

        if binary.contains("..") {
            return Err("structured_exec: path traversal in binary name".to_string());
        }
        if binary.contains('/') {
            return Err("structured_exec: binary must be a bare name, not a path".to_string());
        }

        // Brain gating: hard rule independent of AllowlistMode — even a
        // permissive/empty allowlist must not expose `oxibrain` to an
        // unbound chat (design §4.3).
        if binary == "oxibrain" && self.brain_space().is_none() {
            return Err(
                "structured_exec: 'oxibrain' is denied — no brain connected to this chat. Connect a brain in the chat header first."
                    .into(),
            );
        }
        if !self.config.read().is_binary_allowed(binary) {
            return Err(format!(
                "structured_exec: binary '{binary}' is not in the allowlist"
            ));
        }

        // --- Argument validation ---

        if has_metacharacters(&args) {
            return Err(
                "structured_exec: shell metacharacters or path traversal not allowed in arguments"
                    .to_string(),
            );
        }

        let effective_timeout =
            timeout_ms.clamp(1_000, self.config.read().max_timeout_secs * 1_000);

        let start = std::time::Instant::now();

        // Spawn the child process so we can kill it on shutdown.
        let mut cmd = tokio::process::Command::new(binary);
        cmd.args(&args)
            .env_clear()
            .env("HOME", std::env::var("HOME").unwrap_or_default())
            .env("USER", std::env::var("USER").unwrap_or_default())
            .env("LOGNAME", std::env::var("LOGNAME").unwrap_or_default())
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env(
                "LANG",
                std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".to_string()),
            )
            .env("TERM", "dumb");
        // Chat-scoped brain binding: `oxibrain` discovers its space via env.
        if let Some(space) = self.brain_space() {
            cmd.env("OXIOS_BRAIN_SPACE", space);
        }
        let mut child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("structured spawn error: {e}"))?;
        // Take stdout/stderr handles before the select! so we can read them
        // after wait() completes (wait_with_output consumes the child).
        let stdout_handle = child.stdout.take();
        let stderr_handle = child.stderr.take();

        // Build a shutdown-aware future that either waits for the signal or
        // stays pending forever (so select! only fires when a signal exists).
        let shutdown_fut = async {
            if let Some(rx) = shutdown {
                let _ = rx.await;
            } else {
                std::future::pending::<()>().await;
            }
        };

        let result = tokio::select! {
            status = tokio::time::timeout(
                std::time::Duration::from_millis(effective_timeout),
                child.wait(),
            ) => {
                match status {
                    Ok(Ok(status)) => {
                        let stdout = read_handle(stdout_handle).await;
                        let stderr = read_stderr_handle(stderr_handle).await;
                        Ok(ExecResult {
                            stdout,
                            stderr,
                            exit_code: status.code().unwrap_or(-1),
                            duration_ms: start.elapsed().as_millis() as u64,
                        })
                    }
                    Ok(Err(e)) => Err(format!("structured execution error: {e}")),
                    Err(_) => {
                        // Timeout elapsed — kill the child to avoid orphans.
                        let _ = child.kill().await;
                        let _ = child.wait().await; // reap to avoid zombies
                        Err(format!(
                            "structured command timed out after {effective_timeout}ms"
                        ))
                    }
                }
            }
            _ = shutdown_fut => {
                // Shutdown signal received — kill the child process.
                let _ = child.kill().await;
                let _ = child.wait().await; // reap to avoid zombies
                Err("Execution cancelled by shutdown signal".to_string())
            }
        };

        result
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────

/// Read a piped stdout/stderr handle to a String, returning empty on failure.
async fn read_handle(handle: Option<tokio::process::ChildStdout>) -> String {
    match handle {
        Some(mut h) => {
            let mut buf = Vec::new();
            // Use a timeout so we don't block forever on a stuck pipe.
            match tokio::time::timeout(
                std::time::Duration::from_secs(10),
                tokio::io::AsyncReadExt::read_to_end(&mut h, &mut buf),
            )
            .await
            {
                Ok(Ok(_)) => String::from_utf8_lossy(&buf).to_string(),
                _ => String::new(),
            }
        }
        None => String::new(),
    }
}

/// Read a piped stderr handle to a String, returning empty on failure.
async fn read_stderr_handle(handle: Option<tokio::process::ChildStderr>) -> String {
    match handle {
        Some(mut h) => {
            let mut buf = Vec::new();
            match tokio::time::timeout(
                std::time::Duration::from_secs(10),
                tokio::io::AsyncReadExt::read_to_end(&mut h, &mut buf),
            )
            .await
            {
                Ok(Ok(_)) => String::from_utf8_lossy(&buf).to_string(),
                _ => String::new(),
            }
        }
        None => String::new(),
    }
}

/// Check whether any argument contains shell metacharacters or `..`.
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

/// Format an `ExecResult` into a human-readable output string (matching the
/// format consistent with agent expectations).
fn format_exec_output(result: &ExecResult) -> String {
    let mut output = String::new();

    if result.stdout.is_empty() && result.stderr.is_empty() {
        output.push_str("(no output)");
    } else {
        if !result.stdout.is_empty() {
            output.push_str(&result.stdout);
        }
        if !result.stderr.is_empty() && !result.stdout.is_empty() {
            output.push('\n');
        }
        if !result.stderr.is_empty() {
            output.push_str(&result.stderr);
        }
    }

    if result.exit_code != 0 {
        output.push_str(&format!(
            "\n\nCommand exited with code {}",
            result.exit_code
        ));
    }

    let secs = result.duration_ms / 1000;
    let millis = result.duration_ms % 1000;

    if secs >= 60 {
        let mins = secs / 60;
        let remain_secs = secs % 60;
        output.push_str(&format!(
            "\n\nTook {}m {:.1}s",
            mins,
            remain_secs as f64 + millis as f64 / 1000.0
        ));
    } else {
        output.push_str(&format!(
            "\n\nTook {:.1}s",
            secs as f64 + millis as f64 / 1000.0
        ));
    }

    output
}

// ─── Debug ─────────────────────────────────────────────────────────────────

impl std::fmt::Debug for ExecTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecTool").finish()
    }
}

// ─── AgentTool implementation ──────────────────────────────────────────────

#[async_trait]

impl AgentTool for ExecTool {
    fn name(&self) -> &str {
        "exec"
    }

    fn label(&self) -> &str {
        "Exec"
    }

    fn description(&self) -> &'static str {
        "Execute a command. Use mode='shell' for raw shell strings (pipelines, redirects) or mode='structured' for a specific binary+args with allowlist security."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["shell", "structured"],
                    "description": "Execution mode: 'shell' for bash -c <command>, 'structured' for binary+args with allowlist enforcement"
                },
                "command": {
                    "type": "string",
                    "description": "Shell command string (mode='shell' only)"
                },
                "binary": {
                    "type": "string",
                    "description": "Binary name (mode='structured' only, must be in allowlist)"
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Binary arguments (mode='structured' only)"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds",
                    "default": 120
                }
            },
            "required": ["mode"]
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: Value,
        shutdown: Option<tokio::sync::oneshot::Receiver<()>>,
        _ctx: &ToolContext,
    ) -> Result<AgentToolResult, oxicode_sdk::ToolError> {
        let mode = params.get("mode").and_then(|v| v.as_str()).ok_or_else(|| {
            "Missing required parameter: mode (expected 'shell' or 'structured')".to_string()
        })?;

        let timeout_secs = params
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.config.read().default_timeout_secs);
        let timeout_ms = (timeout_secs * 1000).min(self.config.read().max_timeout_secs * 1000);

        match mode {
            "shell" => {
                let command = match params.get("command").and_then(|v| v.as_str()) {
                    Some(c) => c,
                    None => {
                        return Ok(AgentToolResult::error(
                            "shell mode requires 'command' parameter",
                        ));
                    }
                };
                // RFC-035 Step 2.5 in `GatedTool::execute` now handles exec
                // approval uniformly with all other tools. The bespoke
                // shell-only approval block that previously lived here has
                // been removed; exec has `ToolPolicy::OnDemand` by default,
                // so prompts/auto-runs are governed by the kernel's
                // `[security.approval]` config via `ApprovalGate`.
                match self.shell_exec(command, timeout_ms, shutdown).await {
                    Ok(result) => {
                        let output = format_exec_output(&result);
                        if result.exit_code == 0 {
                            Ok(AgentToolResult::success(output))
                        } else {
                            Ok(AgentToolResult::error(output))
                        }
                    }
                    Err(e) => Ok(AgentToolResult::error(format!("exec (shell): {e}"))),
                }
            }

            "structured" => {
                let binary = match params.get("binary").and_then(|v| v.as_str()) {
                    Some(b) => b,
                    None => {
                        return Ok(AgentToolResult::error(
                            "structured mode requires 'binary' parameter",
                        ));
                    }
                };

                let args: Vec<String> = params
                    .get("args")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                match self
                    .structured_exec(binary, args, timeout_ms, shutdown)
                    .await
                {
                    Ok(result) => {
                        let output = format_exec_output(&result);
                        if result.exit_code == 0 {
                            Ok(AgentToolResult::success(output))
                        } else {
                            Ok(AgentToolResult::error(output))
                        }
                    }
                    Err(e) => Ok(AgentToolResult::error(format!("exec (structured): {e}"))),
                }
            }

            other => Err(format!(
                "Invalid mode '{other}': expected 'shell' or 'structured'"
            )),
        }
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ExecConfig;

    /// Helper: build an `ExecTool` with default config and empty access manager.
    /// Shell mode is enabled for testing purposes. No agent context.
    fn make_tool(allowed_commands: Vec<&str>) -> ExecTool {
        let mut config = ExecConfig {
            allowlist_mode: crate::config::AllowlistMode::Permissive,
            allow_shell_mode: true,
            ..Default::default()
        };
        config.allowed_commands = allowed_commands.into_iter().map(String::from).collect();
        ExecTool::new_unrestricted(
            Arc::new(parking_lot::RwLock::new(config)),
            Arc::new(Mutex::new(AccessManager::new())),
        )
    }

    // ─── shell_exec ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_shell_exec_echo() {
        let tool = make_tool(vec![]);
        let result = tool.shell_exec("echo hello", 5_000, None).await;
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r.exit_code, 0);
        assert!(r.stdout.contains("hello"));
        assert!(r.duration_ms < 5_000);
    }

    #[tokio::test]
    async fn test_shell_exec_pipeline() {
        let tool = make_tool(vec![]);
        let result = tool.shell_exec("echo foo | tr f b", 5_000, None).await;
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r.exit_code, 0);
        assert!(r.stdout.contains("boo"));
    }

    #[tokio::test]
    async fn test_shell_exec_nonzero_exit() {
        let tool = make_tool(vec![]);
        let result = tool.shell_exec("exit 42", 5_000, None).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().exit_code, 42);
    }

    #[tokio::test]
    async fn test_shell_exec_empty_command() {
        let tool = make_tool(vec![]);
        let result = tool.shell_exec("   ", 5_000, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must not be empty"));
    }

    #[tokio::test]
    async fn test_shell_exec_timeout() {
        let tool = make_tool(vec![]);
        let result = tool.shell_exec("sleep 300", 200, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("timed out"));
    }

    // ─── structured_exec ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_structured_exec_echo() {
        let tool = make_tool(vec!["echo"]);
        let result = tool
            .structured_exec("echo", vec!["hello".into()], 5_000, None)
            .await;
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r.exit_code, 0);
        assert!(r.stdout.contains("hello"));
    }

    #[tokio::test]
    async fn test_structured_exec_blocked_binary() {
        let tool = make_tool(vec!["echo"]);
        let result = tool
            .structured_exec("rm", vec!["-rf".into(), "/".into()], 5_000, None)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not in the allowlist"));
    }

    #[tokio::test]
    async fn test_structured_exec_path_binary() {
        let tool = make_tool(vec![]);
        let result = tool
            .structured_exec("/usr/bin/echo", vec![], 5_000, None)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("bare name"));
    }

    #[tokio::test]
    async fn test_structured_exec_traversal_binary() {
        let tool = make_tool(vec![]);
        let result = tool
            .structured_exec("../bin/evil", vec![], 5_000, None)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("path traversal"));
    }

    #[tokio::test]
    async fn test_structured_exec_metachar_args() {
        let tool = make_tool(vec!["echo"]);
        let result = tool
            .structured_exec("echo", vec!["foo; rm -rf /".into()], 5_000, None)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("metacharacters"));
    }

    #[tokio::test]
    async fn test_structured_exec_path_traversal_args() {
        let tool = make_tool(vec!["cat"]);
        let result = tool
            .structured_exec("cat", vec!["../etc/passwd".into()], 5_000, None)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("metacharacters"));
    }

    #[tokio::test]
    async fn test_structured_exec_clean_args() {
        let tool = make_tool(vec!["echo"]);
        let result = tool
            .structured_exec("echo", vec!["hello".into(), "world".into()], 5_000, None)
            .await;
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r.exit_code, 0);
        assert!(r.stdout.contains("hello world"));
    }

    // ─── AgentTool interface ─────────────────────────────────────────

    #[test]
    fn test_name_and_label() {
        let tool = make_tool(vec![]);
        assert_eq!(tool.name(), "exec");
        assert_eq!(tool.label(), "Exec");
    }

    #[test]
    fn test_parameters_schema() {
        let tool = make_tool(vec![]);
        let schema = tool.parameters_schema();

        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("mode"));
        assert!(props.contains_key("command"));
        assert!(props.contains_key("binary"));
        assert!(props.contains_key("args"));
        assert!(props.contains_key("timeout"));

        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|r| r.as_str() == Some("mode")));
    }

    #[tokio::test]
    async fn test_agent_tool_shell_mode() {
        let tool = make_tool(vec![]);

        let result = tool
            .execute(
                "test-1",
                json!({ "mode": "shell", "command": "echo hello" }),
                None,
                &ToolContext::default(),
            )
            .await;

        assert!(result.is_ok());
        let r = result.unwrap();
        assert!(r.success, "Expected success, got: {}", r.output);
        assert!(r.output.contains("hello"));
    }

    #[tokio::test]
    async fn test_agent_tool_structured_mode() {
        let tool = make_tool(vec!["echo"]);

        let result = tool
            .execute(
                "test-2",
                json!({ "mode": "structured", "binary": "echo", "args": ["hi"] }),
                None,
                &ToolContext::default(),
            )
            .await;

        assert!(result.is_ok());
        let r = result.unwrap();
        assert!(r.success, "Expected success, got: {}", r.output);
        assert!(r.output.contains("hi"));
    }

    #[tokio::test]
    async fn test_agent_tool_missing_mode() {
        let tool = make_tool(vec![]);
        let result = tool
            .execute(
                "test-3",
                json!({ "command": "echo hi" }),
                None,
                &ToolContext::default(),
            )
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Missing required parameter: mode")
        );
    }

    #[tokio::test]
    async fn test_agent_tool_invalid_mode() {
        let tool = make_tool(vec![]);
        let result = tool
            .execute(
                "test-4",
                json!({ "mode": "docker" }),
                None,
                &ToolContext::default(),
            )
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid mode"));
    }

    #[tokio::test]
    async fn test_agent_tool_shell_missing_command() {
        let tool = make_tool(vec![]);
        let result = tool
            .execute(
                "test-5",
                json!({ "mode": "shell" }),
                None,
                &ToolContext::default(),
            )
            .await;
        assert!(result.is_ok());
        let r = result.unwrap();
        assert!(!r.success);
        assert!(r.output.contains("shell mode requires 'command' parameter"));
    }

    #[tokio::test]
    async fn test_agent_tool_structured_missing_binary() {
        let tool = make_tool(vec![]);
        let result = tool
            .execute(
                "test-6",
                json!({ "mode": "structured" }),
                None,
                &ToolContext::default(),
            )
            .await;
        assert!(result.is_ok());
        let r = result.unwrap();
        assert!(!r.success);
        assert!(
            r.output
                .contains("structured mode requires 'binary' parameter")
        );
    }

    #[tokio::test]
    async fn test_agent_tool_nonzero_exit() {
        let tool = make_tool(vec![]);

        let result = tool
            .execute(
                "test-7",
                json!({ "mode": "shell", "command": "exit 7" }),
                None,
                &ToolContext::default(),
            )
            .await;

        assert!(result.is_ok());
        let r = result.unwrap();
        assert!(!r.success);
        assert!(r.output.contains("exited with code 7"));
    }

    // ─── format_exec_output ──────────────────────────────────────────

    #[test]
    fn test_format_exec_output_success() {
        let result = ExecResult {
            stdout: "hello".to_string(),
            stderr: String::new(),
            exit_code: 0,
            duration_ms: 1_500,
        };
        let output = format_exec_output(&result);
        assert!(output.contains("hello"));
        assert!(output.contains("Took 1.5s"));
        assert!(!output.contains("exited with code"));
    }

    #[test]
    fn test_format_exec_output_failure() {
        let result = ExecResult {
            stdout: String::new(),
            stderr: "error!".to_string(),
            exit_code: 1,
            duration_ms: 500,
        };
        let output = format_exec_output(&result);
        assert!(output.contains("error!"));
        assert!(output.contains("exited with code 1"));
    }

    #[test]
    fn test_format_exec_output_no_output() {
        let result = ExecResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
            duration_ms: 100,
        };
        let output = format_exec_output(&result);
        assert!(output.contains("(no output)"));
    }

    #[test]
    fn test_format_exec_output_minutes() {
        let result = ExecResult {
            stdout: "done".to_string(),
            stderr: String::new(),
            exit_code: 0,
            duration_ms: 125_000, // 2m 5s
        };
        let output = format_exec_output(&result);
        assert!(output.contains("Took 2m 5.0s"));
    }

    // ─── has_metacharacters ──────────────────────────────────────────

    #[test]
    fn test_has_metacharacters_clean() {
        assert!(!has_metacharacters(&["hello".into(), "world".into()]));
    }

    #[test]
    fn test_has_metacharacters_semicolon() {
        assert!(has_metacharacters(&["foo;bar".into()]));
    }

    #[test]
    fn test_has_metacharacters_pipe() {
        assert!(has_metacharacters(&["a | b".into()]));
    }

    #[test]
    fn test_has_metacharacters_dollar() {
        assert!(has_metacharacters(&["$(whoami)".into()]));
    }

    #[test]
    fn test_has_metacharacters_backtick() {
        assert!(has_metacharacters(&["`id`".into()]));
    }

    #[test]
    fn test_has_metacharacters_traversal() {
        assert!(has_metacharacters(&["../etc/passwd".into()]));
    }

    // ── Access control tests ────────────────────────────────────

    /// Helper: build ExecTool bound to a named agent with specific permissions.
    fn make_agent_tool(agent_name: &str, allowed_tools: &[&str]) -> ExecTool {
        make_brain_tool(agent_name, allowed_tools, None)
    }

    /// Helper: ExecTool bound to a named agent with a chat-scoped brain
    /// binding (`None` = unbound — every `oxibrain` path is denied).
    fn make_brain_tool(
        agent_name: &str,
        allowed_tools: &[&str],
        brain_space: Option<&str>,
    ) -> ExecTool {
        let config = ExecConfig {
            allowlist_mode: crate::config::AllowlistMode::Permissive,
            allow_shell_mode: true,
            ..Default::default()
        };
        let mut access = AccessManager::new();
        // Create default permissions, then set specific allowed tools.
        {
            let perms = access.get_or_create_permissions(agent_name);
            // Clear defaults first, then add only requested tools.
            perms.allowed_tools.clear();
            for tool in allowed_tools {
                perms.allow_tool(tool);
            }
        }
        let mut ctx = crate::access_manager::AgentContext::test_fixture(agent_name);
        ctx.brain_space = brain_space.map(str::to_string);
        ExecTool::new(
            Arc::new(parking_lot::RwLock::new(config)),
            Arc::new(Mutex::new(access)),
            ctx,
        )
    }

    #[tokio::test]
    async fn test_for_agent_structured_exec_allowed() {
        let tool = make_agent_tool("test-agent", &["echo", "ls"]);
        let result = tool
            .structured_exec("echo", vec!["hello".into()], 5_000, None)
            .await;
        assert!(result.is_ok(), "Allowed binary should succeed");
        let r = result.unwrap();
        assert_eq!(r.exit_code, 0);
        assert!(r.stdout.contains("hello"));
    }

    #[tokio::test]
    async fn test_for_agent_structured_exec_denied() {
        let tool = make_agent_tool("test-agent", &["ls"]); // no "echo"
        let result = tool
            .structured_exec("echo", vec!["hello".into()], 5_000, None)
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("not allowed to execute"),
            "Error should mention denial: {err}"
        );
        assert!(
            err.contains("echo"),
            "Error should name the denied binary: {err}"
        );
    }

    #[tokio::test]
    async fn test_for_agent_shell_exec_allowed() {
        let tool = make_agent_tool("test-agent", &["bash"]);
        let result = tool.shell_exec("echo hello", 5_000, None).await;
        assert!(
            result.is_ok(),
            "Agent with 'bash' permission should succeed"
        );
        assert!(result.unwrap().stdout.contains("hello"));
    }

    #[tokio::test]
    async fn test_for_agent_shell_exec_denied() {
        let tool = make_agent_tool("test-agent", &["ls"]); // no "bash"
        let result = tool.shell_exec("echo hello", 5_000, None).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("not allowed to execute"),
            "Error should mention denial: {err}"
        );
        assert!(err.contains("bash"), "Error should name 'bash': {err}");
    }

    #[tokio::test]
    async fn test_no_agent_name_bypasses_access_control() {
        // ExecTool::new_unrestricted() (no context) should NOT check permissions,
        // but shell mode must still be enabled in config.
        let config = ExecConfig {
            allow_shell_mode: true,
            ..Default::default()
        };
        let access = AccessManager::new(); // empty — no permissions for anyone
        let tool = ExecTool::new_unrestricted(
            Arc::new(parking_lot::RwLock::new(config)),
            Arc::new(Mutex::new(access)),
        );
        let result = tool.shell_exec("echo unrestricted", 5_000, None).await;
        assert!(
            result.is_ok(),
            "Shell mode enabled + no agent_name = unrestricted execution"
        );
    }

    #[test]
    fn test_agent_name_set_correctly() {
        let tool = make_agent_tool("my-agent", &[]);
        assert_eq!(tool.agent_name(), Some("my-agent"));
    }

    #[test]
    fn test_new_has_no_agent_name() {
        let tool = make_tool(vec![]);
        assert!(tool.agent_name().is_none());
    }

    // ── Brain gating (per-chat binding, design §4.3) ────────────────

    #[tokio::test]
    async fn test_structured_oxibrain_denied_when_unbound() {
        let tool = make_brain_tool("test-agent", &["oxibrain"], None);
        let result = tool
            .structured_exec("oxibrain", vec!["remember".into()], 5_000, None)
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("no brain connected"),
            "expected brain-gate denial, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_structured_oxibrain_denied_without_context() {
        // Hard rule: no AgentContext (legacy constructors) also counts as
        // unbound — even an allowlisted binary stays denied.
        let tool = make_tool(vec!["oxibrain"]);
        let result = tool.structured_exec("oxibrain", vec![], 5_000, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no brain connected"));
    }

    #[tokio::test]
    async fn test_structured_oxibrain_passes_gate_when_bound() {
        let tool = make_brain_tool("test-agent", &["oxibrain"], Some("work"));
        // `oxibrain` may not be installed here — any outcome EXCEPT the
        // brain-gate denial proves the gate passed for a bound chat.
        if let Err(err) = tool.structured_exec("oxibrain", vec![], 1_000, None).await {
            assert!(
                !err.contains("no brain connected"),
                "bound chat must pass the brain gate, got: {err}"
            );
        }
    }

    #[tokio::test]
    async fn test_shell_oxibrain_denied_when_unbound() {
        let tool = make_brain_tool("test-agent", &["bash"], None);
        let result = tool
            .shell_exec("oxibrain remember hello", 5_000, None)
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("no brain connected"),
            "expected brain-gate denial, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_shell_env_injects_brain_space_when_bound() {
        let tool = make_brain_tool("test-agent", &["bash"], Some("work"));
        let result = tool
            .shell_exec("echo $OXIOS_BRAIN_SPACE", 5_000, None)
            .await;
        assert!(
            result.is_ok(),
            "bound chat must reach spawn: {:?}",
            result.err()
        );
        assert_eq!(result.unwrap().stdout.trim(), "work");
    }

    #[tokio::test]
    async fn test_shell_env_omits_brain_space_when_unbound() {
        let tool = make_brain_tool("test-agent", &["bash"], None);
        let result = tool
            .shell_exec("echo \"[$OXIOS_BRAIN_SPACE]\"", 5_000, None)
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().stdout.trim(), "[]");
    }

    #[tokio::test]
    async fn test_structured_env_injects_brain_space_when_bound() {
        let tool = make_brain_tool("test-agent", &["env"], Some("work"));
        let result = tool.structured_exec("env", vec![], 5_000, None).await;
        assert!(
            result.is_ok(),
            "bound chat must reach spawn: {:?}",
            result.err()
        );
        assert!(result.unwrap().stdout.contains("OXIOS_BRAIN_SPACE=work"));
    }
}
