//! Integration registry (RFC-041 Phase 2).
//!
//! Declarative catalog of host integrations — each entry binds a CLI to detect
//! (optional), install spec(s) (Phase 4), and a credential descriptor. The
//! registry is loaded from `share/default-integrations.toml` and merged with
//! user overrides in `~/.oxios/integrations.d/*.toml` (whole-entry replace by
//! `id`).
//!
//! Credential model (H6): a single `env_var` cannot serve both
//! `CredentialStore::resolve` (provider, 6-source) and `resolve_secret`
//! (non-provider, 3-source). So the descriptor names the store key, the env
//! var, **and** which resolver applies. The status endpoint must call the
//! matching resolver — never poke one env var.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::credential::{CredentialSource, CredentialStore};
use crate::skill::SkillInstallSpec;

/// Coarse category of an integration, used by the UI to group entries with
/// different card layouts (RFC-041 S1). Defaults to `CliTool` for backward
/// compatibility with registries authored before the field existed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationKind {
    /// Package manager — `brew`/`npm`/`cargo`/etc. No credential, no install
    /// specs (it *is* the bootstrap).
    PackageManager,
    /// CLI tool installed via a package manager or download. Has install
    /// specs and (optionally) a credential. The default for legacy entries.
    #[default]
    CliTool,
    /// Credential-only — no binary to detect or install (e.g. a bot token).
    /// The card hides the install/detect affordances.
    CredentialOnly,
}
// ─── Credential descriptor (fix H6) ──────────────────────────────────────────

/// Which resolution path applies to a credential. Per D7 there is **no**
/// `Provider` variant — LLM providers stay in `engine_api` and may only appear
/// as read-only status cards via a separate UI path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "resolver", rename_all = "lowercase")]
pub enum CredentialResolver {
    /// No credential needed (package managers).
    #[default]
    None,
    /// Non-provider secret → `CredentialStore::resolve_secret(store_key, env_var)`.
    /// 3-source: raw env var → oxios store → oxicode-cli store.
    Secret {
        /// Key passed to `load_token`/`save_token` and `resolve_secret`'s `key`.
        store_key: String,
        /// Raw env var name checked first by `resolve_secret`.
        env_var: String,
    },
    /// OAuth device-code → `TokenBundle` stored under `store_key` (Phase 3).
    /// `provider` names a Rust `OAuthProvider` impl (D9); the TOML selects
    /// *which* provider + scopes, Rust implements *how*.
    OAuth {
        store_key: String,
        provider: String,
        #[serde(default)]
        scopes: Vec<String>,
    },
}

/// A single registry entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Integration {
    /// Stable identifier (URL path segment, skill `requires.integrations` ref).
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Binary name to detect. `None` = credential-only (no CLI).
    #[serde(default)]
    pub cli: Option<String>,
    /// Install instructions, reused verbatim from the skill system (Phase 4
    /// executes them). Defaults to empty.
    #[serde(default)]
    pub install: Vec<SkillInstallSpec>,
    /// Credential descriptor — which resolver applies + store key + env var.
    #[serde(default)]
    pub credential: CredentialResolver,
    /// Coarse category for UI grouping (S1). Defaults to `CliTool`.
    #[serde(default)]
    pub kind: IntegrationKind,
}

// ─── Install-spec TOML shim ──────────────────────────────────────────────────
//
// `SkillInstallSpec` derives Deserialize but its `kind` is a typed enum. The
// registry TOML writes `kind = "brew"` etc., which maps cleanly. We deserialize
// through this shim to keep the registry file self-describing and tolerant of
// future InstallKind variants (Cargo/Bun/Pip — Phase 4).
#[derive(Debug, Deserialize)]
struct RawIntegration {
    id: String,
    label: String,
    #[serde(default)]
    cli: Option<String>,
    #[serde(default)]
    install: Vec<SkillInstallSpec>,
    #[serde(default)]
    credential: CredentialResolver,
    #[serde(default)]
    kind: IntegrationKind,
}

