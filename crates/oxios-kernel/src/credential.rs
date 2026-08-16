//! Multi-source credential resolution.
//!
//! Reads API keys from multiple sources with clear priority:
//! 1. `OXIOS_<PROVIDER>_API_KEY` env var (containers/K8s)
//! 2. `config.toml` → `[engine].api_key` (explicit override)
//! 3. oxios auth store (`~/.oxios/auth.json`, via `OXICODE_HOME`) — primary
//! 4. shared oxicode-cli store (`~/.oxicode/auth.json`) — backward-compat read
//!    fallback, so keys registered via the standalone oxicode CLI are still
//!    detected
//! 5. oxicode-ai env var fallback (CI/CD, containers)
//!
//! Writes (`store`/onboarding/`set_api_key`) always target the oxios auth store
//! (`~/.oxios/auth.json`), keeping oxios's credentials isolated in its own
//! product home. The shared `~/.oxicode/auth.json` is treated as read-only here.
//!
//! Both the modern `TokenBundle` and the legacy `oxicode-cli`
//! (`{"type":"api_key","key":"..."}`) shapes are honored in either store.

use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

/// Where a credential was found.
#[derive(Debug, Clone)]
pub enum CredentialSource {
    /// From config.toml [engine].api_key
    Config,
    /// From an auth store (~/.oxios or ~/.oxicode)
    OxicodeAuthStore,
    /// From the Oxi Foundation Keychain locator (RFC-048).
    FoundationKeychain,
    /// From environment variable
    EnvVar,
}

/// Credential-store key holding the Telegram bot token. Shared by the
/// Web UI Secrets section, the telegram channel plugin, and config
/// validation so all three agree on one storage location.
pub const TELEGRAM_TOKEN_STORE_KEY: &str = "telegram_bot_token";

/// Multi-source credential resolver.
pub struct CredentialStore;

impl CredentialStore {
    /// Resolve the best available API key for a provider.
    ///
    /// Priority: OXIOS_<PROVIDER>_API_KEY env → config.toml → oxios auth store
    /// (~/.oxios) → shared oxicode-cli store (~/.oxicode) → oxicode-ai env fallback.
    /// Environment variables take highest priority for container/K8s deployments.
    pub fn resolve(provider: &str, config_key: Option<&str>) -> Option<(String, CredentialSource)> {
        // 1. Explicit Oxios env var: OXIOS_<PROVIDER>_API_KEY (highest priority for containers)
        let env_var = format!("OXIOS_{}_API_KEY", provider.to_uppercase());
        if let Ok(key) = std::env::var(&env_var)
            && !key.is_empty()
        {
            return Some((key, CredentialSource::EnvVar));
        }

        // 2. config.toml explicit key
        if let Some(key) = config_key
            && !key.is_empty()
        {
            return Some((key.to_string(), CredentialSource::Config));
        }

        // 2.5. Oxi Foundation Keychain (RFC-048) — the normal post-migration
        //      source for non-secret profile metadata + Keychain-backed secret.
        if let Some((key, source)) = resolve_from_foundation_keychain(provider) {
            return Some((key, source));
        }

        if let Ok(Some(token)) = oxicode_sdk::load_token(provider)
            && !token.access_token.is_empty()
        {
            return Some((token.access_token, CredentialSource::OxicodeAuthStore));
        }

        // 4. Shared oxicode-cli store (~/.oxicode/auth.json) — backward-compat read.
        if let Some(key) = load_from_shared_store(provider) {
            return Some((key, CredentialSource::OxicodeAuthStore));
        }

        // 5. oxicode-ai env var fallback
        if let Some(key) = oxicode_sdk::get_env_api_key(provider) {
            return Some((key, CredentialSource::EnvVar));
        }

        // 6. Suffix fallback: subscription credentials may be stored under
        //    "<provider>-coding-plan" (e.g. "zai-coding-plan"). Only auth
        //    stores are consulted — env vars use the canonical name.
        {
            let alt = format!("{provider}-coding-plan");
            if let Ok(Some(token)) = oxicode_sdk::load_token(&alt)
                && !token.access_token.is_empty()
            {
                return Some((token.access_token, CredentialSource::OxicodeAuthStore));
            }
            if let Some(key) = load_from_shared_store(&alt) {
                return Some((key, CredentialSource::OxicodeAuthStore));
            }
        }
        None
    }

    /// Check if any credential is available for a provider.
    pub fn has_credential(provider: &str, config_key: Option<&str>) -> bool {
        Self::resolve(provider, config_key).is_some()
    }

