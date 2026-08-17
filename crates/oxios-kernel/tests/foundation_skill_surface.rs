//! Foundation packages surfacing as skills through `SkillManager`
//! (RFC-048 §4): precedence, digest provenance, persona selectivity,
//! and read-only enforcement.
#![allow(clippy::unwrap_used)] // `.unwrap()` in tests is idiomatic (workspace convention)

use std::io::Write;
use std::path::{Path, PathBuf};

use oxios_kernel::foundation::packages::{
    AbstractRequirement, LockEntry, PackageLock, SourceTrust,
};
use oxios_kernel::skill::{SkillManager, SkillSource};

fn zip_bytes(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
    for (name, data) in entries {
        writer.start_file(*name, options).expect("start file");
        writer.write_all(data.as_bytes()).expect("write entry");
    }
    writer.finish().expect("finish zip").into_inner()
}

fn skill_md(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n\n{name} body.\n")
}

fn digest_of(bytes: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(bytes);
    hasher.finalize().to_hex().to_string()
}

struct Fixture {
    /// Keeps the temp home alive for the test body.
    _home_dir: tempfile::TempDir,
    home: PathBuf,
    skills_dir: PathBuf,
    bundled_dir: PathBuf,
}

/// Create a temp home with `skills/` (user), `share/skills` (bundled), and
/// a Foundation tree holding one `<id>.zip` + `packages.lock`.
fn fixture(entries: Vec<(LockEntry, Vec<u8>)>) -> Fixture {
    let home_dir = tempfile::tempdir().expect("temp home");
    let home = home_dir.path().to_path_buf();
    let packages_dir = oxios_kernel::foundation::packages_dir(&home);
    std::fs::create_dir_all(&packages_dir).expect("create packages dir");
    for (entry, archive) in &entries {
        std::fs::write(packages_dir.join(format!("{}.zip", entry.id)), archive)
            .expect("write archive");
    }
    let lock = PackageLock {
        schema_version: 1,
        entries: entries.into_iter().map(|(e, _)| e).collect(),
    };
    std::fs::write(
        oxios_kernel::foundation::versioned_root(&home).join("packages.lock"),
        serde_json::to_string(&lock).unwrap(),
    )
    .expect("write lock");

    let skills_dir = home.join("skills");
    let bundled_dir = home.join("share/skills");
    std::fs::create_dir_all(&skills_dir).expect("skills dir");
    std::fs::create_dir_all(&bundled_dir).expect("bundled dir");
    Fixture {
        _home_dir: home_dir,
        home,
        skills_dir,
        bundled_dir,
    }
}

fn entry(
    id: &str,
    skill_name: &str,
    requires: Vec<AbstractRequirement>,
    persona: Option<&str>,
) -> (LockEntry, Vec<u8>) {
    let archive = zip_bytes(&[("SKILL.md", &skill_md(skill_name, "shared package skill"))]);
    (
        LockEntry {
            id: id.into(),
            version: "0.1.0".into(),
            source: format!("local://./{id}"),
            trust: SourceTrust::Unsigned,
            digest: digest_of(&archive),
            targets: vec!["oxios".into()],
            requires,
            persona: persona.map(String::from),
        },
        archive,
    )
}

fn write_skill(dir: &Path, name: &str) {
    let dir = dir.join(name);
    std::fs::create_dir_all(&dir).expect("skill dir");
    std::fs::write(dir.join("SKILL.md"), skill_md(name, "local skill")).expect("write SKILL.md");
}

#[tokio::test]
async fn loads_foundation_package_as_skill_with_provenance() {
    let f = fixture(vec![entry(
        "oxi.brain-helper",
        "brain-helper",
        vec![AbstractRequirement::BrainQuery],
        None,
    )]);
    let manager =
        SkillManager::new(f.skills_dir.clone(), f.bundled_dir.clone()).with_foundation(&f.home);
    manager.init().await.expect("init");

    let loaded = manager.get_skill("brain-helper").await.expect("loaded");
    assert_eq!(loaded.source, SkillSource::Foundation);
    assert!(loaded.skill.content.contains("brain-helper body"));
    let provenance = loaded.foundation.expect("provenance");
    assert_eq!(provenance.id, "oxi.brain-helper");
    assert_eq!(provenance.version, "0.1.0");
    assert_eq!(provenance.digest.len(), 64);
}