impl From<RawIntegration> for Integration {
    fn from(r: RawIntegration) -> Self {
        Self {
            id: r.id,
            label: r.label,
            cli: r.cli,
            install: r.install,
            credential: r.credential,
            kind: r.kind,
        }
    }
}
#[derive(Debug, Deserialize)]
struct RegistryFile {
    #[serde(default, rename = "integration")]
    integrations: Vec<RawIntegration>,
}

// ─── Registry ────────────────────────────────────────────────────────────────

/// Loaded integration catalog, keyed by `id`. Built by merging the shipped
/// defaults with user overrides (whole-entry replace by id).
#[derive(Debug, Clone, Default)]
pub struct IntegrationRegistry {
    by_id: HashMap<String, Integration>,
}

impl IntegrationRegistry {
    /// Parse a single TOML file's integrations.
    fn parse_file(text: &str) -> Result<Vec<Integration>> {
        let file: RegistryFile = toml::from_str(text).context("parsing integrations TOML")?;
        Ok(file.integrations.into_iter().map(Into::into).collect())
    }

    /// Load defaults from an in-memory TOML string (compiled-in via
    /// `include_str!`), then merge user overrides from `override_dir`.
    /// This is the production path — always present regardless of CWD.
    pub fn load_text(defaults_text: &str, override_dir: &Path) -> Result<Self> {
        let mut by_id: HashMap<String, Integration> = HashMap::new();
        for it in Self::parse_file(defaults_text)? {
            by_id.insert(it.id.clone(), it);
        }
        Self::layer_overrides(&mut by_id, override_dir);
        Ok(Self { by_id })
    }

    /// Load the shipped defaults from a file path, then merge user overrides.
    /// Missing files are not errors. Used by tests.
    pub fn load(defaults_path: &Path, override_dir: &Path) -> Result<Self> {
        let mut by_id: HashMap<String, Integration> = HashMap::new();

        if let Ok(text) = std::fs::read_to_string(defaults_path) {
            for it in Self::parse_file(&text)? {
                by_id.insert(it.id.clone(), it);
            }
        }

        Self::layer_overrides(&mut by_id, override_dir);
        Ok(Self { by_id })
    }

    /// Merge `override_dir/*.toml` into `by_id` (whole-entry replace by id).
    fn layer_overrides(by_id: &mut HashMap<String, Integration>, override_dir: &Path) {
        let Ok(entries) = std::fs::read_dir(override_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                match Self::parse_file(&text) {
                    Ok(items) => {
                        for it in items {
                            by_id.insert(it.id.clone(), it);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "skipping invalid integrations override");
                    }
                }
            }
        }
    }

    /// All integrations in stable (insertion-sorted-by-key) order.
    pub fn all(&self) -> Vec<&Integration> {
        let mut ids: Vec<&String> = self.by_id.keys().collect();
        ids.sort();
        ids.iter().filter_map(|id| self.by_id.get(*id)).collect()
    }

    /// Look up one integration by id.
    pub fn get(&self, id: &str) -> Option<&Integration> {
        self.by_id.get(id)
    }

    /// Names of CLIs this registry wants detected (non-`None` `cli` fields).
    pub fn cli_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.by_id.values().filter_map(|i| i.cli.clone()).collect();
        names.sort();
        names.dedup();
        names
    }
}

// ─── Credential status (H6: calls the matching resolver) ────────────────────

/// Where a resolved credential came from (mirrors `CredentialSource` for JSON).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialStatus {
    /// True when a usable credential exists (resolver found one).
    pub configured: bool,
    /// Source label: `env` | `auth_store` | `config` | `oauth` | `none`.
    pub source: String,
}

