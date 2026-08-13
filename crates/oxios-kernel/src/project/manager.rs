//! ProjectManager: CRUD operations for Projects using SQLite.
//!
//! Replaces SpaceManager with a simpler, project-centric design:
//! - No default project (project-less sessions are natural)
//! - No active/inactive state (activity is per-session)
//! - SQLite persistence alongside memories
//! - Lookup by name, path, or tag

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use parking_lot::RwLock;

use crate::kernel_db::KernelDatabase;

use super::project_db;
use super::{DetectionResult, Project, ProjectId, ProjectSource, detect_project};
use crate::event_bus::{EventBus, KernelEvent};

/// Errors from ProjectManager operations.
#[derive(thiserror::Error, Debug)]
pub enum ProjectManagerError {
    /// Project not found.
    #[error("Project not found: {0}")]
    NotFound(ProjectId),
    /// Project name already taken.
    #[error("Project name already exists: {0}")]
    DuplicateName(String),
    /// Invalid operation.
    #[error("Invalid operation: {0}")]
    Invalid(String),
}

/// Manages Projects: CRUD, lookup, and detection.
///
/// Projects are persisted in the `projects` SQLite table
/// (same `memory.db` as memories).
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
        // Ensure the schema exists (idempotent).
        // Mirrors MountManager::new — KernelDatabase only bootstraps
        // the connection, so project tables must be created here.
        project_db::ensure_project_schema(&db.conn())?;

        let mut projects = HashMap::new();
        let mut name_index = HashMap::new();

        // Load existing projects from SQLite
        let rows = project_db::list_projects(&db.conn())?;
        for project in rows {
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
    pub fn create_project(
        &self,
        name: String,
        tags: Vec<String>,
        emoji: Option<String>,
        description: Option<String>,
        source: ProjectSource,
    ) -> Result<Project> {
        // Check for duplicate name
        {
            let name_index = self.name_index.read();
            if name_index.contains_key(&name) {
                return Err(ProjectManagerError::DuplicateName(name).into());
            }
        }

        let mut project = Project::new(&name, source);
        project.tags = tags;
        if let Some(emoji) = emoji {
            project.emoji = emoji;
        }
        if let Some(description) = description {
            project.description = description;
        }

        // Persist to SQLite
        project_db::save_project(&self.db.conn(), &project)?;

        // Update in-memory indices
        {
            let mut projects = self.projects.write();
            let mut name_index = self.name_index.write();
            name_index.insert(project.name.clone(), project.id);
            projects.insert(project.id, project.clone());
        }

        // Publish event
        if let Some(ref event_bus) = self.event_bus {
            let _ = event_bus.publish(KernelEvent::ProjectCreated {
                project_id: project.id,
                name: project.name.clone(),
                source: source.to_string(),
            });
        }

        tracing::info!(name = %project.name, id = %project.id, "Project created");
        Ok(project)
    }

    /// Update an existing project.
    pub fn update_project(
        &self,
        id: ProjectId,
        name: Option<String>,
        tags: Option<Vec<String>>,
        emoji: Option<String>,
        description: Option<String>,
    ) -> Result<Project> {
        let mut projects = self.projects.write();
        let mut name_index = self.name_index.write();

        let project = projects
            .get_mut(&id)
            .ok_or(ProjectManagerError::NotFound(id))?;

        // If renaming, check for duplicate
        if let Some(ref new_name) = name
            && *new_name != project.name
        {
            if name_index.contains_key(new_name) {
                return Err(ProjectManagerError::DuplicateName(new_name.clone()).into());
            }
            // Remove old name from index
            name_index.remove(&project.name);
            name_index.insert(new_name.clone(), id);
            project.name = new_name.clone();
        }

        if let Some(tags) = tags {
            project.tags = tags;
        }
        if let Some(emoji) = emoji {
            project.emoji = emoji;
        }
        if let Some(description) = description {
            project.description = description;
        }

        project.updated_at = Utc::now();

        // Persist
        let project_clone = project.clone();
        drop(projects);
        drop(name_index);
        project_db::save_project(&self.db.conn(), &project_clone)?;

        tracing::info!(name = %project_clone.name, id = %id, "Project updated");
        Ok(project_clone)
    }

    /// Remove a project.
    pub fn remove_project(&self, id: ProjectId) -> Result<()> {
        {
            let mut projects = self.projects.write();
            let mut name_index = self.name_index.write();

            let project = projects
                .remove(&id)
                .ok_or(ProjectManagerError::NotFound(id))?;
            name_index.remove(&project.name);
        }

        // Remove from SQLite (cascades to project_memory via FK)
        project_db::delete_project(&self.db.conn(), &id.to_string())?;

        tracing::info!(id = %id, "Project removed");
        Ok(())
    }

    /// Record that a project was used in a session.
    pub fn touch(&self, id: ProjectId) {
        if let Some(project) = self.projects.write().get_mut(&id) {
            project.touch();
            let project_clone = project.clone();
            drop(self.projects.write());
            let _ = project_db::save_project(&self.db.conn(), &project_clone);
        }
    }

    /// Try to detect a project from a user message.
    ///
    /// Returns the matched ProjectId, or None.
    pub fn detect(&self, message: &str) -> DetectionResult {
        let projects = self.list_projects();
        detect_project(message, &projects)
    }

    /// Link a memory to a project.
    pub fn link_memory(&self, project_id: ProjectId, memory_id: &str) -> Result<()> {
        {
            let projects = self.projects.read();
            if !projects.contains_key(&project_id) {
                return Err(ProjectManagerError::NotFound(project_id).into());
            }
        }
        project_db::link_project_memory(&self.db.conn(), &project_id.to_string(), memory_id)?;
        Ok(())
    }

    /// Unlink a memory from a project.
    pub fn unlink_memory(&self, project_id: ProjectId, memory_id: &str) -> Result<()> {
        project_db::unlink_project_memory(&self.db.conn(), &project_id.to_string(), memory_id)?;
        Ok(())
    }

    /// Get all memory IDs associated with a project.
    pub fn get_project_memory_ids(&self, project_id: ProjectId) -> Result<Vec<String>> {
        project_db::get_project_memory_ids(&self.db.conn(), &project_id.to_string())
    }

    /// Save (upsert) a project to SQLite directly.
    ///
    /// Used when fields like `memory_visible` need updating
    /// outside the standard `update_project()` flow.
    pub fn save_project(&self, project: &Project) -> Result<()> {
        project_db::save_project(&self.db.conn(), project)?;

        // Refresh in-memory indices
        let mut projects = self.projects.write();
        let mut name_index = self.name_index.write();
        name_index.insert(project.name.clone(), project.id);
        projects.insert(project.id, project.clone());

        Ok(())
    }

    /// Update a Project's RFC-025 fields (mount_ids, instructions).
    pub fn update_project_bundle(
        &self,
        id: ProjectId,
        mount_ids: Option<Vec<crate::mount::MountId>>,
        instructions: Option<String>,
    ) -> Result<Project> {
        let mut projects = self.projects.write();
        let project = projects
            .get_mut(&id)
            .ok_or(ProjectManagerError::NotFound(id))?;

        if let Some(mids) = mount_ids {
            project.mount_ids = mids;
        }
        if let Some(instr) = instructions {
            project.instructions = instr;
        }
        project.updated_at = Utc::now();

        let project_clone = project.clone();
        drop(projects);
        project_db::save_project(&self.db.conn(), &project_clone)?;
        Ok(project_clone)
    }
    /// Clear the legacy `paths` field after the RFC-025 migration has linked
    /// Mounts via `mount_ids`. The field is retained on the struct only as the
    /// migration read-source; once Mounts are linked it is dead weight that
    /// the runtime legacy fallbacks would otherwise keep reading.
    pub fn clear_legacy_paths(&self, id: ProjectId) -> Result<()> {
        let mut projects = self.projects.write();
        let project = projects
            .get_mut(&id)
            .ok_or(ProjectManagerError::NotFound(id))?;
        project.paths.clear();
        project.updated_at = Utc::now();
        let project_clone = project.clone();
        drop(projects);
        project_db::save_project(&self.db.conn(), &project_clone)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: Full integration tests require a SQLite connection.
    // These are unit tests for in-memory operations.

    #[test]
    fn test_project_manager_error_display() {
        let id = ProjectId::new_v4();
        let err = ProjectManagerError::NotFound(id);
        assert!(err.to_string().contains("Project not found"));

        let err = ProjectManagerError::DuplicateName("test".to_string());
        assert!(err.to_string().contains("already exists"));
    }
}
