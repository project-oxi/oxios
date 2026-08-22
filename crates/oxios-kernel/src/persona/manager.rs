//! Persona manager: coordinates persona-aware execution.
//!
//! The PersonaManager manages persona lifecycle and provides
//! the active persona for orchestrator and agent runtime.
//!
//! RFC-039 completion: the manager owns an optional `StateStore` and a
//! reseed callback so that **every** `set_active` call — from the HTTP API,
//! the `PersonaTool`, the Gateway, or the delete handler — automatically
//! persists to disk and re-seeds the intent engine.  There is one code path;
//! callers cannot accidentally skip persistence or re-seeding.

use std::sync::Arc;

use anyhow::Result;
use parking_lot::RwLock;

use super::store::PersonaStore;
use super::{Persona, default_personas};
use crate::state_store::StateStore;

/// Manages persona lifecycle and coordinates persona-aware execution.
pub struct PersonaManager {
    store: PersonaStore,
    active_persona_id: RwLock<Option<String>>,
    /// Optional shared `StateStore` for automatic persistence.
    /// When `None`, persistence calls are no-ops (tests, ephemeral runs).
    state_store: Option<Arc<StateStore>>,
    /// Callback invoked after a successful `set_active` to re-seed the
    /// intent engine's `system_prompt`.  Stored on the manager (not
    /// `PersonaApi`) so ephemeral `PersonaApi` instances in `PersonaTool`
    /// cannot lose it.
    #[allow(clippy::type_complexity)]
    reseed_callback: RwLock<Option<Arc<dyn Fn(Option<String>) + Send + Sync>>>,
}

impl std::fmt::Debug for PersonaManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersonaManager")
            .field("active_persona_id", &self.active_persona_id.read())
            .field("state_store", &self.state_store.is_some())
            .field("reseed_callback", &self.reseed_callback.read().is_some())
            .finish()
    }
}

impl PersonaManager {
    /// Creates a new persona manager with default personas.
    pub fn new() -> Self {
        let store = PersonaStore::new();
        let manager = Self {
            store,
            active_persona_id: RwLock::new(None),
            state_store: None,
            reseed_callback: RwLock::new(None),
        };
        manager.create_default_personas();
        manager
    }

    /// Creates a new persona manager, optionally loading from existing data.
    pub fn with_defaults(personas: Vec<Persona>) -> Self {
        let store = PersonaStore::new();
        store.load_from_slice(&personas);
        let this = Self {
            store,
            active_persona_id: RwLock::new(None),
            state_store: None,
            reseed_callback: RwLock::new(None),
        };
        // Set the first enabled persona as active by default.
        if let Some(first) = this.store.list_enabled().into_iter().next() {
            *this.active_persona_id.write() = Some(first.id);
        }
        this
    }

    /// Builder: attach a shared `StateStore`.
    pub fn with_state_store(mut self, store: Arc<StateStore>) -> Self {
        self.state_store = Some(store);
        self
    }

    /// Set the callback that re-seeds the intent engine's system_prompt.
    /// Called automatically by `set_active`.
    pub fn set_reseed_callback(&self, cb: Option<Arc<dyn Fn(Option<String>) + Send + Sync>>) {
        *self.reseed_callback.write() = cb;
    }

    /// Returns the current active persona, if any.
    pub fn get_active_persona(&self) -> Option<Persona> {
        let active_id = self.active_persona_id.read().clone();
        active_id.and_then(|id| self.store.get(&id))
    }

    /// Returns the system prompt for the active persona.
    /// Falls back to a default prompt if no active persona.
    pub fn active_system_prompt(&self) -> String {
        self.get_active_persona()
            .map(|p| p.system_prompt.clone())
            .unwrap_or_else(|| {
                "You are a helpful AI assistant that follows the Ouroboros methodology: \
                 specify before you build, evaluate before you ship."
                    .to_string()
            })
    }

    /// Creates the default personas (Dev, Review, Research, Architect, Mentor,
    /// Ops, Security, Writer, Planner).
    pub fn create_default_personas(&self) {
        let defaults = default_personas();
        for persona in defaults {
            // Only register if not already present.
            if self.store.get(&persona.id).is_none() {
                self.store.register(persona);
            }
        }
        // Set first persona as active if none is set.
        {
            let mut active = self.active_persona_id.write();
            if active.is_none() {
                *active = Some("dev".to_string());
            }
        }
        tracing::info!("Default personas initialized");
    }

