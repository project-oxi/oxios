//! Persona persistence: load/save the full registry + active_persona_id to
//! `StateStore` (RFC-039).
//!
//! File path: `~/.oxios/state/personas/index.json`
//!
//! Schema (schema_version = 2):
//! ```json
//! {
//!   "schema_version": 2,
//!   "active_persona_id": "dev",
//!   "personas": [ { "id": "...", "name": "...", "role": "...",
//!                   "description": "...", "system_prompt": "...",
//!                   "enabled": true, "model": null,
//!                   "personality_traits": [],
//!                   "capabilities": [] }, ... ]
//! }
//! ```
//! Backward compat: v1 files (no `capabilities` field) load with empty
//! capabilities via `#[serde(default)]` on the `Persona.capabilities` field.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::Persona;
use crate::state_store::StateStore;

pub(crate) const SCHEMA_VERSION: u32 = 2;
/// Oldest schema version still accepted on load (for backward compat).
const MIN_SCHEMA_VERSION: u32 = 1;

/// Serializable snapshot of the persona registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaSnapshot {
    pub schema_version: u32,
    /// Active persona ID. `None` if no persona is active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_persona_id: Option<String>,
    /// All personas (enabled and disabled). Empty Vec is allowed.
    pub personas: Vec<Persona>,
}

/// Load the snapshot from `StateStore`. `Ok(None)` if file is absent.
pub async fn load_from_state_store(store: &StateStore) -> Result<Option<PersonaSnapshot>> {
    let raw: Option<PersonaSnapshot> = store
        .load_json("personas", "index")
        .await
        .context("persona: failed to load index.json")?;
    let Some(snap) = raw else { return Ok(None) };
    if snap.schema_version < MIN_SCHEMA_VERSION || snap.schema_version > SCHEMA_VERSION {
        anyhow::bail!(
            "persona: schema_version {} not supported (expected {}-{})",
            snap.schema_version,
            MIN_SCHEMA_VERSION,
            SCHEMA_VERSION
        );
    }
    // v1 → v2: capabilities field defaults to empty via serde(default).
    // No explicit migration needed — old personas load with [] capabilities.
    Ok(Some(snap))
}

/// Save the snapshot to `StateStore`. Atomic via `StateStore::durable_write`.
pub async fn save_to_state_store(store: &StateStore, snapshot: &PersonaSnapshot) -> Result<()> {
    store
        .save_json("personas", "index", snapshot)
        .await
        .context("persona: failed to save index.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::Persona;

    fn make_persona(id: &str, name: &str, role: &str, enabled: bool) -> Persona {
        Persona {
            id: id.to_string(),
            name: name.to_string(),
            role: role.to_string(),
            description: format!("Test persona {name}"),
            system_prompt: format!("You are {name}."),
            enabled,
            model: None,
            personality_traits: vec!["curious".to_string()],
            capabilities: vec![],
            category: "general".to_string(),
            genre: None,
            default_mount_ids: vec![],
        }
    }

    fn make_store() -> StateStore {
        let dir = tempfile::tempdir().unwrap();
        StateStore::new(dir.keep()).unwrap()
    }

    #[tokio::test]
    async fn test_round_trip() {
        let store = make_store();
        let snap = PersonaSnapshot {
            schema_version: SCHEMA_VERSION,
            active_persona_id: Some("dev".to_string()),
            personas: vec![
                make_persona("dev", "Dev", "developer", true),
                make_persona("qa", "Review", "qa", true),
            ],
        };
        save_to_state_store(&store, &snap).await.unwrap();
        let loaded = load_from_state_store(&store).await.unwrap().unwrap();
        assert_eq!(loaded.schema_version, SCHEMA_VERSION);
        assert_eq!(loaded.active_persona_id, Some("dev".to_string()));
        assert_eq!(loaded.personas.len(), 2);
        assert_eq!(loaded.personas[0].id, "dev");
        assert_eq!(loaded.personas[1].role, "qa");
    }

    #[tokio::test]
    async fn test_v1_backward_compat_loads() {
        // A v1 snapshot (no capabilities field) should load successfully
        // with empty capabilities via #[serde(default)].
        let store = make_store();
        let snap = PersonaSnapshot {
            schema_version: 1,
            active_persona_id: Some("dev".to_string()),
            personas: vec![make_persona("dev", "Dev", "developer", true)],
        };
        save_to_state_store(&store, &snap).await.unwrap();
        let loaded = load_from_state_store(&store).await.unwrap().unwrap();
        assert_eq!(loaded.schema_version, 1);
        assert_eq!(loaded.personas[0].capabilities, Vec::<String>::new());
    }

    #[tokio::test]
    async fn test_no_file_returns_none() {
        let store = make_store();
        let loaded = load_from_state_store(&store).await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_schema_version_mismatch_errors() {
        let store = make_store();
        let snap = PersonaSnapshot {
            schema_version: 99,
            active_persona_id: None,
            personas: vec![],
        };
        save_to_state_store(&store, &snap).await.unwrap();
        let result = load_from_state_store(&store).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("schema_version"));
    }

    #[tokio::test]
    async fn test_empty_personas_round_trip() {
        let store = make_store();
        let snap = PersonaSnapshot {
            schema_version: SCHEMA_VERSION,
            active_persona_id: None,
            personas: vec![],
        };
        save_to_state_store(&store, &snap).await.unwrap();
        let loaded = load_from_state_store(&store).await.unwrap().unwrap();
        assert!(loaded.personas.is_empty());
        assert!(loaded.active_persona_id.is_none());
    }

    #[tokio::test]
    async fn test_overwrite_on_save() {
        let store = make_store();
        let snap1 = PersonaSnapshot {
            schema_version: SCHEMA_VERSION,
            active_persona_id: Some("dev".to_string()),
            personas: vec![make_persona("dev", "Dev", "developer", true)],
        };
        save_to_state_store(&store, &snap1).await.unwrap();
        let snap2 = PersonaSnapshot {
            schema_version: SCHEMA_VERSION,
            active_persona_id: Some("qa".to_string()),
            personas: vec![
                make_persona("dev", "Dev", "developer", true),
                make_persona("qa", "Review", "qa", true),
            ],
        };
        save_to_state_store(&store, &snap2).await.unwrap();
        let loaded = load_from_state_store(&store).await.unwrap().unwrap();
        assert_eq!(loaded.active_persona_id, Some("qa".to_string()));
        assert_eq!(loaded.personas.len(), 2);
    }
}
