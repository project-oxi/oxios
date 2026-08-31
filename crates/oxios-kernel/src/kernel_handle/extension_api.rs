//! Extension API — skills (RFC-009 Final).
//!
//! `ExtensionApi` is the unified facade for the skill system.
//! The legacy ProgramManager and HostToolValidator have been removed.

use crate::skill::{RequirementsCheck, Skill, SkillEntry, SkillManager, SkillMeta};
use std::sync::Arc;

/// Extension system calls.
pub struct ExtensionApi {
    /// Unified skill manager.
    pub(crate) skill_manager: Arc<SkillManager>,
}

impl ExtensionApi {
    /// Create a new ExtensionApi with only a SkillManager.
    pub fn new(skill_manager: Arc<SkillManager>) -> Self {
        Self { skill_manager }
    }

    // ── Skill methods ───────────────────────────────────────────────

    /// List installed skill entries.
    pub async fn list_skills_entries(&self) -> Vec<SkillEntry> {
        self.skill_manager.list_skills().await
    }

    /// Get skill details.
    pub async fn get_skill_entry(&self, name: &str) -> Option<SkillEntry> {
        self.skill_manager.get_skill(name).await
    }

    /// Read an installed skill's full SKILL.md content (frontmatter included).
    /// Used by the agent runtime to inline `/skill <name>` turn invocations.
    pub async fn get_skill_content(&self, name: &str) -> Option<String> {
        self.skill_manager.get_skill_content(name).await
    }

    /// Enable a skill.
    pub async fn enable_skill(&self, name: &str) -> anyhow::Result<()> {
        self.skill_manager.set_enabled(name, true).await
    }

    /// Disable a skill.
    pub async fn disable_skill(&self, name: &str) -> anyhow::Result<()> {
        self.skill_manager.set_enabled(name, false).await
    }

    /// Check requirements for a skill.
    pub async fn check_skill_requirements(&self, name: &str) -> Option<RequirementsCheck> {
        self.skill_manager
            .get_skill(name)
            .await
            .map(|e| e.eligibility)
    }

    /// List all skills (metadata only).
    pub async fn list_skills(&self) -> anyhow::Result<Vec<SkillMeta>> {
        Ok(self.skill_manager.list_skills_meta().await)
    }

    /// Load skill by name.
    pub async fn load_skill(&self, name: &str) -> anyhow::Result<Option<Skill>> {
        self.skill_manager.load_skill(name).await
    }

    /// Create a new skill.
    pub async fn create_skill(
        &self,
        name: &str,
        description: &str,
        content: &str,
    ) -> anyhow::Result<()> {
        self.skill_manager
            .create_skill(name, description, content)
            .await
    }

    /// Delete a skill.
    pub async fn delete_skill(&self, name: &str) -> anyhow::Result<()> {
        self.skill_manager.delete_skill(name).await
    }

    /// Write a skill's raw `SKILL.md` verbatim (frontmatter preserved) and
    /// reindex. For inline edits and `.md` text imports — never strips
    /// rich frontmatter the way [`create_skill`] does.
    pub async fn write_skill_raw(&self, name: &str, raw: &str) -> anyhow::Result<SkillEntry> {
        self.skill_manager.write_skill_raw(name, raw).await
    }

    /// Import a skill from `.zip` / `.skill` archive bytes. Extracts, derives
    /// the name from frontmatter, moves into `skills_dir`, records provenance.
    pub async fn import_skill_zip(
        &self,
        name_hint: &str,
        bytes: &[u8],
    ) -> anyhow::Result<SkillEntry> {
        self.skill_manager.import_skill_zip(name_hint, bytes).await
    }

    /// Import a skill from raw `SKILL.md` text (frontmatter preserved),
    /// deriving the name from the frontmatter. For text-paste / URL imports.
    pub async fn import_skill_text(
        &self,
        content: &str,
        name_hint: Option<&str>,
    ) -> anyhow::Result<SkillEntry> {
        self.skill_manager
            .import_skill_text(content, name_hint)
            .await
    }

    /// Skill manager reference.
    pub fn skill_manager(&self) -> &Arc<SkillManager> {
        &self.skill_manager
    }
}
