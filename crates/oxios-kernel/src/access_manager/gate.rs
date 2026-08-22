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

/// Lexically normalize a path — resolve `.` and `..` components
/// without touching the filesystem. Absolute in ⇒ absolute out.
/// Used by [`AccessGate::with_deny_root`] as the fallback when the
/// root cannot be canonicalized because it does not exist yet.
fn lexical_absolute(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Extract a path-like exec argument and resolve it the same way
/// [`AccessGate::check_path`] would. Recognized forms: absolute
/// (`/…`), home (`~/…`), and explicit relative (`./…`, `../…`).
/// Bare words (`build`, `--release`, `foo`) are NOT paths — they are
/// indistinguishable from ordinary flag values. See `check_exec` for
/// the documented residual of that choice.
fn exec_arg_path(arg: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(dirs::home_dir);
    exec_arg_path_with_home(arg, home.as_deref())
}

/// Pure helper behind [`exec_arg_path`] with the home directory
/// injected — testable without process-env mutation (same pattern as
/// `path_promotion::expand_tilde_with_home`).
fn exec_arg_path_with_home(arg: &str, home: Option<&Path>) -> Option<PathBuf> {
    let candidate = if let Some(rest) = arg.strip_prefix("~/") {
        // No home resolvable → cannot expand; treat as non-path.
        home?.join(rest)
    } else if arg.starts_with('/') {
        PathBuf::from(arg)
    } else if arg.starts_with("./") || arg.starts_with("../") {
        std::env::current_dir().ok()?.join(arg)
    } else {
        return None;
    };
    Some(canonicalize_for_check(&candidate))
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
/// Default whole-root deny entries (T18 R4).
///
/// Single source of truth — `agent_runtime.rs` references this
/// list at gate construction. Whole-root deny is appropriate for
/// surfaces where NO sub-path is safe for file-tool Layer-2.
///
/// Entries covered (per brief + on-disk layout):
/// - `~/.oxi` — shared vault (T15), brain index, settings, sessions
/// - `~/.oxicode` — shared oxicode-cli credential store (legacy
///   `auth.json` fallback that oxios's `CredentialStore` reads;
///   R4 added this so a broadly-allowed agent cannot exfiltrate
///   stored keys via `read`/`grep`)
pub const OXI_HOME_DENY_ROOTS: &[&str] = &[".oxi", ".oxicode"];

/// Default deny-subpath entries for `~/.oxios` (T18 R3).
///
/// Single source of truth — `agent_runtime.rs` and the parity test
/// in `gate.rs::tests` both reference this list so additions cannot
/// desync. Items in this list are present under `~/.oxios/` on a
/// typical oxios home (see `docs/getting-started.md` and the
/// `onboarding::WORKSPACE_SUBDIRS` constant) and MUST NOT be
/// reachable by file-tool Layer-2 (read/write/edit/grep/find/ls
/// through the AccessGate).
///
/// Items covered (per brief + on-disk layout):
/// - `config.toml` + `config.toml.bak` — main config (API keys,
///   model ids, paths)
/// - `preferences.json` — user preferences (may contain tokens)
/// - `auth.json` + `auth.json.bak` — credentials shared with oxi CLI
/// - `agent_log.db` — tamper-evident audit chain (Merkle)
/// - `oxios.lock`, `oxios.pid` — daemon runtime state
/// - `state` — daemon state store (KernelDatabase + AgentLogDb)
/// - `cache` — daemon caches
/// - `catalog` — model catalog overrides
/// - `logs` — daemon logs
/// - `knowledge` — legacy knowledge dir (post-migration writes
///   go to `~/.oxi/vault`)
/// - `assets` — bundled assets
/// - `web` (R3) — web-dist staging + `.active` restart marker
///   (RFC-024 SP3); tampering with the served UI on non-embedded
///   builds is the attack surface
/// - `run` (R3) — RFC-042 local-control socket home; tampering
///   with the owner-only control socket path lets a non-owner
///   peer mint credentials.
/// - `backups` (R4) — output dir for `POST /api/system/backup`
///   tarballs (config + vault). Backups contain credential-bearing
///   entries; the whole subtree is denied once established.
///   Pre-R4 backups landed at `~/.oxios/oxios-backup-<ts>.tar.gz`
///   (no deny entry); R4 relocates them under `backups/`.
pub const OXIOS_HOME_DENY_SUBPATHS: &[&str] = &[
    "config.toml",
    "config.toml.bak",
    "preferences.json",
    "auth.json",
    "auth.json.bak",
    "agent_log.db",
    "oxios.lock",
    "oxios.pid",
    "state",
    "cache",
    "catalog",
    "logs",
    "knowledge",
    "assets",
    // T18 R3 — added after a public-review pass noted the web-dist
    // staging dir and the local-control socket home were both missing
    // from the deny list:
    "web",
    "run",
    // T18 R4 — `backups/` is where `POST /api/system/backup` writes
    // its tarballs (config.toml + vault contents). The tar carries
    // credential-bearing entries, so the whole subtree is denied
    // once the directory exists on disk.
    "backups",
];

pub struct AccessGate {
    /// Agent permission manager (includes RBAC internally).
    access: Arc<Mutex<AccessManager>>,
    /// Execution policy (allowlist, timeouts).
    exec_config: Arc<ExecConfig>,
    /// Audit event destination.
    audit: Arc<dyn AuditSink>,
    /// Canonicalized ecosystem roots whose contents are NEVER reachable by
    /// file-tool Layer-2, regardless of any explicit allow-list entry.
    ///
    /// T18 (vault unification): the default `denied_paths` entry
    /// `".oxi/**"` is a relative-literal glob — the gate feeds
    /// canonicalized absolute paths into `is_path_denied`, which does
    /// full-string glob matching and so never matches. The list-form
    /// eco-root prefix check below is the actual enforcement: any
    /// canonicalized absolute path at/under one of these roots is
    /// denied with `DenyLayer::Permission`, short-circuiting BEFORE
    /// the allow-list. Tests cover both absolute (canonical) and
    /// symlinked-via-target paths.
    deny_roots: Vec<PathBuf>,
    /// Sub-path deny list — `(canonical_root, sub_path)` pairs.
    /// See `with_deny_subpath` for the contract; used for
    /// `~/.oxios/<sensitive>` entries (T18 R2).
    deny_subpaths: Vec<(PathBuf, PathBuf)>,
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
            deny_roots: Vec::new(),
            deny_subpaths: Vec::new(),
        }
    }

    /// Whole-root deny: every canonicalized path at/under `root` is
    /// denied to file-tool Layer-2 (and, since the whole-branch exec
    /// fix, to path-like exec arguments too). Reserved for surfaces
    /// where NO sub-path is safe (T18: `~/.oxi`).
    ///
    /// Roots are canonicalized once at construction. When
    /// canonicalization fails (a root that does not exist YET — e.g. a
    /// never-created `~/.oxi` on a fresh machine), the deny falls back
    /// to the lexically-normalized absolute root instead of being
    /// dropped: request paths go through `canonicalize_for_check`,
    /// which canonicalizes the nearest existing ancestor and re-appends
    /// the missing tail, so a canonical request path still prefix-
    /// matches the lexical root as long as no symlink sits inside the
    /// missing tail (documented residual; canonicalize would have
    /// resolved it were the root present). A warn is still logged so
    /// the fallback is observable.
    pub fn with_deny_root<P: AsRef<Path>>(mut self, root: P) -> Self {
        let root = root.as_ref();
        match root.canonicalize() {
            Ok(canon) => self.deny_roots.push(canon),
            Err(_) => {
                let lexical = lexical_absolute(root);
                tracing::warn!(
                    root = %root.display(),
                    fallback = %lexical.display(),
                    "deny_root canonicalize failed (root missing?); enforcing lexically-normalized root"
                );
                self.deny_roots.push(lexical);
            }
        }
        self
    }

    /// Sub-path deny under `root`: any request whose canonical path
    /// starts with `root/<sub_path>` (or equals it) is denied. Used
    /// for `~/.oxios/<sensitive>` entries — the per-app root also
    /// contains the legitimate workspace tree (T18 R2).
    ///
    /// The root is canonicalized at construction so a `~/.oxios`
    /// symlink pointing elsewhere cannot escape the deny. The
    /// `sub_path` is a relative path component; "a/b" matches both
    /// the literal `root/a/b` and everything under it.
    pub fn with_deny_subpath<P, S>(mut self, root: P, sub_path: S) -> Self
    where
        P: AsRef<Path>,
        S: AsRef<Path>,
    {
        let sub_path = sub_path.as_ref();
        match root.as_ref().canonicalize() {
            Ok(canon_root) => {
                // Defensive check: empty sub_path would degenerate to
                // a whole-root deny, which is what `with_deny_root`
                // is for. Skip rather than silently flip semantics.
                if sub_path.as_os_str().is_empty() {
                    tracing::warn!(
                        root = %canon_root.display(),
                        "deny_subpath called with empty sub_path; skipping"
                    );
                    return self;
                }
                self.deny_subpaths
                    .push((canon_root, sub_path.to_path_buf()));
            }
            Err(_) => tracing::warn!(
                root = %root.as_ref().display(),
                "deny_subpath root canonicalize failed; skipping deny"
            ),
        }
        self
    }

    /// True iff `canonical_path` matches any deny policy: at/under a
    /// whole-root deny, OR at/under `<deny_root>/<deny_sub>` for any
    /// configured pair. All sides are already canonical so the
    /// check is a pure path-component prefix walk — no symlink
    /// bypass, no `..` traversal, no string-collision false positives.
    fn is_denied_by_policy(&self, canonical_path: &Path) -> bool {
        for root in &self.deny_roots {
            if canonical_path == root.as_path() || canonical_path.starts_with(root.as_path()) {
                return true;
            }
        }
        for (root, sub) in &self.deny_subpaths {
            let mut denied = root.to_path_buf();
            denied.push(sub);
            if canonical_path == denied.as_path() || canonical_path.starts_with(denied.as_path()) {
                return true;
            }
        }
        false
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

        // Layer 2 (pre): ecosystem deny — whole-root + sub-path.
        // See `with_deny_root` and `with_deny_subpath` for the
        // contracts. Short-circuits BEFORE the existing
        // `can_access_path` allow/deny check so an admin-supplied
        // `allowed_paths = ["/**"]` cannot grant access to the
        // ecosystem's vault or sensitive config subtrees.
        if self.is_denied_by_policy(&resolved) {
            return Err(AccessDenied {
                agent: ctx.agent_name.clone(),
                resource: path_str.to_string(),
                layer: DenyLayer::Permission,
                reason: format!(
                    "Path '{path_str}' is under a protected ecosystem subpath (vault/config)"
                ),
                suggestion: None,
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

        // Layer 2 (pre): ecosystem deny — the SAME canonical-prefix
        // policy `check_path` applies (vault-unification design
        // §5.3.6). Without this, exec argv was a full bypass:
        // `cat ~/.oxi/config.toml` read deny-listed secrets and
        // `cp x ~/.oxi/vault/y.md` wrote the vault, dodging every
        // file-tool invariant. Path-like arguments (absolute, `~/…`,
        // `./…`, `../…`) are resolved with the same
        // canonicalize-for-check helper `check_path` uses.
        //
        // RESIDUAL (best-effort by design): bare relative words
        // (`cat .oxi/config.toml` with cwd = home) are not extracted
        // — they are indistinguishable from ordinary flag values;
        // closing that requires tracking the exec cwd, which the
        // structured exec tool does not expose to the gate.
        for arg in args {
            if let Some(path) = exec_arg_path(arg)
                && self.is_denied_by_policy(&path)
            {
                return Err(AccessDenied {
                    agent: ctx.agent_name.clone(),
                    resource: binary.to_string(),
                    layer: DenyLayer::Permission,
                    reason: format!(
                        "Exec argument '{arg}' resolves under a protected ecosystem subpath (vault/config)"
                    ),
                    suggestion: None,
                });
            }
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

    // ────────────────────────────────────────────────────────────────
    // Whole-branch review fixes: exec-argument deny (P1) and
    // missing-root lexical fallback (P2).
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn wb_exec_cat_denied_secret_is_denied() {
        // `cat <deny-root>/config.toml` — reading deny-listed secrets
        // through exec argv must hit the same Permission-layer deny
        // that check_path enforces (vault-unification design §5.3.6:
        // exec with cat/cp/tee was a full bypass of every invariant).
        let tmp = tempfile::tempdir().unwrap();
        let (gate, ctx, canon) = make_r2_gate(tmp.path());
        let secret = canon.join(".oxi").join("config.toml");
        std::fs::create_dir_all(secret.parent().unwrap()).unwrap();
        std::fs::write(&secret, "api_key = \"x\"").unwrap();
        let err = gate
            .check(CheckRequest::Exec {
                context: &ctx,
                binary: "cat",
                args: &[secret.to_string_lossy().to_string()],
            })
            .expect_err("cat of a deny-listed secret must be denied");
        assert_eq!(err.layer, DenyLayer::Permission, "{err}");
    }

    #[test]
    fn wb_exec_cp_into_vault_via_home_tilde_is_denied() {
        // `cp foo ~/.oxi/vault/x.md` — the destination resolves under
        // the whole-root deny. HOME is injected through the pure
        // helper (no process-env mutation: parallel-test safety,
        // same pattern as path_promotion::expand_tilde_with_home).
        let tmp = tempfile::tempdir().unwrap();
        let (gate, _ctx, canon) = make_r2_gate(tmp.path());
        let resolved =
            exec_arg_path_with_home("~/.oxi/vault/x.md", Some(&canon)).expect("~ form resolves");
        assert_eq!(
            resolved,
            canon.join(".oxi").join("vault").join("x.md"),
            "canonicalize_for_check appends the missing tail"
        );
        assert!(
            gate.is_denied_by_policy(&resolved),
            "cp destination under ~/.oxi must be denied"
        );
    }

    #[test]
    fn wb_exec_normal_command_unaffected() {
        // Plain builds/flags carry no path-like argument — the deny
        // probe must not reject ordinary exec.
        let tmp = tempfile::tempdir().unwrap();
        let (gate, ctx, _canon) = make_r2_gate(tmp.path());
        let result = gate.check(CheckRequest::Exec {
            context: &ctx,
            binary: "cargo",
            args: &["build".to_string(), "--release".to_string()],
        });
        assert!(result.is_ok(), "normal exec must stay allowed");
    }

    #[test]
    fn wb_deny_root_missing_root_falls_back_to_lexical_prefix() {
        // A root that does not exist YET (fresh machine, never-created
        // `~/.oxi`) used to be warn-skipped, leaving the security
        // control absent. The lexically-normalized fallback must still
        // deny paths under it. canon parent + missing leaf: no symlink
        // indirection (the documented residual).
        let tmp = tempfile::tempdir().unwrap();
        let canon = tmp.path().canonicalize().unwrap();
        let oxi = canon.join(".oxi"); // deliberately NOT created

        let mut access = AccessManager::new();
        let ctx = AgentContext::test_fixture("test-agent");
        let mut perms = AgentPermissions::for_new_agent("test-agent");
        perms.allowed_paths = vec![format!("{}/**", canon.display())];
        perms.allowed_tools = ["read", "write", "exec"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        access.set_permissions(perms);
        access.rbac_manager_mut().assign_role(
            Subject::Agent(ctx.agent_id),
            crate::access_manager::Role::Superuser,
        );
        let gate = AccessGate::new(
            Arc::new(Mutex::new(access)),
            Arc::new(ExecConfig {
                allowlist_mode: AllowlistMode::Permissive,
                ..Default::default()
            }),
            Arc::new(NoOpAuditSink),
        )
        .with_deny_root(&oxi);

        let secret = canon.join(".oxi").join("config.toml"); // also not created
        let err = gate
            .check(CheckRequest::Path {
                context: &ctx,
                path: &secret,
                mode: PathMode::Read,
            })
            .expect_err("never-created deny root must still deny");
        assert_eq!(err.layer, DenyLayer::Permission, "{err}");
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

    // ────────────────────────────────────────────────────────────────
    // T18 R2: scoped ecosystem-deny policy.
    //
    // `~/.oxi` ⇒ whole-root deny (no safe sub-path).
    // `~/.oxios` ⇒ sub-path deny only, scoped to the sensitive
    //   subtrees (config, auth, audit, daemon state, etc.) — leaves
    //   `~/.oxios/workspace/**` and RFC-025 mount paths accessible,
    //   preserving the legitimate grants wired at agent_runtime.rs
    //   and agent_lifecycle.rs.
    //
    // Helpers below wire these two policies so the gate tests can
    // exercise them through `CheckRequest::Path` with absolute
    // canonical paths — exactly the flow that R1 caught as broken.
    // ────────────────────────────────────────────────────────────────

    /// Construct an AccessGate wired with the T18 R4 production
    /// policy. Mirrors `agent_runtime.rs` exactly:
    ///
    /// * whole-root deny at `~/.oxi` (R1) and `~/.oxicode` (R4 —
    ///   shared credential fallback)
    /// * sub-path deny at `~/.oxios/<OXIOS_HOME_DENY_SUBPATHS>` (R2/R3)
    ///
    /// The whole-root deny list and the sub-path list are the
    /// single-source-of-truth `OXI_HOME_DENY_ROOTS` /
    /// `OXIOS_HOME_DENY_SUBPATHS` constants from this module so the
    /// production wiring and the parity test here cannot drift.
    ///
    /// Returns `(gate, ctx, canon_tmp)`. Callers MUST use the
    /// canonicalized path for any request path they intend to
    /// query — macOS `tempfile::tempdir` returns a `/var/folders/...`
    /// symlink target that resolves to `/private/var/folders/...`,
    /// and the gate canonicalizes request paths through
    /// `canonicalize_for_check`. A non-canonical request path
    /// will fail to match the broad allow-list we install here.
    fn make_r2_gate(tmp: &std::path::Path) -> (AccessGate, AgentContext, std::path::PathBuf) {
        let canon_tmp = tmp.canonicalize().unwrap_or_else(|_| tmp.to_path_buf());
        let mut access = AccessManager::new();
        let ctx = AgentContext::test_fixture("test-agent");

        // Grant EVERYTHING under the canonicalized tempdir — the
        // deny rules have to overrule anyway.
        let mut perms = AgentPermissions::for_new_agent("test-agent");
        perms.allowed_paths = vec![format!("{}/**", canon_tmp.display())];
        perms.allowed_tools = [
            "read",
            "write",
            "edit",
            "grep",
            "find",
            "ls",
            "bash",
            "exec",
            "web_search",
            "get_search_results",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        access.set_permissions(perms);
        access.rbac_manager_mut().assign_role(
            Subject::Agent(ctx.agent_id),
            crate::access_manager::Role::Superuser,
        );

        // Mirror the production wiring: install ALL deny-roots
        // listed in `OXI_HOME_DENY_ROOTS` and ALL deny-subpaths
        // listed in `OXIOS_HOME_DENY_SUBPATHS`. The directory
        // existence check at canonicalize-time gates whether the
        // root registers (a non-existing home/<leaf> simply
        // canonicalizes-fails and is logged, not dropped).
        for leaf in OXI_HOME_DENY_ROOTS {
            std::fs::create_dir_all(canon_tmp.join(leaf)).unwrap();
        }
        let oxios_root = canon_tmp.join(".oxios");
        std::fs::create_dir_all(&oxios_root).unwrap();
        // Some entries in `OXIOS_HOME_DENY_SUBPATHS` reference
        // sub-paths that may not exist yet (the deny is
        // policy-level, not data-level). The directory is created
        // on demand; for the test, a no-op canonicalize on
        // `home/<leaf>` is enough — `with_deny_subpath` invokes
        // canonicalize on the root (here, `~/.oxios`), not the
        // subpath.
        let mut gate = AccessGate::new(
            Arc::new(Mutex::new(access)),
            Arc::new(ExecConfig {
                allowlist_mode: AllowlistMode::Permissive,
                ..Default::default()
            }),
            Arc::new(NoOpAuditSink),
        );
        for leaf in OXI_HOME_DENY_ROOTS {
            gate = gate.with_deny_root(canon_tmp.join(leaf));
        }
        for sub in OXIOS_HOME_DENY_SUBPATHS {
            gate = gate.with_deny_subpath(&oxios_root, sub);
        }

        (gate, ctx, canon_tmp)
    }

    #[test]
    fn r2_deny_root_denies_absolute_path_under_oxi() {
        // R2 (c): the `~/.oxi` whole-root deny is unchanged from R1.
        let tmp = tempfile::tempdir().unwrap();
        let canon_tmp_for_path = tmp
            .path()
            .canonicalize()
            .unwrap_or_else(|_| tmp.path().to_path_buf());
        let vault_note = canon_tmp_for_path
            .join(".oxi")
            .join("vault")
            .join("notes")
            .join("foo.md");
        std::fs::create_dir_all(vault_note.parent().unwrap()).unwrap();
        std::fs::write(&vault_note, b"# secret note\n").unwrap();

        let (gate, ctx, _canon_tmp) = make_r2_gate(tmp.path());

        let result = gate.check(CheckRequest::Path {
            context: &ctx,
            path: &vault_note,
            mode: PathMode::Read,
        });
        let err = result.expect_err("vault note under ~/.oxi must be denied");
        assert_eq!(err.layer, DenyLayer::Permission);
    }

    #[test]
    fn r2_deny_subpath_denies_oxios_config_toml() {
        // R2 (b): `~/.oxios/config.toml` is in the sensitive list and
        // MUST be denied even with a broad allow-list.
        let tmp = tempfile::tempdir().unwrap();
        let canon_tmp_for_path = tmp
            .path()
            .canonicalize()
            .unwrap_or_else(|_| tmp.path().to_path_buf());
        let target = canon_tmp_for_path.join(".oxios").join("config.toml");
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&target, b"api_key = \"sk-test\"").unwrap();

        let (gate, ctx, _canon_tmp) = make_r2_gate(tmp.path());

        let result = gate.check(CheckRequest::Path {
            context: &ctx,
            path: &target,
            mode: PathMode::Write,
        });
        let err = result.expect_err("~/.oxios/config.toml must be denied (sub-path policy)");
        assert_eq!(err.layer, DenyLayer::Permission);
        assert!(
            err.reason.contains("protected ecosystem"),
            "reason should name the deny source; got {:?}",
            err.reason,
        );
    }

    #[test]
    fn r2_workspace_subtree_is_not_denied() {
        // R2 (a): `~/.oxios/workspace/<session>/x` MUST remain
        // accessible. This is the regression that R2 fixed — the
        // agent-runtime workspace grant (`~/.oxios/workspace/**`,
        // wired at agent_runtime.rs:824-826 and agent_lifecycle.rs
        // :305-307) and RFC-025 mount paths land here. Whole-root
        // denial would silently break the agent's documented
        // operating context.
        let tmp = tempfile::tempdir().unwrap();
        let canon_tmp_for_path = tmp
            .path()
            .canonicalize()
            .unwrap_or_else(|_| tmp.path().to_path_buf());
        let session_file = canon_tmp_for_path
            .join(".oxios")
            .join("workspace")
            .join("session-7")
            .join("notes.md");
        std::fs::create_dir_all(session_file.parent().unwrap()).unwrap();
        std::fs::write(&session_file, b"# session").unwrap();

        let (gate, ctx, _canon_tmp) = make_r2_gate(tmp.path());

        let result = gate.check(CheckRequest::Path {
            context: &ctx,
            path: &session_file,
            mode: PathMode::Read,
        });
        assert!(
            result.is_ok(),
            "~/.oxios/workspace/<session>/x must remain accessible; got {:?}",
            result,
        );
    }

    #[test]
    fn r2_parity_with_oxios_home_deny_subpaths_constant() {
        // T18 R3 — parity-by-construction. The expected list is
        // derived from the same production constant
        // (`OXIOS_HOME_DENY_SUBPATHS`) used by `agent_runtime.rs`,
        // so a future addition cannot desync this test from the
        // production wiring. A separate plain-data assertion at
        // the end of the function guarantees the constant has
        // non-trivial content (and would scream if someone
        // accidentally emptied it).
        for sensitive in OXIOS_HOME_DENY_SUBPATHS {
            let tmp = tempfile::tempdir().unwrap();
            let canon_tmp_for_path = tmp
                .path()
                .canonicalize()
                .unwrap_or_else(|_| tmp.path().to_path_buf());
            let target = canon_tmp_for_path.join(".oxios").join(sensitive);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&target, b"x").unwrap();

            let (gate, ctx, _canon_tmp) = make_r2_gate(tmp.path());

            let result = gate.check(CheckRequest::Path {
                context: &ctx,
                path: &target,
                mode: PathMode::Read,
            });
            assert!(
                result.is_err(),
                "sensitive subpath ~/.oxios/{} must be denied; got {:?}",
                sensitive,
                result,
            );
        }
        // Belt-and-braces: also catch any future dropping below
        // a sane minimum. Production needs ALL of the items below;
        // a missing entry is a security regression.
        let required = [
            "config.toml",
            "auth.json",
            "agent_log.db",
            "state",
            // T18 R3: web-dist staging + RFC-042 control socket
            "web",
            "run",
            // T18 R4: backup output directory
            "backups",
        ];
        for r in required {
            assert!(
                OXIOS_HOME_DENY_SUBPATHS.contains(&r),
                "OXIOS_HOME_DENY_SUBPATHS must contain `{}`; got {:?}",
                r,
                OXIOS_HOME_DENY_SUBPATHS,
            );
        }
    }

    #[test]
    fn r3_oxios_web_dir_is_denied() {
        // T18 R3 (a): web-dist staging + `.active` restart marker
        // is in the deny list — under R1's whole-root deny it was
        // covered, R2's narrowing dropped it. On non-embedded
        // builds (`cargo install`), the served UI comes from
        // `~/.oxios/web/dist-<version>/` + `~/.oxios/web/.active`.
        // An agent with broad allow can otherwise tamper the
        // served assets.
        let tmp = tempfile::tempdir().unwrap();
        let canon_tmp_for_path = tmp
            .path()
            .canonicalize()
            .unwrap_or_else(|_| tmp.path().to_path_buf());
        let marker = canon_tmp_for_path
            .join(".oxios")
            .join("web")
            .join(".active");
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, b"/tmp/staged-dist").unwrap();

        let (gate, ctx, _canon_tmp) = make_r2_gate(tmp.path());

        let result = gate.check(CheckRequest::Path {
            context: &ctx,
            path: &marker,
            mode: PathMode::Read,
        });
        let err = result.expect_err("~/.oxios/web/.active must be denied");
        assert_eq!(err.layer, DenyLayer::Permission);
    }

    #[test]
    fn r3_oxios_run_dir_is_denied() {
        // T18 R3 (a): the RFC-042 local-control socket home
        // (`~/.oxios/run/control.sock`) is added now as cheap
        // future-proofing — same surface class as the rest.
        let tmp = tempfile::tempdir().unwrap();
        let canon_tmp_for_path = tmp
            .path()
            .canonicalize()
            .unwrap_or_else(|_| tmp.path().to_path_buf());
        let sock = canon_tmp_for_path
            .join(".oxios")
            .join("run")
            .join("control.sock");
        std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
        std::fs::write(&sock, b"").unwrap();

        let (gate, ctx, _canon_tmp) = make_r2_gate(tmp.path());

        let result = gate.check(CheckRequest::Path {
            context: &ctx,
            path: &sock,
            mode: PathMode::Write,
        });
        let err = result.expect_err("~/.oxios/run/control.sock must be denied");
        assert_eq!(err.layer, DenyLayer::Permission);
    }

    #[test]
    fn r4_oxicode_auth_json_is_denied() {
        // T18 R4 (a): `~/.oxicode/auth.json` is the shared oxicode-cli
        // credential store that oxios's `CredentialStore` reads as a
        // legacy fallback. A broadly-allowed agent must not be able
        // to exfiltrate stored keys via `read`/`grep`. Whole-root
        // deny on `~/.oxicode` is appropriate (no safe sub-path).
        let tmp = tempfile::tempdir().unwrap();
        let canon_tmp_for_path = tmp
            .path()
            .canonicalize()
            .unwrap_or_else(|_| tmp.path().to_path_buf());
        let auth = canon_tmp_for_path.join(".oxicode").join("auth.json");
        std::fs::create_dir_all(auth.parent().unwrap()).unwrap();
        std::fs::write(&auth, b"{\"anthropic\":{\"key\":\"sk-ant-x\"}}").unwrap();

        let (gate, ctx, _canon_tmp) = make_r2_gate(tmp.path());

        let result = gate.check(CheckRequest::Path {
            context: &ctx,
            path: &auth,
            mode: PathMode::Read,
        });
        let err = result.expect_err("~/.oxicode/auth.json must be denied");
        assert_eq!(err.layer, DenyLayer::Permission);
    }

    #[test]
    fn r4_oxicode_neighboring_files_remain_allowable_in_deny_root() {
        // After R4 whole-root-deny on `~/.oxicode`, even files
        // outside the credential store under `~/.oxicode` are
        // blocked — that's the whole point of the policy (no safe
        // sub-path exists). This test pins the policy so a future
        // relaxation back to `with_deny_subpath` would require an
        // explicit test change AND a doc-comment update.
        let tmp = tempfile::tempdir().unwrap();
        let canon_tmp_for_path = tmp
            .path()
            .canonicalize()
            .unwrap_or_else(|_| tmp.path().to_path_buf());
        let unrelated = canon_tmp_for_path.join(".oxicode").join("unrelated.txt");
        std::fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
        std::fs::write(&unrelated, b"x").unwrap();

        let (gate, ctx, _canon_tmp) = make_r2_gate(tmp.path());

        let result = gate.check(CheckRequest::Path {
            context: &ctx,
            path: &unrelated,
            mode: PathMode::Read,
        });
        let err =
            result.expect_err("R4 denies the whole ~/.oxicode root, including unrelated files");
        assert_eq!(err.layer, DenyLayer::Permission);
    }

    #[test]
    fn r4_oxios_backups_subtree_is_denied() {
        // T18 R4 (b): `~/.oxios/backups/` is where POST
        // `/api/system/backup` writes its tarballs
        // (config.toml + vault contents). The tar carries
        // credential-bearing entries; the whole subtree is denied.
        let tmp = tempfile::tempdir().unwrap();
        let canon_tmp_for_path = tmp
            .path()
            .canonicalize()
            .unwrap_or_else(|_| tmp.path().to_path_buf());
        let backup_tar = canon_tmp_for_path
            .join(".oxios")
            .join("backups")
            .join("oxios-backup-20260821.tar.gz");
        std::fs::create_dir_all(backup_tar.parent().unwrap()).unwrap();
        std::fs::write(&backup_tar, b"tar contents").unwrap();

        let (gate, ctx, _canon_tmp) = make_r2_gate(tmp.path());

        let result = gate.check(CheckRequest::Path {
            context: &ctx,
            path: &backup_tar,
            mode: PathMode::Read,
        });
        let err = result.expect_err("~/.oxios/backups/<tarball> must be denied");
        assert_eq!(err.layer, DenyLayer::Permission);
    }

    #[test]
    fn r4_oxios_root_workspace_still_allowable() {
        // Regression: the R4 additions (whole-root `~/.oxicode`,
        // sub-path `backups`) must not affect anything that was
        // already passing in R2/R3. In particular, the workspace
        // subtree under `~/.oxios` keeps its allowance.
        let tmp = tempfile::tempdir().unwrap();
        let canon_tmp_for_path = tmp
            .path()
            .canonicalize()
            .unwrap_or_else(|_| tmp.path().to_path_buf());
        let workspace_file = canon_tmp_for_path
            .join(".oxios")
            .join("workspace")
            .join("session")
            .join("notes.md");
        std::fs::create_dir_all(workspace_file.parent().unwrap()).unwrap();
        std::fs::write(&workspace_file, b"# note").unwrap();

        let (gate, ctx, _canon_tmp) = make_r2_gate(tmp.path());

        let result = gate.check(CheckRequest::Path {
            context: &ctx,
            path: &workspace_file,
            mode: PathMode::Read,
        });
        assert!(
            result.is_ok(),
            "R4 must not regress: workspace grant preserved; got {:?}",
            result,
        );
    }

    #[test]
    fn r4_oxi_home_deny_roots_constant_contains_required_entries() {
        // T18 R4 — the production wiring uses `OXI_HOME_DENY_ROOTS`
        // as the single source of truth for whole-root denies. Make
        // sure the bare minimum (the two roots we know we need) is
        // present so a future hand-edit cannot silently drop one
        // and deny nothing or deny everything.
        for required_root in [".oxi", ".oxicode"] {
            assert!(
                OXI_HOME_DENY_ROOTS.contains(&required_root),
                "OXI_HOME_DENY_ROOTS must contain `{}`; got {:?}",
                required_root,
                OXI_HOME_DENY_ROOTS,
            );
        }
    }

    #[test]
    fn r2_deny_subpath_with_subdirectory_tree() {
        // A sensitive directory entry (e.g. `state`) must deny the
        // whole subtree — `state/agents/<id>.json` is also a deny.
        let tmp = tempfile::tempdir().unwrap();
        let canon_tmp_for_path = tmp
            .path()
            .canonicalize()
            .unwrap_or_else(|_| tmp.path().to_path_buf());
        let deep = canon_tmp_for_path
            .join(".oxios")
            .join("state")
            .join("agents")
            .join("a1b2.json");
        std::fs::create_dir_all(deep.parent().unwrap()).unwrap();
        std::fs::write(&deep, b"{}").unwrap();

        let (gate, ctx, _canon_tmp) = make_r2_gate(tmp.path());

        let result = gate.check(CheckRequest::Path {
            context: &ctx,
            path: &deep,
            mode: PathMode::Read,
        });
        let err = result.expect_err("subtree of a denied dir must be denied too");
        assert_eq!(err.layer, DenyLayer::Permission);
    }

    #[test]
    fn r2_deny_root_canonicalizes_symlinked_oxi() {
        // R2 (d): if `~/.oxi` is a symlink to elsewhere, the
        // canonicalized root IS that target and every request path
        // is canonicalized through `canonicalize_for_check` — the
        // deny still holds.
        let tmp = tempfile::tempdir().unwrap();
        let real_vault = tmp.path().join("real-vault");
        std::fs::create_dir_all(&real_vault).unwrap();
        let note = real_vault.join("note.md");
        std::fs::write(&note, b"x").unwrap();
        let oxi_link = tmp.path().join("oxi");
        std::os::unix::fs::symlink(&real_vault, &oxi_link).unwrap();

        let canon_tmp = tmp
            .path()
            .canonicalize()
            .unwrap_or_else(|_| tmp.path().to_path_buf());
        let mut access = AccessManager::new();
        let ctx = AgentContext::test_fixture("test-agent");
        let mut perms = AgentPermissions::for_new_agent("test-agent");
        perms.allowed_paths = vec![format!("{}/**", canon_tmp.display())];
        access.set_permissions(perms);
        access.rbac_manager_mut().assign_role(
            Subject::Agent(ctx.agent_id),
            crate::access_manager::Role::Superuser,
        );

        let gate = AccessGate::new(
            Arc::new(Mutex::new(access)),
            Arc::new(ExecConfig {
                allowlist_mode: AllowlistMode::Permissive,
                ..Default::default()
            }),
            Arc::new(NoOpAuditSink),
        )
        .with_deny_root(&oxi_link);

        let result = gate.check(CheckRequest::Path {
            context: &ctx,
            path: &note,
            mode: PathMode::Read,
        });
        let err = result.expect_err(
            "request through a symlinked ~/.oxi must still be denied (canonicalize both sides)",
        );
        assert_eq!(err.layer, DenyLayer::Permission);
    }

    #[test]
    fn r2_deny_subpath_canonicalizes_symlinked_oxios() {
        // R2 (d) sub-path variant: `~/.oxios` symlink pointing
        // elsewhere; the canonicalized root IS the target and a
        // sensitive subpath under it is still denied.
        let tmp = tempfile::tempdir().unwrap();
        let real_oxios = tmp.path().join("real-oxios");
        std::fs::create_dir_all(&real_oxios).unwrap();
        let real_config = real_oxios.join("config.toml");
        std::fs::write(&real_config, b"api_key = \"x\"").unwrap();
        let oxios_link = tmp.path().join(".oxios");
        std::os::unix::fs::symlink(&real_oxios, &oxios_link).unwrap();

        let canon_tmp = tmp
            .path()
            .canonicalize()
            .unwrap_or_else(|_| tmp.path().to_path_buf());
        let mut access = AccessManager::new();
        let ctx = AgentContext::test_fixture("test-agent");
        let mut perms = AgentPermissions::for_new_agent("test-agent");
        perms.allowed_paths = vec![format!("{}/**", canon_tmp.display())];
        access.set_permissions(perms);
        access.rbac_manager_mut().assign_role(
            Subject::Agent(ctx.agent_id),
            crate::access_manager::Role::Superuser,
        );

        let gate = AccessGate::new(
            Arc::new(Mutex::new(access)),
            Arc::new(ExecConfig {
                allowlist_mode: AllowlistMode::Permissive,
                ..Default::default()
            }),
            Arc::new(NoOpAuditSink),
        )
        .with_deny_subpath(&oxios_link, "config.toml");

        let result = gate.check(CheckRequest::Path {
            context: &ctx,
            path: &real_config,
            mode: PathMode::Read,
        });
        let err = result.expect_err(
            "request through a symlinked ~/.oxios to a sensitive file must still be denied",
        );
        assert_eq!(err.layer, DenyLayer::Permission);
    }

    #[test]
    fn r2_gate_without_any_deny_is_a_no_op() {
        // Regression: a gate built without any deny policy (legacy
        // callers, tests) must NOT suddenly start denying everything.
        // Existing allow-list semantics apply unchanged.
        let tmp = tempfile::tempdir().unwrap();
        let canon_tmp = tmp
            .path()
            .canonicalize()
            .unwrap_or_else(|_| tmp.path().to_path_buf());
        let project = canon_tmp.join("project").join("file.rs");
        std::fs::create_dir_all(project.parent().unwrap()).unwrap();
        std::fs::write(&project, b"x").unwrap();

        let mut access = AccessManager::new();
        let ctx = AgentContext::test_fixture("test-agent");
        let mut perms = AgentPermissions::for_new_agent("test-agent");
        perms.allowed_paths = vec![format!("{}/**", canon_tmp.display())];
        access.set_permissions(perms);
        access.rbac_manager_mut().assign_role(
            Subject::Agent(ctx.agent_id),
            crate::access_manager::Role::Superuser,
        );

        let gate = AccessGate::new(
            Arc::new(Mutex::new(access)),
            Arc::new(ExecConfig {
                allowlist_mode: AllowlistMode::Permissive,
                ..Default::default()
            }),
            Arc::new(NoOpAuditSink),
        );
        // No deny policy.

        let result = gate.check(CheckRequest::Path {
            context: &ctx,
            path: &project,
            mode: PathMode::Read,
        });
        assert!(
            result.is_ok(),
            "no deny configured ⇒ deny must not fire; got {:?}",
            result,
        );
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
