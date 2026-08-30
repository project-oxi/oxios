//! Project module: project-owned filesystem roots.
//!
//! A Project is the persisted unit of workspace context: a unique name, an
//! ordered list of canonical filesystem roots, and project-specific
//! instructions. Sessions reference a project by id; the orchestrator
//! resolves the whole workspace from the project alone.
//!
//! ## Structure
//!
//! - `mod.rs` — `Project` record (this file)
//! - `manager.rs` — `ProjectManager` (CRUD, root validation, lookup)
//! - `project_db.rs` — SQLite persistence
//! - `conversation_buffer.rs` — topic-shift detection buffer

pub mod conversation_buffer;
pub mod manager;
pub mod milestone;
pub mod paths;
pub mod project_db;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

// ── Re-exports ──────────────────────────────────────────────
pub use conversation_buffer::{ConversationBuffer, ConversationTurn};
pub use manager::{ProjectManager, ProjectManagerError, validate_root_paths};
pub use milestone::{Milestone, MilestonePatch, MilestoneProgress, MilestoneStatus};
pub use paths::{project_dir, project_issues_dir, project_milestones_path};

/// Unique identifier for a Project.
pub type ProjectId = Uuid;

/// A registered work context (code project, writing project, etc).
///
/// The persisted record carries only fields with durable product meaning;
/// presentation (icon, emoji, tags) is derived by the UI when needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Unique identifier.
    pub id: ProjectId,
    /// Human-readable name (unique).
    pub name: String,
    /// Canonical, de-duplicated, existing directories. May be empty.
    ///
    /// Order is intentional: `root_paths[0]` is the working directory; all
    /// roots are available to filesystem tools and shown in the workspace
    /// context.
    pub root_paths: Vec<PathBuf>,
    /// Project-specific instructions injected only while this project is
    /// active.
    pub instructions: String,
    /// Default brain space inherited by NEW chats created in this Project
    /// when the user has not explicitly picked a brain. The Web layer applies
    /// it to new chats; the kernel only stores and serves the value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_brain_space: Option<String>,
    /// When this project was created.
    pub created_at: DateTime<Utc>,
    /// When this project was last modified.
    pub updated_at: DateTime<Utc>,
    /// When this project was last active (used in a session).
    pub last_active_at: DateTime<Utc>,
}

impl Project {
    /// Create a new Project. All three timestamps are stamped `Utc::now()`.
    pub fn new(
        name: impl Into<String>,
        root_paths: Vec<PathBuf>,
        instructions: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: ProjectId::new_v4(),
            name: name.into(),
            root_paths,
            instructions: instructions.into(),
            default_brain_space: None,
            created_at: now,
            updated_at: now,
            last_active_at: now,
        }
    }

    /// Record that this project was used in a session.
    pub fn touch(&mut self) {
        self.last_active_at = Utc::now();
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_new_stamps_all_timestamps() {
        let before = Utc::now();
        let p = Project::new("oxios", vec![PathBuf::from("/tmp")], "be careful");
        assert_eq!(p.name, "oxios");
        assert_eq!(p.root_paths, vec![PathBuf::from("/tmp")]);
        assert_eq!(p.instructions, "be careful");
        assert!(p.default_brain_space.is_none());
        assert!(p.created_at >= before);
        assert_eq!(p.created_at, p.updated_at);
        assert_eq!(p.created_at, p.last_active_at);
    }

    #[test]
    fn test_project_touch_bumps_activity() {
        let mut p = Project::new("oxios", vec![], "");
        std::thread::sleep(std::time::Duration::from_millis(5));
        p.touch();
        assert!(p.last_active_at > p.created_at);
        assert!(p.updated_at > p.created_at);
    }
}
