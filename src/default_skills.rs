//! Bundled default-skills embedding.
//!
//! Default skills (e.g. `skill-creator`, `code-review`) ship under
//! `share/default-skills/`. In a dev checkout (`cargo run` from source) that
//! path is reachable via `CARGO_MANIFEST_DIR`. In an *installed* binary
//! (`cargo install`) `CARGO_MANIFEST_DIR` points at the build host and the
//! path does not exist, so default skills silently fail to load.
//!
//! This module embeds the whole `share/default-skills` tree into the binary at
//! compile time and, when the on-disk path is absent, extracts it into the
//! workspace at runtime. The workspace lives under the kernel workspace
//! (`~/.oxios/workspace/**`), which is in every agent's `allowed_paths`, so
//! bundled skill resources (`references/`, `scripts/`) stay readable by agents.

use include_dir::{Dir, DirEntry, include_dir};
use std::fs;
use std::path::{Path, PathBuf};

/// The default-skills tree, baked in at compile time.
static EMBEDDED_DEFAULT_SKILLS: Dir<'static> =
    include_dir!("$CARGO_MANIFEST_DIR/share/default-skills");

/// Resolve the `share` directory that holds `default-skills/`.
///
/// 1. **Dev checkout** — if `<CARGO_MANIFEST_DIR>/share/default-skills` exists
///    on disk, use `<CARGO_MANIFEST_DIR>/share` directly (no copy).
/// 2. **Installed binary** — otherwise extract the embedded tree to
///    `<workspace>/share/default-skills` and return `<workspace>/share`.
///
/// The extraction overwrites stale files so upgrades pick up new/changed
/// bundled skills. The tree is small, so re-extracting on each start is cheap.
pub fn resolve_share_dir(workspace: &Path) -> PathBuf {
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("share");
    if dev.join("default-skills").is_dir() {
        return dev;
    }

    let cache_root = workspace.join("share");
    let target = cache_root.join("default-skills");
    if let Err(e) = extract_dir(&EMBEDDED_DEFAULT_SKILLS, &target) {
        tracing::warn!(error = %e, target = %target.display(), "failed to extract embedded default skills");
    }
    cache_root
}

/// Recursively write a [`Dir`] tree to `target`, creating directories as
/// needed. Files overwrite existing entries.
fn extract_dir(dir: &Dir<'_>, target: &Path) -> std::io::Result<()> {
    for entry in dir.entries() {
        let path = target.join(entry.path());
        match entry {
            DirEntry::Dir(d) => {
                fs::create_dir_all(&path)?;
                extract_dir(d, target)?;
            }
            DirEntry::File(f) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&path, f.contents())?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_tree_contains_skill_creator() {
        // The compile-time-embedded tree must include the skill-creator skill.
        let mut found = false;
        for entry in EMBEDDED_DEFAULT_SKILLS.entries() {
            if entry.path().to_string_lossy() == "skill-creator" {
                found = true;
            }
        }
        assert!(found, "embedded default-skills must contain skill-creator/");
    }

    #[test]
    fn embedded_brain_skill_parses() {
        // The brain skill replaces the removed memory tools (oxibrain 0.8
        // daemonless cutover) — it must ship and parse under its name.
        let file = EMBEDDED_DEFAULT_SKILLS
            .get_file("brain/SKILL.md")
            .expect("embedded default-skills must contain brain/SKILL.md");
        let content = std::str::from_utf8(file.contents()).expect("utf-8 SKILL.md");
        let (parsed, body) = oxios_kernel::skill::frontmatter::parse_skill(content, file.path())
            .expect("brain SKILL.md must parse");
        assert_eq!(parsed.name, "brain");
        assert!(
            !parsed.description.is_empty(),
            "brain skill must carry a description"
        );
        assert!(body.contains("oxibrain"), "body must teach the CLI lane");
    }

    #[test]
    fn extract_dir_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("extracted");
        extract_dir(&EMBEDDED_DEFAULT_SKILLS, &out).unwrap();
        assert!(out.join("skill-creator/SKILL.md").exists());
        assert!(out.join("skill-creator/references/schemas.md").exists());
        assert!(out.join("skill-creator/agents/grader.md").exists());
    }

    #[test]
    fn resolve_prefers_dev_path_when_present() {
        // In the dev checkout the real share/default-skills exists, so the
        // resolver must return the CARGO_MANIFEST_DIR share path and NOT the
        // workspace fallback.
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("share");
        let resolved = resolve_share_dir(Path::new("/nonexistent/workspace"));
        assert_eq!(resolved, dev);
    }
}
