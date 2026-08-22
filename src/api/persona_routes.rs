//! Persona API routes: CRUD and active persona management.
//!
//! RFC-039: every mutation (`create`/`update`/`delete`/`set_active`) calls
//! `PersonaApi::persist` after the in-memory change so the on-disk state at
//! `~/.oxios/state/personas/index.json` matches the in-memory registry.
//! HTTP path intentionally skips the LLM judge (`security_review`) that
//! `PersonaTool` uses — that asymmetry is documented in
//! `docs/rfc-039-persona-completion.md` §3.9.
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::api::server::AppState;

// ---------------------------------------------------------------------------
// Personas
// ---------------------------------------------------------------------------

/// Persona summary for listing.
#[derive(Debug, Serialize)]
pub struct PersonaSummary {
    id: String,
    name: String,
    role: String,
    description: String,
    enabled: bool,
    personality_traits: Vec<String>,
    /// RFC-044 §8.2 capability flags driving UI affordances.
    capabilities: Vec<String>,
    /// UI taxonomy bucket (normal | coding | writing | research |
    /// operations | general; free string).
    category: String,
    /// Writing sub-category (novel | scenario | essay | blog).
    #[serde(skip_serializing_if = "Option::is_none")]
    genre: Option<String>,
    /// Mount IDs auto-attached by the chat composer when selected.
    default_mount_ids: Vec<String>,
}

/// GET /api/personas — List all personas.
pub async fn handle_personas_list(state: State<Arc<AppState>>) -> Json<Vec<PersonaSummary>> {
    let personas = state.kernel.persona.list();
    Json(
        personas
            .into_iter()
            .map(|p| PersonaSummary {
                id: p.id,
                name: p.name,
                role: p.role,
                description: p.description,
                enabled: p.enabled,
                personality_traits: p.personality_traits,
                capabilities: p.capabilities,
                category: p.category,
                genre: p.genre,
                default_mount_ids: p.default_mount_ids,
            })
            .collect(),
    )
}

/// GET /api/personas/:id — Get a specific persona.
pub async fn handle_persona_get(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match state.kernel.persona.get(&id) {
        Some(p) => Ok(Json(serde_json::json!({
            "id": p.id,
            "name": p.name,
            "role": p.role,
            "description": p.description,
            "system_prompt": p.system_prompt,
            "enabled": p.enabled,
            "model": p.model,
            "personality_traits": p.personality_traits,
            "capabilities": p.capabilities,
            "category": p.category,
            "genre": p.genre,
            "default_mount_ids": p.default_mount_ids,
        }))),
        None => Err(StatusCode::NOT_FOUND),
    }
}
/// Request body for creating a persona.
#[derive(Debug, Deserialize)]
pub struct PersonaCreateRequest {
    name: String,
    #[serde(default = "default_role")]
    role: String,
    description: String,
    system_prompt: String,
    #[serde(default = "default_true")]
    enabled: bool,
    model: Option<String>,
    #[serde(default)]
    personality_traits: Vec<String>,
    #[serde(default)]
    capabilities: Option<Vec<String>>,
    /// UI taxonomy bucket. Defaults to `general` when absent.
    #[serde(default)]
    category: Option<String>,
    /// Writing sub-category (novel | scenario | essay | blog).
    #[serde(default)]
    genre: Option<String>,
    /// Mount IDs auto-attached by the chat composer when selected.
    #[serde(default)]
    default_mount_ids: Option<Vec<String>>,
}

fn default_true() -> bool {
    true
}

fn default_category() -> String {
    "general".to_string()
}

fn default_role() -> String {
    "assistant".to_string()
}