    /// Returns the first enabled persona, for wiring into OuroborosEngine.
    pub fn first_enabled(&self) -> Option<Persona> {
        self.store.list_enabled().into_iter().next()
    }

    /// Returns the persona store for direct access.
    pub fn store(&self) -> &PersonaStore {
        &self.store
    }

    /// Returns the ID of the active persona.
    pub fn active_persona_id(&self) -> Option<String> {
        self.active_persona_id.read().clone()
    }

    /// RFC-048 §4: filter Foundation shared packages by persona
    /// compatibility. A package that declares a persona hint is offered
    /// only when the active persona's id or name matches it
    /// (case-insensitive); packages without a hint are generally
    /// available. This keeps package content selective — an agent's
    /// prompt never receives every shared SKILL.md.
    pub fn compatible_foundation_packages(
        &self,
        packages: &[crate::foundation::packages::ImportedPackage],
    ) -> Vec<crate::foundation::packages::ImportedPackage> {
        let active = self.get_active_persona();
        let matches = |hint: &str| {
            active.as_ref().is_some_and(|p| {
                p.id.eq_ignore_ascii_case(hint) || p.name.eq_ignore_ascii_case(hint)
            })
        };
        packages
            .iter()
            .filter(|pkg| pkg.persona.as_deref().is_none_or(&matches))
            .cloned()
            .collect()
    }
    // ── RFC-039 ────────────────────────────────────────────────────────────

    /// Active persona 의 우선순위 결정:
    ///   1. StateStore `index.json` 의 `active_persona_id` (enabled 면 적용)
    ///   2. `PersonaConfig.default_persona_id` (enabled 면 적용)
    ///   3. store 의 첫 번째 enabled
    ///   4. None
    ///
    /// `&self` — `Arc<PersonaManager>` 뒤에서 호출됨.
    pub fn apply_config(&self, cfg: &crate::config::PersonaConfig) {
        // 우선순위 1: 기존 active_persona_id 가 enabled 면 유지
        // (load_from_state_store 가 이미 StateStore 의 active_persona_id 를 박았음).
        if let Some(id) = self.active_persona_id()
            && self.store.get(&id).map(|p| p.enabled).unwrap_or(false)
        {
            return;
        }
        // 우선순위 2: config.default_persona_id
        if let Some(id) = cfg.default_persona_id.as_ref()
            && self.store.get(id).map(|p| p.enabled).unwrap_or(false)
        {
            *self.active_persona_id.write() = Some(id.clone());
            return;
        }
        // 우선순위 3: 첫 번째 enabled
        if let Some(p) = self.store.list_enabled().into_iter().next() {
            *self.active_persona_id.write() = Some(p.id);
        }
    }

    /// StateStore 에서 페르소나 + active_persona_id 를 로드.
    /// 손상·부재 시 silent fallback 하지 않고 `Result::Err` 로 전파.
    /// 호출자는 defaults 가 이미 new() 로 박혀 있음을 알고 있어야 함.
    pub async fn load_from_state_store(&self, store: &StateStore) -> Result<()> {
        let snap = crate::persona::persistence::load_from_state_store(store)
            .await?
            .ok_or_else(|| anyhow::anyhow!("persona: no snapshot present"))?;
        // store 가 이미 기본 페르소나를 들고 있어도 디스크가 우선.
        for p in &snap.personas {
            self.store.register(p.clone());
        }
        // Vault-of-record backfill: v2 records persisted before the
        // category taxonomy exist load with the serde default ("general").
        // The nine legacy default ids get their intended category so the
        // picker/groups are correct on the first boot after upgrade.
        // User-created personas are never touched.
        let defaults = default_personas();
        for p in &snap.personas {
            if p.category == "general"
                && let Some(d) = defaults.iter().find(|d| d.id == p.id)
                && d.category != "general"
                && let Some(mut updated) = self.store.get(&p.id)
            {
                updated.category = d.category.clone();
                let _ = self.store.update(&p.id, updated);
            }
        }
        if let Some(active) = snap.active_persona_id
            && snap.personas.iter().any(|p| p.id == active && p.enabled)
        {
            *self.active_persona_id.write() = Some(active);
        }
        Ok(())
    }
    /// `state_store` 가 설정되지 않은 경우 no-op (`Ok(())`).
    /// 메모리 상태는 유지, IO 실패는 `Result::Err` 로 전파.
    pub async fn persist(&self) -> Result<()> {
        let Some(ref store) = self.state_store else {
            return Ok(());
        };
        let snapshot = crate::persona::persistence::PersonaSnapshot {
            schema_version: crate::persona::persistence::SCHEMA_VERSION,
            active_persona_id: self.active_persona_id(),
            personas: self.store.list_all(),
        };
        crate::persona::persistence::save_to_state_store(store, &snapshot).await
    }

