#![allow(missing_docs)]
//! Domain types for the skill system.

use super::format::SkillFormat;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Requirements {
    #[serde(default)]
    pub bins: Vec<String>,
    #[serde(default, rename = "anyBins")]
    pub any_bins: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub config: Vec<String>,
    /// Integration IDs this skill hard-requires (RFC-041 D10). The skill is
    /// ineligible unless every listed integration's credential is satisfied.
    /// Checked where the registry is available (API/UI), not in the
    /// host-local `check_requirements`.
    #[serde(default)]
    pub integrations: Vec<String>,
    /// Integration IDs of which at least one must be satisfied (soft, mirrors
    /// `any_bins`). Empty = no constraint.
    #[serde(default, rename = "anyIntegrations")]
    pub any_integrations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInstallSpec {
    pub kind: InstallKind,
    #[serde(default)]
    pub formula: Option<String>,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub archive: Option<String>,
    #[serde(default)]
    pub extract: Option<bool>,
    #[serde(default, rename = "stripComponents")]
    pub strip_components: Option<u32>,
    #[serde(default, rename = "targetDir")]
    pub target_dir: Option<String>,
    #[serde(default)]
    pub os: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallKind {
    Brew,
    Node,
    Bun,
    Cargo,
    Pip,
    Go,
    #[serde(rename = "uv")]
    Uv,
    Download,
}

impl std::fmt::Display for InstallKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallKind::Brew => write!(f, "brew"),
            InstallKind::Node => write!(f, "node"),
            InstallKind::Bun => write!(f, "bun"),
            InstallKind::Cargo => write!(f, "cargo"),
            InstallKind::Pip => write!(f, "pip"),
            InstallKind::Go => write!(f, "go"),
            InstallKind::Uv => write!(f, "uv"),
            InstallKind::Download => write!(f, "download"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RequirementsCheck {
    pub missing_bins: Vec<String>,
    pub missing_any_bins: Vec<String>,
    pub missing_env: Vec<String>,
    pub missing_config: Vec<String>,
    pub missing_os: Vec<String>,
    pub eligible: bool,
    pub config_checks: Vec<ConfigCheck>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigCheck {
    pub path: String,
    pub satisfied: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillStatus {
    Ready,
    NeedsSetup,
    Disabled,
}

impl std::fmt::Display for SkillStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillStatus::Ready => write!(f, "ready"),
            SkillStatus::NeedsSetup => write!(f, "needs_setup"),
            SkillStatus::Disabled => write!(f, "disabled"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    Bundled,
    Managed,
    Workspace,
    /// Shared immutable package from the Foundation registry (RFC-048 §4).
    /// Content is read-only; enable/disable and edits require a local copy.
    Foundation,
}

/// Provenance of a skill loaded from a Foundation shared package
/// (RFC-048 §4). Recorded in the runtime snapshot so audits can pin the
/// exact package version/digest that contributed to an agent's prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundationPackageInfo {
    /// Lockfile entry id (e.g. `oxi.brain-helper`).
    pub id: String,
    pub version: String,
    /// blake3 hex digest of the verified package archive.
    pub digest: String,
    /// Persona hint from the lockfile — when set, the package content is
    /// only offered to agents running that persona.
    pub persona: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInvocationPolicy {
    #[serde(default = "default_true")]
    pub user_invocable: bool,
    #[serde(default)]
    pub disable_model_invocation: bool,
}
impl Default for SkillInvocationPolicy {
    fn default() -> Self {
        Self {
            user_invocable: true,
            disable_model_invocation: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillMetadata {
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub requires: Requirements,
    #[serde(default)]
    pub os: Vec<String>,
    #[serde(default)]
    pub install: Vec<SkillInstallSpec>,
    #[serde(default)]
    pub always: bool,
    /// RFC-031 §6.3: this skill may fire unattended during a token-maxing
    /// window. Defaults false — frequency is never the gate, only this flag.
    #[serde(default)]
    pub autonomous: bool,
    #[serde(default, rename = "primaryEnv")]
    pub primary_env: Option<String>,
    #[serde(default, rename = "skillKey")]
    pub skill_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub config: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillState {
    pub enabled: bool,
    pub installed_at: String,
    pub last_modified: String,
}
impl Default for SkillState {
    fn default() -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            enabled: true,
            installed_at: now.clone(),
            last_modified: now,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
    pub path: PathBuf,
    pub base_dir: PathBuf,
    pub file_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
}
impl From<&Skill> for SkillMeta {
    fn from(s: &Skill) -> Self {
        SkillMeta {
            name: s.name.clone(),
            description: s.description.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub skill: Skill,
    pub metadata: Option<SkillMetadata>,
    pub eligibility: RequirementsCheck,
    pub status: SkillStatus,
    pub bundled: bool,
    pub source: SkillSource,
    /// Set when this skill came from a Foundation shared package.
    pub foundation: Option<FoundationPackageInfo>,
    pub invocation: SkillInvocationPolicy,
    pub format: SkillFormat,
    pub raw_yaml: serde_yaml::Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRef {
    pub name: String,
    pub description: String,
    pub file_path: String,
    pub primary_env: Option<String>,
    pub required_env: Vec<String>,
    /// Integration IDs this skill hard-requires (RFC-041). The UI cross-
    /// references these against `/api/integrations` credential status.
    pub required_integrations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSnapshot {
    pub prompt: String,
    pub skills: Vec<SkillRef>,
    pub skill_filter: Option<Vec<String>>,
    /// Foundation packages whose content is included in this snapshot.
    /// Each entry carries the verified digest (RFC-048 §4 audit trail).
    #[serde(default)]
    pub foundation_packages: Vec<FoundationPackageInfo>,
}
pub(crate) fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_install_kind_display() {
        assert_eq!(InstallKind::Brew.to_string(), "brew");
        assert_eq!(InstallKind::Node.to_string(), "node");
        assert_eq!(InstallKind::Go.to_string(), "go");
        assert_eq!(InstallKind::Uv.to_string(), "uv");
        assert_eq!(InstallKind::Download.to_string(), "download");
    }

    #[test]
    fn test_install_kind_serialization() {
        for (kind, expected) in [
            (InstallKind::Brew, "\"brew\""),
            (InstallKind::Node, "\"node\""),
            (InstallKind::Go, "\"go\""),
            (InstallKind::Uv, "\"uv\""),
            (InstallKind::Download, "\"download\""),
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, expected);
            let restored: InstallKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, restored);
        }
    }

    #[test]
    fn test_skill_status_display() {
        assert_eq!(SkillStatus::Ready.to_string(), "ready");
        assert_eq!(SkillStatus::NeedsSetup.to_string(), "needs_setup");
        assert_eq!(SkillStatus::Disabled.to_string(), "disabled");
    }

    #[test]
    fn test_skill_status_serialization() {
        for status in [
            SkillStatus::Ready,
            SkillStatus::NeedsSetup,
            SkillStatus::Disabled,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let restored: SkillStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, restored);
        }
    }

    #[test]
    fn test_requirements_default() {
        let req = Requirements::default();
        assert!(req.bins.is_empty());
        assert!(req.any_bins.is_empty());
        assert!(req.env.is_empty());
        assert!(req.config.is_empty());
    }

    #[test]
    fn test_requirements_serialization() {
        let req = Requirements {
            bins: vec!["cargo".to_string(), "node".to_string()],
            any_bins: vec!["python3".to_string()],
            env: vec!["API_KEY".to_string()],
            config: vec!["server.host".to_string()],
            integrations: vec!["github".to_string()],
            any_integrations: vec![],
        };
        let json = serde_json::to_string(&req).unwrap();
        let restored: Requirements = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.bins, req.bins);
        assert_eq!(restored.any_bins, req.any_bins);
        assert_eq!(restored.env, req.env);
        assert_eq!(restored.config, req.config);
    }

    #[test]
    fn test_skill_install_spec_minimal() {
        let spec = SkillInstallSpec {
            kind: InstallKind::Brew,
            formula: Some("git".to_string()),
            package: None,
            module: None,
            url: None,
            archive: None,
            extract: None,
            strip_components: None,
            target_dir: None,
            os: vec![],
        };
        let json = serde_json::to_string(&spec).unwrap();
        let restored: SkillInstallSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.kind, InstallKind::Brew);
        assert_eq!(restored.formula.as_deref(), Some("git"));
    }

    #[test]
    fn test_requirements_check_default() {
        let check = RequirementsCheck::default();
        assert!(check.missing_bins.is_empty());
        assert!(check.missing_any_bins.is_empty());
        assert!(check.missing_env.is_empty());
        assert!(check.missing_config.is_empty());
        assert!(check.missing_os.is_empty());
        // Default eligible is false (no derive default_true for bool)
        assert!(!check.eligible);
        assert!(check.config_checks.is_empty());
    }

    #[test]
    fn test_requirements_check_ineligible() {
        let check = RequirementsCheck {
            missing_bins: vec!["nonexistent".to_string()],
            missing_any_bins: vec![],
            missing_env: vec!["SECRET_KEY".to_string()],
            missing_config: vec![],
            missing_os: vec![],
            eligible: false,
            config_checks: vec![],
        };
        assert!(!check.eligible);
        assert_eq!(check.missing_bins.len(), 1);
        assert_eq!(check.missing_env.len(), 1);
    }

    #[test]
    fn test_skill_invocation_policy_default() {
        let policy = SkillInvocationPolicy::default();
        assert!(policy.user_invocable);
        assert!(!policy.disable_model_invocation);
    }

    #[test]
    fn test_skill_config_default() {
        let config = SkillConfig::default();
        // Default derived: enabled is false (serde default_true only applies on deserialization)
        assert!(!config.enabled);
        assert!(config.env.is_empty());
        assert!(config.config.is_empty());
    }

    #[test]
    fn test_skill_config_deserialization_default_enabled() {
        // When deserializing from empty JSON, default_true should kick in
        let json = "{}";
        let config: SkillConfig = serde_json::from_str(json).unwrap();
        assert!(config.enabled);
        assert!(config.env.is_empty());
    }

    #[test]
    fn test_skill_state_default() {
        let state = SkillState::default();
        assert!(state.enabled);
        assert!(!state.installed_at.is_empty());
        assert!(!state.last_modified.is_empty());
    }

    #[test]
    fn test_skill_metadata_default() {
        let meta = SkillMetadata::default();
        assert!(meta.author.is_none());
        assert!(meta.version.is_none());
        assert!(meta.emoji.is_none());
        assert!(meta.homepage.is_none());
        assert!(meta.install.is_empty());
        assert!(!meta.always);
        assert!(meta.primary_env.is_none());
    }

    #[test]
    fn test_skill_meta_from_skill() {
        let skill = Skill {
            name: "test".to_string(),
            description: "desc".to_string(),
            content: "body".to_string(),
            path: PathBuf::from("/tmp"),
            base_dir: PathBuf::from("/tmp"),
            file_path: PathBuf::from("/tmp/SKILL.md"),
        };
        let meta = SkillMeta::from(&skill);
        assert_eq!(meta.name, "test");
        assert_eq!(meta.description, "desc");
    }

    #[test]
    fn test_skill_snapshot_serialization() {
        let snap = SkillSnapshot {
            prompt: "You are helpful".to_string(),
            skills: vec![SkillRef {
                name: "bash".to_string(),
                description: "shell".to_string(),
                file_path: "/skills/bash.md".to_string(),
                primary_env: None,
                required_env: vec![],
                required_integrations: vec![],
            }],
            skill_filter: Some(vec!["bash".to_string()]),
            foundation_packages: vec![],
        };
        let json = serde_json::to_string(&snap).unwrap();
        let restored: SkillSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.prompt, "You are helpful");
        assert_eq!(restored.skills.len(), 1);
        assert_eq!(restored.skill_filter.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_config_check() {
        let check = ConfigCheck {
            path: "server.port".to_string(),
            satisfied: true,
        };
        assert_eq!(check.path, "server.port");
        assert!(check.satisfied);
    }
}
