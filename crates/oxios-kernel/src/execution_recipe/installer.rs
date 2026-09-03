//! Gated behavior-tool installer — the sole seam between the oxicode-sdk
//! behavior pack and the Oxios tool registry (design "One registration path").
//!
//! Every canonical pack tool passes through [`BehaviorToolInstaller::install`],
//! where it is:
//!
//! 1. reconciled against the static `PACK_TOOL_MAP` (missing mapping,
//!    mismatched exposed name, or mismatched side-effect class is a hard
//!    error — the exact descriptor/registration drift the design forbids);
//! 2. preflighted with the SAME `AccessGate` check `GatedTool` applies on
//!    every call, so a pack tool the turn's policy would always deny is
//!    degraded (or fails the essential install) before any model call;
//! 3. wrapped in `GatedTool::with_approval` — audit, approval, path
//!    sandboxing, structured events — and registered on the turn's existing
//!    `ToolRegistry` (last-write-wins replaces native same-name tools).
//!
//! Native overlays (`web_search`, `get_search_results`, `todo`) and
//! lifecycle delegation are rejected with structured `HostRejected` reasons;
//! the SDK install loop turns those into degradation records on the
//! [`InstalledBehaviorManifest`].

use std::sync::Arc;

use oxicode_sdk::ToolError;
use oxicode_sdk::behavior::{
    BehaviorInstallError, BehaviorToolDescriptor, BehaviorToolInstaller, InstalledBehaviorManifest,
    SideEffectClass,
};
use oxicode_sdk::{AgentTool, AgentToolResult, ToolContext, ToolRegistry};

use crate::access_manager::{AccessGate, AgentContext};
use crate::approval::ApprovalGate;
use crate::event_bus::EventBus;
use crate::tools::PendingPathAccess;
use crate::tools::PendingToolApprovals;
use crate::tools::gated_tool::GatedTool;

/// `GatedTool<T>` is generic over a concrete `AgentTool`; the pack hands us
/// `Arc<dyn AgentTool>`, which has no blanket `AgentTool` impl. This newtype
/// delegates so the gated wrapper can own it.
struct PackTool(Arc<dyn AgentTool>);

#[async_trait::async_trait]
impl AgentTool for PackTool {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn label(&self) -> &str {
        self.0.label()
    }

    fn description(&self) -> &str {
        self.0.description()
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.0.parameters_schema()
    }

    async fn execute(
        &self,
        tool_call_id: &str,
        params: serde_json::Value,
        signal: Option<tokio::sync::oneshot::Receiver<()>>,
        ctx: &ToolContext,
    ) -> Result<AgentToolResult, ToolError> {
        self.0.execute(tool_call_id, params, signal, ctx).await
    }
}

/// What the host does with one canonical pack tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackAction {
    /// Canonical pack tool replaces the native same-name tool for this turn
    /// (registry insert order makes the pack tool win).
    InstallShadowNative,
    /// New model-visible name; no native collision.
    Install,
    /// Oxios overlay stays registered; the pack tool is rejected and the SDK
    /// install loop records a `HostRejected` degradation.
    KeepNativeOverlay(&'static str),
}

/// Static reconciliation row: SDK implementation id -> Oxios policy
/// (design "Descriptor reconciliation" — keyed by stable implementation
/// identity, never by model-visible name).
struct PackToolPolicy {
    id: &'static str,
    exposed_name: &'static str,
    action: PackAction,
    /// Expected advisory classification; a mismatch is a programming error
    /// in the pack vs. this table and fails the install loudly.
    side_effect: SideEffectClass,
}

const fn p(
    id: &'static str,
    exposed_name: &'static str,
    action: PackAction,
    side_effect: SideEffectClass,
) -> PackToolPolicy {
    PackToolPolicy {
        id,
        exposed_name,
        action,
        side_effect,
    }
}

