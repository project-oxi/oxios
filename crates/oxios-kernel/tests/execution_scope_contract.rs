//! Execution-scope contract tests (project-roots workbench, Task 2).
//!
//! Pins the no-ambient-grant contract, the multi-root CWD rule, the persona
//! resolution chain, the affordance matrix, and the EffectiveProfile
//! serialization keys.

use std::path::PathBuf;

use oxios_kernel::access_manager::AccessManager;
use oxios_kernel::agent_runtime::{
    AgentRuntimeConfig, apply_execution_grants, derive_execution_paths,
};
use oxios_kernel::orchestrator::Orchestrator;
use oxios_kernel::persona::{
    AFFORDANCE_DIFF, AFFORDANCE_FILES, AFFORDANCE_TERMINAL, AFFORDANCE_WORKTREE_FANOUT,
    EffectiveProfile, PersonaManager, PresentationLens, ToolProfile, derive_affordances,
    resolve_effective_profile,
};
use oxios_kernel::project::ProjectManager;
use oxios_kernel::session_context::SessionContext;
use oxios_kernel::turn_registry::TurnRegistry;
use oxios_kernel::{AgentLifecycleManager, EventBus, KernelDatabase, StateStore};
use oxios_ouroboros::ExecEnv;

fn workspace_orchestrator() -> (Orchestrator, std::sync::Arc<ProjectManager>) {
    let event_bus = EventBus::new(64);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_store =
        std::sync::Arc::new(StateStore::new(tmp.path().to_path_buf()).expect("state store"));
    let access_manager = std::sync::Arc::new(parking_lot::Mutex::new(
        oxios_kernel::access_manager::AccessManager::new(),
    ));
    let a2a = std::sync::Arc::new(oxios_kernel::a2a::A2AProtocol::new(event_bus.clone()));
    let lifecycle = AgentLifecycleManager::new(
        std::sync::Arc::new(oxios_kernel::supervisor::NoOpSupervisor),
        access_manager,
        a2a,
        event_bus.clone(),
        300,
        vec![],
        true,
        "/tmp/oxios-test-workspace".to_string(),
        std::sync::Arc::new(TurnRegistry::new()),
    );
    let orch = Orchestrator::new(event_bus, state_store, lifecycle);
    let db = std::sync::Arc::new(KernelDatabase::open_in_memory().expect("db"));
    let pm = std::sync::Arc::new(ProjectManager::new(db, None).expect("project manager"));
    orch.set_project_manager(pm.clone());
    (orch, pm)
}

fn test_config(workspace_dir: Option<PathBuf>) -> AgentRuntimeConfig {
    AgentRuntimeConfig {
        model_id: "test/model".to_string(),
        workspace_dir,
        ..Default::default()
    }
}

#[test]
fn exec_env_empty_roots_means_no_scope() {
    let (orch, _pm) = workspace_orchestrator();
    let ws = orch.resolve_project_workspace(None);
    assert!(ws.root_paths.is_empty());
    assert!(ws.context_body.is_empty());
    assert!(ws.project.is_none());
}

#[test]
fn exec_env_folderless_project_yields_instructions_only() {
    let (orch, pm) = workspace_orchestrator();
    let project = pm
        .create("writing", vec![], "write in English")
        .expect("create");
    let ws = orch.resolve_project_workspace(Some(&project.id.to_string()));
    assert!(ws.project.is_some());
    assert!(ws.root_paths.is_empty());
    assert!(ws.context_body.contains("write in English"));
    assert!(!ws.context_body.contains("Filesystem roots"));
}

#[test]
fn exec_env_multi_root_cwd_is_first_root() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let dir_a = dir.path().join("a");
    let dir_b = dir.path().join("b");
    std::fs::create_dir_all(&dir_a).expect("mkdir a");
    std::fs::create_dir_all(&dir_b).expect("mkdir b");
    let env = ExecEnv {
        root_paths: vec![dir_a.clone(), dir_b.clone()],
        ..Default::default()
    };
    // No configured workspace_dir: the fallback temp path must NOT leak
    // into the grants.
    let (cwd, grants) = derive_execution_paths(&env, &test_config(None));
    assert_eq!(cwd, dir_a, "CWD is the first root");
    assert_eq!(grants.len(), 2, "every root is granted exactly once");
    assert!(grants.contains(&dir_a));
    assert!(grants.contains(&dir_b));
}

#[test]
fn folderless_agent_gains_no_ambient_grants() {
    let ws_dir = PathBuf::from("/tmp/oxios-test-ambient-workspace");
    let env = ExecEnv::default();
    let (cwd, grants) = derive_execution_paths(&env, &test_config(Some(ws_dir.clone())));
    assert_eq!(cwd, ws_dir, "CWD falls back to the configured workspace");
    assert!(
        grants.is_empty(),
        "empty roots must grant NOTHING (cwd is a working directory, not authority)"
    );
}

