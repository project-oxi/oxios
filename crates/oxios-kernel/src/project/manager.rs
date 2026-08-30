//! ProjectManager: CRUD operations for Projects backed by SQLite.
//!
//! - No default project (project-less sessions are natural)
//! - SQLite persistence in the shared kernel database
//! - Root paths are validated (absolute, existing directory, canonicalized,
//!   de-duplicated, order-stable) before anything is persisted
//! - Project names are unique (in-memory pre-check + DB unique index backstop)

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use parking_lot::RwLock;

use super::project_db;
use super::{Project, ProjectId};
use crate::event_bus::{EventBus, KernelEvent};
use crate::kernel_db::KernelDatabase;

/// Errors from ProjectManager operations.
#[derive(thiserror::Error, Debug)]
pub enum ProjectManagerError {
    /// Project not found.
    #[error("Project not found: {0}")]
    NotFound(ProjectId),
    /// A project root path failed validation.
    #[error("invalid root path {path:?}: {reason}")]
    InvalidRoot {
        /// The offending path.
        path: PathBuf,
        /// English explanation of why the path was rejected.
        reason: String,
    },
    /// Project name already taken.
    #[error("project name already exists: {0}")]
    NameExists(String),
    /// Invalid operation.
    #[error("Invalid operation: {0}")]
    Invalid(String),
    /// Database failure.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
}