/// Reconciliation table for every `coding-omp-v1` descriptor.
static PACK_TOOL_MAP: &[PackToolPolicy] = &[
    p(
        "read.file.v1",
        "read",
        PackAction::InstallShadowNative,
        SideEffectClass::ReadOnly,
    ),
    p(
        "write.file.v1",
        "write",
        PackAction::InstallShadowNative,
        SideEffectClass::Mutating,
    ),
    p(
        "edit.hashline.v1",
        "edit",
        PackAction::InstallShadowNative,
        SideEffectClass::Mutating,
    ),
    p(
        "bash.session.v1",
        "bash",
        PackAction::Install,
        SideEffectClass::ProcessSpawning,
    ),
    p(
        "grep.search.v1",
        "grep",
        PackAction::InstallShadowNative,
        SideEffectClass::ReadOnly,
    ),
    p(
        "find.search.v1",
        "find",
        PackAction::InstallShadowNative,
        SideEffectClass::ReadOnly,
    ),
    p(
        "ls.fs.v1",
        "ls",
        PackAction::InstallShadowNative,
        SideEffectClass::ReadOnly,
    ),
    p(
        "ast-grep.search.v1",
        "ast_grep",
        PackAction::Install,
        SideEffectClass::ReadOnly,
    ),
    p(
        "ast-edit.write.v1",
        "ast_edit",
        PackAction::Install,
        SideEffectClass::Mutating,
    ),
    p(
        "web-search.network.v1",
        "web_search",
        PackAction::KeepNativeOverlay(
            "oxios kernel web_search overlay: managed provider + StructuredResultBus results",
        ),
        SideEffectClass::Networked,
    ),
    p(
        "search-results.cache.v1",
        "get_search_results",
        PackAction::KeepNativeOverlay("oxios kernel search-results overlay"),
        SideEffectClass::ReadOnly,
    ),
    p(
        "todo.session.v1",
        "todo",
        PackAction::KeepNativeOverlay(
            "oxios session todo overlay: kernel todo state + StructuredResultBus",
        ),
        SideEffectClass::Mutating,
    ),
    p(
        "subagent.delegation.v1",
        "subagent",
        PackAction::KeepNativeOverlay(
            "lifecycle-managed coding delegation lands in rollout step 6; the text-only runner is not coding-equivalent",
        ),
        SideEffectClass::ProcessSpawning,
    ),
    p(
        "lsp.host.v1",
        "lsp",
        PackAction::Install,
        SideEffectClass::ReadOnly,
    ),
    p(
        "eval.kernel.v2",
        "eval",
        PackAction::Install,
        SideEffectClass::ProcessSpawning,
    ),
    p(
        "debug.dap.v2",
        "debug",
        PackAction::Install,
        SideEffectClass::ProcessSpawning,
    ),
];

/// The host-controlled installer: reconciles, preflight-checks, gates, and
/// registers one canonical tool per call. Rejecting an optional tool yields
/// a structured degradation; rejecting an essential tool fails the whole
/// pack install BEFORE any model call (structured preflight failure).
pub struct GatedBehaviorInstaller<'a> {
    registry: &'a ToolRegistry,
    gate: Arc<AccessGate>,
    agent_context: AgentContext,
    approval_gate: Option<Arc<ApprovalGate>>,
    event_bus: Option<EventBus>,
    pending_approvals: Option<Arc<PendingToolApprovals>>,
    pending_path_access: Option<Arc<PendingPathAccess>>,
}

impl<'a> GatedBehaviorInstaller<'a> {
    /// Bind the installer to the turn's registry and policy bundle — the
    /// exact bundle `run_agent` builds for native tools.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry: &'a ToolRegistry,
        gate: Arc<AccessGate>,
        agent_context: AgentContext,
        approval_gate: Option<Arc<ApprovalGate>>,
        event_bus: Option<EventBus>,
        pending_approvals: Option<Arc<PendingToolApprovals>>,
        pending_path_access: Option<Arc<PendingPathAccess>>,
    ) -> Self {
        GatedBehaviorInstaller {
            registry,
            gate,
            agent_context,
            approval_gate,
            event_bus,
            pending_approvals,
            pending_path_access,
        }
    }
}

fn reconcile_error(descriptor: &BehaviorToolDescriptor, reason: &str) -> BehaviorInstallError {
    BehaviorInstallError::HostRejected {
        descriptor: descriptor.id.clone(),
        exposed_name: descriptor.exposed_name.clone(),
        reason: format!("descriptor reconciliation failed: {reason}"),
    }
}

