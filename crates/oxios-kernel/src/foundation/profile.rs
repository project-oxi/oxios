//! Foundation profile registry (RFC-048 §3).
//!
//! Parses and validates the shared `profiles.json` schema. Profiles hold
//! non-secret metadata and an OS Keychain `{ service, account }` locator;
//! raw API keys are explicitly rejected.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use thiserror::Error;

/// Roles a profile can be selected for.
///
/// Mirrors the role allow-list documented in RFC-048 §3. Adding a new role
/// here is a coordinated change with the engine resolver — unknown roles
/// are refused at parse time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileRole {
    /// Extracts structured facts from raw conversation/memory.
    MemoryExtract,
    /// Produces derived/sourced/uncertain episodes.
    MemoryConsolidate,
    /// Default coding model.
    CodingPrimary,
    /// Default conversational/general assistant.
    AssistantGeneral,
}

impl fmt::Display for ProfileRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MemoryExtract => f.write_str("memory.extract"),
            Self::MemoryConsolidate => f.write_str("memory.consolidate"),
            Self::CodingPrimary => f.write_str("coding.primary"),
            Self::AssistantGeneral => f.write_str("assistant.general"),
        }
    }
}

/// Provider kind, mapped to the embedded `oxicode_sdk` provider factory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Anthropic,
    Openai,
    Google,
    /// Catch-all for SDK-supported providers not enumerated here.
    /// Validation rejects names the kernel cannot build.
    Custom,
}

/// Model capability declaration. Profiles advertise what they support so
/// the resolver can fail fast on mismatched requests.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelCapabilities {
    /// Provider-reported context window (tokens). 0 = unknown.
    #[serde(default)]
    pub context_window: u32,
    /// Whether the model emits tool calls.
    #[serde(default)]
    pub tool_use: bool,
    /// Whether the model returns streaming deltas.
    #[serde(default = "default_streaming")]
    pub streaming: bool,
}

fn default_streaming() -> bool {
    true
}

/// OS Keychain locator. Carries a `{ service, account }` pair so a separate
/// Keychain store can be used per profile without storing the secret in
/// `profiles.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeychainLocator {
    /// Keychain service label (e.g. `oxios.foundation`).
    pub service: String,
    /// Account label (e.g. profile ID + provider).
    pub account: String,
}

/// A single Foundation profile. Profiles contain zero raw credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// Stable identifier; unique across the registry.
    pub id: String,
    /// Provider kind.
    pub provider: ProviderKind,
    /// Provider endpoint override (optional).
    #[serde(default)]
    pub endpoint: String,
    /// Model identifier.
    pub model: String,
    /// Declared model capabilities.
    #[serde(default)]
    pub capabilities: ModelCapabilities,
    /// Roles this profile is allowed to satisfy.
    pub roles: Vec<ProfileRole>,
    /// Keychain locator for this profile's secret.
    pub keychain: KeychainLocator,
}

/// Profile registry root. Versioned so a future schema bump can co-exist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileRegistry {
    /// Schema version. Must equal [`super::SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Profiles keyed by `Profile::id`.
    pub profiles: Vec<Profile>,
}

/// Strict profile parse/validation errors. Every variant maps to a
/// concrete failure surfaced in `foundation status` and rejected at boot.
#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("profile registry schema version {found} is not supported (expected {expected})")]
    UnsupportedSchema { found: u32, expected: u32 },
    #[error("duplicate profile id `{0}` in registry")]
    DuplicateId(String),
    #[error("profile `{0}` has no roles")]
    NoRoles(String),
    #[error("profile `{0}` has unknown role `{1}`")]
    UnknownRole(String, String),
    #[error("profile `{0}` has unknown provider kind `{1}`")]
    UnknownProviderKind(String, String),
    #[error("profile `{0}` is missing model identifier")]
    MissingModel(String),
    #[error("profile `{0}` is missing keychain locator")]
    MissingKeychain(String),
    #[error(
        "profile `{0}` contains raw credential field `{1}` — secrets must live in the Keychain"
    )]
    RawCredentialField(String, &'static str),
    #[error("profile `{0}` keychain service must not be empty")]
    EmptyKeychainService(String),
    #[error("profile `{0}` keychain account must not be empty")]
    EmptyKeychainAccount(String),
    #[error("profile `{0}` endpoint `{1}` is not a valid URL")]
    InvalidEndpoint(String, String),
    #[error("profile `{0}` model `{1}` is empty or unparseable")]
    InvalidModel(String, String),
    #[error("malformed JSON in profile registry: {0}")]
    Json(String),
}