    /// Store an API key to oxios's auth store (`~/.oxios/auth.json`).
    ///
    /// Writes are isolated to the oxios product home (`OXICODE_HOME`); the shared
    /// `~/.oxicode/auth.json` (oxicode CLI) is never written from here. If the oxios
    /// auth store contains legacy entries that don't deserialize as
    /// `TokenBundle`, they are auto-migrated before saving.
    pub fn store(provider: &str, api_key: &str) -> Result<()> {
        let token = oxicode_sdk::TokenBundle {
            access_token: api_key.to_string(),
            refresh_token: None,
            token_type: "Bearer".to_string(),
            obtained_at: chrono::Utc::now(),
            expires_in: 0,
            scope: None,
        };
        Self::store_token(provider, token)
    }

    /// Store an arbitrary [`oxicode_sdk::TokenBundle`] (e.g. an OAuth token with a
    /// `refresh_token`) to the oxios auth store, applying the same legacy
    /// migration as [`store`](Self::store). Used by the OAuth broker.
    pub fn store_token(provider: &str, token: oxicode_sdk::TokenBundle) -> Result<()> {
        // Try the normal path first.
        if let Err(e) = oxicode_sdk::save_token(provider, &token) {
            // If the auth store has legacy entries (e.g. `oxicode-cli` wrote
            // `{"type":"api_key","key":"..."}`), `save_token` fails because
            // it can't deserialize them as `TokenBundle`.  Migrate and retry.
            if is_legacy_auth_error(&e) {
                tracing::info!("auth.json has legacy format, migrating to TokenBundle");
                migrate_legacy_auth_store(provider, &token)?;
            } else {
                return Err(e.into());
            }
        }

        tracing::info!(provider = %provider, "credential stored to oxios auth store");
        Ok(())
    }

    /// Delete a credential from both auth stores.
    ///
    /// Removes the entry from the oxios store (`~/.oxios/auth.json`) and, if
    /// present, from the shared oxicode-cli store (`~/.oxicode/auth.json`). No-op if
    /// neither contains the key.
    pub fn delete(key: &str) -> Result<()> {
        // Primary: oxios store (OXICODE_HOME). Best-effort.
        let _ = oxicode_sdk::remove_token(key);

        // Shared: oxicode-cli store (~/.oxicode/auth.json).
        if let Ok(path) = shared_auth_json_path()
            && path.exists()
            && let Ok(raw) = std::fs::read_to_string(&path)
            && let Ok(mut map) =
                serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&raw)
            && map.remove(key).is_some()
        {
            std::fs::write(&path, serde_json::to_string_pretty(&map)?)?;
            tracing::info!(key = %key, "Credential deleted from shared oxicode-cli auth store");
        }
        Ok(())
    }
    /// Resolve a non-provider secret (telegram token, email password, etc.).
    ///
    /// Unlike [`resolve`](Self::resolve) — which checks `OXIOS_<PROVIDER>_API_KEY`
    /// and config.toml — this checks an explicit env var name first, then the
    /// auth stores. Used by the `/api/secrets` endpoints for keys that are not
    /// LLM provider credentials.
    pub fn resolve_secret(key: &str, env_var: &str) -> Option<(String, CredentialSource)> {
        // 1. Environment variable
        if let Ok(val) = std::env::var(env_var)
            && !val.is_empty()
        {
            return Some((val, CredentialSource::EnvVar));
        }
        // 2. oxios auth store (~/.oxios via OXICODE_HOME) — primary
        if let Ok(Some(token)) = oxicode_sdk::load_token(key)
            && !token.access_token.is_empty()
        {
            return Some((token.access_token, CredentialSource::OxicodeAuthStore));
        }
        // 3. Shared oxicode-cli store (~/.oxicode/auth.json) — backward-compat read.
        if let Some(val) = load_from_shared_store(key) {
            return Some((val, CredentialSource::OxicodeAuthStore));
        }
        None
    }

    /// Extract the provider name from a model ID.
    /// "anthropic/claude-sonnet-4-20250514" → "anthropic"
    /// Returns `None` if the model ID is empty or has no provider prefix.
    pub fn provider_from_model(model_id: &str) -> Option<&str> {
        if model_id.is_empty() {
            return None;
        }
        model_id.split_once('/').map(|(p, _)| p)
    }
}