#[tokio::test]
async fn precedence_bundled_lt_foundation_lt_user() {
    // "shared" exists in all three layers; "foundation-only" in two.
    let f = fixture(vec![
        entry("oxi.shared", "shared", vec![], None),
        entry("oxi.only", "foundation-only", vec![], None),
    ]);
    write_skill(&f.bundled_dir, "shared");
    write_skill(&f.bundled_dir, "foundation-only");
    write_skill(&f.skills_dir, "shared");

    let manager =
        SkillManager::new(f.skills_dir.clone(), f.bundled_dir.clone()).with_foundation(&f.home);
    manager.init().await.expect("init");

    // User overrides Foundation.
    let shared = manager.get_skill("shared").await.expect("shared");
    assert_eq!(shared.source, SkillSource::Managed);
    assert!(shared.foundation.is_none());
    // Foundation overrides bundled for a name the user did not take.
    let only = manager.get_skill("foundation-only").await.expect("only");
    assert_eq!(only.source, SkillSource::Foundation);
}

#[tokio::test]
async fn corrupt_digest_package_is_skipped_individually() {
    // Healthy entry, but with a tampered digest in the lock.
    let (mut tampered, archive) = entry("oxi.good", "good-skill", vec![], None);
    tampered.digest = digest_of(b"tampered bytes");
    // Entry whose archive is absent from the packages dir.
    let (mut orphan, _) = entry("oxi.orphan", "orphan-skill", vec![], None);
    orphan.digest = "0".repeat(64);
    let f = fixture(vec![(tampered, archive), (orphan.clone(), Vec::new())]);
    std::fs::remove_file(
        oxios_kernel::foundation::packages_dir(&f.home).join(format!("{}.zip", orphan.id)),
    )
    .expect("remove orphan archive");

    let manager =
        SkillManager::new(f.skills_dir.clone(), f.bundled_dir.clone()).with_foundation(&f.home);
    manager.init().await.expect("init");

    assert!(
        manager.get_skill("good-skill").await.is_none(),
        "digest mismatch must be rejected"
    );
    assert!(
        manager.get_skill("orphan-skill").await.is_none(),
        "missing archive must be skipped"
    );
}

#[tokio::test]
async fn persona_scoped_packages_are_selective() {
    let f = fixture(vec![
        entry("oxi.general", "general-skill", vec![], None),
        entry(
            "oxi.research-kit",
            "research-skill",
            vec![],
            Some("research"),
        ),
    ]);
    let manager =
        SkillManager::new(f.skills_dir.clone(), f.bundled_dir.clone()).with_foundation(&f.home);
    manager.init().await.expect("init");

    // Default snapshot: persona-less packages only.
    let plain = manager.build_snapshot(None, None).await;
    assert!(plain.prompt.contains("general-skill"));
    assert!(!plain.prompt.contains("research-skill"));
    assert_eq!(plain.foundation_packages.len(), 1);
    assert_eq!(plain.foundation_packages[0].id, "oxi.general");

    // Matching persona unlocks the scoped package and records its digest.
    let scoped = manager
        .build_snapshot_for(None, None, Some("research"))
        .await;
    assert!(scoped.prompt.contains("research-skill"));
    assert_eq!(scoped.foundation_packages.len(), 2);
    let kit = scoped
        .foundation_packages
        .iter()
        .find(|p| p.id == "oxi.research-kit")
        .expect("kit recorded");
    assert_eq!(kit.digest.len(), 64);

    // Non-matching persona still excludes it.
    let dev = manager.build_snapshot_for(None, None, Some("dev")).await;
    assert!(!dev.prompt.contains("research-skill"));
}

#[tokio::test]
async fn set_enabled_rejects_readonly_foundation_skill() {
    let f = fixture(vec![entry("oxi.locked", "locked-skill", vec![], None)]);
    let manager =
        SkillManager::new(f.skills_dir.clone(), f.bundled_dir.clone()).with_foundation(&f.home);
    manager.init().await.expect("init");

    let err = manager
        .set_enabled("locked-skill", false)
        .await
        .expect_err("foundation skills are read-only");
    assert!(err.to_string().contains("read-only Foundation package"));
}

#[tokio::test]
async fn missing_lock_is_not_an_error() {
    let f = fixture(vec![]);
    std::fs::remove_file(oxios_kernel::foundation::versioned_root(&f.home).join("packages.lock"))
        .expect("remove lock");
    let manager =
        SkillManager::new(f.skills_dir.clone(), f.bundled_dir.clone()).with_foundation(&f.home);
    manager.init().await.expect("init without lock");
    assert!(manager.list_skills().await.is_empty());
}
