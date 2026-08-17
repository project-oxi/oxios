//! Engine provider — wraps oxicode-sdk's `Oxicode` for the kernel.
//!
//! All provider/model resolution goes through `oxicode_sdk::OxicodeBuilder`.
//! The `OxiosEngine` struct wraps the SDK instance and exposes a clean API
//! with support for routing, credentials, provider pooling, and multi-provider fallback.
//!
//! # Architecture
//!
//! ```text
//! OxiosEngine (OxicodeBuilder → Oxicode)
//!   ├── resolve_model("provider/model") → Model
//!   ├── create_provider("anthropic")     → Arc<dyn Provider>
//!   ├── pooled_provider("anthropic")     → Arc<dyn Provider> (rate-limited)
//!   ├── oxi()                            → &Oxicode (for AgentBuilder, etc.)
//!   └── agent(AgentConfig)               → AgentBuilder
//! ```

use anyhow::Result;
use oxicode_sdk::{CatalogConfig, FileModelCatalog, ModelCatalog, Oxicode, OxicodeBuilder};
use std::sync::Arc;

use oxios_ouroboros::{ModelResolver, ResolvedModel};

use crate::credential::{CredentialAuthProvider, CredentialStore, discover_auth_store_providers};

/// The kernel's engine — wraps oxicode-sdk's Oxicode instance.
///
/// Created via [`OxiosEngine::new()`] or [`OxiosEngine::builder()`].
/// Provides access to providers, models, routing, pooling, and agent construction.
///
/// # RFC-014 Phase D
///
/// `authorizer` / `tracer` / `cost_tracker` are optional, engine-level
/// observability and security handles. When set, they are propagated to
/// every agent built via [`OxiosEngine::oxi().agent()`][Oxicode::agent] using
/// the new `AgentBuilder::authorizer()` / `.tracer()` / `.cost_tracker()`
/// API. All three are `None` by default, keeping the existing call sites
/// fully backward compatible.
pub struct OxiosEngine {
    oxi: Oxicode,
    default_model_id: String,
    /// Runtime routing control for dynamic model selection.
    routing_control: Option<oxicode_sdk::RoutingControl>,
    /// ── RFC-014 Phase D: engine-level observability/security handles ──
    /// When `Some`, these are attached to every `Agent` built via the
    /// `AgentBuilder` API in `agent_runtime.rs::run_agent()`.
    /// Default: `None` (preserves pre-Phase-D behavior).
    authorizer: Option<Arc<oxicode_sdk::Authorizer>>,
    tracer: Option<Arc<oxicode_sdk::Tracer>>,
    cost_tracker: Option<Arc<oxicode_sdk::CostTracker>>,
}

impl OxiosEngine {
    /// Create a new engine with the given default model.
    ///
    /// Internally calls `OxicodeBuilder::new().with_builtins()` to load all
    /// built-in models and providers.
    pub fn new(default_model_id: impl Into<String>) -> Self {
        let model_id = default_model_id.into();
        let oxi = OxicodeBuilder::new()
            .with_builtins()
            .with_auth(Arc::new(CredentialAuthProvider))
            .build();
        Self {
            oxi,
            default_model_id: model_id,
            routing_control: None,
            // RFC-014 Phase D: optional, off by default
            authorizer: None,
            tracer: None,
            cost_tracker: None,
        }
    }

    /// Create a new engine with credentials from config.
    ///
    /// Resolves API keys from CredentialStore for each known provider
    /// and injects them into the OxicodeBuilder. This enables the engine
    /// to create properly authenticated providers.
    ///
    /// Resolution order (per provider): env var → config.toml → ~/.oxicode/auth.json
    ///
    /// No model catalog is wired (resolves via the static registry only).
    /// For dynamic models.dev metadata use
    /// [`from_config_with_catalog`](Self::from_config_with_catalog).
    pub fn from_config(default_model_id: impl Into<String>, config_api_key: Option<&str>) -> Self {
        Self::from_config_with_catalog_opt(default_model_id, config_api_key, None)
    }

    /// Like [`from_config`](Self::from_config) but wires a model catalog port
    /// into the engine.
    ///
    /// Pass the shared catalog from
    /// [`init_file_catalog`](Self::init_file_catalog) so dynamic models.dev
    /// metadata (live prices/limits, user overrides, local discovery) is
    /// reused across engine hot-swaps instead of re-initialized. `resolve_model`
    /// then consults the catalog before falling back to the static registry.
    pub fn from_config_with_catalog(
        default_model_id: impl Into<String>,
        config_api_key: Option<&str>,
        catalog: Arc<dyn ModelCatalog>,
    ) -> Self {
        Self::from_config_with_catalog_opt(default_model_id, config_api_key, Some(catalog))
    }

    fn from_config_with_catalog_opt(
        default_model_id: impl Into<String>,
        config_api_key: Option<&str>,
        catalog: Option<Arc<dyn ModelCatalog>>,
    ) -> Self {
        let model_id = default_model_id.into();

        // Resolve the primary provider's credential
        let primary_provider = model_id
            .split_once('/')
            .map(|(p, _)| p)
            .unwrap_or("anthropic");

        let mut builder = OxicodeBuilder::new()
            .with_builtins()
            // Auth port: consults CredentialStore on every provider
            // construction, so stored keys (~/.oxios, ~/.oxicode) work
            // without env vars. Without this, providers fall through to
            // bare env vars and fail with `Missing API key`.
            .with_auth(Arc::new(CredentialAuthProvider));

        // Collect all providers that need credential injection:
        // 1. Known major providers (always try to resolve)
        // 2. Any provider found in ~/.oxicode/auth.json (discovered dynamically)
        // 3. The primary provider (from the default model)
        let mut providers_to_try: Vec<String> = vec![
            "anthropic".into(),
            "openai".into(),
            "google".into(),
            "deepseek".into(),
            "xai".into(),
            "groq".into(),
            "openrouter".into(),
            "mistral".into(),
            "cerebras".into(),
            "fireworks".into(),
            "github-copilot".into(),
            "huggingface".into(),
            "together".into(),
            "minimax".into(),
            "moonshotai".into(),
            "kimi-coding".into(),
            "zai".into(),
            "opencode".into(),
        ];

        // Discover any additional providers from auth.json that aren't in the
        // known list (e.g. custom/third-party providers).
        if let Ok(extra) = discover_auth_store_providers() {
            for p in extra {
                if !providers_to_try.contains(&p) {
                    providers_to_try.push(p);
                }
            }
        }

        // Ensure the primary provider is always included.
        let primary_owned = primary_provider.to_string();
        if !providers_to_try.contains(&primary_owned) {
            providers_to_try.push(primary_owned);
        }

        for provider in &providers_to_try {
            // Use the config-level key only for the primary provider;
            // other providers resolve from env/auth.json.
            let config_key = if provider == primary_provider {
                config_api_key
            } else {
                None
            };

            if let Some((key, source)) = CredentialStore::resolve_exact(provider, config_key) {
                tracing::debug!(
                    provider,
                    source = ?source,
                    "Injected credential into engine"
                );
                builder = builder.api_key(provider, key);
            }
        }

        let builder = match catalog {
            Some(cat) => builder.with_catalog(cat),
            None => builder,
        };
        let oxi = builder.build();
        Self {
            oxi,
            default_model_id: model_id,
            routing_control: None,
            // RFC-014 Phase D: optional, off by default
            authorizer: None,
            tracer: None,
            cost_tracker: None,
        }
    }