impl BehaviorToolInstaller for GatedBehaviorInstaller<'_> {
    fn install(
        &mut self,
        descriptor: &BehaviorToolDescriptor,
        tool: Arc<dyn AgentTool>,
    ) -> Result<(), BehaviorInstallError> {
        // 1. Reconcile against the static table (fail loud on drift).
        let policy = PACK_TOOL_MAP
            .iter()
            .find(|p| p.id == descriptor.id.0.as_str())
            .ok_or_else(|| {
                reconcile_error(descriptor, "no oxios mapping for this behavior tool id")
            })?;
        if policy.exposed_name != descriptor.exposed_name {
            return Err(reconcile_error(
                descriptor,
                &format!(
                    "exposed name drift: map says {:?}, pack says {:?}",
                    policy.exposed_name, descriptor.exposed_name
                ),
            ));
        }
        if policy.side_effect != descriptor.side_effect {
            return Err(reconcile_error(
                descriptor,
                &format!(
                    "side-effect class drift: map says {:?}, pack says {:?}",
                    policy.side_effect, descriptor.side_effect
                ),
            ));
        }

        // 2. Native overlays stay; the pack tool is rejected with the
        //    documented reason (optional tools degrade; if the pack ever
        //    marks one essential, the install fails — correct).
        if let PackAction::KeepNativeOverlay(reason) = policy.action {
            return Err(BehaviorInstallError::HostRejected {
                descriptor: descriptor.id.clone(),
                exposed_name: descriptor.exposed_name.clone(),
                reason: reason.to_string(),
            });
        }

        // 3. Preflight with the same gate check GatedTool applies per call:
        //    a tool this turn's CSpace/RBAC always denies is degraded (or
        //    fails the essential install) instead of being registered and
        //    denied forever — the documented Layer-0 catch-22.
        let check = crate::access_manager::CheckRequest::Tool {
            context: &self.agent_context,
            tool_name: descriptor.exposed_name.as_str(),
        };
        if let Err(denied) = self.gate.check(check) {
            return Err(BehaviorInstallError::HostRejected {
                descriptor: descriptor.id.clone(),
                exposed_name: descriptor.exposed_name.clone(),
                reason: format!("preflight denied by access gate: {denied}"),
            });
        }

        // 4. Wrap and register. Registry inserts overwrite, so
        //    InstallShadowNative cleanly replaces the native tool.
        let gated = GatedTool::with_approval(
            PackTool(tool),
            self.gate.clone(),
            self.agent_context.clone(),
            self.approval_gate.clone(),
            self.event_bus.clone(),
            self.pending_approvals.clone(),
            self.pending_path_access.clone(),
        );
        self.registry.register_arc(Arc::new(gated));
        Ok(())
    }
}

/// Extend the agent's Layer-2 tool allowlist with the names a coding recipe
/// will install (design: a recipe may narrow, never widen — this only lets
/// the already-authorized tools reach Layer 2; the Layer-0 CSpace check and
/// every per-call `GatedTool` check still apply).
///
/// Mirrors `AgentLifecycleManager::ensure_permissions`, recipe-scoped: only
/// runs when the resolver selected a coding recipe.
pub fn grant_pack_tool_permissions(
    access_manager: &Arc<parking_lot::Mutex<crate::access_manager::AccessManager>>,
    agent_name: &str,
    exposed_names: &[&str],
) {
    let mut access = access_manager.lock();
    let perms = access.get_or_create_permissions(agent_name);
    for name in exposed_names {
        if !perms.allowed_tools.contains(*name) {
            perms.allow_tool(name);
        }
    }
}

/// Model-visible names a coding recipe installs (shadow + new names; native
/// overlays are NOT granted — the pack versions of those stay rejected).
pub fn pack_installable_names() -> Vec<&'static str> {
    PACK_TOOL_MAP
        .iter()
        .filter(|p| {
            matches!(
                p.action,
                PackAction::Install | PackAction::InstallShadowNative
            )
        })
        .map(|p| p.exposed_name)
        .collect()
}

