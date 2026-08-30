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

/// Test-only scoped override of [`data_home`].
///
/// `OXIOS_DATA_HOME` is process-global and Rust runs unit tests in parallel
/// threads of one process, so every test that touches it must hold the *same*
/// lock. This module owns both the lock and the setter precisely so a second
/// module cannot introduce a second mutex over the same variable — which is
/// not synchronization at all, just two tests that each think they are safe.
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::Path;
    use std::sync::{Mutex, MutexGuard};

    use super::DATA_HOME_ENV;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Held for as long as the override should apply. Restores the previous
    /// value on drop, so a panicking test cannot leak it into the next one.
    pub(crate) struct DataHomeGuard {
        _lock: MutexGuard<'static, ()>,
        prev: Option<String>,
        _tmp: Option<tempfile::TempDir>,
    }

    impl Drop for DataHomeGuard {
        fn drop(&mut self) {
            // SAFETY: the lock is still held; no other test thread can be
            // reading or writing this variable.
            match self.prev.take() {
                Some(v) => unsafe { std::env::set_var(DATA_HOME_ENV, v) },
                None => unsafe { std::env::remove_var(DATA_HOME_ENV) },
            }
        }
    }

    fn acquire(dir: &Path, tmp: Option<tempfile::TempDir>) -> DataHomeGuard {
        // `unwrap_or_else(into_inner)`: one panicking test must not poison the
        // lock and cascade into every other test in the binary.
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var(DATA_HOME_ENV).ok();
        // SAFETY: guarded by ENV_LOCK for the guard's whole lifetime.
        unsafe { std::env::set_var(DATA_HOME_ENV, dir) };
        DataHomeGuard {
            _lock: lock,
            prev,
            _tmp: tmp,
        }
    }

    /// Point `data_home()` at a fresh temp directory for the guard's lifetime.
    pub(crate) fn temp_data_home() -> DataHomeGuard {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().to_path_buf();
        acquire(&path, Some(tmp))
    }

    /// Point `data_home()` at `dir` for the guard's lifetime.
    pub(crate) fn data_home_at(dir: &Path) -> DataHomeGuard {
        acquire(dir, None)
    }

    /// Take the lock without overriding, for a test that asserts the default.
    pub(crate) fn without_override() -> DataHomeGuard {
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var(DATA_HOME_ENV).ok();
        // SAFETY: guarded by ENV_LOCK for the guard's whole lifetime.
        unsafe { std::env::remove_var(DATA_HOME_ENV) };
        DataHomeGuard {
            _lock: lock,
            prev,
            _tmp: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{data_home_at, without_override};
    use super::*;

    fn with_data_home<T>(dir: &std::path::Path, f: impl FnOnce() -> T) -> T {
        let _guard = data_home_at(dir);
        f()
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
        let _guard = without_override();
        let home = data_home();
        assert!(
            home.ends_with("oxios")
                && home
                    .parent()
                    .is_some_and(|p| p.file_name() == Some(std::ffi::OsStr::new(".oxi"))),
            "expected ~/.oxi/oxios, got {}",
            home.display()
        );
    }
}
