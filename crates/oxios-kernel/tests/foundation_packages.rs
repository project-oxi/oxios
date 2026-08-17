//! Integration tests for the Foundation shared package importer (RFC-048 §4).

use std::collections::BTreeMap;

use oxios_kernel::foundation::packages::{
    AbstractRequirement, LockEntry, PackageLock, SourceTrust,
};

fn minimal_entry(id: &str, requires: Vec<AbstractRequirement>, digest: &str) -> LockEntry {
    LockEntry {
        id: id.into(),
        version: "0.1.0".into(),
        source: format!("local://./{id}"),
        trust: SourceTrust::Unsigned,
        digest: digest.into(),
        targets: vec!["oxios".into()],
        requires,
        persona: None,
    }
}

#[test]
fn imports_minimal_lock() {
    let archive = b"package payload";
    let mut hasher = blake3::Hasher::new();
    hasher.update(archive);
    let digest = hasher.finalize().to_hex().to_string();

    let lock = PackageLock {
        schema_version: 1,
        entries: vec![minimal_entry(
            "oxi.brain-helper",
            vec![AbstractRequirement::BrainQuery],
            &digest,
        )],
    };

    let mut archives = BTreeMap::new();
    archives.insert("oxi.brain-helper".to_string(), archive.to_vec());
    let imported = lock.import(&archives).expect("imports");
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].capabilities.len(), 1);
}

#[test]
fn rejects_lock_without_oxios_target() {
    let lock = PackageLock {
        schema_version: 1,
        entries: vec![LockEntry {
            id: "oxi.x".into(),
            version: "0.0.1".into(),
            source: "local://x".into(),
            trust: SourceTrust::Unsigned,
            digest: "0".repeat(64),
            targets: vec!["memo".into()],
            requires: vec![],
            persona: None,
        }],
    };
    let json = serde_json::to_string(&lock).unwrap();
    let err = PackageLock::parse(&json).unwrap_err();
    assert!(matches!(
        err,
        oxios_kernel::foundation::packages::PackageError::TargetMismatch(_, _)
    ));
}

#[test]
fn rejects_malformed_digest() {
    let lock = PackageLock {
        schema_version: 1,
        entries: vec![minimal_entry("oxi.x", vec![], "short-digest")],
    };
    let json = serde_json::to_string(&lock).unwrap();
    let err = PackageLock::parse(&json).unwrap_err();
    assert!(matches!(
        err,
        oxios_kernel::foundation::packages::PackageError::BadDigestFormat(_, _)
    ));
}

#[test]
fn rejects_unsupported_schema() {
    let raw = r#"{"schema_version": 999, "entries": []}"#;
    let err = PackageLock::parse(raw).unwrap_err();
    assert!(matches!(
        err,
        oxios_kernel::foundation::packages::PackageError::UnsupportedSchema { .. }
    ));
}

#[test]
fn brain_query_maps_to_readonly_brain_cspace() {
    use oxios_kernel::capability::template::CapabilityTemplate;
    use oxios_kernel::capability::{ResourceRef, Rights};

    let archive = b"package payload";
    let digest = {
        let mut hasher = blake3::Hasher::new();
        hasher.update(archive);
        hasher.finalize().to_hex().to_string()
    };
    let lock = PackageLock {
        schema_version: 1,
        entries: vec![minimal_entry(
            "oxi.brain-helper",
            vec![AbstractRequirement::BrainQuery],
            &digest,
        )],
    };
    let mut archives = BTreeMap::new();
    archives.insert("oxi.brain-helper".to_string(), archive.to_vec());
    let pkg = &lock.import(&archives).expect("imports")[0];

    // Applied through the reviewed table only.
    let template =
        oxios_kernel::foundation::packages::apply_to_template(CapabilityTemplate::worker(), pkg);
    let cspace = template.build();
    let brain = ResourceRef::KernelDomain {
        domain: "brain".into(),
    };
    // brain.query grants READ — never Brain write.
    assert!(cspace.can(&brain, Rights::READ));
    assert!(!cspace.can(&brain, Rights::WRITE));
    // The worker base caps are preserved alongside the package caps.
    assert!(cspace.can(
        &ResourceRef::Exec {
            mode: "shell".into()
        },
        Rights::EXECUTE
    ));
}