/// Convenience wrapper: resolve `pack_ids` and install them through a fresh
/// [`GatedBehaviorInstaller`]. Returns the installed manifest (tools,
/// degradations, prompt layers, compatibility).
#[allow(clippy::too_many_arguments)]
pub fn install_coding_recipe(
    resolver: &oxicode_sdk::behavior::BehaviorPackResolver,
    pack_ids: &[oxicode_sdk::behavior::BehaviorPackId],
    registry: &ToolRegistry,
    gate: Arc<AccessGate>,
    agent_context: AgentContext,
    services: &oxicode_sdk::behavior::BehaviorSessionServices,
    approval_gate: Option<Arc<ApprovalGate>>,
    event_bus: Option<EventBus>,
    pending_approvals: Option<Arc<PendingToolApprovals>>,
    pending_path_access: Option<Arc<PendingPathAccess>>,
) -> Result<InstalledBehaviorManifest, BehaviorInstallError> {
    let resolved = resolver.resolve(pack_ids, services)?;
    let mut installer = GatedBehaviorInstaller::new(
        registry,
        gate,
        agent_context,
        approval_gate,
        event_bus,
        pending_approvals,
        pending_path_access,
    );
    resolved.install(services, &mut installer)
}

/// What one coding turn's install produced.
pub struct CodingTurnInstall {
    /// Installed-behavior manifest (tools, degradations, prompt layers,
    /// compatibility). Recorded as turn metadata + the
    /// `execution_recipe_resolved` event payload.
    pub manifest: InstalledBehaviorManifest,
    /// The session's snapshot store — the recipe-validated value for
    /// `AgentConfig.snapshot_store`.
    pub snapshot_store: Arc<dyn oxicode_sdk::oxicode_hashline::SnapshotStore>,
    /// Manager-owned extension statuses for the `runtime_extension_status`
    /// events.
    pub statuses: Vec<super::extensions::ExtensionStatus>,
}

