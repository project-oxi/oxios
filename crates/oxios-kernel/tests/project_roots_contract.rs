//! Contract tests for the project `root_paths` cutover (design §4.1).
//!
//! Locks the produced surface of Task 1: `validate_root_paths` invariants
//! (absolute + existing dir + canonicalized + deduped, order-stable, empty
//! allowed), manager create/update persistence semantics, unique project
//! names, and removal.

use std::path::PathBuf;
use std::sync::Arc;

use oxios_kernel::{KernelDatabase, ProjectManager, ProjectManagerError, validate_root_paths};

/// A real, existing directory that outlives the test body.
struct DirGuard {
    /// Pins the temp dir's lifetime; never read directly.
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl DirGuard {
    fn new(name: &str) -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join(name);
        std::fs::create_dir_all(&path).expect("create dir");
        Self { _dir: dir, path }
    }

    fn path(&self) -> PathBuf {
        self.path.clone()
    }
}

fn manager() -> ProjectManager {
    let db = Arc::new(KernelDatabase::open_in_memory().expect("in-memory db"));
    ProjectManager::new(db, None).expect("project manager")
}

fn nonexistent_dir() -> PathBuf {
    PathBuf::from(format!("/nonexistent-oxios-test-{}", uuid::Uuid::new_v4()))
}

#[test]
fn validate_accepts_absolute_existing_dirs_and_dedupes_order_stable() {
    // Duplicate "/tmp" entries collapse to one canonical entry (macOS /tmp is
    // a symlink to /private/tmp — canonicalization must resolve it).
    let out = validate_root_paths(vec![PathBuf::from("/tmp"), PathBuf::from("/tmp")])
        .expect("valid roots");
    assert_eq!(out.len(), 1, "duplicates must dedupe to one entry");
    assert_eq!(
        out[0],
        std::fs::canonicalize("/tmp").expect("canonicalize /tmp")
    );

    // Two distinct dirs: order preserved, both canonicalized.
    let a = DirGuard::new("a");
    let b = DirGuard::new("b");
    let out = validate_root_paths(vec![a.path(), b.path()]).expect("valid roots");
    assert_eq!(out.len(), 2, "distinct dirs must both be kept");
    assert_eq!(
        out[0],
        std::fs::canonicalize(a.path()).expect("canonicalize a")
    );
    assert_eq!(
        out[1],
        std::fs::canonicalize(b.path()).expect("canonicalize b")
    );
}

#[test]
fn validate_rejects_relative_path() {
    let err = validate_root_paths(vec![PathBuf::from("relative/dir")])
        .expect_err("relative path must be rejected");
    match err {
        ProjectManagerError::InvalidRoot { reason, .. } => {
            assert!(
                reason.contains("absolute"),
                "reason must mention absolute, got: {reason}"
            );
        }
        other => panic!("expected InvalidRoot, got: {other:?}"),
    }
}

#[test]
fn validate_rejects_missing_dir() {
    let missing = nonexistent_dir();
    let err = validate_root_paths(vec![missing.clone()]).expect_err("missing dir must be rejected");
    match err {
        ProjectManagerError::InvalidRoot { path, reason } => {
            assert_eq!(path, missing);
            assert!(
                reason.contains("does not exist"),
                "reason must say it does not exist, got: {reason}"
            );
        }
        other => panic!("expected InvalidRoot, got: {other:?}"),
    }
}

#[test]
fn validate_rejects_file_not_dir() {
    let file_guard = tempfile::TempDir::new().expect("tempdir");
    let file = file_guard.path().join("regular-file.txt");
    std::fs::write(&file, "content").expect("write file");

    let err = validate_root_paths(vec![file]).expect_err("plain file must be rejected");
    match err {
        ProjectManagerError::InvalidRoot { reason, .. } => {
            assert!(
                reason.contains("not a directory"),
                "reason must say not a directory, got: {reason}"
            );
        }
        other => panic!("expected InvalidRoot, got: {other:?}"),
    }
}

#[test]
fn validate_accepts_empty() {
    let out = validate_root_paths(vec![]).expect("empty roots are valid");
    assert!(out.is_empty(), "empty input must stay empty");
}

#[test]
fn create_persists_roundtrip_and_unique_name() {
    let pm = manager();
    let dir = DirGuard::new("roundtrip");

    let project = pm
        .create(
            "roundtrip-project",
            vec![dir.path()],
            "always run cargo clippy first",
        )
        .expect("create");

    let loaded = pm.get_project(project.id).expect("project after create");
    assert_eq!(loaded.name, "roundtrip-project");
    assert_eq!(
        loaded.root_paths,
        validate_root_paths(vec![dir.path()]).expect("valid roots")
    );
    assert_eq!(loaded.instructions, "always run cargo clippy first");

    // Second create with the same name must fail with NameExists.
    let dup = pm.create("roundtrip-project", vec![], "");
    match dup {
        Err(ProjectManagerError::NameExists(name)) => assert_eq!(name, "roundtrip-project"),
        other => panic!("expected NameExists, got: {other:?}"),
    }
}

#[test]
fn update_revalidates_roots() {
    let pm = manager();
    let dir = DirGuard::new("update-me");
    let project = pm
        .create("update-target", vec![dir.path()], "")
        .expect("create");

    // Update carrying a nonexistent dir must fail without mutating the record.
    let missing = nonexistent_dir();
    let err = pm
        .update(project.id, None, Some(vec![missing]), None)
        .expect_err("invalid roots must fail the update");
    match err {
        ProjectManagerError::InvalidRoot { reason, .. } => {
            assert!(
                reason.contains("does not exist"),
                "reason must say it does not exist, got: {reason}"
            );
        }
        other => panic!("expected InvalidRoot, got: {other:?}"),
    }
    let loaded = pm.get_project(project.id).expect("project still present");
    assert_eq!(
        loaded.root_paths,
        validate_root_paths(vec![dir.path()]).expect("valid roots"),
        "roots must be unchanged after failed update"
    );
    assert_eq!(loaded.instructions, project.instructions);
}

#[test]
fn remove_then_get_is_none() {
    let pm = manager();
    let project = pm.create("doomed", vec![], "").expect("create");
    pm.remove_project(project.id).expect("remove");
    assert!(
        pm.get_project(project.id).is_none(),
        "removed project must not resolve"
    );
}