/// Resolve an API key via the Oxi Foundation Keychain (RFC-048).
///
/// Looks up `~/.oxi/foundation/v1/profiles.json`, finds the profile whose
/// `id` matches `provider`, and reads the entry from the OS Keychain
/// using the profile's `{ service, account }` locator. Returns `None`
/// when no profile exists, the registry is missing, or the Keychain
/// entry is absent — callers should fall back to legacy sources.
pub fn resolve_from_foundation_keychain(provider: &str) -> Option<(String, CredentialSource)> {
    use crate::foundation::profile::ProfileRegistry;

    let home = dirs::home_dir()?;
    let registry_path =
        crate::foundation::versioned_root(&home).join(crate::foundation::PROFILES_FILE);
    if !registry_path.exists() {
        return None;
    }
    let raw = std::fs::read_to_string(&registry_path).ok()?;
    let registry = ProfileRegistry::parse(&raw).ok()?;
    let profile = registry.profiles.iter().find(|p| p.id == provider)?;
    let locator = &profile.keychain;
    let secret = foundation_keychain_read(&locator.service, &locator.account)?;
    Some((secret, CredentialSource::FoundationKeychain))
}

/// Read a secret from the OS Keychain via the `keyring` crate when
/// available; falls back to `None` (so legacy paths retain priority)
/// when the dependency is missing or the entry is absent.
#[cfg(feature = "foundation-keychain")]
fn foundation_keychain_read(service: &str, account: &str) -> Option<String> {
    use keyring::Entry;
    let entry = Entry::new(service, account).ok()?;
    entry.get_password().ok()
}

#[cfg(not(feature = "foundation-keychain"))]
fn foundation_keychain_read(_service: &str, _account: &str) -> Option<String> {
    None
}

// ── Shared oxicode-cli store (read-only fallback) ──────────────────────────────

/// Legacy entry from `oxicode-cli`: `{"type":"api_key","key":"..."}`.
#[derive(serde::Deserialize)]
struct LegacyEntry {
    #[allow(dead_code)]
    r#type: String,
    key: String,
}

/// Parse an auth.json blob and extract a provider's access token, accepting
/// both the modern `TokenBundle` shape and the legacy
/// `{"type":"api_key","key":...}` shape. Returns `None` when the provider is
/// absent or its entry parses to neither shape.
fn extract_credential(provider: &str, raw: &str) -> Option<String> {
    let map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(raw).ok()?;
    let entry = map.get(provider)?;
    // Modern TokenBundle first.
    if let Ok(bundle) = serde_json::from_value::<oxicode_sdk::TokenBundle>(entry.clone())
        && !bundle.access_token.is_empty()
    {
        return Some(bundle.access_token);
    }
    // Legacy `oxicode-cli` shape.
    if let Ok(legacy) = serde_json::from_value::<LegacyEntry>(entry.clone())
        && !legacy.key.is_empty()
    {
        return Some(legacy.key);
    }
    None
}

/// Load a credential from the shared oxicode-cli store (`~/.oxicode/auth.json`), which
/// oxios treats as a read-only secondary source. Tries `TokenBundle` first,
/// then the legacy `oxicode-cli` shape. Returns `None` when the file or entry is
/// absent.
fn load_from_shared_store(provider: &str) -> Option<String> {
    let path = match shared_auth_json_path() {
        Ok(p) => p,
        Err(_) => return None,
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(
                provider = %provider,
                path = %path.display(),
                error = %e,
                "shared auth.json exists but could not be read; skipping",
            );
            return None;
        }
    };
    extract_credential(provider, &raw)
}

/// Check if an error is caused by a legacy-format auth.json.
fn is_legacy_auth_error(err: &oxicode_sdk::OAuthError) -> bool {
    matches!(err, oxicode_sdk::OAuthError::Json(_))
}

/// Migrate a legacy auth.json to `TokenBundle` format, preserving entries that
/// can be converted and writing the new token for `provider`.
fn migrate_legacy_auth_store(provider: &str, new_token: &oxicode_sdk::TokenBundle) -> Result<()> {
    let path = shared_auth_json_path()?;
    let raw = std::fs::read_to_string(&path)?;

    // Parse as a flat JSON map.
    let entries: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&raw).unwrap_or_default();

    let mut migrated = HashMap::new();

    for (key, value) in &entries {
        if key == provider {
            continue; // will be replaced with new_token below
        }

        // Try parsing as TokenBundle first.
        if let Ok(bundle) = serde_json::from_value::<oxicode_sdk::TokenBundle>(value.clone()) {
            migrated.insert(key.clone(), bundle);
            continue;
        }

        // Try parsing as legacy `{"type":"api_key","key":"..."}`.
        if let Ok(legacy) = serde_json::from_value::<LegacyEntry>(value.clone()) {
            migrated.insert(
                key.clone(),
                oxicode_sdk::TokenBundle {
                    access_token: legacy.key,
                    refresh_token: None,
                    token_type: "Bearer".to_string(),
                    obtained_at: chrono::Utc::now(),
                    expires_in: 0,
                    scope: None,
                },
            );
            continue;
        }

        tracing::warn!(provider = %key, "skipping unparseable auth.json entry during migration");
    }

    // Insert the new token.
    migrated.insert(provider.to_string(), new_token.clone());

    // Write back as proper AuthStore (under OXICODE_HOME → ~/.oxios/auth.json).
    let store = oxicode_sdk::AuthStore { tokens: migrated };
    oxicode_sdk::save_auth_store(&store)?;
    Ok(())
}