/// POST /api/personas — Create a new persona.
pub async fn handle_persona_create(
    state: State<Arc<AppState>>,
    Json(body): Json<PersonaCreateRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    use oxios_kernel::Persona;
    let persona = Persona {
        id: uuid::Uuid::new_v4().to_string(),
        name: body.name,
        role: body.role,
        description: body.description,
        system_prompt: body.system_prompt,
        enabled: body.enabled,
        model: body.model,
        personality_traits: body.personality_traits,
        capabilities: body.capabilities.unwrap_or_default(),
        category: body.category.unwrap_or_else(default_category),
        genre: body.genre,
        default_mount_ids: body.default_mount_ids.unwrap_or_default(),
    };
    let created_id = persona.id.clone();
    let created_name = persona.name.clone();
    state.kernel.persona.create(persona);
    // RFC-039: persist so the new persona survives restart.
    state
        .kernel
        .persona
        .persist()
        .await
        .map_err(|e: anyhow::Error| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    tracing::info!(persona = %created_name, "Persona created via API + persisted");
    Ok(Json(serde_json::json!({
        "status": "created",
        "id": created_id,
        "name": created_name,
    })))
}

/// Request body for updating a persona.
#[derive(Debug, Deserialize)]
pub struct PersonaUpdateRequest {
    name: Option<String>,
    role: Option<String>,
    description: Option<String>,
    system_prompt: Option<String>,
    enabled: Option<bool>,
    model: Option<String>,
    personality_traits: Option<Vec<String>>,
    capabilities: Option<Vec<String>>,
    category: Option<String>,
    genre: Option<String>,
    default_mount_ids: Option<Vec<String>>,
}

/// PUT /api/personas/:id — Update a persona.
pub async fn handle_persona_update(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PersonaUpdateRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    use oxios_kernel::Persona;
    let existing = state
        .kernel
        .persona
        .get(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Persona '{id}' not found")))?;

    let updated = Persona {
        id: existing.id,
        name: body.name.unwrap_or(existing.name),
        role: body.role.unwrap_or(existing.role),
        description: body.description.unwrap_or(existing.description),
        system_prompt: body.system_prompt.unwrap_or(existing.system_prompt),
        enabled: body.enabled.unwrap_or(existing.enabled),
        model: body.model.or(existing.model),
        personality_traits: body
            .personality_traits
            .unwrap_or(existing.personality_traits),
        capabilities: body.capabilities.unwrap_or(existing.capabilities),
        category: body.category.unwrap_or(existing.category),
        genre: body.genre.or(existing.genre),
        default_mount_ids: body.default_mount_ids.unwrap_or(existing.default_mount_ids),
    };

    state
        .kernel
        .persona
        .update(&id, updated)
        .map_err(|e: anyhow::Error| (StatusCode::BAD_REQUEST, e.to_string()))?;
    // RFC-039: persist so the edit survives restart.
    state
        .kernel
        .persona
        .persist()
        .await
        .map_err(|e: anyhow::Error| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    tracing::info!(persona_id = %id, "Persona updated via API + persisted");
    Ok(Json(serde_json::json!({
        "status": "updated",
        "id": id,
    })))
}

/// DELETE /api/personas/:id — Delete a persona.
pub async fn handle_persona_delete(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Prevent deleting the last persona.
    if state.kernel.persona.count() <= 1 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Cannot delete the last persona".to_string(),
        ));
    }

    state
        .kernel
        .persona
        .delete(&id)
        .map_err(|e: anyhow::Error| (StatusCode::NOT_FOUND, e.to_string()))?;

    // If deleted persona was active, clear the active reference.
    if let Some(active) = state.kernel.persona.active()
        && active.id == id
    {
        // Try to set another persona as active.
        if let Some(next) = state.kernel.persona.list_enabled().into_iter().next() {
            let _ = state.kernel.persona.set_active(&next.id).await;
        }
    }
    // RFC-039: persist so the delete survives restart.
    state
        .kernel
        .persona
        .persist()
        .await
        .map_err(|e: anyhow::Error| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(persona_id = %id, "Persona deleted via API + persisted");
    Ok(Json(serde_json::json!({
        "status": "deleted",
        "id": id,
    })))
}

/// GET /api/personas/active — Get the currently active persona.
pub async fn handle_persona_active_get(state: State<Arc<AppState>>) -> Json<serde_json::Value> {
    match state.kernel.persona.active() {
        Some(p) => Json(serde_json::json!({
            "id": p.id,
            "name": p.name,
            "role": p.role,
            "description": p.description,
            "system_prompt": p.system_prompt,
            "enabled": p.enabled,
            "capabilities": p.capabilities,
        })),
        None => Json(serde_json::json!({
            "active": false,
            "message": "No active persona set"
        })),
    }
}

/// Request body for setting active persona.
#[derive(Debug, Deserialize)]
pub struct PersonaActiveRequest {
    id: String,
}

/// PUT /api/personas/active — Set the active persona.
pub async fn handle_persona_active_set(
    state: State<Arc<AppState>>,
    Json(body): Json<PersonaActiveRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let new_prompt = state
        .kernel
        .persona
        .set_active(&body.id)
        .await
        .map_err(|e: anyhow::Error| (StatusCode::BAD_REQUEST, e.to_string()))?
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "no prompt returned".to_string(),
            )
        })?;
    tracing::info!(
        persona_id = %body.id,
        prompt_len = new_prompt.len(),
        "active persona changed; persisted + intent engine re-seeded automatically"
    );
    let persona = state.kernel.persona.active();
    Ok(Json(serde_json::json!({
        "status": "active",
        "id": body.id,
        "name": persona.map(|p| p.name).unwrap_or_default(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F1 regression: the list endpoint must echo back `capabilities`.
    /// The original bug was that the field never reached the client, so
    /// every capability-gated affordance (diff-viewer, worktree-fanout,
    /// terminal) was unreachable dead code. As long as `PersonaSummary`
    /// serializes the field, the compiler forces every construction site
    /// (e.g. `handle_personas_list`) to populate it — a missing field is a
    /// hard compile error, not a silent omission.
    #[test]
    fn persona_summary_serializes_capabilities() {
        let summary = PersonaSummary {
            id: "dev".into(),
            name: "Dev".into(),
            role: "developer".into(),
            description: "pragmatic dev".into(),
            enabled: true,
            personality_traits: vec!["pragmatic".into()],
            capabilities: vec!["terminal".into(), "diff-viewer".into()],
        };
        let json = serde_json::to_value(&summary).expect("serialize");
        let caps: Vec<String> = json
            .get("capabilities")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .expect("capabilities array missing from PersonaSummary JSON");
        assert_eq!(
            caps,
            vec!["terminal".to_string(), "diff-viewer".to_string()]
        );
    }
}