/// Fields that are never allowed inside `Profile` because they would
/// invite a secret leak. Their presence is a hard error.
const FORBIDDEN_RAW_CREDENTIAL_FIELDS: &[&str] = &[
    "api_key",
    "apiKey",
    "apikey",
    "secret",
    "password",
    "access_token",
    "accessToken",
    "token",
];

impl ProfileRegistry {
    /// Parse and validate a profile registry JSON document.
    pub fn parse(json: &str) -> Result<Self, ProfileError> {
        // Pre-scan for raw credential fields before serde runs, so we can
        // attribute them to a profile id once parsed.
        let raw: serde_json::Value =
            serde_json::from_str(json).map_err(|e| ProfileError::Json(e.to_string()))?;

        Self::reject_raw_credentials(&raw)?;
        let registry: ProfileRegistry =
            serde_json::from_value(raw).map_err(|e| ProfileError::Json(e.to_string()))?;
        registry.validate()?;
        Ok(registry)
    }

    /// Validate an already-decoded registry. Used by tests and by callers
    /// that build the registry programmatically.
    pub fn validate(&self) -> Result<(), ProfileError> {
        if self.schema_version != super::SCHEMA_VERSION {
            return Err(ProfileError::UnsupportedSchema {
                found: self.schema_version,
                expected: super::SCHEMA_VERSION,
            });
        }

        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for profile in &self.profiles {
            if !seen.insert(profile.id.clone()) {
                return Err(ProfileError::DuplicateId(profile.id.clone()));
            }
            validate_profile(profile)?;
        }
        Ok(())
    }

    /// Build an index keyed by profile id for O(1) lookup.
    pub fn by_id(&self) -> BTreeMap<String, &Profile> {
        self.profiles.iter().map(|p| (p.id.clone(), p)).collect()
    }

    /// Find the first profile allowed for a role. The resolver expects
    /// the caller to choose the profile explicitly when more than one is
    /// available — this helper is only used for the default mapping.
    pub fn first_for_role(&self, role: ProfileRole) -> Option<&Profile> {
        self.profiles
            .iter()
            .find(|p| p.roles.iter().any(|r| *r == role))
    }