/// Resolve the shared oxicode-cli auth store path (`~/.oxicode/auth.json`).
///
/// This is deliberately **independent of `OXICODE_HOME`**: it always points at the
/// oxicode CLI's home so oxios can read keys registered by the standalone oxicode CLI
/// tool as a backward-compat source, even while oxios's own writes are isolated
/// under `OXICODE_HOME` (`~/.oxios/auth.json`).
fn shared_auth_json_path() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| anyhow::anyhow!("Cannot determine home directory"))?;
    Ok(PathBuf::from(home).join(".oxicode").join("auth.json"))
}

/// Discover all provider names stored in either auth store.
///
/// Returns the union of provider IDs from the oxios store (`~/.oxios/auth.json`
/// via `OXICODE_HOME`) and the shared oxicode-cli store (`~/.oxicode/auth.json`). Special
/// keys like `"version"` are filtered out. Used by `OxiosEngine::from_config`
/// to inject credentials for providers beyond the hardcoded known list.
pub fn discover_auth_store_providers() -> Result<Vec<String>> {
    let mut providers: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let keep = |k: &str| k != "version" && !k.starts_with('_');

    // Primary: oxios store (OXICODE_HOME).
    if let Ok(store) = oxicode_sdk::load_auth_store() {
        for k in store.tokens.keys() {
            if keep(k) {
                providers.insert(k.clone());
            }
        }
    }
    // Shared: oxicode-cli store (~/.oxicode/auth.json) — top-level keys, any shape.
    if let Ok(path) = shared_auth_json_path()
        && let Ok(raw) = std::fs::read_to_string(&path)
        && let Ok(map) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&raw)
    {
        for k in map.keys() {
            if keep(k) {
                providers.insert(k.clone());
            }
        }
    }
    Ok(providers.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_from_model() {
        assert_eq!(
            CredentialStore::provider_from_model("anthropic/claude-sonnet-4-20250514"),
            Some("anthropic")
        );
        assert_eq!(
            CredentialStore::provider_from_model("openai/gpt-4o"),
            Some("openai")
        );
        assert_eq!(CredentialStore::provider_from_model("bare-model"), None);
        assert_eq!(CredentialStore::provider_from_model(""), None);
    }

    #[test]
    fn test_config_key_takes_priority() {
        // If config_key is set, it's always returned (even if other sources exist)
        let result = CredentialStore::resolve("anthropic", Some("sk-test-config-key"));
        assert!(result.is_some());
        let (key, source) = result.unwrap();
        assert_eq!(key, "sk-test-config-key");
        assert!(matches!(source, CredentialSource::Config));
    }

    #[test]
    fn test_empty_config_key_skipped() {
        // Empty string is treated as None — falls through to next source
        // (result depends on whether auth.json or env vars exist)
        let _ = CredentialStore::resolve("anthropic", Some(""));
    }

    #[test]
    fn test_none_config_key_skipped() {
        let _ = CredentialStore::resolve("anthropic", None); // depends on system state
    }

    #[test]
    fn extract_credential_roundtrips_token_bundle() {
        // Self-consistent: serialize a real TokenBundle, then read it back.
        let bundle = oxicode_sdk::TokenBundle {
            access_token: "sk-bundle".into(),
            refresh_token: None,
            token_type: "Bearer".into(),
            obtained_at: chrono::Utc::now(),
            expires_in: 0,
            scope: None,
        };
        let mut map = serde_json::Map::new();
        map.insert("openai".into(), serde_json::to_value(&bundle).unwrap());
        let raw = serde_json::to_string(&map).unwrap();
        assert_eq!(
            extract_credential("openai", &raw).as_deref(),
            Some("sk-bundle")
        );
    }

    #[test]
    fn extract_credential_reads_legacy_shape() {
        let raw = r#"{"anthropic":{"type":"api_key","key":"sk-legacy"}}"#;
        assert_eq!(
            extract_credential("anthropic", raw).as_deref(),
            Some("sk-legacy")
        );
    }

    #[test]
    fn extract_credential_absent_provider_is_none() {
        let raw = r#"{"openai":{"type":"api_key","key":"sk-x"}}"#;
        assert!(extract_credential("anthropic", raw).is_none());
    }

    #[test]
    fn extract_credential_empty_token_is_none() {
        let raw = r#"{"openai":{"type":"api_key","key":""}}"#;
        assert!(extract_credential("openai", raw).is_none());
    }
}