    /// Create an engine builder for advanced configuration.
    ///
    /// Use this when you need credential injection, routing, or
    /// custom provider registration.
    ///
    /// # Catalog
    ///
    /// For dynamic models.dev metadata, initialize the catalog via
    /// [`OxiosEngine::init_file_catalog`] and attach it with
    /// [`with_catalog`](OxiosEngineBuilder::with_catalog):
    ///
    /// ```no_run
    /// # async fn doc() -> anyhow::Result<()> {
    /// use oxios_kernel::engine::OxiosEngine;
    ///
    /// let catalog = OxiosEngine::init_file_catalog().await?;
    /// let engine = OxiosEngine::builder()
    ///     .default_model("anthropic/claude-sonnet-4-20250514")
    ///     .with_catalog(catalog)
    ///     .build();
    /// # Ok(()) }
    /// ```
    ///
    /// # RFC-014 Phase D
    ///
    /// The builder also exposes `.with_authorizer()` / `.with_tracer()` /
    /// `.with_cost_tracker()` for attaching engine-level observability
    /// and security handles. All three are `None` by default.
    pub fn builder() -> OxiosEngineBuilder {
        OxiosEngineBuilder {
            inner: OxicodeBuilder::new()
                .with_builtins()
                .with_auth(Arc::new(CredentialAuthProvider)),
            default_model_id: "anthropic/claude-sonnet-4-20250514".to_string(),
            // RFC-014 Phase D: optional, off by default
            authorizer: None,
            tracer: None,
            cost_tracker: None,
            router_config: None,
            hook_specs: None,
        }
    }

    /// Build a [`CatalogConfig`] rooted at the oxios home (`~/.oxios/`).
    ///
    /// Keeps the models.dev cache/overrides self-hosted under oxios's own
    /// directory (not oxi's `~/.oxicode/`), consistent with the MCP cache/consent
    /// path customization. Local-server discovery (`ollama`/`lmstudio`) is
    /// left empty — wire it later if oxios wants to auto-discover local
    /// models.
    pub fn catalog_config() -> CatalogConfig {
        let home = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".oxios");
        CatalogConfig {
            cache_path: home.join("cache/models-dev.json"),
            etag_path: home.join("cache/models-dev.json.etag"),
            override_path: home.join("catalog/overrides.toml"),
            snapshot_path: home.join("cache/models-dev.json"),
            // oxios doesn't probe local servers yet.
            local_discovery_urls: Vec::new(),
            ..CatalogConfig::default()
        }
    }

    /// Initialize the shared [`FileModelCatalog`] for the engine.
    ///
    /// Loads the embedded models.dev snapshot + runtime cache, applies user
    /// overrides, and (if the cache is stale) attempts one live refresh
    /// (failure is silent — the snapshot serves as fallback). The returned
    /// `Arc<dyn ModelCatalog>` is cheap to clone and should be **shared**
    /// across engine hot-swaps: the catalog is lazy/on-call (no background
    /// tasks), so re-initializing it on every rebuild would just reload the
    /// snapshot needlessly.
    pub async fn init_file_catalog() -> Result<Arc<dyn ModelCatalog>> {
        let catalog: Arc<dyn ModelCatalog> =
            FileModelCatalog::init(Self::catalog_config())
                .await
                .map_err(|e| anyhow::anyhow!("Failed to initialize model catalog: {e}"))?;
        Ok(catalog)
    }

    /// Get a reference to the underlying Oxicode instance.
    ///
    /// Use this when you need to pass the engine to oxicode-sdk APIs directly
    /// (e.g., `AgentBuilder`, `MessageBus`, `AgentGroup`).
    pub fn oxi(&self) -> &Oxicode {
        &self.oxi
    }

    /// RFC-014 Phase D: get the engine-level `Authorizer`, if any.
    ///
    /// When `Some`, the authorizer is attached to every `Agent` built via
    /// `Oxicode::agent().authorizer(...)` in `agent_runtime.rs::run_agent()`.
    pub fn authorizer(&self) -> Option<&Arc<oxicode_sdk::Authorizer>> {
        self.authorizer.as_ref()
    }

    /// RFC-014 Phase D: get the engine-level `Tracer`, if any.
    ///
    /// When `Some`, the tracer is attached to every `Agent` built via
    /// `Oxicode::agent().tracer(...)` in `agent_runtime.rs::run_agent()`.
    pub fn tracer(&self) -> Option<&Arc<oxicode_sdk::Tracer>> {
        self.tracer.as_ref()
    }

    /// RFC-014 Phase D: get the engine-level `CostTracker`, if any.
    ///
    /// When `Some`, the cost tracker is attached to every `Agent` built via
    /// `Oxicode::agent().cost_tracker(...)` in `agent_runtime.rs::run_agent()`.
    pub fn cost_tracker(&self) -> Option<&Arc<oxicode_sdk::CostTracker>> {
        self.cost_tracker.as_ref()
    }

    /// Resolve a model ID through the Oxi Foundation profile resolver
    /// first (RFC-048 §3). Falls back to the embedded SDK when no
    /// profile matches the supplied role hint.
    pub fn resolve_model_via_foundation(
        &self,
        role: crate::foundation::profile::ProfileRole,
    ) -> Result<Option<crate::foundation::resolver::ResolvedModel>> {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let resolver =
            match crate::foundation::resolver::FoundationProfileResolver::load_default(&home) {
                Ok(r) => r,
                Err(_) => return Ok(None),
            };
        Ok(resolver.resolve_for_role(role))
    }
    pub fn resolve_model(&self, model_id: &str) -> Result<oxicode_sdk::Model> {
        // oxicode-sdk 0.66 returns `Result<_, SdkError>` (R7 typed errors); convert
        // into the kernel's `anyhow::Result` via `?`.
        Ok(self.oxi.resolve_model(model_id)?)
    }

    /// Create a provider for the given provider name.
    pub fn create_provider(&self, name: &str) -> Result<Arc<dyn oxicode_sdk::Provider>> {
        Ok(self.oxi.create_provider(name)?)
    }

    /// Get the default model ID.
    pub fn default_model_id(&self) -> &str {
        &self.default_model_id
    }

    /// Get the routing control, if routing is enabled.
    pub fn routing_control(&self) -> Option<&oxicode_sdk::RoutingControl> {
        self.routing_control.as_ref()
    }
}