    /// 글로벌 활성 페르소나 변경.
    ///
    /// 단일 진리 경로: 슬롯 변경 → persist (내부 StateStore) → reseed (callback).
    /// 모든 호출자 (HTTP, PersonaTool, Gateway, delete handler) 가 이 메서드를
    /// 거치므로 영속화 누락이나 intent engine 미재시드가 발생하지 않는다.
    ///
    /// 새 system_prompt 를 `Ok(Some(prompt))` 로 반환.
    ///
    /// `&self` — interior mutability (Arc 뒤 호출 가능).
    pub async fn set_active(&self, id: &str) -> Result<Option<String>> {
        let persona = self
            .store
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("Persona '{id}' not found"))?;
        if !persona.enabled {
            anyhow::bail!("Persona '{id}' is disabled");
        }
        let prev = self.active_persona_id.read().clone();
        *self.active_persona_id.write() = Some(id.to_string());
        tracing::info!(persona_id = %id, name = %persona.name, "Active persona set");

        // Persist then re-seed. On IO failure, roll back the slot and
        // propagate so the in-memory state never silently diverges from
        // disk — consistent with the create/update/delete handlers, which
        // also propagate persist errors.
        if let Err(e) = self.persist().await {
            *self.active_persona_id.write() = prev;
            return Err(e);
        }

        let prompt = persona.system_prompt.clone();
        if let Some(cb) = self.reseed_callback.read().as_ref() {
            cb(Some(prompt.clone()));
        }
        Ok(Some(prompt))
    }
}