    fn reject_raw_credentials(value: &serde_json::Value) -> Result<(), ProfileError> {
        // We treat the *first* offending profile as the source of the
        // error so the operator can locate it. Walk the JSON tree looking
        // for forbidden keys at any depth.
        fn walk(value: &serde_json::Value, current_id: Option<&str>) -> Result<(), ProfileError> {
            match value {
                serde_json::Value::Object(map) => {
                    let id_here = map.get("id").and_then(|v| v.as_str()).or(current_id);
                    for (k, v) in map {
                        if FORBIDDEN_RAW_CREDENTIAL_FIELDS.contains(&k.as_str()) {
                            let owner = id_here.unwrap_or("<unknown>").to_string();
                            return Err(ProfileError::RawCredentialField(
                                owner,
                                FORBIDDEN_RAW_CREDENTIAL_FIELDS
                                    .iter()
                                    .find(|f| **f == k.as_str())
                                    .copied()
                                    .unwrap_or("?"),
                            ));
                        }
                        walk(v, id_here)?;
                    }
                }
                serde_json::Value::Array(items) => {
                    for item in items {
                        walk(item, current_id)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        walk(value, None)
    }
}

fn validate_profile(profile: &Profile) -> Result<(), ProfileError> {
    if profile.id.is_empty() {
        return Err(ProfileError::DuplicateId(String::new()));
    }
    if profile.model.trim().is_empty() {
        return Err(ProfileError::MissingModel(profile.id.clone()));
    }
    if profile.roles.is_empty() {
        return Err(ProfileError::NoRoles(profile.id.clone()));
    }
    if profile.keychain.service.trim().is_empty() {
        return Err(ProfileError::EmptyKeychainService(profile.id.clone()));
    }
    if profile.keychain.account.trim().is_empty() {
        return Err(ProfileError::EmptyKeychainAccount(profile.id.clone()));
    }
    if !profile.endpoint.is_empty() && url::Url::parse(&profile.endpoint).is_err() {
        return Err(ProfileError::InvalidEndpoint(
            profile.id.clone(),
            profile.endpoint.clone(),
        ));
    }
    match profile.provider {
        ProviderKind::Anthropic | ProviderKind::Openai | ProviderKind::Google => {}
        ProviderKind::Custom => {
            // Custom is allowed but the model must look like `name` or
            // `provider/name` to be buildable by the SDK factory.
            if !profile.model.contains('/') && profile.model.contains(char::is_whitespace) {
                return Err(ProfileError::InvalidModel(
                    profile.id.clone(),
                    profile.model.clone(),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_profile(id: &str, role: ProfileRole) -> Profile {
        Profile {
            id: id.to_string(),
            provider: ProviderKind::Anthropic,
            endpoint: String::new(),
            model: "claude-sonnet-4-5".to_string(),
            capabilities: ModelCapabilities::default(),
            roles: vec![role],
            keychain: KeychainLocator {
                service: "oxios.foundation".to_string(),
                account: format!("profile.{id}"),
            },
        }
    }

    #[test]
    fn parses_minimal_registry() {
        let registry = ProfileRegistry {
            schema_version: 1,
            profiles: vec![
                minimal_profile("coding-default", ProfileRole::CodingPrimary),
                minimal_profile("assistant-default", ProfileRole::AssistantGeneral),
            ],
        };
        assert!(registry.validate().is_ok());
        assert_eq!(registry.by_id().len(), 2);
        assert!(
            registry
                .first_for_role(ProfileRole::CodingPrimary)
                .is_some()
        );
        assert!(
            registry
                .first_for_role(ProfileRole::AssistantGeneral)
                .is_some()
        );
        assert!(
            registry
                .first_for_role(ProfileRole::MemoryExtract)
                .is_none()
        );
    }

    #[test]
    fn rejects_duplicate_ids() {
        let registry = ProfileRegistry {
            schema_version: 1,
            profiles: vec![
                minimal_profile("dup", ProfileRole::CodingPrimary),
                minimal_profile("dup", ProfileRole::AssistantGeneral),
            ],
        };
        let err = registry.validate().unwrap_err();
        assert!(matches!(err, ProfileError::DuplicateId(id) if id == "dup"));
    }

    #[test]
    fn rejects_raw_api_key_field() {
        let raw = r#"{
            "schema_version": 1,
            "profiles": [
                {
                    "id": "leaky",
                    "provider": "anthropic",
                    "model": "claude-sonnet-4-5",
                    "roles": ["coding.primary"],
                    "keychain": {"service": "oxios", "account": "leaky"},
                    "api_key": "sk-ant-..."
                }
            ]
        }"#;
        let err = ProfileRegistry::parse(raw).unwrap_err();
        assert!(matches!(err, ProfileError::RawCredentialField(_, _)));
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let raw = r#"{"schema_version": 999, "profiles": []}"#;
        let err = ProfileRegistry::parse(raw).unwrap_err();
        assert!(matches!(err, ProfileError::UnsupportedSchema { .. }));
    }

    #[test]
    fn rejects_empty_roles() {
        let mut p = minimal_profile("noroles", ProfileRole::CodingPrimary);
        p.roles.clear();
        let registry = ProfileRegistry {
            schema_version: 1,
            profiles: vec![p],
        };
        let err = registry.validate().unwrap_err();
        assert!(matches!(err, ProfileError::NoRoles(_)));
    }

    #[test]
    fn rejects_empty_model() {
        let mut p = minimal_profile("nomodel", ProfileRole::CodingPrimary);
        p.model.clear();
        let registry = ProfileRegistry {
            schema_version: 1,
            profiles: vec![p],
        };
        let err = registry.validate().unwrap_err();
        assert!(matches!(err, ProfileError::MissingModel(_)));
    }

    #[test]
    fn rejects_invalid_endpoint() {
        let mut p = minimal_profile("badurl", ProfileRole::CodingPrimary);
        p.endpoint = "not a url".to_string();
        let registry = ProfileRegistry {
            schema_version: 1,
            profiles: vec![p],
        };
        let err = registry.validate().unwrap_err();
        assert!(matches!(err, ProfileError::InvalidEndpoint(_, _)));
    }

    #[test]
    fn rejects_empty_keychain_locator() {
        let mut p = minimal_profile("nokeychain", ProfileRole::CodingPrimary);
        p.keychain.service.clear();
        let registry = ProfileRegistry {
            schema_version: 1,
            profiles: vec![p],
        };
        let err = registry.validate().unwrap_err();
        assert!(matches!(err, ProfileError::EmptyKeychainService(_)));
    }
}