// ---------------------------------------------------------------------------
// EngineBuilder
// ---------------------------------------------------------------------------

/// Builder for creating an `OxiosEngine` with advanced configuration.
pub struct OxiosEngineBuilder {
    inner: OxicodeBuilder,
    default_model_id: String,
    // ── RFC-014 Phase D: optional engine-level observability/security handles ──
    // All default to `None` so existing builder chains remain unchanged.
    authorizer: Option<Arc<oxicode_sdk::Authorizer>>,
    tracer: Option<Arc<oxicode_sdk::Tracer>>,
    cost_tracker: Option<Arc<oxicode_sdk::CostTracker>>,
    router_config: Option<crate::config::RouterConfig>,
    hook_specs: Option<Vec<oxicode_sdk::ports::hooks::HookSpec>>,
}

impl OxiosEngineBuilder {
    /// Set the default model ID.
    pub fn default_model(mut self, model_id: impl Into<String>) -> Self {
        self.default_model_id = model_id.into();
        self
    }

    pub fn api_key(self, provider: &str, key: impl Into<String>) -> Self {
        Self {
            inner: self.inner.api_key(provider, key),
            default_model_id: self.default_model_id,
            authorizer: self.authorizer,
            tracer: self.tracer,
            cost_tracker: self.cost_tracker,
            router_config: self.router_config,
            hook_specs: self.hook_specs,
        }
    }

    /// Register a full credential (API key + optional base URL).
    pub fn credential(
        self,
        provider: &str,
        api_key: impl Into<String>,
        base_url: Option<&str>,
    ) -> Self {
        Self {
            inner: self.inner.credential(provider, api_key, base_url),
            default_model_id: self.default_model_id,
            authorizer: self.authorizer,
            tracer: self.tracer,
            cost_tracker: self.cost_tracker,
            router_config: self.router_config,
            hook_specs: self.hook_specs,
        }
    }

    /// Attach the SDK auth port consulted on every provider construction.
    ///
    /// [`OxiosEngine::builder`] already wires
    /// [`crate::credential::CredentialAuthProvider`]; this exists so embedders
    /// and tests can substitute their own credential source.
    pub fn with_auth(mut self, auth: Arc<dyn oxicode_sdk::ports::AuthProvider>) -> Self {
        self.inner = self.inner.with_auth(auth);
        self
    }

    pub fn provider(self, name: &str, p: impl oxicode_sdk::Provider + 'static) -> Self {
        Self {
            inner: self.inner.provider(name, p),
            default_model_id: self.default_model_id,
            authorizer: self.authorizer,
            tracer: self.tracer,
            cost_tracker: self.cost_tracker,
            router_config: self.router_config,
            hook_specs: self.hook_specs,
        }
    }

    /// Register lifecycle hook specifications.
    pub fn with_hook_specs(mut self, specs: Vec<oxicode_sdk::ports::hooks::HookSpec>) -> Self {
        self.hook_specs = Some(specs);
        self
    }

    /// Wire lifecycle hook specs into the underlying OxicodeBuilder.
    ///
    /// Called from both [`build`](Self::build) and
    /// [`build_with_routing`](Self::build_with_routing) so the legacy
    /// `routing_enabled` path receives hooks too. No-op when no specs are
    /// configured.
    fn wire_hooks(mut self) -> Self {
        if let Some(specs) = &self.hook_specs
            && !specs.is_empty()
        {
            let runner = Arc::new(crate::hook_runner::CommandHookRunner::new(specs.clone()));
            self.inner = self.inner.with_hooks(runner);
        }
        self
    }

    /// Build the engine.
    ///
    /// If a router was configured via [`with_router`](Self::with_router) AND it
    /// is `enabled`, this also registers a `RouterProvider` as provider
    /// `"router"` plus synthetic model entries for each configured profile so
    /// `resolve_model("router/<profile>")` succeeds.
    pub fn build(self) -> OxiosEngine {
        let this = self.wire_hooks();

        let oxi = this.inner.build();
        register_router(&oxi, this.router_config.as_ref());

        OxiosEngine {
            oxi,
            default_model_id: this.default_model_id.clone(),
            routing_control: None,
            // RFC-014 Phase D: optional, off by default
            authorizer: this.authorizer,
            tracer: this.tracer,
            cost_tracker: this.cost_tracker,
        }
    }

    /// Build the engine with routing enabled.
    ///
    /// Returns `(OxiosEngine, RoutingControl)` for runtime routing control.
    /// If a router was configured via [`with_router`](Self::with_router) AND
    /// it is `enabled`, the router is registered post-build so the legacy
    /// routing path still resolves `router/<profile>` model ids.
    pub fn build_with_routing(self) -> (OxiosEngine, oxicode_sdk::RoutingControl) {
        use oxicode_sdk::RoutingControl;

        let this = self.wire_hooks();

        let routing_config = oxicode_sdk::routing::RoutingConfig::default();
        let routing_control = RoutingControl::new(routing_config);
        let oxi = this.inner.build();
        register_router(&oxi, this.router_config.as_ref());
        let engine = OxiosEngine {
            oxi,
            default_model_id: this.default_model_id,
            routing_control: Some(routing_control.clone()),
            // RFC-014 Phase D: optional, off by default
            authorizer: this.authorizer,
            tracer: this.tracer,
            cost_tracker: this.cost_tracker,
        };
        (engine, routing_control)
    }
    // ── RFC-014 Phase D: engine-level observability/security handles ──
    //
    // These methods let callers attach shared `Authorizer` / `Tracer` /
    // `CostTracker` instances to the engine. `agent_runtime.rs::run_agent()`
    // reads them via `OxiosEngine::authorizer()` / `.tracer()` /
    // `.cost_tracker()` and propagates them to the new `AgentBuilder` API.
    //
    // Backward compatible: all three are `None` by default.

    /// Attach an `Authorizer` to the engine. Agents built via `Oxicode::agent()`
    /// will receive this authorizer through the new `AgentBuilder::authorizer()` API.
    pub fn with_authorizer(mut self, authorizer: Arc<oxicode_sdk::Authorizer>) -> Self {
        self.authorizer = Some(authorizer);
        self
    }

    /// Attach a `Tracer` to the engine. Agents built via `Oxicode::agent()`
    /// will receive this tracer through the new `AgentBuilder::tracer()` API.
    pub fn with_tracer(mut self, tracer: Arc<oxicode_sdk::Tracer>) -> Self {
        self.tracer = Some(tracer);
        self
    }