impl Default for PersonaManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for PersonaManager {
    fn clone(&self) -> Self {
        let personas: Vec<Persona> = self.store.list_all();
        let store = PersonaStore::new();
        store.load_from_slice(&personas);
        Self {
            store,
            active_persona_id: RwLock::new(self.active_persona_id.read().clone()),
            // Arc clone — shares the same underlying store/callback.
            state_store: self.state_store.clone(),
            reseed_callback: RwLock::new(self.reseed_callback.read().clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PersonaConfig;

    fn make_store() -> Arc<StateStore> {
        let dir = tempfile::tempdir().unwrap();
        Arc::new(StateStore::new(dir.keep()).unwrap())
    }

    #[tokio::test]
    async fn test_load_from_state_store_round_trip() {
        let store = make_store();
        let pm = PersonaManager::new().with_state_store(store.clone());
        // Create a custom persona, persist, then load into a fresh manager.
        let custom = Persona {
            id: "custom-1".to_string(),
            name: "Custom".to_string(),
            role: "custom".to_string(),
            description: "Custom test persona".to_string(),
            system_prompt: "You are Custom.".to_string(),
            enabled: true,
            model: None,
            personality_traits: vec![],
            capabilities: vec![],
            category: "general".to_string(),
            genre: None,
            default_mount_ids: vec![],
        };
        pm.store().register(custom);
        pm.set_active("custom-1").await.unwrap();
        pm.persist().await.unwrap();

        // Fresh manager — load from disk.
        let pm2 = PersonaManager::new().with_state_store(store.clone());
        pm2.load_from_state_store(&store).await.unwrap();
        assert!(pm2.store().get("custom-1").is_some());
        assert_eq!(pm2.active_persona_id(), Some("custom-1".to_string()));
        // Defaults should also still be present (new() created them).
        assert!(pm2.store().get("dev").is_some());
    }

    #[tokio::test]
    async fn test_load_backfills_categories_for_legacy_defaults() {
        let store = make_store();
        // Simulate a v2 file persisted before the taxonomy: the default
        // personas carry no category (serde default "general") and a
        // user-created persona must stay untouched.
        let mut legacy = default_personas();
        for p in &mut legacy {
            p.category = "general".to_string();
        }
        let mut user_made = Persona::new("Code", "coder", "user-made", "You write code.");
        user_made.id = "user-code".to_string();
        legacy.push(user_made);
        crate::persona::persistence::save_to_state_store(
            &store,
            &crate::persona::persistence::PersonaSnapshot {
                schema_version: crate::persona::persistence::SCHEMA_VERSION,
                active_persona_id: Some("dev".to_string()),
                personas: legacy,
            },
        )
        .await
        .unwrap();

        let pm = PersonaManager::new().with_state_store(store.clone());
        pm.load_from_state_store(&store).await.unwrap();

        // Legacy defaults regained their intended categories…
        assert_eq!(pm.store().get("dev").unwrap().category, "coding");
        assert_eq!(pm.store().get("writer").unwrap().category, "writing");
        assert_eq!(pm.store().get("ops").unwrap().category, "operations");
        // …architect/mentor/planner legitimately stay general…
        assert_eq!(pm.store().get("architect").unwrap().category, "general");
        // …and user-created personas are never rewritten.
        assert_eq!(pm.store().get("user-code").unwrap().category, "general");
        assert_eq!(pm.store().get("user-code").unwrap().name, "Code");
    }

    #[tokio::test]
    async fn test_load_no_file_is_ok() {
        let store = make_store();
        let pm = PersonaManager::new().with_state_store(store.clone());
        let result = pm.load_from_state_store(&store).await;
        // No file → Err (we require a snapshot). But defaults from new() remain.
        assert!(result.is_err());
        assert!(pm.store().get("dev").is_some());
    }

    #[tokio::test]
    async fn test_apply_config_default_persona_id() {
        let pm = PersonaManager::new();
        // Clear active so apply_config must pick from config.
        *pm.active_persona_id.write() = None;
        let cfg = PersonaConfig {
            default_persona_id: Some("review".to_string()),
        };
        pm.apply_config(&cfg);
        assert_eq!(pm.active_persona_id(), Some("review".to_string()));
    }

    #[tokio::test]
    async fn test_apply_config_falls_back_to_first_enabled() {
        let pm = PersonaManager::new();
        *pm.active_persona_id.write() = None;
        let cfg = PersonaConfig {
            default_persona_id: None,
        };
        pm.apply_config(&cfg);
        // First enabled persona is "dev" (order: dev, review, research).
        assert_eq!(pm.active_persona_id(), Some("dev".to_string()));
    }

    #[tokio::test]
    async fn test_apply_config_skips_disabled_default() {
        let pm = PersonaManager::new();
        *pm.active_persona_id.write() = None;
        // "review" is enabled by default; disable it.
        pm.store().set_enabled("review", false).unwrap();
        let cfg = PersonaConfig {
            default_persona_id: Some("review".to_string()),
        };
        pm.apply_config(&cfg);
        // review is disabled → fall back to first enabled = dev.
        assert_eq!(pm.active_persona_id(), Some("dev".to_string()));
    }

    #[tokio::test]
    async fn test_apply_config_keeps_existing_active() {
        let pm = PersonaManager::new();
        pm.set_active("research").await.unwrap();
        let cfg = PersonaConfig {
            default_persona_id: Some("dev".to_string()),
        };
        pm.apply_config(&cfg);
        // Existing active (research, enabled) wins over config default.
        assert_eq!(pm.active_persona_id(), Some("research".to_string()));
    }

    #[tokio::test]
    async fn test_set_active_rejects_disabled() {
        let pm = PersonaManager::new();
        pm.store().set_enabled("review", false).unwrap();
        let result = pm.set_active("review").await;
        assert!(result.is_err());
        assert_eq!(pm.active_persona_id(), Some("dev".to_string()));
    }

    #[tokio::test]
    async fn test_set_active_rejects_unknown_id() {
        let pm = PersonaManager::new();
        let result = pm.set_active("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_set_active_returns_system_prompt() {
        let pm = PersonaManager::new();
        let prompt = pm.set_active("review").await.unwrap();
        assert!(prompt.is_some());
        assert!(prompt.unwrap().contains("Review"));
    }

    #[tokio::test]
    async fn test_set_active_fires_reseed_callback() {
        let pm = PersonaManager::new();
        let received = Arc::new(parking_lot::Mutex::new(None::<String>));
        let received_cb = received.clone();
        pm.set_reseed_callback(Some(Arc::new(move |prompt| {
            *received_cb.lock() = prompt;
        })));
        pm.set_active("review").await.unwrap();
        assert!(received.lock().as_ref().unwrap().contains("Review"));
    }

    #[tokio::test]
    async fn test_set_active_no_callback_still_works() {
        let pm = PersonaManager::new();
        // No callback set — should not panic.
        let result = pm.set_active("research").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_set_active_persists_when_store_set() {
        let store = make_store();
        let pm = PersonaManager::new().with_state_store(store.clone());
        pm.set_active("research").await.unwrap();

        // Fresh manager loads the persisted active.
        let pm2 = PersonaManager::new().with_state_store(store.clone());
        pm2.load_from_state_store(&store).await.unwrap();
        assert_eq!(pm2.active_persona_id(), Some("research".to_string()));
    }

    #[tokio::test]
    async fn test_set_active_no_persist_without_store() {
        let pm = PersonaManager::new();
        // No state_store — persist is no-op, should succeed.
        pm.set_active("research").await.unwrap();
        assert_eq!(pm.active_persona_id(), Some("research".to_string()));
    }

    #[tokio::test]
    async fn test_persist_round_trip_preserves_active() {
        let store = make_store();
        let pm = PersonaManager::new().with_state_store(store.clone());
        pm.set_active("research").await.unwrap();
        pm.persist().await.unwrap();

        let pm2 = PersonaManager::new().with_state_store(store.clone());
        pm2.load_from_state_store(&store).await.unwrap();
        assert_eq!(pm2.active_persona_id(), Some("research".to_string()));
    }

    #[tokio::test]
    async fn test_clone_preserves_state_store_and_callback() {
        let store = make_store();
        let pm = PersonaManager::new().with_state_store(store.clone());
        let received = Arc::new(parking_lot::Mutex::new(None::<String>));
        let received_cb = received.clone();
        pm.set_reseed_callback(Some(Arc::new(move |prompt| {
            *received_cb.lock() = prompt;
        })));

        let cloned = pm.clone();
        // Cloned manager should persist and reseed via the shared Arcs.
        cloned.set_active("research").await.unwrap();
        assert!(received.lock().is_some());
    }

    // ─── RFC-048 §4: Foundation package persona filtering ──────────

    fn pkg(id: &str, persona: Option<&str>) -> crate::foundation::packages::ImportedPackage {
        crate::foundation::packages::ImportedPackage {
            id: id.into(),
            version: "0.1.0".into(),
            source: format!("local://./{id}"),
            digest: "0".repeat(64),
            trust: crate::foundation::packages::SourceTrust::Unsigned,
            targets: vec!["oxios".into()],
            capabilities: vec![],
            persona: persona.map(String::from),
        }
    }

    #[tokio::test]
    async fn compatible_foundation_packages_filters_by_active_persona() {
        let pm = PersonaManager::new();
        // Default active persona is "dev".
        pm.set_active("dev").await.unwrap();
        let packages = vec![
            pkg("oxi.general", None),
            pkg("oxi.dev-tool", Some("dev")),
            pkg("oxi.research-kit", Some("research")),
        ];
        let compatible = pm.compatible_foundation_packages(&packages);
        let ids: Vec<&str> = compatible.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["oxi.general", "oxi.dev-tool"]);

        // Switching the active persona exposes the research kit instead.
        pm.set_active("research").await.unwrap();
        let compatible = pm.compatible_foundation_packages(&packages);
        let ids: Vec<&str> = compatible.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["oxi.general", "oxi.research-kit"]);
    }
}
