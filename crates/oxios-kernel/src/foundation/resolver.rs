//! Foundation profile resolver (RFC-048 §3).
//!
//! Bridges `FoundationProfile` metadata to the embedded `oxicode-sdk`
//! provider factory. **Never** shells out to an external worker — the
//! resolver provides model metadata + a credential to the existing
//! in-process SDK construction.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::foundation::profile::{Profile, ProfileRegistry, ProfileRole};

/// Resolved model + credential for a role request.
///
/// The shape is deliberately tiny — callers pass the metadata into the
/// SDK's provider factory and continue to use `oxicode_sdk` APIs for
/// everything else.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedModel {
    pub profile_id: String,
    pub provider: String,
    pub model: String,
    pub endpoint: String,
}

/// Resolver that loads the Foundation registry from disk and answers
/// `role → model` requests.
#[derive(Debug, Clone)]
pub struct FoundationProfileResolver {
    registry: ProfileRegistry,
    registry_path: PathBuf,
}

impl FoundationProfileResolver {
    /// Load the registry from the default location (`~/.oxi/foundation/v1/profiles.json`).
    pub fn load_default(home: &Path) -> Result<Self> {
        let path = crate::foundation::versioned_root(home).join(crate::foundation::PROFILES_FILE);
        Self::load_from(&path)
    }

    /// Load the registry from an explicit path. The registry must exist
    /// — Foundation bootstrap is what creates it on a fresh install.
    pub fn load_from(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read profile registry {}", path.display()))?;
        let registry = ProfileRegistry::parse(&raw)
            .with_context(|| format!("parse profile registry {}", path.display()))?;
        Ok(Self {
            registry,
            registry_path: path.to_path_buf(),
        })
    }

    /// Resolve a model for a role. Returns `None` when no profile is
    /// registered for that role — callers fall back to existing config /
    /// env / auth.json sources.
    pub fn resolve_for_role(&self, role: ProfileRole) -> Option<ResolvedModel> {
        let profile = self.registry.first_for_role(role)?;
        Some(resolved_from_profile(profile))
    }

    /// Resolve a model by an explicit profile id.
    pub fn resolve_by_id(&self, profile_id: &str) -> Result<ResolvedModel> {
        let profile = self
            .registry
            .by_id()
            .get(profile_id)
            .copied()
            .with_context(|| format!("profile `{profile_id}` not in registry"))?;
        Ok(resolved_from_profile(profile))
    }

    /// Refuse to resolve a profile that lacks the requested role. Used
    /// when an operator attempts to use the wrong profile for the wrong
    /// role (e.g. asking a `coding.primary` profile for memory.consolidate).
    pub fn resolve_with_role(&self, profile_id: &str, role: ProfileRole) -> Result<ResolvedModel> {
        let profile = self
            .registry
            .by_id()
            .get(profile_id)
            .copied()
            .with_context(|| format!("profile `{profile_id}` not in registry"))?;
        if !profile.roles.contains(&role) {
            bail!(
                "profile `{}` does not allow role `{role}` — declared roles: {:?}",
                profile.id,
                profile.roles
            );
        }
        Ok(resolved_from_profile(profile))
    }

    pub fn registry(&self) -> &ProfileRegistry {
        &self.registry
    }

    pub fn registry_path(&self) -> &Path {
        &self.registry_path
    }
}

fn resolved_from_profile(profile: &Profile) -> ResolvedModel {
    ResolvedModel {
        profile_id: profile.id.clone(),
        provider: provider_label(profile).to_string(),
        model: profile.model.clone(),
        endpoint: profile.endpoint.clone(),
    }
}

fn provider_label(profile: &Profile) -> &'static str {
    match profile.provider {
        crate::foundation::profile::ProviderKind::Anthropic => "anthropic",
        crate::foundation::profile::ProviderKind::Openai => "openai",
        crate::foundation::profile::ProviderKind::Google => "google",
        crate::foundation::profile::ProviderKind::Custom => "custom",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::profile::{
        KeychainLocator, ModelCapabilities, ProfileRole, ProviderKind,
    };
    use tempfile::tempdir;

    fn write_registry(home: &Path) -> PathBuf {
        let path = crate::foundation::versioned_root(home).join(crate::foundation::PROFILES_FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let registry = ProfileRegistry {
            schema_version: 1,
            profiles: vec![
                Profile {
                    id: "coding-default".into(),
                    provider: ProviderKind::Anthropic,
                    endpoint: String::new(),
                    model: "claude-sonnet-4-5".into(),
                    capabilities: ModelCapabilities::default(),
                    roles: vec![ProfileRole::CodingPrimary],
                    keychain: KeychainLocator {
                        service: "oxios.foundation".into(),
                        account: "profile.coding-default".into(),
                    },
                },
                Profile {
                    id: "assistant-default".into(),
                    provider: ProviderKind::Openai,
                    endpoint: String::new(),
                    model: "gpt-5".into(),
                    capabilities: ModelCapabilities::default(),
                    roles: vec![ProfileRole::AssistantGeneral],
                    keychain: KeychainLocator {
                        service: "oxios.foundation".into(),
                        account: "profile.assistant-default".into(),
                    },
                },
            ],
        };
        std::fs::write(&path, serde_json::to_string_pretty(&registry).unwrap()).unwrap();
        path
    }

    #[test]
    fn resolves_role_to_model() {
        let tmp = tempdir().unwrap();
        write_registry(tmp.path());
        let r = FoundationProfileResolver::load_default(tmp.path()).unwrap();
        let coding = r.resolve_for_role(ProfileRole::CodingPrimary).unwrap();
        assert_eq!(coding.provider, "anthropic");
        assert_eq!(coding.model, "claude-sonnet-4-5");
        assert_eq!(coding.profile_id, "coding-default");
        let assistant = r.resolve_for_role(ProfileRole::AssistantGeneral).unwrap();
        assert_eq!(assistant.provider, "openai");
        assert!(r.resolve_for_role(ProfileRole::MemoryConsolidate).is_none());
    }

    #[test]
    fn role_denied_when_profile_lacks_role() {
        let tmp = tempdir().unwrap();
        write_registry(tmp.path());
        let r = FoundationProfileResolver::load_default(tmp.path()).unwrap();
        let err = r
            .resolve_with_role("coding-default", ProfileRole::MemoryConsolidate)
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not allow role"));
    }

    #[test]
    fn unknown_profile_errors() {
        let tmp = tempdir().unwrap();
        write_registry(tmp.path());
        let r = FoundationProfileResolver::load_default(tmp.path()).unwrap();
        let err = r.resolve_by_id("ghost").unwrap_err().to_string();
        assert!(err.contains("not in registry"));
    }
}