impl CredentialStatus {
    /// Run the matching resolver for a credential descriptor and report status.
    ///
    /// Per H6, this NEVER pokes a single env var — it calls the full chain:
    /// - `None` → always unconfigured.
    /// - `Secret` → `CredentialStore::resolve_secret(store_key, env_var)` (3-source).
    /// - `OAuth` → checks for a stored `TokenBundle` under `store_key`.
    pub fn resolve(cred: &CredentialResolver) -> Self {
        match cred {
            CredentialResolver::None => Self {
                configured: false,
                source: "none".into(),
            },
            CredentialResolver::Secret { store_key, env_var } => {
                match CredentialStore::resolve_secret(store_key, env_var) {
                    Some((_, src)) => Self {
                        configured: true,
                        source: source_label(&src),
                    },
                    None => Self {
                        configured: false,
                        source: "none".into(),
                    },
                }
            }
            CredentialResolver::OAuth { store_key, .. } => {
                // Phase 3 populates TokenBundle via save_token; for now just
                // check presence so the status is truthful today.
                let present = oxicode_sdk::load_token(store_key)
                    .ok()
                    .flatten()
                    .map(|t| !t.access_token.is_empty())
                    .unwrap_or(false);
                Self {
                    configured: present,
                    source: if present {
                        "oauth".into()
                    } else {
                        "none".into()
                    },
                }
            }
        }
    }
}

fn source_label(s: &CredentialSource) -> String {
    match s {
        CredentialSource::Config => "config",
        CredentialSource::OxicodeAuthStore => "auth_store",
        CredentialSource::FoundationKeychain => "foundation_keychain",
        CredentialSource::EnvVar => "env",
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_TOML: &str = r#"
[[integration]]
id = "brew"
label = "Homebrew"
cli = "brew"
credential = { resolver = "none" }

[[integration]]
id = "resend"
label = "Resend"
cli = "resend"
install = [{ kind = "node", package = "resend" }]
credential = { resolver = "secret", store_key = "resend", env_var = "RESEND_API_KEY" }

[[integration]]
id = "github"
label = "GitHub CLI"
cli = "gh"
credential = { resolver = "oauth", store_key = "github", provider = "github", scopes = ["repo"] }
"#;

    #[test]
    fn parses_integration_toml() {
        let items = IntegrationRegistry::parse_file(SAMPLE_TOML).unwrap();
        assert_eq!(items.len(), 3);

        let brew = items.iter().find(|i| i.id == "brew").unwrap();
        assert_eq!(brew.credential, CredentialResolver::None);

        let resend = items.iter().find(|i| i.id == "resend").unwrap();
        match &resend.credential {
            CredentialResolver::Secret { store_key, env_var } => {
                assert_eq!(store_key, "resend");
                assert_eq!(env_var, "RESEND_API_KEY");
            }
            other => panic!("expected Secret, got {other:?}"),
        }
        assert_eq!(resend.install.len(), 1);

        let gh = items.iter().find(|i| i.id == "github").unwrap();
        match &gh.credential {
            CredentialResolver::OAuth {
                provider, scopes, ..
            } => {
                assert_eq!(provider, "github");
                assert_eq!(scopes, &["repo"]);
            }
            other => panic!("expected OAuth, got {other:?}"),
        }
    }

    #[test]
    fn shipped_registry_parses() {
        let text = include_str!("../../share/default-integrations.toml");
        let items = IntegrationRegistry::parse_file(text).unwrap();
        assert!(items.iter().any(|integration| integration.id == "github"));
        assert!(items.iter().any(|integration| integration.id == "resend"));
    }

    #[test]
    fn cli_names_dedups() {
        let reg = IntegrationRegistry {
            by_id: toml::from_str::<RegistryFile>(SAMPLE_TOML)
                .unwrap()
                .integrations
                .into_iter()
                .map(Into::into)
                .map(|i: Integration| (i.id.clone(), i))
                .collect(),
        };
        let names = reg.cli_names();
        assert_eq!(names, vec!["brew", "gh", "resend"]);
    }

    #[test]
    fn none_resolver_is_unconfigured() {
        let s = CredentialStatus::resolve(&CredentialResolver::None);
        assert!(!s.configured);
        assert_eq!(s.source, "none");
    }
}
