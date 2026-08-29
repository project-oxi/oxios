//! Filesystem addresses for per-project data (issues, milestones).
//!
//! ```text
//! ~/.oxi/oxios/projects/<project-uuid>/
//!   issues/
//!     0001-fix-login.md
//!     .alive/<ownership-id>
//!   milestones.yaml
//! ```
//!
//! # Why `~/.oxi/oxios` and not `~/.oxios`
//!
//! `~/.oxi/` is the per-user ecosystem umbrella: shared assets (`vault/`,
//! `brain/`, `foundation/`) sit beside a namespaced subdirectory per app.
//! Project data is written at that final address now; migrating the rest of
//! `~/.oxios/` is tracked separately (see
//! `docs/designs/2026-08-29-issues-milestones-todo-design.md` §11).
//!
//! # Why this address is the secure one
//!
//! [`crate::access_manager::gate::OXI_HOME_DENY_ROOTS`] denies `~/.oxi`
//! wholesale to the file tools (`read`/`write`/`edit`/`grep`/`find`/`ls`), so
//! project data is unreachable through them with no new deny entry. That is
//! the intended property, not a side effect: [`crate::kernel_handle::IssueApi`]
//! reaches the files through `std::fs` without passing `AccessGate`, so the
//! agent can only mutate issues through the `issue` tool — which maintains the
//! content-hash CAS and the assignment lock. An agent that could `edit` an
//! issue file directly would defeat both.

use std::path::PathBuf;

use super::ProjectId;

/// Environment override for [`data_home`]. Test-only seam — not a documented
/// user setting; production always resolves `~/.oxi/oxios`.
const DATA_HOME_ENV: &str = "OXIOS_DATA_HOME";

/// Oxios application data home (`~/.oxi/oxios`).
///
/// Falls back to a relative `.oxi/oxios` only when `HOME` is unset, which in
/// practice means a test or a broken environment; callers create directories
/// lazily so a wrong-but-relative path fails loudly at the first write rather
/// than silently writing to the filesystem root.
pub fn data_home() -> PathBuf {
    if let Ok(explicit) = std::env::var(DATA_HOME_ENV)
        && !explicit.is_empty()
    {
        return PathBuf::from(explicit);
    }
    match dirs::home_dir() {
        Some(home) => home.join(".oxi").join("oxios"),
        None => PathBuf::from(".oxi").join("oxios"),
    }
}

/// Directory holding one project's data. Created lazily on first write.
pub fn project_dir(id: ProjectId) -> PathBuf {
    data_home().join("projects").join(id.to_string())
}

/// Issue store directory for a project.
pub fn project_issues_dir(id: ProjectId) -> PathBuf {
    project_dir(id).join("issues")
}

/// Milestone definition file for a project.
pub fn project_milestones_path(id: ProjectId) -> PathBuf {
    project_dir(id).join("milestones.yaml")
}

/// Remove a project's entire data directory.
///
/// Called from the project-delete flow. Missing directory is success — a
/// project that never had an issue has nothing to clean up.
pub fn remove_project_dir(id: ProjectId) -> std::io::Result<()> {
    let dir = project_dir(id);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `OXIOS_DATA_HOME` is process-global; these tests must not run
    /// concurrently with each other.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_data_home<T>(dir: &std::path::Path, f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var(DATA_HOME_ENV).ok();
        // SAFETY: guarded by ENV_LOCK; restored before the guard drops.
        unsafe { std::env::set_var(DATA_HOME_ENV, dir) };
        let out = f();
        match prev {
            Some(v) => unsafe { std::env::set_var(DATA_HOME_ENV, v) },
            None => unsafe { std::env::remove_var(DATA_HOME_ENV) },
        }
        out
    }

    #[test]
    fn project_paths_nest_under_the_project_id() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ProjectId::new_v4();
        with_data_home(tmp.path(), || {
            let dir = project_dir(id);
            assert_eq!(dir, tmp.path().join("projects").join(id.to_string()));
            assert_eq!(project_issues_dir(id), dir.join("issues"));
            assert_eq!(project_milestones_path(id), dir.join("milestones.yaml"));
        });
    }

    #[test]
    fn distinct_projects_get_distinct_directories() {
        let tmp = tempfile::tempdir().unwrap();
        with_data_home(tmp.path(), || {
            assert_ne!(
                project_issues_dir(ProjectId::new_v4()),
                project_issues_dir(ProjectId::new_v4()),
            );
        });
    }

    #[test]
    fn remove_is_idempotent_and_deletes_content() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ProjectId::new_v4();
        with_data_home(tmp.path(), || {
            let issues = project_issues_dir(id);
            std::fs::create_dir_all(&issues).unwrap();
            std::fs::write(issues.join("0001-x.md"), "body").unwrap();

            remove_project_dir(id).unwrap();
            assert!(!project_dir(id).exists());
            // Second call on a missing directory is success, not an error.
            remove_project_dir(id).unwrap();
        });
    }

    #[test]
    fn data_home_defaults_under_the_dot_oxi_umbrella() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var(DATA_HOME_ENV).ok();
        // SAFETY: guarded by ENV_LOCK; restored below.
        unsafe { std::env::remove_var(DATA_HOME_ENV) };
        let home = data_home();
        assert!(
            home.ends_with("oxios") && home.parent().is_some_and(|p| p.ends_with(".oxi")),
            "expected ~/.oxi/oxios, got {}",
            home.display()
        );
        if let Some(v) = prev {
            unsafe { std::env::set_var(DATA_HOME_ENV, v) };
        }
    }
}