    /// Attach a `CostTracker` to the engine. Agents built via `Oxicode::agent()`
    /// will receive this cost tracker through the new `AgentBuilder::cost_tracker()` API.
    pub fn with_cost_tracker(mut self, cost_tracker: Arc<oxicode_sdk::CostTracker>) -> Self {
        self.cost_tracker = Some(cost_tracker);
        self
    }

    /// Wire a model catalog port (e.g. [`FileModelCatalog`]) into the engine.
    ///
    /// When set, `Oxicode::resolve_model()` consults the catalog first (dynamic
    /// models.dev metadata: live prices/limits, user overrides, local
    /// discovery) before falling back to the static registry. Without this,
    /// the engine uses a [`NoopModelCatalog`](oxicode_sdk::NoopModelCatalog) and
    /// resolves via the static `model_db` only.
    ///
    /// Initialize the catalog once via
    /// [`OxiosEngine::init_file_catalog`] and reuse the `Arc` across rebuilds.
    pub fn with_catalog(mut self, catalog: Arc<dyn oxicode_sdk::ModelCatalog>) -> Self {
        self.inner = self.inner.with_catalog(catalog);
        self
    }

    /// Enable multi-model routing with the given configuration.
    ///
    /// Registers a `RouterProvider` as provider `"router"` and synthetic
    /// model entries for each profile so `resolve_model("router/<profile>")` works.
    pub fn with_router(mut self, router_config: crate::config::RouterConfig) -> Self {
        self.router_config = Some(router_config);
        self
    }
}

// ---------------------------------------------------------------------------
// EngineProvider trait (for testability and dependency inversion)
// ---------------------------------------------------------------------------

/// Engine provider trait — abstracts how the kernel obtains AI providers.
///
/// Implemented by `OxiosEngine` directly. Use a mock for testing.
pub trait EngineProvider: Send + Sync {
    /// Create a provider for the given provider name.
    fn create_provider(&self, provider_name: &str) -> Result<Arc<dyn oxicode_sdk::Provider>>;

    /// Resolve a "provider/model" string to a Model.
    fn resolve_model(&self, model_id: &str) -> Result<oxicode_sdk::Model>;

    /// Get the default model ID.
    fn default_model_id(&self) -> &str;
}

impl EngineProvider for OxiosEngine {
    fn create_provider(&self, provider_name: &str) -> Result<Arc<dyn oxicode_sdk::Provider>> {
        self.create_provider(provider_name)
    }

    fn resolve_model(&self, model_id: &str) -> Result<oxicode_sdk::Model> {
        self.resolve_model(model_id)
    }

    fn default_model_id(&self) -> &str {
        &self.default_model_id
    }
}

impl std::fmt::Debug for OxiosEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OxiosEngine")
            .field("default_model_id", &self.default_model_id)
            .field("routing_enabled", &self.routing_control.is_some())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// EngineHandle — hot-swappable engine reference
// ---------------------------------------------------------------------------

/// Shared, hot-swappable reference to the active [`OxiosEngine`].
///
/// Wraps `RwLock<Arc<OxiosEngine>>` so that:
/// - **Writers** (`EngineApi`) can atomically replace the engine on config change
/// - **Readers** (`AgentRuntime`) always get the current engine at execution time
///
/// # Cost
///
/// Rebuilding `OxiosEngine` is cheap: `OxicodeBuilder::new().with_builtins().build()`
/// populates registries from static `model_db` data (~1μs, no I/O, no network).
///
/// # Concurrency
///
/// - `parking_lot::RwLock` is not async-aware, but engine swap only occurs on
///   explicit user action (Web UI / CLI config change) — never in a hot path.
/// - Agent execution reads the engine once at the start of `execute()` and
///   uses the same `Arc<OxiosEngine>` for the entire run (consistent within one execution).
pub struct EngineHandle {
    inner: parking_lot::RwLock<Arc<OxiosEngine>>,
    /// Provider cache keyed by provider name. Survives across reads within one
    /// engine generation; cleared on [`swap`](Self::swap) so credential /
    /// provider changes take effect. Avoids rebuilding providers per phase call.
    provider_cache:
        parking_lot::RwLock<std::collections::HashMap<String, Arc<dyn oxicode_sdk::Provider>>>,
}
impl EngineHandle {
    /// Create a new handle wrapping the given engine.
    pub fn new(engine: Arc<OxiosEngine>) -> Self {
        Self {
            inner: parking_lot::RwLock::new(engine),
            provider_cache: parking_lot::RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Get a snapshot of the current engine.
    ///
    /// The returned `Arc` is stable — it won't change even if another thread
    /// calls `swap()` concurrently.
    pub fn get(&self) -> Arc<OxiosEngine> {
        Arc::clone(&self.inner.read())
    }

    /// Atomically replace the engine with a new one.
    ///
    /// Callers should rebuild `OxiosEngine` with updated credentials/model
    /// before calling this.
    pub fn swap(&self, new_engine: OxiosEngine) {
        {
            let mut guard = self.inner.write();
            let old_id = guard.default_model_id().to_string();
            *guard = Arc::new(new_engine);
            tracing::info!(
                old_model = %old_id,
                new_model = %guard.default_model_id(),
                "Engine hot-swapped"
            );
        }
        // Invalidate cached providers so credential / provider changes
        // (set_model / set_api_key) take effect on the next resolve.
        self.provider_cache.write().clear();
        tracing::debug!("Provider cache cleared on engine swap");
    }

    /// Resolve the live default model + a cached provider.
    ///
    /// This is the single source of truth for "which model does this task
    /// use", shared by the Ouroboros phases (via the [`ModelResolver`]
    /// impl) and the kernel's `AgentRuntime` (execute). Reads the engine's
    /// current `default_model_id` — which reflects hot-swaps — so a model
    /// change via the Web UI takes effect on the next phase call.
    ///
    /// Providers are cached per provider name and invalidated on [`swap`],
    /// so repeated resolution within one engine generation is cheap.
    pub fn resolve_default(&self) -> Result<ResolvedModel> {
        let engine = self.get();
        let model_id = engine.default_model_id().to_string();
        let model = engine.resolve_model(&model_id)?;
        let provider = self.cached_provider(&model.provider)?;
        Ok(ResolvedModel {
            model,
            provider,
            model_id,
        })
    }

    /// Resolve a specific model by ID against the live engine catalog.
    ///
    /// For callers that need a model other than the default (e.g. the
    /// Ouroboros lightweight model). Honors hot-swaps like `resolve_default`
    /// and reuses the same provider cache.
    pub fn resolve(&self, id: &str) -> Result<ResolvedModel> {
        let engine = self.get();
        let model = engine.resolve_model(id)?;
        let provider = self.cached_provider(&model.provider)?;
        Ok(ResolvedModel {
            model,
            provider,
            model_id: id.to_string(),
        })
    }

    /// Get a (cached) provider for a provider name, creating it on first use.
    fn cached_provider(&self, name: &str) -> Result<Arc<dyn oxicode_sdk::Provider>> {
        if let Some(p) = self.provider_cache.read().get(name) {
            return Ok(Arc::clone(p));
        }
        let provider = self.get().create_provider(name)?;
        self.provider_cache
            .write()
            .insert(name.to_string(), Arc::clone(&provider));
        Ok(provider)
    }
}

impl ModelResolver for EngineHandle {
    fn resolve_default(&self) -> Result<ResolvedModel> {
        EngineHandle::resolve_default(self)
    }

    fn resolve(&self, id: &str) -> Result<ResolvedModel> {
        EngineHandle::resolve(self, id)
    }
}

impl std::fmt::Debug for EngineHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let engine = self.inner.read();
        f.debug_struct("EngineHandle")
            .field("current_model", &engine.default_model_id())
            .finish()
    }
}

