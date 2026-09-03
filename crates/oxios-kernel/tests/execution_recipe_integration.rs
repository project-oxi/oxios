//! Integration tests for the execution-recipe coding host (design:
//! `docs/designs/2026-08-31-execution-recipe-coding-host-design.md`).
//!
//! Covers the full per-turn composition that `run_agent` performs for a
//! coding-recipe turn — resolve → acquire extensions → recipe-scoped
//! permission grant → gated pack install → registry shape — plus the
//! general-v1 no-op guarantee, without requiring a live model.

use oxicode_sdk::behavior::BehaviorPackId;
use oxios_kernel::config::ExecutionRecipeConfig;
use oxios_kernel::execution_recipe::extensions::RuntimeExtensionManager;
use oxios_kernel::execution_recipe::{
    CODING_OMP_V1, ExecutionRecipeResolver, RecipeRequest, install_coding_turn,
    pack_installable_names,
};
use std::collections::HashMap;
use std::sync::Arc;

fn coding_config() -> ExecutionRecipeConfig {
    ExecutionRecipeConfig {
        enabled: true,
        coding_pilot: true,
        coding_shell: true,
        coding_eval: true,
        project_bindings: HashMap::from([(uuid::Uuid::nil().to_string(), CODING_OMP_V1.into())]),
    }
}

fn permissive_gate(
    access: Arc<parking_lot::Mutex<oxios_kernel::access_manager::AccessManager>>,
) -> Arc<oxios_kernel::access_manager::AccessGate> {
    Arc::new(oxios_kernel::access_manager::AccessGate::new(
        access,
        Arc::new(oxios_kernel::config::ExecConfig::default()),
        Arc::new(oxios_kernel::access_manager::TracingAuditSink),
    ))
}

fn coding_agent_context() -> oxios_kernel::access_manager::AgentContext {
    use oxios_kernel::capability::{Capability, ResourceRef, Rights};
    let agent_id = oxios_kernel::types::AgentId::new_v4();
    let mut cspace = oxios_kernel::capability::CSpace::new(agent_id);
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
    oxios_kernel::access_manager::AgentContext {
        agent_id,
        agent_name: "agent-test".into(),
        cspace: Arc::new(cspace),
        brain_space: None,
    }
}

fn resolved_coding() -> oxios_kernel::execution_recipe::ResolvedExecution {
    let resolver = ExecutionRecipeResolver::new(coding_config());
    resolver.resolve(RecipeRequest {
        project_id: Some(uuid::Uuid::nil()),
        explicit: None,
        persona_id: None,
    })
}

#[test]
fn general_v1_leaves_the_composition_untouched() {
    // Default config: pilot off. Even with a bound project the recipe is
    // general-v1 and the install seam is never invoked — the existing CSpace
    // composition is the whole story for the turn.
    let resolver = ExecutionRecipeResolver::new(ExecutionRecipeConfig::default());
    let resolved = resolver.resolve(RecipeRequest {
        project_id: Some(uuid::Uuid::nil()),
        explicit: None,
        persona_id: None,
    });
    assert!(!resolved.is_coding());
    assert_eq!(resolved.recipe.0, "general-v1");
}

