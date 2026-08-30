//! Project API — Project management system calls.
//!
//! Provides listing, lookup, create/update/remove over the reduced project
//! record (`root_paths` + `instructions`). Root paths are validated by the
//! manager; validation failures and name conflicts surface as typed
//! [`ProjectManagerError`] values with English messages.
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::project::{Project, ProjectManager, ProjectManagerError};

/// Serialized Project info for API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    pub root_paths: Vec<String>,
    pub instructions: String,
    /// Brain space inherited by NEW chats in this Project (`None` = none set).
    pub default_brain_space: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_active_at: String,
}

impl From<&Project> for ProjectInfo {
    fn from(project: &Project) -> Self {
        Self {
            id: project.id.to_string(),
            name: project.name.clone(),
            root_paths: project
                .root_paths
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
            instructions: project.instructions.clone(),
            default_brain_space: project.default_brain_space.clone(),
            created_at: project.created_at.to_rfc3339(),
            updated_at: project.updated_at.to_rfc3339(),
            last_active_at: project.last_active_at.to_rfc3339(),
        }
    }
}

/// Project system calls.
///
/// Lookup methods return `Option`; mutations return a typed
/// [`ProjectManagerError`] so callers can map validation failures precisely.
pub struct ProjectApi {
    /// Project manager for Project lifecycle.
    pub(crate) project_manager: Arc<ProjectManager>,
}

impl ProjectApi {
    /// Create a new ProjectApi.
    pub fn new(project_manager: Arc<ProjectManager>) -> Self {
        Self { project_manager }
    }

    /// Underlying `ProjectManager` for profile resolution.
    pub fn manager(&self) -> &ProjectManager {
        &self.project_manager
    }

    /// List all Projects.
    pub fn list_projects(&self) -> Vec<ProjectInfo> {
        self.project_manager
            .list_projects()
            .iter()
            .map(ProjectInfo::from)
            .collect()
    }

    /// Get Project details by ID.
    pub fn get_project(&self, id: &str) -> Option<ProjectInfo> {
        let project_id = Uuid::parse_str(id).ok()?;
        self.project_manager
            .get_project(project_id)
            .as_ref()
            .map(ProjectInfo::from)
    }

    /// Create a new project. Root paths are validated before persistence.
    pub fn create_project(
        &self,
        name: &str,
        root_paths: Vec<String>,
        instructions: &str,
    ) -> std::result::Result<ProjectInfo, ProjectManagerError> {
        let paths: Vec<PathBuf> = root_paths.into_iter().map(PathBuf::from).collect();
        let project = self.project_manager.create(name, paths, instructions)?;
        Ok(ProjectInfo::from(&project))
    }

    /// Update a project. Only non-`None` fields are changed; root paths are
    /// re-validated when provided.
    pub fn update_project(
        &self,
        id: &str,
        name: Option<String>,
        root_paths: Option<Vec<String>>,
        instructions: Option<String>,
    ) -> std::result::Result<ProjectInfo, ProjectManagerError> {
        let project_id = Uuid::parse_str(id)
            .map_err(|_| ProjectManagerError::Invalid(format!("invalid project ID: {id}")))?;
        let paths =
            root_paths.map(|ps| ps.into_iter().map(PathBuf::from).collect::<Vec<PathBuf>>());
        let project = self
            .project_manager
            .update(project_id, name, paths, instructions)?;
        Ok(ProjectInfo::from(&project))
    }

    /// Remove a project.
    pub fn remove_project(&self, id: &str) -> std::result::Result<(), ProjectManagerError> {
        let project_id = Uuid::parse_str(id)
            .map_err(|_| ProjectManagerError::Invalid(format!("invalid project ID: {id}")))?;
        self.project_manager.remove_project(project_id)
    }

    /// Update a Project's bundle fields (instructions) and the default brain
    /// space. For `default_brain_space`, the outer `Option` selects change
    /// (`Some`) vs not-provided (`None`, preserve existing); the inner `None`
    /// clears the value.
    pub fn update_project_bundle(
        &self,
        id: &str,
        instructions: Option<String>,
        default_brain_space: Option<Option<String>>,
    ) -> Result<ProjectInfo> {
        let pid = Uuid::parse_str(id).context("Invalid project ID")?;
        let project =
            self.project_manager
                .update_project_bundle(pid, instructions, default_brain_space)?;
        Ok(ProjectInfo::from(&project))
    }
}
