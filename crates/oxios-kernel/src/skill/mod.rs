//! Skill system: multi-format SKILL.md parsing with format detection,
//! plus multi-registry marketplace sources (ClawHub, Skills.sh).

pub mod archive;
pub mod clawhub;
pub mod format;
pub mod frontmatter;
pub mod manager;
pub mod prompt;
pub mod requirements;
pub mod skills_sh;
pub mod types;

pub use format::SkillFormat;
pub use manager::SkillManager;
pub use prompt::{compact_path, escape_xml};
pub use requirements::check_requirements;
pub use types::{
    ConfigCheck, FoundationPackageInfo, InstallKind, Requirements, RequirementsCheck, Skill,
    SkillConfig, SkillEntry, SkillInstallSpec, SkillInvocationPolicy, SkillMeta, SkillMetadata,
    SkillRef, SkillSnapshot, SkillSource, SkillState, SkillStatus,
};

/// Returns true if `rel` is a safe relative path: no parent/root/prefix
/// components that could escape a target directory (Zip Slip / path traversal).
///
/// Used by skill installers (ClawHub zip extraction, skills.sh file writes)
/// to reject archive entries whose names contain `..`, absolute paths, or
/// Windows drive prefixes.
pub(crate) fn is_safe_relative_path(rel: &str) -> bool {
    let p = std::path::Path::new(rel);
    !p.components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    })
}