#[test]
fn coding_pilot_installs_the_full_composition() {
    let resolved = resolved_coding();
    assert!(resolved.is_coding());
    assert_eq!(resolved.pack_ids.len(), 1);
    assert_eq!(resolved.pack_ids[0].0, CODING_OMP_V1);

    let registry = oxicode_sdk::ToolRegistry::new();
    let access = Arc::new(parking_lot::Mutex::new(
        oxios_kernel::access_manager::AccessManager::new(),
    ));
    let extensions = RuntimeExtensionManager::default();
    let gate = permissive_gate(access.clone());

    let workspace = std::env::temp_dir().join("oxios-recipe-it");
    let _ = std::fs::create_dir_all(&workspace);

    let installed = install_coding_turn(
        &extensions,
        &access,
        &registry,
        gate,
        &coding_agent_context(),
        None,
        None,
        None,
        None,
        &resolved,
        &workspace,
        Some("chat-it"),
        Some(uuid::Uuid::nil()),
    )
    .expect("coding turn installs");

    // Canonical tools registered (shadowing native same-name tools).
    for name in pack_installable_names() {
        assert!(registry.get(name).is_some(), "{name} must be registered");
    }
    // Native overlays rejected with structured degradations.
    for overlay in ["web_search", "get_search_results", "todo", "subagent"] {
        assert!(
            installed
                .manifest
                .degraded
                .iter()
                .any(|d| d.feature == overlay),
            "{overlay} must degrade as an oxios overlay"
        );
        assert!(registry.get(overlay).is_none());
    }
    // The manifest carries the pinned OMP compatibility target.
    assert!(installed.manifest.compatibility.target.contains("omp@"));
    // Extension statuses: hashline always; shell/eval per the service flags.
    let mut started: Vec<&str> = installed
        .statuses
        .iter()
        .filter(|s| s.state == "started")
        .map(|s| s.extension.as_str())
        .collect();
    started.sort_unstable();
    assert_eq!(started, vec!["eval", "hashline", "shell"]);
    // The recipe-validated snapshot-store wiring rides back for AgentConfig.
    let store: Arc<dyn oxicode_sdk::oxicode_hashline::SnapshotStore> =
        installed.snapshot_store.clone();
    let _ = store.head("probe"); // trait object is live
    // Layer-2 allowlist was extended recipe-scoped.
    let am = access.lock();
    let perms = am
        .get_permissions("agent-test")
        .expect("agent permissions exist");
    for name in pack_installable_names() {
        assert!(perms.allowed_tools.contains(name), "{name} must be allowed");
    }
}

#[test]
fn pilot_off_never_selects_the_coding_pack() {
    let cfg = ExecutionRecipeConfig {
        project_bindings: HashMap::from([(uuid::Uuid::nil().to_string(), CODING_OMP_V1.into())]),
        ..Default::default()
    };
    let resolver = ExecutionRecipeResolver::new(cfg);
    let resolved = resolver.resolve(RecipeRequest {
        project_id: Some(uuid::Uuid::nil()),
        explicit: None,
        persona_id: None,
    });
    assert_eq!(resolved.recipe.0, "general-v1");
    assert_eq!(resolved.pack_ids, Vec::<BehaviorPackId>::new());
}

#[test]
fn session_reuse_keeps_one_shell_per_chat_workspace() {
    let resolved = resolved_coding();
    let extensions = RuntimeExtensionManager::default();
    let registry = oxicode_sdk::ToolRegistry::new();
    let access = Arc::new(parking_lot::Mutex::new(
        oxios_kernel::access_manager::AccessManager::new(),
    ));
    let gate = permissive_gate(access.clone());
    let workspace = std::env::temp_dir().join("oxios-recipe-it2");
    let _ = std::fs::create_dir_all(&workspace);

    let args = (
        &extensions,
        &access,
        &registry,
        gate,
        resolved.clone(),
        workspace.clone(),
    );
    let _ = install_coding_turn(
        args.0,
        args.1,
        args.2,
        args.3.clone(),
        &coding_agent_context(),
        None,
        None,
        None,
        None,
        &args.4,
        &args.5,
        Some("chat-42"),
        None,
    )
    .expect("first turn installs");
    let second = install_coding_turn(
        args.0,
        args.1,
        args.2,
        args.3,
        &coding_agent_context(),
        None,
        None,
        None,
        None,
        &args.4,
        &args.5,
        Some("chat-42"),
        None,
    )
    .expect("second turn installs");
    // Same chat + workspace → the SAME persistent shell arc (reused, not
    // respawned): cwd/env continuity across turns.
    assert!(
        second
            .statuses
            .iter()
            .any(|s| s.extension == "shell" && s.state == "reused")
    );
}