/// Validate and canonicalize project root paths (design §4.1).
///
/// Every path must be absolute and an existing directory; accepted paths are
/// canonicalized (symlinks resolved) and de-duplicated while preserving
/// first-occurrence order. An empty list is valid — it means the project has
/// no filesystem scope.
pub fn validate_root_paths(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>, ProjectManagerError> {
    let mut canonical: Vec<PathBuf> = Vec::with_capacity(paths.len());
    for path in paths {
        if !path.is_absolute() {
            return Err(ProjectManagerError::InvalidRoot {
                path,
                reason: "path must be absolute".to_string(),
            });
        }
        let metadata = std::fs::metadata(&path).map_err(|_| ProjectManagerError::InvalidRoot {
            path: path.clone(),
            reason: "path does not exist".to_string(),
        })?;
        if !metadata.is_dir() {
            return Err(ProjectManagerError::InvalidRoot {
                path: path.clone(),
                reason: "path is not a directory".to_string(),
            });
        }
        let canon = std::fs::canonicalize(&path).map_err(|_| ProjectManagerError::InvalidRoot {
            path: path.clone(),
            reason: "path does not exist".to_string(),
        })?;
        if !canonical.contains(&canon) {
            canonical.push(canon);
        }
    }
    Ok(canonical)
}

/// True when the SQLite error is a constraint violation (the `name` unique
/// index backstop catching a create/update race).
fn is_unique_violation(err: &rusqlite::Error) -> bool {
    matches!(
        err.sqlite_error_code(),
        Some(rusqlite::ErrorCode::ConstraintViolation)
    )
}

/// Manages Projects: CRUD, validation, and lookup.
///
/// Projects are persisted in the `projects` SQLite table (the shared
/// `kernel.db`) and mirrored in an in-memory index for fast lookup.
pub struct ProjectManager {
    /// In-memory index of all Projects (loaded at startup).
    projects: RwLock<HashMap<ProjectId, Project>>,
    /// Name → ID index for fast name lookup.
    name_index: RwLock<HashMap<String, ProjectId>>,
    /// SQLite database for persistence.
    db: Arc<KernelDatabase>,
    /// Event bus for publishing project events.
    event_bus: Option<EventBus>,
}

impl ProjectManager {
    /// Create a new ProjectManager, loading existing projects from SQLite.
    pub fn new(db: Arc<KernelDatabase>, event_bus: Option<EventBus>) -> Result<Self> {
        // Ensure the schema exists (idempotent). KernelDatabase only
        // bootstraps the connection, so the project table is created here.
        project_db::ensure_project_schema(&db.conn())?;

        let mut projects = HashMap::new();
        let mut name_index = HashMap::new();

        // Load existing projects from SQLite
        for project in project_db::list_projects(&db.conn())? {
            name_index.insert(project.name.clone(), project.id);
            projects.insert(project.id, project);
        }

        tracing::info!(count = projects.len(), "ProjectManager initialized");
        Ok(Self {
            projects: RwLock::new(projects),
            name_index: RwLock::new(name_index),
            db,
            event_bus,
        })
    }

    /// List all projects.
    pub fn list_projects(&self) -> Vec<Project> {
        self.projects.read().values().cloned().collect()
    }

    /// Get a project by ID.
    pub fn get_project(&self, id: ProjectId) -> Option<Project> {
        self.projects.read().get(&id).cloned()
    }

    /// Get a project by name.
    pub fn get_project_by_name(&self, name: &str) -> Option<Project> {
        let name_index = self.name_index.read();
        let id = name_index.get(name)?;
        self.projects.read().get(id).cloned()
    }

    /// Create a new project.
    ///
    /// Validates the name (non-empty, unique) and the root paths before any
    /// state is written; the DB unique index on `name` is the concurrency
    /// backstop for the in-memory pre-check.
    pub fn create(
        &self,
        name: &str,
        root_paths: Vec<PathBuf>,
        instructions: &str,
    ) -> std::result::Result<Project, ProjectManagerError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ProjectManagerError::Invalid(
                "project name must not be empty".to_string(),
            ));
        }
        if self.name_index.read().contains_key(name) {
            return Err(ProjectManagerError::NameExists(name.to_string()));
        }

        let roots = validate_root_paths(root_paths)?;
        let project = Project::new(name, roots, instructions);

        if let Err(e) = project_db::save_project(&self.db.conn(), &project) {
            if is_unique_violation(&e) {
                return Err(ProjectManagerError::NameExists(project.name.clone()));
            }
            return Err(e.into());
        }

        // Update in-memory indices
        {
            let mut projects = self.projects.write();
            self.name_index
                .write()
                .insert(project.name.clone(), project.id);
            projects.insert(project.id, project.clone());
        }

        // Publish event
        if let Some(event_bus) = &self.event_bus {
            let _ = event_bus.publish(KernelEvent::ProjectCreated {
                project_id: project.id,
                name: project.name.clone(),
                source: "manual".to_string(),
            });
        }

        tracing::info!(name = %project.name, id = %project.id, "Project created");
        Ok(project)
    }

    /// Update an existing project. Only the provided fields change; root
    /// paths are re-validated before any state is mutated.
    pub fn update(
        &self,
        id: ProjectId,
        name: Option<String>,
        root_paths: Option<Vec<PathBuf>>,
        instructions: Option<String>,
    ) -> std::result::Result<Project, ProjectManagerError> {
        if let Some(new_name) = &name
            && new_name.trim().is_empty()
        {
            return Err(ProjectManagerError::Invalid(
                "project name must not be empty".to_string(),
            ));
        }
        // Validate roots up-front so a rejected path leaves the record
        // untouched (design §9: invalid roots never reach persistence).
        let roots = match root_paths {
            Some(paths) => Some(validate_root_paths(paths)?),
            None => None,
        };

        let project_clone = {
            let mut name_index = self.name_index.write();
            let mut projects = self.projects.write();
            let project = projects
                .get_mut(&id)
                .ok_or(ProjectManagerError::NotFound(id))?;

            // If renaming, check for duplicates and reindex.
            if let Some(new_name) = name.map(|n| n.trim().to_string())
                && new_name != project.name
            {
                if name_index.contains_key(&new_name) {
                    return Err(ProjectManagerError::NameExists(new_name));
                }
                name_index.remove(&project.name);
                name_index.insert(new_name.clone(), id);
                project.name = new_name;
            }

            if let Some(roots) = roots {
                project.root_paths = roots;
            }
            if let Some(instructions) = instructions {
                project.instructions = instructions;
            }
            project.updated_at = Utc::now();
            project.clone()
        };

        if let Err(e) = project_db::save_project(&self.db.conn(), &project_clone) {
            if is_unique_violation(&e) {
                return Err(ProjectManagerError::NameExists(project_clone.name.clone()));
            }
            return Err(e.into());
        }

        tracing::info!(name = %project_clone.name, id = %id, "Project updated");
        Ok(project_clone)
    }

    /// Remove a project.
    pub fn remove_project(&self, id: ProjectId) -> Result<(), ProjectManagerError> {
        {
            let mut projects = self.projects.write();
            let project = projects
                .remove(&id)
                .ok_or(ProjectManagerError::NotFound(id))?;
            self.name_index.write().remove(&project.name);
        }

        project_db::delete_project(&self.db.conn(), &id.to_string())?;

        // Remove the project's on-disk data (issues, milestones). A missing
        // directory is success; any other failure degrades to a warning — the
        // SQLite row is already gone, and the orphaned files are inert.
        if let Err(e) = crate::project::paths::remove_project_dir(id) {
            tracing::warn!(id = %id, error = %e, "failed to remove project data directory");
        }

        tracing::info!(id = %id, "Project removed");
        Ok(())
    }

    /// Record that a project was used in a session.
    pub fn touch(&self, id: ProjectId) {
        let project_clone = {
            let mut projects = self.projects.write();
            let Some(project) = projects.get_mut(&id) else {
                return;
            };
            project.touch();
            project.clone()
        };
        let _ = project_db::save_project(&self.db.conn(), &project_clone);
    }

    /// Update a Project's bundle fields (instructions, default brain space).
    ///
    /// For `default_brain_space`, the outer `Option` selects change (`Some`)
    /// vs leave-untouched (`None`); the inner `None` clears the value. The
    /// reduced model carries no `mount_ids`; presentation is UI-derived.
    pub fn update_project_bundle(
        &self,
        id: ProjectId,
        instructions: Option<String>,
        default_brain_space: Option<Option<String>>,
    ) -> Result<Project> {
        let mut projects = self.projects.write();
        let project = projects
            .get_mut(&id)
            .ok_or(ProjectManagerError::NotFound(id))?;

        if let Some(instr) = instructions {
            project.instructions = instr;
        }
        if let Some(space) = default_brain_space {
            project.default_brain_space = space;
        }
        project.updated_at = Utc::now();

        let project_clone = project.clone();
        drop(projects);
        project_db::save_project(&self.db.conn(), &project_clone)?;
        Ok(project_clone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_manager_error_display() {
        let id = ProjectId::new_v4();
        let err = ProjectManagerError::NotFound(id);
        assert!(err.to_string().contains("Project not found"));

        let err = ProjectManagerError::NameExists("test".to_string());
        assert!(err.to_string().contains("already exists"));

        let err = ProjectManagerError::InvalidRoot {
            path: PathBuf::from("/x"),
            reason: "path does not exist".to_string(),
        };
        assert!(err.to_string().contains("invalid root path"));
    }

    #[test]
    fn remove_project_deletes_the_project_data_directory() {
        use crate::kernel_handle::issue_api::{IssueApi, NewIssue};
        use crate::project::paths::{self, test_support::temp_data_home};

        let _home = temp_data_home();
        let db = Arc::new(KernelDatabase::open_in_memory().expect("open db"));
        let manager = ProjectManager::new(db, None).expect("manager");
        let project = manager
            .create("cleanup", vec![], "")
            .expect("create project");

        let issues = IssueApi::new();
        issues
            .create_milestone(project.id, "v1".into(), None, None, None)
            .expect("create milestone");
        issues
            .create(project.id, NewIssue::new("fix it").session("s"))
            .expect("create issue");

        let dir = paths::project_dir(project.id);
        assert!(
            dir.is_dir(),
            "data dir should exist before removal: {}",
            dir.display()
        );

        manager.remove_project(project.id).expect("remove project");

        assert!(
            !dir.exists(),
            "data dir should be gone after removal: {}",
            dir.display()
        );
    }
}