/// Register a `RouterProvider` on the given [`Oxicode`] if `cfg` is `Some` AND
/// `cfg.enabled` is true.
///
/// Shared between [`OxiosEngineBuilder::build`] and
/// [`OxiosEngineBuilder::build_with_routing`] so the legacy routing path
/// no longer silently drops router configuration. Logs a warning and skips
/// registration when the router is misconfigured (e.g. all profiles have no
/// tiers); a fully-valid config always emits at least one synthetic model
/// so `resolve_model("router/<default_profile>")` succeeds.
fn register_router(oxi: &Oxicode, cfg: Option<&crate::config::RouterConfig>) {
    let Some(cfg) = cfg else { return };
    if !cfg.enabled {
        return;
    }

    let sdk_router_config = router_config_to_sdk(cfg);
    if sdk_router_config.profiles.is_empty() {
        tracing::warn!(
            "Router enabled but no profiles produced valid tiers — router not registered"
        );
        return;
    }

    let registry = oxi.providers_arc();
    let router = oxicode_sdk::router::RouterProvider::new(&sdk_router_config, registry);
    oxi.providers().register_arc("router", Arc::new(router));

    // Register synthetic models for each profile so
    // resolve_model("router/<profile>") succeeds. The synthetic model points
    // at provider "router"; the RouterProvider selects the concrete delegate
    // at runtime.
    for profile_name in sdk_router_config.profiles.keys() {
        let model = oxicode_sdk::Model::new(
            profile_name.clone(),
            format!("Router {}", profile_name),
            oxicode_sdk::Api::OpenAiCompletions, // router delegates; dialect irrelevant
            "router",
            "", // no direct base URL; delegate providers handle it
        );
        oxi.models().register(model);
    }
}

/// Map an oxios thinking-budget value to an SDK [`ThinkingLevel`].
///
/// Documented monotonic mapping:
/// - `0`                → `Off`
/// - `1..=2048`         → `Low`
/// - `2049..=8192`      → `Medium`   (matches the design-doc example `balanced` = 4000)
/// - `8193..=32768`     → `High`     (matches the design-doc example `strong` = 16000)
/// - `> 32768`          → `XHigh`
fn thinking_level_from_budget(budget: u32) -> oxicode_sdk::ThinkingLevel {
    use oxicode_sdk::ThinkingLevel;
    match budget {
        0 => ThinkingLevel::Off,
        1..=2048 => ThinkingLevel::Low,
        2049..=8192 => ThinkingLevel::Medium,
        8193..=32768 => ThinkingLevel::High,
        _ => ThinkingLevel::XHigh,
    }
}

/// Build a [`RoutedTierConfig`] from an oxios [`RouterTierConfig`], using the
/// provided fallback `model` string if the tier itself is absent. Emits the
/// tier's `thinking` budget when present.
fn routed_tier_from(
    tier: Option<&crate::config::RouterTierConfig>,
    fallback_model: &str,
) -> oxicode_sdk::router::RoutedTierConfig {
    let Some(t) = tier else {
        return oxicode_sdk::router::RoutedTierConfig {
            model: fallback_model.to_string(),
            thinking: None,
            fallbacks: vec![],
        };
    };
    oxicode_sdk::router::RoutedTierConfig {
        model: t.model.clone(),
        thinking: t
            .thinking
            .as_ref()
            .map(|c| thinking_level_from_budget(c.budget)),
        fallbacks: t.fallbacks.clone(),
    }
}

