//! Marketplace API — ClawHub + Skills.sh integration facade.
//!
//! Exposes search, install, and update for skills from multiple registries:
//! - **ClawHub** (`clawhub.ai`) — zip-based skill distribution
//! - **Skills.sh** (`skills.sh`) — JSON API skill distribution (Vercel Labs ecosystem)

use std::sync::Arc;

use crate::skill::clawhub::{
    ClawHubClient, ClawHubInstaller, ClawHubSearchResult, ClawHubSkillDetail, InstallResult,
    UpdateAvailable, UpdateResult,
};
use crate::skill::skills_sh::{
    SkillsShClient, SkillsShInstallResult, SkillsShInstaller, SkillsShSearchResponse,
    SkillsShSkillDetail,
};

/// Marketplace system calls — multi-registry skill management.
#[derive(Clone)]
pub struct MarketplaceApi {
    // ClawHub
    clawhub_installer: Arc<ClawHubInstaller>,
    clawhub_client: Arc<ClawHubClient>,
    // Skills.sh
    skills_sh_installer: Arc<SkillsShInstaller>,
    skills_sh_client: Arc<SkillsShClient>,
}

impl MarketplaceApi {
    /// Create a new MarketplaceApi with both registries.
    pub fn new(
        clawhub_installer: Arc<ClawHubInstaller>,
        clawhub_client: Arc<ClawHubClient>,
        skills_sh_installer: Arc<SkillsShInstaller>,
        skills_sh_client: Arc<SkillsShClient>,
    ) -> Self {
        Self {
            clawhub_installer,
            clawhub_client,
            skills_sh_installer,
            skills_sh_client,
        }
    }

    // ─── ClawHub Operations ─────────────────────────────────────────

    /// Search ClawHub for skills by query.
    pub async fn search_clawhub(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<ClawHubSearchResult>> {
        self.clawhub_client.search_skills(query, limit).await
    }

    /// Get full detail for a ClawHub skill by slug.
    pub async fn get_clawhub_skill(&self, slug: &str) -> anyhow::Result<ClawHubSkillDetail> {
        self.clawhub_client.get_skill(slug).await
    }

    /// Install a skill from ClawHub.
    pub async fn install_clawhub(
        &self,
        slug: &str,
        version: Option<&str>,
    ) -> anyhow::Result<InstallResult> {
        self.clawhub_installer.install(slug, version).await
    }

    /// Check which installed ClawHub skills have newer versions.
    pub async fn check_clawhub_updates(&self) -> anyhow::Result<Vec<UpdateAvailable>> {
        self.clawhub_installer.check_updates().await
    }

    /// Update a specific installed ClawHub skill.
    pub async fn update_clawhub(&self, slug: &str) -> anyhow::Result<UpdateResult> {
        self.clawhub_installer.update(slug).await
    }

    /// Update all installed ClawHub skills.
    pub async fn update_clawhub_all(&self) -> anyhow::Result<Vec<UpdateResult>> {
        self.clawhub_installer.update_all().await
    }

    // ─── Skills.sh Operations ───────────────────────────────────────

    /// Whether the skills.sh client has an API key configured.
    ///
    /// The skills.sh upstream requires a Bearer token on every endpoint.
    /// When no key is present, callers should fail fast with a clear
    /// "credentials missing" error instead of a guaranteed upstream 401.
    pub fn skills_sh_configured(&self) -> bool {
        self.skills_sh_client.has_api_key()
    }

    /// Search skills.sh for skills.
    pub async fn search_skills_sh(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<SkillsShSearchResponse> {
        self.skills_sh_client.search(query, limit).await
    }

    /// List skills from the skills.sh leaderboard.
    pub async fn list_skills_sh(
        &self,
        view: Option<&str>,
        page: Option<i64>,
        per_page: Option<i64>,
    ) -> anyhow::Result<crate::skill::skills_sh::SkillsShListResponse> {
        self.skills_sh_client.list(view, page, per_page).await
    }

    /// Get detailed info for a skills.sh skill (including file contents).
    pub async fn get_skills_sh_skill(&self, id: &str) -> anyhow::Result<SkillsShSkillDetail> {
        self.skills_sh_client.get_skill(id).await
    }

    /// Install a skill from skills.sh by its full id.
    pub async fn install_skills_sh(&self, skill_id: &str) -> anyhow::Result<SkillsShInstallResult> {
        self.skills_sh_installer.install(skill_id).await
    }

    /// Update a skill from skills.sh.
    pub async fn update_skills_sh(&self, skill_id: &str) -> anyhow::Result<SkillsShInstallResult> {
        self.skills_sh_installer.update(skill_id).await
    }

    /// Get security audit results for a skills.sh skill.
    pub async fn audit_skills_sh(
        &self,
        id: &str,
    ) -> anyhow::Result<crate::skill::skills_sh::SkillsShAuditResponse> {
        self.skills_sh_client.audit(id).await
    }

    // ─── Legacy Compatibility ───────────────────────────────────────
    // These methods default to ClawHub for backward compatibility.

    /// Search for skills (defaults to ClawHub).
    pub async fn search(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<ClawHubSearchResult>> {
        self.search_clawhub(query, limit).await
    }

    /// Get skill detail (defaults to ClawHub).
    pub async fn get_skill(&self, slug: &str) -> anyhow::Result<ClawHubSkillDetail> {
        self.get_clawhub_skill(slug).await
    }

    /// Install a skill (defaults to ClawHub).
    pub async fn install(
        &self,
        slug: &str,
        version: Option<&str>,
    ) -> anyhow::Result<InstallResult> {
        self.install_clawhub(slug, version).await
    }

    /// Check for updates (defaults to ClawHub).
    pub async fn check_updates(&self) -> anyhow::Result<Vec<UpdateAvailable>> {
        self.check_clawhub_updates().await
    }

    /// Update a skill (defaults to ClawHub).
    pub async fn update(&self, slug: &str) -> anyhow::Result<UpdateResult> {
        self.update_clawhub(slug).await
    }

    /// Update all skills (defaults to ClawHub).
    pub async fn update_all(&self) -> anyhow::Result<Vec<UpdateResult>> {
        self.update_clawhub_all().await
    }
}