#[tokio::test]
async fn persona_resolution_turn_overrides_session_overrides_global() {
    let persona_manager = PersonaManager::new();
    persona_manager
        .set_active("research")
        .await
        .expect("active");
    // A disabled persona that must fall through to the global active.
    persona_manager
        .store()
        .set_enabled("review", false)
        .expect("disable");

    let db = KernelDatabase::open_in_memory().expect("db");
    let project_manager = ProjectManager::new(std::sync::Arc::new(db), None).expect("pm");

    // Unknown project ⇒ has_roots false ⇒ folderless affordances.
    let profile =
        resolve_effective_profile(Some(&persona_manager), Some(&project_manager), None, None)
            .expect("global persona resolves");
    assert_eq!(profile.persona_id, "research");
    assert_eq!(profile.tool_profile, "code");
    assert_eq!(profile.affordances, vec![AFFORDANCE_DIFF]);

    // Turn override (enabled) wins over the global active.
    let profile = resolve_effective_profile(
        Some(&persona_manager),
        Some(&project_manager),
        Some("dev"),
        None,
    )
    .expect("turn persona resolves");
    assert_eq!(profile.persona_id, "dev");

    // Disabled turn persona falls through to the global active.
    let profile = resolve_effective_profile(
        Some(&persona_manager),
        Some(&project_manager),
        Some("review"),
        None,
    )
    .expect("falls through to global");
    assert_eq!(profile.persona_id, "research");

    // Unknown project id resolves has_roots=false (Code ⇒ [diff] only).
    let profile = resolve_effective_profile(
        Some(&persona_manager),
        Some(&project_manager),
        None,
        Some("00000000-0000-0000-0000-000000000000"),
    )
    .expect("persona resolves with unknown project");
    assert_eq!(profile.affordances, vec![AFFORDANCE_DIFF]);

    // No managers, no ids ⇒ None.
    assert!(resolve_effective_profile(None, None, None, None).is_none());
}

#[test]
fn affordances_matrix() {
    use ToolProfile::{Base, Code, Control, Minimal};
    let with_roots = [
        AFFORDANCE_DIFF,
        AFFORDANCE_FILES,
        AFFORDANCE_TERMINAL,
        AFFORDANCE_WORKTREE_FANOUT,
    ];
    for p in [Base, Code] {
        assert_eq!(derive_affordances(&p, true), with_roots);
        assert_eq!(derive_affordances(&p, false), vec![AFFORDANCE_DIFF]);
    }
    for p in [Minimal, Control] {
        assert!(derive_affordances(&p, true).is_empty());
        assert!(derive_affordances(&p, false).is_empty());
    }
}

#[test]
fn effective_profile_serializes_bilingual_keys() {
    let profile = EffectiveProfile {
        persona_id: "dev".to_string(),
        tool_profile: "code".to_string(),
        affordances: vec![AFFORDANCE_FILES.to_string()],
        presentation_lens: PresentationLens::Research,
    };
    let json: serde_json::Value = serde_json::to_value(&profile).expect("serialize");
    let mut keys: Vec<&str> = json
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "affordances",
            "persona_id",
            "presentation_lens",
            "tool_profile"
        ]
    );
}

#[test]
fn session_context_holds_singular_project_id() {
    let ctx = SessionContext::new();
    assert!(ctx.project_id.is_none());
    let ctx = SessionContext {
        project_id: Some("p1".to_string()),
    };
    assert_eq!(ctx.project_id.as_deref(), Some("p1"));
}

/// Pattern root: strip the trailing `/**` glob so grants can be compared as
/// directories.
fn grant_root(pattern: &str) -> &str {
    pattern.strip_suffix("/**").unwrap_or(pattern)
}

#[test]
fn real_grant_path_folderless_gains_no_ambient_grants() {
    let kernel_ws = tempfile::TempDir::new().expect("kernel ws tempdir");
    let mut am = AccessManager::new();
    let perms = am.get_or_create_permissions("agent-contract");

    // The REAL grant path exercised by run_agent, with empty project roots.
    apply_execution_grants(perms, &[], kernel_ws.path());

    let perms = am.get_or_create_permissions("agent-contract");
    let cwd = std::env::current_dir().expect("cwd");
    let home = std::env::var("HOME").map(std::path::PathBuf::from).ok();

    for pattern in &perms.allowed_paths {
        let root = grant_root(pattern);
        let root = std::path::Path::new(root);
        assert!(
            !cwd.starts_with(root),
            "no grant may cover the process CWD (violated by {pattern})"
        );
        if let Some(home) = &home {
            assert!(
                !home.starts_with(root) && !root.starts_with(home),
                "no grant may cover a user directory (violated by {pattern})"
            );
        }
    }

    // Exactly the kernel-infrastructure entries remain.
    let mut grants: Vec<&str> = perms.allowed_paths.iter().map(String::as_str).collect();
    grants.sort_unstable();
    assert_eq!(grants.len(), 2, "only kernel-ws + /tmp: {grants:?}");
    assert!(grants.contains(&"/tmp/**"));
    assert!(
        grants
            .iter()
            .any(|g| g.starts_with(kernel_ws.path().to_string_lossy().as_ref()))
    );
}

#[test]
fn real_grant_path_roots_are_granted_verbatim() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let root_a = dir.path().join("a");
    std::fs::create_dir_all(&root_a).expect("mkdir");
    let kernel_ws = tempfile::TempDir::new().expect("kernel ws tempdir");
    let mut am = AccessManager::new();
    let perms = am.get_or_create_permissions("agent-roots");
    apply_execution_grants(perms, std::slice::from_ref(&root_a), kernel_ws.path());

    let perms = am.get_or_create_permissions("agent-roots");
    let want = format!("{}/**", root_a.to_string_lossy().trim_end_matches('/'));
    assert!(
        perms.allowed_paths.iter().any(|p| p == &want),
        "project root granted verbatim: {:?}",
        perms.allowed_paths
    );
}
