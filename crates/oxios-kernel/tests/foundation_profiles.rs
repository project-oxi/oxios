//! Integration tests for the Foundation profile registry (RFC-048 §3).
#![allow(clippy::unwrap_used)] // `.unwrap()` in tests is idiomatic (workspace convention)

use oxios_kernel::foundation::profile::{
    KeychainLocator, ModelCapabilities, Profile, ProfileRegistry, ProfileRole, ProviderKind,
};

fn sample_profile(id: &str, role: ProfileRole) -> Profile {
    Profile {
        id: id.to_string(),
        provider: ProviderKind::Anthropic,
        endpoint: String::new(),
        model: "claude-sonnet-4-5".into(),
        capabilities: ModelCapabilities::default(),
        roles: vec![role],
        keychain: KeychainLocator {
            service: "oxios.foundation".into(),
            account: format!("profile.{id}"),
        },
    }
}

#[test]
fn parses_valid_registry_json() {
    let raw = r#"{
        "schema_version": 1,
        "profiles": [
            {
                "id": "anthropic-coding",
                "provider": "anthropic",
                "model": "claude-sonnet-4-5",
                "roles": ["coding_primary"],
                "keychain": {"service": "oxios.foundation", "account": "anthropic-coding"}
            }
        ]
    }"#;
    let registry = ProfileRegistry::parse(raw).expect("parses");
    assert_eq!(registry.profiles.len(), 1);
    assert_eq!(registry.profiles[0].id, "anthropic-coding");
}

#[test]
fn rejects_secret_field_in_registry() {
    let raw = r#"{
        "schema_version": 1,
        "profiles": [
            {
                "id": "leaky",
                "provider": "anthropic",
                "model": "claude-sonnet-4-5",
                "roles": ["coding_primary"],
                "keychain": {"service": "oxios.foundation", "account": "leaky"},
                "api_key": "sk-ant-secret"
            }
        ]
    }"#;
    let err = ProfileRegistry::parse(raw).unwrap_err();
    assert!(matches!(
        err,
        oxios_kernel::foundation::profile::ProfileError::RawCredentialField(_, _)
    ));
}

#[test]
fn rejects_unsupported_schema_version() {
    let raw = r#"{"schema_version": 9999, "profiles": []}"#;
    let err = ProfileRegistry::parse(raw).unwrap_err();
    assert!(matches!(
        err,
        oxios_kernel::foundation::profile::ProfileError::UnsupportedSchema { .. }
    ));
}

#[test]
fn rejects_duplicate_ids() {
    let registry = ProfileRegistry {
        schema_version: 1,
        profiles: vec![
            sample_profile("dup", ProfileRole::CodingPrimary),
            sample_profile("dup", ProfileRole::AssistantGeneral),
        ],
    };
    let err = registry.validate().unwrap_err();
    assert!(matches!(
        err,
        oxios_kernel::foundation::profile::ProfileError::DuplicateId(_)
    ));
}

#[test]
fn resolver_returns_none_when_no_match() {
    let registry = ProfileRegistry {
        schema_version: 1,
        profiles: vec![sample_profile("coding", ProfileRole::CodingPrimary)],
    };
    assert!(
        registry
            .first_for_role(ProfileRole::AssistantGeneral)
            .is_none()
    );
}

#[test]
fn resolver_returns_first_match() {
    let registry = ProfileRegistry {
        schema_version: 1,
        profiles: vec![
            sample_profile("first", ProfileRole::CodingPrimary),
            sample_profile("second", ProfileRole::CodingPrimary),
        ],
    };
    let p = registry.first_for_role(ProfileRole::CodingPrimary).unwrap();
    assert_eq!(p.id, "first");
}