/// Convert oxios [`RouterConfig`] to SDK [`oxicode_sdk::router::RouterConfig`].
///
/// Tier mapping: oxios `fast` → SDK `Low`, `balanced` → `Medium`, `strong` → `High`.
/// Scoring weight: oxios `context` → SDK `context_budget`.
///
/// Tier fallback policy (ensures no SDK tier ends up with an empty model id):
/// - `fast`     missing → use `fast` of the next-higher configured tier
/// - `balanced` missing → use `fast`'s model, then `strong`'s
/// - `strong`   missing → use `balanced`'s model, then `fast`'s
///
/// Profiles with no tiers configured at all are skipped (with a warning).
fn router_config_to_sdk(cfg: &crate::config::RouterConfig) -> oxicode_sdk::router::RouterConfig {
    use oxicode_sdk::router::{RouterConfig as SdkRouterConfig, RouterProfile, ScoringWeights};
    let mut profiles = std::collections::HashMap::new();
    for (name, profile) in &cfg.profiles {
        // Pick a non-empty fallback model for each missing tier by walking
        // through configured tiers in priority order.
        let configured: Vec<&str> = [
            profile.tiers.fast.as_ref().map(|t| t.model.as_str()),
            profile.tiers.balanced.as_ref().map(|t| t.model.as_str()),
            profile.tiers.strong.as_ref().map(|t| t.model.as_str()),
        ]
        .into_iter()
        .flatten()
        .filter(|m| !m.is_empty())
        .collect();

        if configured.is_empty() {
            tracing::warn!(
                profile = %name,
                "Router profile has no configured tiers — skipping"
            );
            continue;
        }

        // Per-tier fallback model chosen by walking configured tiers in the
        // documented priority order (strong → balanced → fast for SDK High,
        // balanced → strong → fast for SDK Medium, fast → balanced → strong
        // for SDK Low).
        let strong_model = profile
            .tiers
            .strong
            .as_ref()
            .and_then(|t| (!t.model.is_empty()).then_some(t.model.as_str()))
            .or_else(|| {
                profile
                    .tiers
                    .balanced
                    .as_ref()
                    .and_then(|t| (!t.model.is_empty()).then_some(t.model.as_str()))
            })
            .or_else(|| {
                profile
                    .tiers
                    .fast
                    .as_ref()
                    .and_then(|t| (!t.model.is_empty()).then_some(t.model.as_str()))
            });
        let balanced_model = profile
            .tiers
            .balanced
            .as_ref()
            .and_then(|t| (!t.model.is_empty()).then_some(t.model.as_str()))
            .or_else(|| {
                profile
                    .tiers
                    .fast
                    .as_ref()
                    .and_then(|t| (!t.model.is_empty()).then_some(t.model.as_str()))
            })
            .or_else(|| {
                profile
                    .tiers
                    .strong
                    .as_ref()
                    .and_then(|t| (!t.model.is_empty()).then_some(t.model.as_str()))
            });
        let fast_model = profile
            .tiers
            .fast
            .as_ref()
            .and_then(|t| (!t.model.is_empty()).then_some(t.model.as_str()))
            .or_else(|| {
                profile
                    .tiers
                    .balanced
                    .as_ref()
                    .and_then(|t| (!t.model.is_empty()).then_some(t.model.as_str()))
            })
            .or_else(|| {
                profile
                    .tiers
                    .strong
                    .as_ref()
                    .and_then(|t| (!t.model.is_empty()).then_some(t.model.as_str()))
            });

        let fast = routed_tier_from(
            profile.tiers.fast.as_ref(),
            fast_model.expect("at least one tier is configured"),
        );
        let balanced = routed_tier_from(
            profile.tiers.balanced.as_ref(),
            balanced_model.expect("at least one tier is configured"),
        );
        let strong = routed_tier_from(
            profile.tiers.strong.as_ref(),
            strong_model.expect("at least one tier is configured"),
        );

        profiles.insert(
            name.clone(),
            RouterProfile {
                high: strong,
                medium: balanced,
                low: fast,
            },
        );
    }

    SdkRouterConfig::new(
        cfg.default_profile.clone(),
        cfg.classifier_model.clone(),
        cfg.context_upgrade_threshold,
        cfg.max_session_budget,
        profiles,
        ScoringWeights {
            structural: cfg.scoring.structural,
            behavioral: cfg.scoring.behavioral,
            context_budget: cfg.scoring.context,
            vision: cfg.scoring.vision,
            message: cfg.scoring.message,
        },
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolve_default_reflects_hot_swap() {
        // The single source of truth: after a swap, resolve_default returns
        // the NEW default model. This is what makes Ouroboros (interview) and
        // AgentRuntime (execute) agree after a Web UI model change.
        let engine = OxiosEngine::new("anthropic/claude-sonnet-4-20250514");
        let handle = EngineHandle::new(Arc::new(engine));
        let r1 = handle.resolve_default().expect("initial resolve");
        assert_eq!(r1.model_id, "anthropic/claude-sonnet-4-20250514");
        assert_eq!(r1.model.provider, "anthropic");

        handle.swap(OxiosEngine::new("openai/gpt-4o"));
        let r2 = handle.resolve_default().expect("post-swap resolve");
        assert_eq!(r2.model_id, "openai/gpt-4o");
        assert_eq!(r2.model.provider, "openai");
    }

    #[test]
    fn resolve_default_fails_for_unknown_model() {
        // A bad default model surfaces immediately at resolve_default, not at
        // some later phase — the fix for the "interview works, execute fails"
        // divergence.
        let engine = OxiosEngine::new("zai-coding-plan/glm-5-turbo");
        let handle = EngineHandle::new(Arc::new(engine));
        assert!(handle.resolve_default().is_err());
    }

    #[test]
    fn model_resolver_impl_delegates_to_resolve_default() {
        // EngineHandle implements the Ouroboros ModelResolver port by delegating
        // to resolve_default — verify the trait path returns the same id.
        let engine = OxiosEngine::new("anthropic/claude-sonnet-4-20250514");
        let handle = EngineHandle::new(Arc::new(engine));
        let via_trait: &dyn ModelResolver = &handle;
        let r = via_trait.resolve_default().expect("trait resolve");
        assert_eq!(r.model_id, "anthropic/claude-sonnet-4-20250514");
    }

    #[test]
    fn test_resolve_model_with_provider_prefix() {
        let engine = OxiosEngine::new("anthropic/claude-sonnet-4-20250514");
        let model = engine.resolve_model("openai/gpt-4o").unwrap();
        assert_eq!(model.provider, "openai");
        assert_eq!(model.id, "gpt-4o");
    }

    #[test]
    fn test_resolve_model_without_provider_prefix() {
        let engine = OxiosEngine::new("anthropic/claude-sonnet-4-20250514");
        let model = engine.resolve_model("claude-sonnet-4-20250514").unwrap();
        assert_eq!(model.provider, "anthropic");
    }

    #[test]
    fn test_default_model_id() {
        let engine = OxiosEngine::new("anthropic/claude-sonnet-4-20250514");
        assert_eq!(
            engine.default_model_id(),
            "anthropic/claude-sonnet-4-20250514"
        );
    }

    #[test]
    fn test_resolve_model_not_found() {
        let engine = OxiosEngine::new("anthropic/claude-sonnet-4-20250514");
        let result = engine.resolve_model("nonexistent/model-xyz");
        assert!(result.is_err());
    }

    #[test]
    fn test_create_provider_anthropic() {
        let engine = OxiosEngine::new("anthropic/claude-sonnet-4-20250514");
        let provider = engine.create_provider("anthropic");
        assert!(provider.is_ok());
    }

    #[test]
    fn test_create_provider_not_found() {
        let engine = OxiosEngine::new("anthropic/claude-sonnet-4-20250514");
        let result = engine.create_provider("nonexistent_provider");
        assert!(result.is_err());
    }

    #[test]
    fn test_builder_with_credential() {
        let engine = OxiosEngine::builder()
            .default_model("openai/gpt-4o")
            .credential("openai", "sk-test", None)
            .build();
        assert_eq!(engine.default_model_id(), "openai/gpt-4o");
    }

    #[test]
    fn test_engine_provider_trait_on_engine() {
        let engine = OxiosEngine::new("anthropic/claude-sonnet-4-20250514");
        let provider: &dyn EngineProvider = &engine;
        assert!(provider.create_provider("anthropic").is_ok());
        assert!(provider.resolve_model("openai/gpt-4o").is_ok());
    }

    // ── EngineHandle tests ──

    #[test]
    fn test_engine_handle_get_returns_current() {
        let engine = OxiosEngine::new("anthropic/claude-sonnet-4-20250514");
        let handle = EngineHandle::new(Arc::new(engine));
        let e = handle.get();
        assert_eq!(e.default_model_id(), "anthropic/claude-sonnet-4-20250514");
    }

    #[test]
    fn test_engine_handle_swap_updates() {
        let engine = OxiosEngine::new("anthropic/claude-sonnet-4-20250514");
        let handle = EngineHandle::new(Arc::new(engine));

        let new_engine = OxiosEngine::new("openai/gpt-4o");
        handle.swap(new_engine);

        let e = handle.get();
        assert_eq!(e.default_model_id(), "openai/gpt-4o");
    }

    #[test]
    fn test_engine_handle_swap_preserves_old_arc() {
        // An Arc obtained before swap should remain valid.
        let engine = OxiosEngine::new("anthropic/claude-sonnet-4-20250514");
        let handle = EngineHandle::new(Arc::new(engine));

        let old = handle.get();
        assert_eq!(old.default_model_id(), "anthropic/claude-sonnet-4-20250514");

        handle.swap(OxiosEngine::new("openai/gpt-4o"));

        // `old` still points to the pre-swap engine.
        assert_eq!(old.default_model_id(), "anthropic/claude-sonnet-4-20250514");

        // New get() returns the swapped engine.
        let current = handle.get();
        assert_eq!(current.default_model_id(), "openai/gpt-4o");
    }

    // ── RFC-014 Phase D: engine-level observability/security handles ──

    #[test]
    fn test_rfc014_phase_d_default_fields_are_none() {
        // Backward compatibility: `OxiosEngine::new()` / `from_config()` /
        // `builder().build()` must all leave the new optional fields as
        // `None` so existing call sites are unaffected.
        let engine = OxiosEngine::new("anthropic/claude-sonnet-4-20250514");
        assert!(engine.authorizer().is_none());
        assert!(engine.tracer().is_none());
        assert!(engine.cost_tracker().is_none());

        let engine = OxiosEngine::from_config("anthropic/claude-sonnet-4-20250514", None);
        assert!(engine.authorizer().is_none());
        assert!(engine.tracer().is_none());
        assert!(engine.cost_tracker().is_none());

        let engine = OxiosEngine::builder()
            .default_model("openai/gpt-4o")
            .build();
        assert!(engine.authorizer().is_none());
        assert!(engine.tracer().is_none());
        assert!(engine.cost_tracker().is_none());

        let (engine, _rc) = OxiosEngine::builder()
            .default_model("openai/gpt-4o")
            .build_with_routing();
        assert!(engine.authorizer().is_none());
        assert!(engine.tracer().is_none());
        assert!(engine.cost_tracker().is_none());
    }

    #[test]
    fn test_rfc014_phase_d_with_tracer() {
        // `with_tracer` attaches a `Tracer`; accessor returns `Some`.
        let tracer = Arc::new(oxicode_sdk::Tracer::new());
        let engine = OxiosEngine::builder()
            .default_model("openai/gpt-4o")
            .with_tracer(tracer.clone())
            .build();
        assert!(engine.tracer().is_some());
        assert!(engine.authorizer().is_none());
        assert!(engine.cost_tracker().is_none());
    }

    #[test]
    fn test_rfc014_phase_d_with_cost_tracker() {
        // `with_cost_tracker` attaches a `CostTracker`; accessor returns `Some`.
        // `CostTracker::new` needs an `Arc<ModelRegistry>`; the engine's
        // own registry (via `models_arc`) is fine for construction-only
        // assertions like this one.
        let oxi_for_registry = oxicode_sdk::OxicodeBuilder::new().with_builtins().build();
        let model_registry = oxi_for_registry.models_arc();
        let cost_tracker = Arc::new(oxicode_sdk::CostTracker::new(
            model_registry,
            oxicode_sdk::CostTrackerConfig::default(),
        ));
        let engine = OxiosEngine::builder()
            .default_model("openai/gpt-4o")
            .with_cost_tracker(cost_tracker)
            .build();
        assert!(engine.cost_tracker().is_some());
        assert!(engine.authorizer().is_none());
        assert!(engine.tracer().is_none());
    }

    #[test]
    fn test_rfc014_phase_d_with_authorizer() {
        // `with_authorizer` attaches an `Authorizer`; accessor returns `Some`.
        let audit = Arc::new(oxicode_sdk::AuditLog::new(16));
        let authorizer = Arc::new(oxicode_sdk::Authorizer::new(audit));
        let engine = OxiosEngine::builder()
            .default_model("openai/gpt-4o")
            .with_authorizer(authorizer)
            .build();
        assert!(engine.authorizer().is_some());
        assert!(engine.tracer().is_none());
        assert!(engine.cost_tracker().is_none());
    }

    #[test]
    fn test_rfc014_phase_d_all_three_handles() {
        // All three handles can be set at once. The build chain must
        // preserve them through `api_key` / `credential` / `provider`
        // builder methods (they should be no-ops for the new fields).
        let audit = Arc::new(oxicode_sdk::AuditLog::new(16));
        let authorizer = Arc::new(oxicode_sdk::Authorizer::new(audit));
        let tracer = Arc::new(oxicode_sdk::Tracer::new());
        let oxi_for_registry = oxicode_sdk::OxicodeBuilder::new().with_builtins().build();
        let model_registry = oxi_for_registry.models_arc();
        let cost_tracker = Arc::new(oxicode_sdk::CostTracker::new(
            model_registry,
            oxicode_sdk::CostTrackerConfig::default(),
        ));

        let engine = OxiosEngine::builder()
            .default_model("openai/gpt-4o")
            .api_key("openai", "sk-test")
            .with_authorizer(authorizer)
            .with_tracer(tracer)
            .with_cost_tracker(cost_tracker)
            .build();

        assert!(engine.authorizer().is_some());
        assert!(engine.tracer().is_some());
        assert!(engine.cost_tracker().is_some());
        assert_eq!(engine.default_model_id(), "openai/gpt-4o");
    }

    // ── Router registration (oxicode-sdk 0.66.0+) ──
    //
    // Wires a `RouterProvider` into the engine via `with_router` and verifies:
    // 1. The provider is reachable under the name `"router"`.
    // 2. Synthetic model entries for each profile are resolvable.
    // 3. The configured default model id flows through unchanged.
    #[test]
    fn test_router_registration() {
        let router_cfg = crate::config::RouterConfig {
            enabled: true,
            default_profile: "auto".into(),
            profiles: {
                let mut m = std::collections::HashMap::new();
                let tiers = crate::config::RouterTiersConfig {
                    fast: Some(crate::config::RouterTierConfig {
                        model: "anthropic/claude-haiku-4-20250514".into(),
                        fallbacks: vec![],
                        thinking: None,
                    }),
                    balanced: Some(crate::config::RouterTierConfig {
                        model: "anthropic/claude-sonnet-4-20250514".into(),
                        fallbacks: vec![],
                        thinking: None,
                    }),
                    strong: Some(crate::config::RouterTierConfig {
                        model: "anthropic/claude-opus-4-20250514".into(),
                        fallbacks: vec![],
                        thinking: None,
                    }),
                };
                m.insert("auto".into(), crate::config::RouterProfileConfig { tiers });
                m
            },
            ..Default::default()
        };

        let engine = OxiosEngine::builder()
            .default_model("router/auto")
            .with_router(router_cfg)
            .build();

        // Router provider should be registered.
        assert!(engine.create_provider("router").is_ok());

        // Router profile models should be resolvable.
        assert!(engine.resolve_model("router/auto").is_ok());

        // Default model should match.
        assert_eq!(engine.default_model_id(), "router/auto");
    }

    #[test]
    fn test_router_registration_with_routing_path() {
        // The legacy `build_with_routing()` path must also register the router.
        // Regression guard for: "build_with_routing() silently drops router_config".
        let router_cfg = crate::config::RouterConfig {
            enabled: true,
            default_profile: "auto".into(),
            profiles: {
                let mut m = std::collections::HashMap::new();
                let tiers = crate::config::RouterTiersConfig {
                    fast: Some(crate::config::RouterTierConfig {
                        model: "anthropic/claude-haiku-4-20250514".into(),
                        fallbacks: vec![],
                        thinking: None,
                    }),
                    balanced: Some(crate::config::RouterTierConfig {
                        model: "anthropic/claude-sonnet-4-20250514".into(),
                        fallbacks: vec![],
                        thinking: None,
                    }),
                    strong: Some(crate::config::RouterTierConfig {
                        model: "anthropic/claude-opus-4-20250514".into(),
                        fallbacks: vec![],
                        thinking: None,
                    }),
                };
                m.insert("auto".into(), crate::config::RouterProfileConfig { tiers });
                m
            },
            ..Default::default()
        };

        let (engine, _rc) = OxiosEngine::builder()
            .default_model("router/auto")
            .with_router(router_cfg)
            .build_with_routing();

        // Provider must be registered even on the routing path.
        assert!(engine.create_provider("router").is_ok());
        // Synthetic model must resolve.
        assert!(engine.resolve_model("router/auto").is_ok());
        // RoutingControl is still surfaced.
        assert!(engine.routing_control().is_some());
    }

    #[test]
    fn test_thinking_budget_maps_to_level() {
        // Documented monotonic mapping for the router thinking-budget field.
        use oxicode_sdk::ThinkingLevel;
        assert_eq!(thinking_level_from_budget(0), ThinkingLevel::Off);
        assert_eq!(thinking_level_from_budget(1), ThinkingLevel::Low);
        assert_eq!(thinking_level_from_budget(2048), ThinkingLevel::Low);
        assert_eq!(thinking_level_from_budget(2049), ThinkingLevel::Medium);
        // Design-doc example: balanced budget 4000 → Medium.
        assert_eq!(thinking_level_from_budget(4000), ThinkingLevel::Medium);
        assert_eq!(thinking_level_from_budget(8192), ThinkingLevel::Medium);
        assert_eq!(thinking_level_from_budget(8193), ThinkingLevel::High);
        // Design-doc example: strong budget 16000 → High.
        assert_eq!(thinking_level_from_budget(16000), ThinkingLevel::High);
        assert_eq!(thinking_level_from_budget(32768), ThinkingLevel::High);
        assert_eq!(thinking_level_from_budget(32769), ThinkingLevel::XHigh);
        assert_eq!(thinking_level_from_budget(100_000), ThinkingLevel::XHigh);
    }

    #[test]
    fn test_router_tier_fallback_fills_empty_models() {
        // A profile with only the `balanced` tier must still produce
        // non-empty `model` strings for SDK High/Medium/Low tiers.
        let tiers = crate::config::RouterTiersConfig {
            balanced: Some(crate::config::RouterTierConfig {
                model: "anthropic/claude-sonnet-4-20250514".into(),
                fallbacks: vec![],
                thinking: Some(crate::config::RouterThinkingConfig { budget: 4000 }),
            }),
            ..Default::default()
        };
        let cfg = crate::config::RouterConfig {
            enabled: true,
            default_profile: "auto".into(),
            profiles: {
                let mut m = std::collections::HashMap::new();
                m.insert("auto".into(), crate::config::RouterProfileConfig { tiers });
                m
            },
            ..Default::default()
        };

        let sdk_cfg = router_config_to_sdk(&cfg);
        let profile = sdk_cfg.profiles.get("auto").expect("profile present");
        // All three SDK tiers must have non-empty model ids.
        assert!(
            !profile.high.model.is_empty(),
            "SDK High tier must not be empty"
        );
        assert!(
            !profile.medium.model.is_empty(),
            "SDK Medium tier must not be empty"
        );
        assert!(
            !profile.low.model.is_empty(),
            "SDK Low tier must not be empty"
        );
        // Thinking budget → ThinkingLevel.
        assert_eq!(
            profile.medium.thinking,
            Some(oxicode_sdk::ThinkingLevel::Medium)
        );
    }

    #[test]
    fn test_router_empty_profile_skipped() {
        // A profile with no tiers at all must be skipped, not emitted with empty
        // model strings.
        let cfg = crate::config::RouterConfig {
            enabled: true,
            default_profile: "auto".into(),
            profiles: {
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "broken".into(),
                    crate::config::RouterProfileConfig {
                        tiers: crate::config::RouterTiersConfig::default(),
                    },
                );
                m
            },
            ..Default::default()
        };

        let sdk_cfg = router_config_to_sdk(&cfg);
        assert!(
            !sdk_cfg.profiles.contains_key("broken"),
            "empty profile must be skipped"
        );
    }

    //
    // `#[ignore]` because `init_file_catalog` may touch the network for a
    // one-shot models.dev refresh and writes to `~/.oxios/cache/`. Run with
    // `cargo test -p oxios-kernel --lib catalog_integration -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn catalog_integration_init_and_resolve() {
        // 1. Catalog initializes (SNAP + cache + optional live refresh).
        let catalog = OxiosEngine::init_file_catalog()
            .await
            .expect("catalog init should succeed (SNAP is always embedded)");

        // 2. The embedded snapshot always carries providers/models, so a wired
        //    catalog is non-empty.
        assert!(
            catalog.model_count_sync() > 0,
            "catalog should expose models from the embedded snapshot"
        );
        assert!(!catalog.list_providers_sync().is_empty());

        // 3. An engine built with the catalog resolves through it first.
        let engine = OxiosEngine::builder()
            .default_model("anthropic/claude-sonnet-4-20250514")
            .with_catalog(catalog)
            .build();
        let model = engine
            .resolve_model("openai/gpt-4o")
            .expect("catalog-backed resolve_model should succeed");
        assert_eq!(model.provider, "openai");
        assert_eq!(model.id, "gpt-4o");
    }
}
