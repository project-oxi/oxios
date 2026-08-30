//! Marketplace API routes — ClawHub + Skills.sh search, skills, install, updates.
//!
//! Route groups:
//! - `/api/marketplace/*` — ClawHub (legacy, backward compatible)
//! - `/api/marketplace/skills-sh/*` — Skills.sh (Vercel Labs ecosystem)

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
};

use oxios_kernel::{ClawHubSearchResult, ClawHubSkillDetail};

use crate::api::error::AppError;
use crate::api::server::AppState;

// ─── Query types ─────────────────────────────────────────────────────────────

/// Query params for marketplace search.
#[derive(Debug, serde::Deserialize)]
pub struct SearchQuery {
    /// Search query string.
    pub q: String,
    /// Max results to return.
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    20
}

/// Query params for skills.sh listing.
#[derive(Debug, serde::Deserialize)]
pub struct SkillsShListQuery {
    /// Leaderboard view: `"all-time"`, `"trending"`, or `"hot"`.
    #[serde(default = "default_view")]
    pub view: String,
    /// 0-indexed page number.
    #[serde(default)]
    pub page: Option<i64>,
    /// Results per page (1–500).
    #[serde(default = "default_per_page")]
    pub per_page: i64,
}

fn default_view() -> String {
    "all-time".to_string()
}

fn default_per_page() -> i64 {
    50
}

/// Request body for installing a ClawHub skill.
#[derive(Debug, serde::Deserialize)]
pub struct InstallBody {
    /// Specific version to install (None = latest).
    pub version: Option<String>,
}

// ─── ClawHub Handlers ────────────────────────────────────────────────────────

/// GET /api/marketplace/search — Search ClawHub for skills.
pub(crate) async fn handle_marketplace_search(
    state: State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<ClawHubSearchResult>>, AppError> {
    let results = state
        .kernel
        .marketplace_api()
        .search(&query.q, Some(query.limit))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(results))
}

/// GET /api/marketplace/skills/{slug} — Get skill detail from ClawHub.
pub(crate) async fn handle_marketplace_skill_detail(
    state: State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> Result<Json<ClawHubSkillDetail>, AppError> {
    let detail = state
        .kernel
        .marketplace_api()
        .get_skill(&slug)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(detail))
}

/// POST /api/marketplace/skills/{slug}/install — Install a skill from ClawHub.
pub(crate) async fn handle_marketplace_install(
    state: State<Arc<AppState>>,
    Path(slug): Path<String>,
    Json(body): Json<InstallBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let result = state
        .kernel
        .marketplace_api()
        .install(&slug, body.version.as_deref())
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(serde_json::json!({
        "ok": result.ok,
        "slug": result.slug,
        "version": result.version,
        "targetDir": result.target_dir.to_string_lossy(),
        "changelog": result.changelog,
    })))
}

/// GET /api/marketplace/updates — Check for updates to installed ClawHub skills.
pub(crate) async fn handle_marketplace_updates(
    state: State<Arc<AppState>>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let updates = state
        .kernel
        .marketplace_api()
        .check_updates()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let results: Vec<serde_json::Value> = updates
        .into_iter()
        .map(|u| {
            serde_json::json!({
                "slug": u.slug,
                "currentVersion": u.current_version,
                "latestVersion": u.latest_version,
                "changelog": u.changelog,
            })
        })
        .collect();

    Ok(Json(results))
}

// ─── Skills.sh Handlers ──────────────────────────────────────────────────────

/// GET /api/marketplace/skills-sh/search — Search skills.sh for skills.
///
/// Single-word queries use fuzzy matching. Multi-word queries use semantic search.
pub(crate) async fn handle_skills_sh_search(
    state: State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    skills_sh_guard(&state)?;
    let resp = state
        .kernel
        .marketplace_api()
        .search_skills_sh(&query.q, Some(query.limit))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(serde_json::json!({
        "data": resp.data,
        "query": resp.query,
        "searchType": resp.search_type,
        "count": resp.count,
        "durationMs": resp.duration_ms,
    })))
}

/// GET /api/marketplace/skills-sh/list — List skills from skills.sh leaderboard.
///
/// Query params: `view` (all-time|trending|hot), `page`, `per_page`.
pub(crate) async fn handle_skills_sh_list(
    state: State<Arc<AppState>>,
    Query(query): Query<SkillsShListQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    skills_sh_guard(&state)?;
    let resp = state
        .kernel
        .marketplace_api()
        .list_skills_sh(Some(&query.view), query.page, Some(query.per_page))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(serde_json::json!({
        "data": resp.data,
        "pagination": {
            "page": resp.pagination.page,
            "perPage": resp.pagination.per_page,
            "total": resp.pagination.total,
            "hasMore": resp.pagination.has_more,
        },
    })))
}

/// GET /api/marketplace/skills-sh/skill/{id} — Get skill detail from skills.sh.
///
/// The `{id}` is the full skill identifier, e.g. `vercel-labs/agent-skills/frontend-design`.
pub(crate) async fn handle_skills_sh_skill_detail(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    skills_sh_guard(&state)?;
    let detail = state
        .kernel
        .marketplace_api()
        .get_skills_sh_skill(&id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(serde_json::to_value(&detail).unwrap_or_default()))
}

/// GET /api/marketplace/skills-sh/skill/{id}/audit — Get security audit for a skills.sh skill.
pub(crate) async fn handle_skills_sh_skill_audit(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    skills_sh_guard(&state)?;
    let audit = state
        .kernel
        .marketplace_api()
        .audit_skills_sh(&id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(serde_json::to_value(&audit).unwrap_or_default()))
}

/// POST /api/marketplace/skills-sh/skill/{id}/install — Install a skill from skills.sh.
pub(crate) async fn handle_skills_sh_install(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    skills_sh_guard(&state)?;
    let result = state
        .kernel
        .marketplace_api()
        .install_skills_sh(&id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(serde_json::json!({
        "ok": result.ok,
        "slug": result.slug,
        "source": result.source,
        "skillId": result.skill_id,
        "targetDir": result.target_dir.to_string_lossy(),
        "fileCount": result.file_count,
        "installs": result.installs,
        "hash": result.hash,
    })))
}

/// Fail fast with a descriptive 503 when skills.sh has no API key.
///
/// The upstream requires a Bearer token on every endpoint; without one each
/// request is a guaranteed 401 that previously surfaced as an opaque 500.
fn skills_sh_guard(state: &State<Arc<AppState>>) -> Result<(), AppError> {
    if state.kernel.marketplace_api().skills_sh_configured() {
        Ok(())
    } else {
        Err(AppError::ServiceUnavailable(
            "skills.sh requires an API key. Set `api_key` under `[skills_sh]` in config.toml \
             or export SKILLS_SH_TOKEN. Request a key from skills-api@vercel.com."
                .into(),
        ))
    }
}
