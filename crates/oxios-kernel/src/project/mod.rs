//! Project module: work context management.
//!
//! Replaces the Space system with a project-centric model:
//! - Projects are registered aliases for filesystem paths
//! - Sessions reference projects (1 primary + N secondary)
//! - Memories link to projects via a junction table (N:M)
//!
//! ## Structure
//!
//! - `mod.rs` — Project struct and ProjectSource enum (this file)
//! - `manager.rs` — ProjectManager (CRUD, lookup, detection)
//! - `detection.rs` — Detection logic (name/path/tag matching)
//! - `paths.rs` — Per-project data directory (`~/.oxi/oxios/projects/<id>/`)
//! - `milestone.rs` — Milestone definitions + the `milestone:` issue label

pub mod conversation_buffer;
pub mod detection;
pub mod manager;
pub mod milestone;
pub mod paths;
pub mod project_db;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

// ── Re-exports ──────────────────────────────────────────────
pub use conversation_buffer::{ConversationBuffer, ConversationTurn};
pub use detection::{DetectionResult, detect_project, extract_path, find_by_id, find_by_name};

pub use manager::{ProjectManager, ProjectManagerError};
pub use milestone::{Milestone, MilestonePatch, MilestoneProgress, MilestoneStatus};
pub use paths::{project_dir, project_issues_dir, project_milestones_path};

/// Unique identifier for a Project.
pub type ProjectId = Uuid;

/// How a Project was registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectSource {
    /// User explicitly created via UI/CLI.
    Manual,
    /// OS auto-detected from a path in the conversation.
    AutoDetected,
}

impl std::fmt::Display for ProjectSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectSource::Manual => write!(f, "manual"),
            ProjectSource::AutoDetected => write!(f, "auto_detected"),
        }
    }
}

/// A registered work context (code project, writing project, etc).
///
/// Projects are the primary unit of workspace context in Oxios.
/// Sessions reference a primary project (for CWD) and optional secondary projects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Unique identifier.
    pub id: ProjectId,
    /// Human-readable name (unique, e.g. "oxios", "pi", "my-blog").
    pub name: String,
    /// Optional description for UI display.
    pub description: String,
    /// LEGACY RFC-025 migration read-source.
    ///
    /// Paths now live on Mounts (`mount_ids`); this field is retained solely
    /// so the one-time `migrate_projects_to_mounts` boot step can read
    /// pre-RFC-025 data. New code MUST NOT read it — resolve paths via
    /// `mount_ids`. Removed from the struct + DB in a follow-up release.
    pub paths: Vec<PathBuf>,
    /// Tags for keyword matching (detection layer 3).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Display emoji for UI.
    #[serde(default = "default_emoji")]
    pub emoji: String,
    /// How this project was registered.
    pub source: ProjectSource,
    /// Whether this project allows cross-project memory access.
    #[serde(default = "default_true")]
    pub memory_visible: bool,
    /// When this project was created.
    pub created_at: DateTime<Utc>,
    /// When this project was last modified.
    pub updated_at: DateTime<Utc>,
    /// When this project was last active (used in a session).
    pub last_active_at: DateTime<Utc>,

    // ── RFC-025: Project as instruction/memory bundle ──
    /// Mounts this Project references. Empty for non-code Projects or
    /// Projects created before RFC-025 migration.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mount_ids: Vec<crate::mount::MountId>,
    /// Custom system-prompt instructions. Injected into `## Workspace Context`
    /// when this Project is active.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub instructions: String,
    /// Default brain space inherited by NEW chats created in this Project
    /// when the user has not explicitly picked a brain. The Web layer applies
    /// it to new chats; the kernel only stores and serves the value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_brain_space: Option<String>,
}

fn default_emoji() -> String {
    "📦".to_string()
}

fn default_true() -> bool {
    true
}

impl Project {
    /// Create a new Project with the given name.
    pub fn new(name: impl Into<String>, source: ProjectSource) -> Self {
        let now = Utc::now();
        Self {
            id: ProjectId::new_v4(),
            name: name.into(),
            description: String::new(),
            paths: Vec::new(),
            tags: Vec::new(),
            emoji: default_emoji(),
            source,
            memory_visible: true,
            created_at: now,
            updated_at: now,
            last_active_at: now,
            mount_ids: Vec::new(),
            instructions: String::new(),
            default_brain_space: None,
        }
    }

    /// Create a Project from a filesystem path.
    ///
    /// Derives the name from the directory name.
    pub fn from_path(path: &Path) -> Self {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        let mut project = Self::new(&name, ProjectSource::AutoDetected);
        project.paths.push(path.to_path_buf());
        project
    }

    /// Record that this project was used in a session.
    pub fn touch(&mut self) {
        self.last_active_at = Utc::now();
        self.updated_at = Utc::now();
    }

    /// Add a filesystem path.
    pub fn add_path(&mut self, path: PathBuf) {
        if !self.paths.contains(&path) {
            self.paths.push(path.clone());
            self.updated_at = Utc::now();
        }
    }

    /// Remove a filesystem path.
    pub fn remove_path(&mut self, path: &PathBuf) -> bool {
        if let Some(pos) = self.paths.iter().position(|p| p == path) {
            self.paths.remove(pos);
            self.updated_at = Utc::now();
            true
        } else {
            false
        }
    }

    /// Add a tag for keyword matching.
    pub fn add_tag(&mut self, tag: impl Into<String>) {
        let tag = tag.into();
        if !self.tags.contains(&tag) {
            self.tags.push(tag);
            self.updated_at = Utc::now();
        }
    }

    /// Whether this project has any filesystem paths.
    pub fn has_paths(&self) -> bool {
        !self.paths.is_empty()
    }

    /// Get the primary path (CWD source).
    pub fn primary_path(&self) -> Option<&PathBuf> {
        self.paths.first()
    }

    /// Get the display tag (e.g. "[🔧 oxios]").
    pub fn tag(&self) -> String {
        format!("[{} {}]", self.emoji, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_new() {
        let p = Project::new("oxios", ProjectSource::Manual);
        assert_eq!(p.name, "oxios");
        assert_eq!(p.source, ProjectSource::Manual);
        assert!(p.paths.is_empty());
        assert_eq!(p.emoji, "📦");
    }

    #[test]
    fn test_project_from_path() {
        let path = PathBuf::from("/Volumes/MERCURY/PROJECTS/oxios");
        let p = Project::from_path(&path);
        assert_eq!(p.name, "oxios");
        assert_eq!(p.source, ProjectSource::AutoDetected);
        assert_eq!(p.paths, vec![path]);
    }

    #[test]
    fn test_project_add_path() {
        let mut p = Project::new("oxios", ProjectSource::Manual);
        assert!(!p.has_paths());

        p.add_path(PathBuf::from("/Volumes/MERCURY/PROJECTS/oxios"));
        assert!(p.has_paths());
        assert_eq!(
            p.primary_path(),
            Some(&PathBuf::from("/Volumes/MERCURY/PROJECTS/oxios"))
        );

        // Duplicate path should not be added
        p.add_path(PathBuf::from("/Volumes/MERCURY/PROJECTS/oxios"));
        assert_eq!(p.paths.len(), 1);
    }

    #[test]
    fn test_project_tag() {
        let mut p = Project::new("oxios", ProjectSource::Manual);
        p.emoji = "🔧".to_string();
        assert_eq!(p.tag(), "[🔧 oxios]");
    }
}
