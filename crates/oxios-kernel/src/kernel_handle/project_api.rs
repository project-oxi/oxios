//! Project API — Project management system calls (RFC-011).
//!
//! Provides API endpoints for:
//! - Listing and querying Projects
//! - CRUD operations on Projects
//! - Project-Memory association (link/unlink)

use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::project::{Project, ProjectManager, ProjectSource};

/// Serialized Project info for API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: String,
    pub tags: Vec<String>,
    pub emoji: String,
    pub memory_visible: bool,
    /// RFC-025: Mounts this Project references.
    pub mount_ids: Vec<String>,
    /// RFC-025: Custom instructions injected into the system prompt.
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
            description: project.description.clone(),
            source: project.source.to_string(),
            tags: project.tags.clone(),
            emoji: project.emoji.clone(),
            memory_visible: project.memory_visible,
            mount_ids: project.mount_ids.iter().map(|m| m.to_string()).collect(),
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
/// All methods return `Result` for operations that can fail,
/// and `Option` for lookup operations.
#[allow(dead_code)]
pub struct ProjectApi {
    /// Project manager for Project lifecycle.
    pub(crate) project_manager: Arc<ProjectManager>,
}

impl ProjectApi {
    /// Create a new ProjectApi.
    pub fn new(project_manager: Arc<ProjectManager>) -> Self {
        Self { project_manager }
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

    /// Create a new project.
    pub fn create_project(
        &self,
        name: String,
        tags: Vec<String>,
        emoji: Option<String>,
        description: Option<String>,
    ) -> Result<ProjectInfo> {
        let project = self.project_manager.create_project(
            name,
            tags,
            emoji,
            description,
            ProjectSource::Manual,
        )?;
        Ok(ProjectInfo::from(&project))
    }

    /// Update a project. Only non-None fields are changed.
    #[allow(clippy::too_many_arguments)]
    pub fn update_project(
        &self,
        id: &str,
        name: Option<String>,
        tags: Option<Vec<String>>,
        emoji: Option<String>,
        description: Option<String>,
        memory_visible: Option<bool>,
    ) -> Result<ProjectInfo> {
        let project_id = Uuid::parse_str(id).context("Invalid project ID")?;

        let mut project =
            self.project_manager
                .update_project(project_id, name, tags, emoji, description)?;

        // memory_visible requires separate save (not part of update_project signature)
        if let Some(visible) = memory_visible {
            project.memory_visible = visible;
            project.updated_at = Utc::now();
            self.project_manager.save_project(&project)?;
        }

        Ok(ProjectInfo::from(&project))
    }

    /// Remove a project.
    pub fn remove_project(&self, id: &str) -> Result<()> {
        let project_id = Uuid::parse_str(id).context("Invalid project ID")?;
        self.project_manager.remove_project(project_id)
    }

    /// Link a memory to a project.
    pub fn link_memory(&self, project_id: &str, memory_id: &str) -> Result<()> {
        let pid = Uuid::parse_str(project_id).context("Invalid project ID")?;
        self.project_manager.link_memory(pid, memory_id)
    }

    /// Unlink a memory from a project.
    pub fn unlink_memory(&self, project_id: &str, memory_id: &str) -> Result<()> {
        let pid = Uuid::parse_str(project_id).context("Invalid project ID")?;
        self.project_manager.unlink_memory(pid, memory_id)
    }

    /// Get all memory IDs linked to a project.
    pub fn get_project_memory_ids(&self, project_id: &str) -> Result<Vec<String>> {
        let pid = Uuid::parse_str(project_id).context("Invalid project ID")?;
        self.project_manager.get_project_memory_ids(pid)
    }

    /// Update a Project's RFC-025 bundle fields (mount_ids, instructions) and
    /// the default brain space. For `default_brain_space`, the outer `Option`
    /// selects change (`Some`) vs not-provided (`None`, preserve existing);
    /// the inner `None` clears the value.
    pub fn update_project_bundle(
        &self,
        id: &str,
        mount_ids: Option<Vec<String>>,
        instructions: Option<String>,
        default_brain_space: Option<Option<String>>,
    ) -> Result<ProjectInfo> {
        let pid = Uuid::parse_str(id).context("Invalid project ID")?;
        // Reject invalid UUIDs rather than silently dropping them. Collecting
        // all the bad IDs lets the caller see every offender in one error.
        let mount_ids = match mount_ids {
            Some(ids) => {
                let mut parsed = Vec::with_capacity(ids.len());
                let mut bad = Vec::new();
                for s in ids {
                    match uuid::Uuid::parse_str(&s) {
                        Ok(u) => parsed.push(u),
                        Err(_) => bad.push(s),
                    }
                }
                if !bad.is_empty() {
                    anyhow::bail!("Invalid mount ID(s): {}", bad.join(", "));
                }
                Some(parsed)
            }
            None => None,
        };
        let project = self.project_manager.update_project_bundle(
            pid,
            mount_ids,
            instructions,
            default_brain_space,
        )?;
        Ok(ProjectInfo::from(&project))
    }
}
