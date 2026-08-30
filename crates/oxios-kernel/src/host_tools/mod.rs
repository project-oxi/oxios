//! Host Integrations subsystem (RFC-041).
//!
//! Three capabilities, all driven by a declarative registry:
//! - **Discovery** ([`scanner::HostToolScanner`]) — enumerate host CLIs across PATH and
//!   package-manager install roots; cross-platform; symlink-aware; TTL-cached.
//! - **OAuth** (Phase 3) — device-code handshake with refresh + revoke.
//! - **Provisioning** (Phase 4) — execute `SkillInstallSpec` as a privileged kernel op.
//!
//! The facade is [`HostToolsApi`], exposed on `KernelHandle` as `host_tools`.

pub mod oauth;
pub mod provisioner;
pub mod registry;
pub mod scanner;
pub use oauth::{DeviceCodeResponse, OAuthBroker, PollOutcome, PollResponse};
pub use provisioner::{InstallOutput, install as install_spec};
pub use registry::{
    CredentialResolver, CredentialStatus, Integration, IntegrationKind, IntegrationRegistry,
};
pub use scanner::{DetectedTool, HostProbe, HostToolScanner, RealProbe, ToolSource};

use std::sync::Arc;

/// Kernel facade for host-tool discovery + integration registry + OAuth
/// (RFC-041). Owns the shared [`HostToolScanner`] (TTL cache), the loaded
/// [`IntegrationRegistry`], and the [`OAuthBroker`].
pub struct HostToolsApi {
    scanner: Arc<HostToolScanner>,
    registry: IntegrationRegistry,
    oauth: OAuthBroker,
}

impl HostToolsApi {
    /// Assemble with the real host probe, a 60s scan TTL, and the integration
    /// registry loaded from the default paths.
    pub fn new() -> Self {
        let registry = Self::load_registry().unwrap_or_else(|e| {
            tracing::warn!(error = %e, "failed to load integration registry; using empty");
            IntegrationRegistry::default()
        });
        Self {
            scanner: Arc::new(HostToolScanner::real()),
            registry,
            oauth: OAuthBroker::new(),
        }
    }

    /// Assemble with an explicit scanner (tests). Registry defaults to empty.
    pub fn with_scanner(scanner: Arc<HostToolScanner>) -> Self {
        Self {
            scanner,
            registry: IntegrationRegistry::default(),
            oauth: OAuthBroker::new(),
        }
    }

    /// Resolve the integration registry. Defaults are **compiled in** via
    /// `include_str!` (always present regardless of CWD/install layout), with
    /// an optional filesystem override at `<oxios home>/share/default-integrations.toml`
    /// and per-file user overrides layered on top from `<oxios home>/integrations.d/`.
    fn load_registry() -> anyhow::Result<IntegrationRegistry> {
        const EMBEDDED: &str = include_str!("../../share/default-integrations.toml");
        let home = crate::oxi_home::oxios_home();
        let fs_defaults = home.join("share/default-integrations.toml");
        let defaults_text = if fs_defaults.exists() {
            std::fs::read_to_string(&fs_defaults).unwrap_or_else(|_| EMBEDDED.to_string())
        } else {
            EMBEDDED.to_string()
        };
        let overrides = home.join("integrations.d");
        IntegrationRegistry::load_text(&defaults_text, &overrides)
    }

    /// Detect a single binary by name (cache-aware).
    pub async fn detect(&self, name: &str) -> Option<DetectedTool> {
        self.scanner.detect(name).await
    }

    /// Detect many names; returns only the ones found (cache-aware).
    pub async fn detect_many(&self, names: &[String]) -> Vec<DetectedTool> {
        self.scanner.detect_many(names).await
    }

    /// Invalidate the scan cache (force fresh detection on next call).
    pub fn invalidate(&self) {
        self.scanner.invalidate();
    }

    /// All integrations in the registry (stable order).
    pub fn integrations(&self) -> Vec<&Integration> {
        self.registry.all()
    }

    /// Look up one integration by id.
    pub fn integration(&self, id: &str) -> Option<&Integration> {
        self.registry.get(id)
    }

    /// CLI names the registry wants detected (drives `/api/host-tools`).
    pub fn integration_cli_names(&self) -> Vec<String> {
        self.registry.cli_names()
    }

    /// Credential status for one integration — calls the matching resolver
    /// (H6: never pokes one env var). `None` if the id is unknown.
    pub fn credential_status(&self, id: &str) -> Option<CredentialStatus> {
        self.registry
            .get(id)
            .map(|i| CredentialStatus::resolve(&i.credential))
    }

    /// Provision an integration — runs its first applicable `SkillInstallSpec`
    /// as a privileged kernel op (D8: not via ExecTool). User-triggered only;
    /// the API layer gates consent + audit-logs.
    pub async fn install(&self, id: &str) -> anyhow::Result<InstallOutput> {
        let it = self
            .registry
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("integration '{id}' not found"))?;
        anyhow::ensure!(
            !it.install.is_empty(),
            "integration '{id}' has no install specs"
        );
        install_spec(&it.install).await
    }

    /// Start an OAuth device-code flow for an integration. The `device_code`
    /// stays daemon-side (H1); only the user-facing data is returned.
    pub async fn oauth_start(&self, id: &str) -> anyhow::Result<DeviceCodeResponse> {
        let it = self
            .registry
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("integration '{id}' not found"))?;
        let (provider, store_key, scopes) = match &it.credential {
            CredentialResolver::OAuth {
                provider,
                store_key,
                scopes,
            } => (provider.clone(), store_key.clone(), scopes.clone()),
            other => anyhow::bail!("integration '{id}' is {:?}, not oauth", other),
        };
        self.oauth.start(&provider, &store_key, &scopes).await
    }

    /// Poll an OAuth flow by opaque handle (H1). Terminal outcomes drop the flow.
    pub async fn oauth_poll(&self, handle: &str) -> anyhow::Result<PollResponse> {
        self.oauth.poll(handle).await
    }
}

impl Default for HostToolsApi {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for HostToolsApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostToolsApi").finish_non_exhaustive()
    }
}