/// Full per-turn composition for a coding recipe: acquire extensions, extend
/// the Layer-2 allowlist recipe-scoped, build the service inventory from what
/// the host actually holds, and install through the gated seam. This is the
/// exact sequence `run_agent` runs; an error here fails the turn BEFORE any
/// model call.
#[allow(clippy::too_many_arguments)]
pub fn install_coding_turn(
    extensions: &super::extensions::RuntimeExtensionManager,
    access_manager: &Arc<parking_lot::Mutex<crate::access_manager::AccessManager>>,
    registry: &ToolRegistry,
    gate: Arc<AccessGate>,
    agent_context: &AgentContext,
    approval_gate: Option<Arc<ApprovalGate>>,
    event_bus: Option<EventBus>,
    pending_approvals: Option<Arc<PendingToolApprovals>>,
    pending_path_access: Option<Arc<PendingPathAccess>>,
    resolved: &super::types::ResolvedExecution,
    workspace: &std::path::Path,
    session_id: Option<&str>,
    project_id: Option<uuid::Uuid>,
) -> Result<CodingTurnInstall, BehaviorInstallError> {
    let key = super::extensions::ExtensionKey {
        project_id,
        session_id: session_id.unwrap_or("turn").to_string(),
        workspace: workspace.to_path_buf(),
    };
    let ext = extensions.acquire(key, resolved.services);

    // Layer-2 allowlist, recipe-scoped (the gate still applies per call).
    grant_pack_tool_permissions(access_manager, &agent_context.agent_name, &{
        let mut names = pack_installable_names();
        names.sort_unstable();
        names
    });

    let services = oxicode_sdk::behavior::BehaviorSessionServices::new(workspace.to_path_buf())
        .with_snapshot_store(ext.snapshot_store.clone());
    let services = match &ext.shell_session {
        Some(shell) => services.with_shell_session(shell.clone()),
        None => services,
    };
    let services = ext
        .eval_kernels
        .iter()
        .fold(services, |s, k| s.with_eval_kernel(k.clone()));

    let resolver = oxicode_sdk::behavior::BehaviorPackResolver::with_builtin_packs()?;
    let manifest = install_coding_recipe(
        &resolver,
        &resolved.pack_ids,
        registry,
        gate,
        agent_context.clone(),
        &services,
        approval_gate,
        event_bus,
        pending_approvals,
        pending_path_access,
    )?;
    Ok(CodingTurnInstall {
        manifest,
        snapshot_store: ext.snapshot_store.clone(),
        statuses: ext.statuses.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access_manager::LAYER0_EXEMPT_TOOLS;
    use crate::access_manager::{AccessManager, AgentPermissions, NoOpAuditSink, Role, Subject};
    use crate::config::ExecConfig;
    use oxicode_sdk::behavior::{BehaviorPackId, BehaviorPackResolver, BehaviorSessionServices};
    use parking_lot::Mutex;
    use std::path::PathBuf;

    fn permissive_gate() -> Arc<AccessGate> {
        let access = Arc::new(Mutex::new(AccessManager::new()));
        let perms = AgentPermissions::for_new_agent("test-agent");
        access.lock().set_permissions(perms);
        let subject = Subject::Agent(uuid::Uuid::new_v4());
        access
            .lock()
            .rbac_manager_mut()
            .assign_role(subject, Role::Superuser);
        // Production parity: run_agent extends the Layer-2 allowlist with the
        // recipe's installable names before installing (recipe-scoped grant).
        grant_pack_tool_permissions(&access, "test-agent", &pack_installable_names());
        Arc::new(AccessGate::new(
            access,
            Arc::new(ExecConfig::default()),
            Arc::new(NoOpAuditSink),
        ))
    }

    fn restrictive_gate() -> Arc<AccessGate> {
        // No permissions, no roles: every Tool check is denied.
        Arc::new(AccessGate::new(
            Arc::new(Mutex::new(AccessManager::new())),
            Arc::new(ExecConfig::default()),
            Arc::new(NoOpAuditSink),
        ))
    }

    /// Turn context whose CSpace grants everything the coding pack's tools
    /// need — the Layer-0-exempt set rides along implicitly; the CSpace-driven
    /// tools (bash/ast_grep/ast_edit/lsp/eval/debug) get explicit EXECUTE
    /// grants exactly like `resolve_cspace` would produce for a Code profile.
    fn coding_agent_context() -> AgentContext {
        use crate::capability::{Capability, ResourceRef, Rights};
        let agent_id = crate::types::AgentId::new_v4();
        let mut cspace = crate::capability::CSpace::new(agent_id);
        cspace.insert(Capability::kernel(
            ResourceRef::Exec {
                mode: "shell".into(),
            },
            Rights::ALL,
        ));
        for tool in ["bash", "ast_grep", "ast_edit", "lsp", "eval", "debug"] {
            cspace.insert(Capability::kernel(
                ResourceRef::KernelDomain {
                    domain: tool.into(),
                },
                Rights::EXECUTE,
            ));
        }
        AgentContext::test_fixture_with_cspace("test-agent", cspace)
    }

    fn agent_context() -> AgentContext {
        AgentContext::test_fixture("test-agent")
    }

    fn coding_pack_id() -> BehaviorPackId {
        BehaviorPackId("coding-omp-v1".to_string())
    }

    fn pack_descriptors() -> Vec<BehaviorToolDescriptor> {
        let resolver = BehaviorPackResolver::with_builtin_packs().expect("builtin packs");
        resolver
            .pack(&coding_pack_id())
            .expect("coding-omp-v1 registered")
            .tools
            .clone()
    }

    #[test]
    fn every_pack_descriptor_is_reconciled() {
        let descriptors = pack_descriptors();
        assert_eq!(descriptors.len(), 16, "pack shape changed; update the map");
        for d in &descriptors {
            let rows: Vec<&PackToolPolicy> = PACK_TOOL_MAP
                .iter()
                .filter(|p| p.id == d.id.0.as_str())
                .collect();
            assert_eq!(rows.len(), 1, "descriptor {} must map exactly once", d.id);
            assert_eq!(
                rows[0].exposed_name, d.exposed_name,
                "name drift on {}",
                d.id
            );
            assert_eq!(
                rows[0].side_effect, d.side_effect,
                "side-effect drift on {}",
                d.id
            );
        }
    }

    #[test]
    fn layer0_names_stay_consistent_with_the_pack() {
        for p in PACK_TOOL_MAP {
            let exempt = LAYER0_EXEMPT_TOOLS.contains(&p.exposed_name);
            if exempt {
                // A Layer-0-exempt name the pack exposes must be an explicit
                // shadow or an explicit native overlay — never a new gate
                // decision invented here.
                assert!(
                    matches!(
                        p.action,
                        PackAction::InstallShadowNative | PackAction::KeepNativeOverlay(_)
                    ),
                    "{} is Layer-0-exempt and must be shadow/overlay",
                    p.exposed_name
                );
            } else {
                assert!(
                    !matches!(p.action, PackAction::InstallShadowNative),
                    "{} claims shadow-native but is not Layer-0-exempt",
                    p.exposed_name
                );
            }
        }
        // The shadow set itself must be exactly the exempt names the pack
        // replaces (read/write/edit/grep/find/ls).
        let shadowed: Vec<&str> = PACK_TOOL_MAP
            .iter()
            .filter(|p| p.action == PackAction::InstallShadowNative)
            .map(|p| p.exposed_name)
            .collect();
        assert_eq!(
            shadowed,
            vec!["read", "write", "edit", "grep", "find", "ls"]
        );
    }

    #[test]
    fn install_wraps_pack_tools_and_keeps_overlays_intact() {
        let resolver = BehaviorPackResolver::with_builtin_packs().expect("builtin packs");
        let registry = ToolRegistry::new();
        let gate = permissive_gate();
        let services = BehaviorSessionServices::new(PathBuf::from("/tmp/oxios-behavior-test"))
            .with_snapshot_store(Arc::new(
                oxicode_sdk::oxicode_hashline::InMemorySnapshotStore::new(),
            ));
        let manifest = install_coding_recipe(
            &resolver,
            &[coding_pack_id()],
            &registry,
            gate,
            coding_agent_context(),
            &services,
            None,
            None,
            None,
            None,
        )
        .expect("coding pack installs");

        // 12 canonical tools installed: read/write/edit/bash/grep/find/ls/
        // ast_grep/ast_edit install outright; lsp/eval/debug install with the
        // pack's reduced fallback semantics while their extension services
        // are absent (the SDK records extension-level degradations). The
        // 4 native-overlay rejections (web_search/get_search_results/todo/
        // subagent) never reach the registry from this installer.
        assert_eq!(manifest.tools.len(), 12, "installed: {:?}", manifest.tools);

        for name in [
            "read", "write", "edit", "bash", "grep", "find", "ls", "ast_grep", "ast_edit", "lsp",
            "eval", "debug",
        ] {
            assert!(registry.get(name).is_some(), "{name} must be registered");
        }
        for name in ["web_search", "get_search_results", "todo", "subagent"] {
            assert!(
                manifest.degraded.iter().any(|d| d.feature == name),
                "{name} must appear as a structured degradation"
            );
            assert!(
                registry.get(name).is_none(),
                "{name} overlay must stay native"
            );
        }
        // The compatibility ledger rides the manifest.
        assert!(!manifest.compatibility.target.is_empty());
    }

    #[test]
    fn required_hashline_missing_fails_the_install() {
        let resolver = BehaviorPackResolver::with_builtin_packs().expect("builtin packs");
        let registry = ToolRegistry::new();
        // No snapshot store: HashlineState is a REQUIRED extension.
        let services = BehaviorSessionServices::new(PathBuf::from("/tmp/oxios-behavior-test"));
        let err = install_coding_recipe(
            &resolver,
            &[coding_pack_id()],
            &registry,
            permissive_gate(),
            coding_agent_context(),
            &services,
            None,
            None,
            None,
            None,
        )
        .expect_err("missing required extension must fail loudly");
        assert!(matches!(
            err,
            BehaviorInstallError::RequiredExtensionMissing { .. }
                | BehaviorInstallError::RequiredServiceMissing { .. }
        ));
    }

    #[test]
    fn denied_capability_is_preflight_failure_not_a_dead_tool() {
        let resolver = BehaviorPackResolver::with_builtin_packs().expect("builtin packs");
        let registry = ToolRegistry::new();
        let services = BehaviorSessionServices::new(PathBuf::from("/tmp/oxios-behavior-test"))
            .with_snapshot_store(Arc::new(
                oxicode_sdk::oxicode_hashline::InMemorySnapshotStore::new(),
            ));
        // Gate with no grants: `read` (essential) is denied at preflight —
        // the install fails structurally before any model call, instead of
        // registering a tool that can never run.
        let err = install_coding_recipe(
            &resolver,
            &[coding_pack_id()],
            &registry,
            restrictive_gate(),
            agent_context(),
            &services,
            None,
            None,
            None,
            None,
        )
        .expect_err("essential denial must fail the install");
        assert!(
            err.to_string().contains("preflight denied"),
            "unexpected error: {err}"
        );
        assert!(registry.names().is_empty(), "nothing may be registered");
    }
}
