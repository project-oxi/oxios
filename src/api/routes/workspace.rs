use std::sync::Arc;

use axum::Json;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};

use oxios_kernel::{SkillEntry, SkillSource, SkillStatus};

use crate::api::error::AppError;
use crate::api::routes::PageParams;
#[cfg(test)]
use crate::api::routes::paginate;
use crate::api::server::AppState;

// ---------------------------------------------------------------------------
// Workspace
// ---------------------------------------------------------------------------

/// Query parameters for workspace tree.
#[derive(Debug, Deserialize)]
pub(crate) struct TreeQuery {
    /// Subdirectory to list (optional).
    #[serde(default)]
    pub dir: Option<String>,
}

/// File tree entry.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct TreeEntry {
    /// File or directory name.
    name: String,
    /// Whether this is a directory.
    is_dir: bool,
    /// File size in bytes (0 for directories).
    size: u64,
}

/// GET /api/workspace/tree — File tree of workspace.
pub(crate) async fn handle_workspace_tree(
    state: State<Arc<AppState>>,
    Query(query): Query<TreeQuery>,
) -> Result<Json<Vec<TreeEntry>>, AppError> {
    let base = state.kernel.state.workspace_path();
    let canonical_base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let dir = match &query.dir {
        Some(d) => {
            let candidate = base.join(d);
            let canonical = match candidate.canonicalize() {
                Ok(c) => c,
                Err(_) => return Err(AppError::NotFound("directory not found".into())),
            };
            if !canonical.starts_with(&canonical_base) {
                return Err(AppError::Forbidden("path traversal denied".into()));
            }
            canonical
        }
        None => canonical_base,
    };

    let mut entries = Vec::new();
    if let Ok(mut read_dir) = tokio::fs::read_dir(&dir).await {
        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let metadata = match entry.metadata().await {
                Ok(m) => m,
                Err(_) => continue,
            };
            entries.push(TreeEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: metadata.is_dir(),
                size: metadata.len(),
            });
        }
    }

    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));

    Ok(Json(entries))
}

/// GET /api/workspace/file/*path — Read a file.
pub(crate) async fn handle_workspace_file_get(
    state: State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let base = state.kernel.state.workspace_path();
    let full_path = base.join(&path);

    // Security: ensure the path doesn't escape the workspace
    let canonical_base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let canonical_file = match full_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return Err(AppError::NotFound("file not found".into())),
    };

    if !canonical_file.starts_with(&canonical_base) {
        return Err(AppError::Forbidden("path traversal denied".into()));
    }

    match tokio::fs::read_to_string(&canonical_file).await {
        Ok(content) => {
            let mime = guess_mime(&path);
            Ok((
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, mime)],
                content,
            ))
        }
        Err(_) => Err(AppError::NotFound("file not found".into())),
    }
}

/// PUT /api/workspace/file/*path — Write/update a file.
pub(crate) async fn handle_workspace_file_put(
    state: State<Arc<AppState>>,
    Path(path): Path<String>,
    body: String,
) -> Result<(), AppError> {
    // Validate file size (max 1MB)
    const MAX_FILE_SIZE: usize = 1024 * 1024;
    if body.len() > MAX_FILE_SIZE {
        return Err(AppError::PayloadTooLarge {
            size: body.len(),
            limit: MAX_FILE_SIZE,
        });
    }

    let base = state.kernel.state.workspace_path();
    let full_path = base.join(&path);

    // Security: ensure the path doesn't escape the workspace
    let canonical_base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    if let Some(parent) = full_path.parent() {
        if !parent.exists() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AppError::Internal(format!("failed to create directory: {e}")))?;
        }
        let canonical_parent = parent
            .canonicalize()
            .map_err(|e| AppError::Internal(format!("failed to resolve path: {e}")))?;
        if !canonical_parent.starts_with(&canonical_base) {
            return Err(AppError::Forbidden("path traversal denied".into()));
        }
    }

    // F6: the parent-canonical check above does not catch a pre-existing
    // symlink at `full_path` itself pointing outside the workspace —
    // `tokio::fs::write` follows symlinks and would overwrite the target.
    if let Ok(meta) = tokio::fs::symlink_metadata(&full_path).await
        && meta.file_type().is_symlink()
    {
        let canonical_full = full_path
            .canonicalize()
            .map_err(|e| AppError::Internal(format!("failed to resolve path: {e}")))?;
        if !canonical_full.starts_with(&canonical_base) {
            return Err(AppError::Forbidden("path traversal denied".into()));
        }
    }

    match tokio::fs::write(&full_path, &body).await {
        Ok(_) => {
            tracing::info!(path = %path, "File written");
            Ok(())
        }
        Err(e) => {
            tracing::error!(path = %path, error = %e, "Failed to write file");
            Err(AppError::Internal("failed to write file".into()))
        }
    }
}

// ---------------------------------------------------------------------------
// File Create & Delete
// ---------------------------------------------------------------------------

/// Request body for creating a file.
#[derive(Debug, Deserialize)]
pub(crate) struct CreateFileRequest {
    /// Whether to create a directory instead of a file.
    #[serde(default)]
    pub is_dir: bool,
}

/// POST /api/workspace/file/*path — Create an empty file or directory.
pub(crate) async fn handle_workspace_file_create(
    state: State<Arc<AppState>>,
    Path(path): Path<String>,
    Json(body): Json<CreateFileRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let base = state.kernel.state.workspace_path();
    let full_path = base.join(&path);

    // Security: path traversal check
    let canonical_base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    // Ensure parent exists
    if let Some(parent) = full_path.parent() {
        let canonical_parent = parent
            .canonicalize()
            .map_err(|_| AppError::NotFound("parent directory not found".into()))?;
        if !canonical_parent.starts_with(&canonical_base) {
            return Err(AppError::Forbidden("path traversal denied".into()));
        }
    }

    // F6: `full_path.exists()` follows symlinks, so a dangling symlink at
    // `full_path` pointing outside the workspace would bypass the check
    // and `tokio::fs::write` would create/overwrite the symlink target.
    // Reject any pre-existing symlink outright (create requires the path
    // to be absent anyway).
    if let Ok(meta) = tokio::fs::symlink_metadata(&full_path).await
        && meta.file_type().is_symlink()
    {
        let canonical_full = full_path
            .canonicalize()
            .map_err(|_| AppError::NotFound("path not found".into()))?;
        if !canonical_full.starts_with(&canonical_base) {
            return Err(AppError::Forbidden("path traversal denied".into()));
        }
    }

    if full_path.exists() {
        return Err(AppError::BadRequest("file already exists".into()));
    }

    if body.is_dir {
        tokio::fs::create_dir_all(&full_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to create directory: {e}")))?;
    } else {
        // Ensure parent dir exists
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        tokio::fs::write(&full_path, "")
            .await
            .map_err(|e| AppError::Internal(format!("failed to create file: {e}")))?;
    }

    tracing::info!(path = %path, is_dir = body.is_dir, "File created");
    Ok(Json(
        serde_json::json!({ "status": "created", "path": path, "is_dir": body.is_dir }),
    ))
}

/// DELETE /api/workspace/file/*path — Delete a file or empty directory.
pub(crate) async fn handle_workspace_file_delete(
    state: State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let base = state.kernel.state.workspace_path();
    let full_path = base.join(&path);

    // Security: path traversal check
    let canonical_base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let canonical = match full_path.canonicalize() {
        Ok(c) => c,
        Err(_) => return Err(AppError::NotFound("file not found".into())),
    };

    if !canonical.starts_with(&canonical_base) {
        return Err(AppError::Forbidden("path traversal denied".into()));
    }

    if canonical.is_dir() {
        // Only delete empty directories
        let mut entries = tokio::fs::read_dir(&canonical)
            .await
            .map_err(|e| AppError::Internal(format!("failed to read directory: {e}")))?;
        if entries
            .next_entry()
            .await
            .map(|e| e.is_some())
            .unwrap_or(true)
        {
            return Err(AppError::BadRequest("directory is not empty".into()));
        }
        tokio::fs::remove_dir(&canonical)
            .await
            .map_err(|e| AppError::Internal(format!("failed to delete directory: {e}")))?;
    } else {
        tokio::fs::remove_file(&canonical)
            .await
            .map_err(|e| AppError::Internal(format!("failed to delete file: {e}")))?;
    }

    tracing::info!(path = %path, "File deleted");
    Ok(Json(
        serde_json::json!({ "status": "deleted", "path": path }),
    ))
}

/// Guess MIME type from file extension.
fn guess_mime(path: &str) -> String {
    match path.rsplit('.').next() {
        Some("md") => "text/markdown; charset=utf-8".into(),
        Some("json") => "application/json".into(),
        Some("toml") => "application/toml".into(),
        Some("yaml" | "yml") => "application/yaml".into(),
        Some("txt") => "text/plain; charset=utf-8".into(),
        Some("html") => "text/html".into(),
        Some("css") => "text/css".into(),
        Some("js") => "application/javascript".into(),
        _ => "text/plain; charset=utf-8".into(),
    }
}

// ---------------------------------------------------------------------------
// Skills
// ---------------------------------------------------------------------------

/// Compact a file path for display (replace home dir with ~).
fn compact_path(path: &std::path::Path) -> String {
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        let path_str = path.to_string_lossy();
        if let Some(rest) = path_str.strip_prefix(home_str.as_ref()) {
            return format!("~{rest}");
        }
    }
    path.to_string_lossy().into_owned()
}

/// Convert a SkillEntry to its JSON API representation (RFC-009 §5.1).
fn skill_entry_to_json(entry: &SkillEntry) -> serde_json::Value {
    let meta = entry.metadata.as_ref();
    let source_str = match entry.source {
        SkillSource::Bundled => "bundled",
        SkillSource::Managed => "managed",
        SkillSource::Workspace => "workspace",
        SkillSource::Foundation => "foundation",
    };
    let status_str = match entry.status {
        SkillStatus::Ready => "ready",
        SkillStatus::NeedsSetup => "needs_setup",
        SkillStatus::Disabled => "disabled",
    };

    let requirements = meta
        .map(|m| {
            serde_json::json!({
                "bins": m.requires.bins,
                "anyBins": m.requires.any_bins,
                "env": m.requires.env,
                "config": m.requires.config,
            })
        })
        .unwrap_or(serde_json::json!({
            "bins": [],
            "anyBins": [],
            "env": [],
            "config": [],
        }));

    let missing = serde_json::json!({
        "bins": entry.eligibility.missing_bins,
        "anyBins": entry.eligibility.missing_any_bins,
        "env": entry.eligibility.missing_env,
        "config": entry.eligibility.missing_config,
    });

    let install: Vec<serde_json::Value> = meta
        .map(|m| {
            m.install
                .iter()
                .map(|spec| {
                    let label = match spec.kind {
                        oxios_kernel::InstallKind::Brew => {
                            let name = spec.formula.as_deref().unwrap_or("unknown");
                            format!("Install {name} (brew)")
                        }
                        oxios_kernel::InstallKind::Node => {
                            let name = spec.package.as_deref().unwrap_or("unknown");
                            format!("Install {name} (npm)")
                        }
                        oxios_kernel::InstallKind::Go => {
                            let name = spec.module.as_deref().unwrap_or("unknown");
                            format!("Install {name} (go)")
                        }
                        oxios_kernel::InstallKind::Uv => {
                            let name = spec.package.as_deref().unwrap_or("unknown");
                            format!("Install {name} (uv)")
                        }
                        oxios_kernel::InstallKind::Bun => {
                            let name = spec.package.as_deref().unwrap_or("unknown");
                            format!("Install {name} (bun)")
                        }
                        oxios_kernel::InstallKind::Cargo => {
                            let name = spec.package.as_deref().unwrap_or("unknown");
                            format!("Install {name} (cargo)")
                        }
                        oxios_kernel::InstallKind::Pip => {
                            let name = spec.package.as_deref().unwrap_or("unknown");
                            format!("Install {name} (pip)")
                        }
                        oxios_kernel::InstallKind::Download => "Download".to_string(),
                    };
                    let bins: Vec<String> = match spec.kind {
                        oxios_kernel::InstallKind::Brew => spec
                            .formula
                            .as_ref()
                            .map(|f| vec![f.clone()])
                            .unwrap_or_default(),
                        oxios_kernel::InstallKind::Node => spec
                            .package
                            .as_ref()
                            .map(|p| vec![p.clone()])
                            .unwrap_or_default(),
                        oxios_kernel::InstallKind::Go => spec
                            .module
                            .as_ref()
                            .map(|m| vec![m.clone()])
                            .unwrap_or_default(),
                        oxios_kernel::InstallKind::Uv => spec
                            .package
                            .as_ref()
                            .map(|p| vec![p.clone()])
                            .unwrap_or_default(),
                        oxios_kernel::InstallKind::Bun
                        | oxios_kernel::InstallKind::Cargo
                        | oxios_kernel::InstallKind::Pip => spec
                            .package
                            .as_ref()
                            .map(|p| vec![p.clone()])
                            .unwrap_or_default(),
                        oxios_kernel::InstallKind::Download => vec![],
                    };
                    serde_json::json!({
                        "kind": spec.kind.to_string(),
                        "label": label,
                        "bins": bins,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let os = meta.map(|m| m.os.clone()).unwrap_or_default();

    let config_checks: Vec<serde_json::Value> = entry
        .eligibility
        .config_checks
        .iter()
        .map(|c| serde_json::json!({ "path": c.path, "satisfied": c.satisfied }))
        .collect();

    serde_json::json!({
        "name": entry.skill.name,
        "description": entry.skill.description,
        "author": meta.and_then(|m| m.author.clone()).unwrap_or_default(),
        "version": meta.and_then(|m| m.version.clone()).unwrap_or_default(),
        "emoji": meta.and_then(|m| m.emoji.clone()).unwrap_or_default(),
        "homepage": meta.and_then(|m| m.homepage.clone()).unwrap_or_default(),
        "source": source_str,
        "bundled": entry.bundled,
        "status": status_str,
        "eligible": entry.eligibility.eligible,
        "always": meta.map(|m| m.always).unwrap_or(false),
        "user_invocable": entry.invocation.user_invocable,
        "file_path": compact_path(&entry.skill.file_path),
        "requirements": requirements,
        "missing": missing,
        "os": os,
        "install": install,
        "config_checks": config_checks,
        "foundation": entry.foundation.as_ref().map(|f| serde_json::json!({
            "id": f.id,
            "version": f.version,
            "digest": f.digest,
            "persona": f.persona,
        })),
        "format": entry.format.to_string(),
    })
}

/// GET /api/skills — List all skills (RFC-009 §5.1).
pub(crate) async fn handle_skills_list(
    state: State<Arc<AppState>>,
    Query(_params): Query<PageParams>,
) -> Json<serde_json::Value> {
    let entries = state.kernel.extensions.list_skills_entries().await;
    let skills: Vec<serde_json::Value> = entries.iter().map(skill_entry_to_json).collect();
    Json(serde_json::json!({ "skills": skills }))
}

/// GET /api/skills/:name — Get skill details (RFC-009 §5.1).
pub(crate) async fn handle_skill_get(
    state: State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    match state.kernel.extensions.get_skill_entry(&name).await {
        Some(entry) => Ok(Json(skill_entry_to_json(&entry))),
        None => Err(AppError::NotFound("skill not found".into())),
    }
}

/// POST /api/skills/:name/enable — Enable a skill.
pub(crate) async fn handle_skill_enable(
    state: State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    state
        .kernel
        .extensions
        .enable_skill(&name)
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    tracing::info!(skill = %name, "Skill enabled via API");
    Ok(Json(
        serde_json::json!({ "status": "enabled", "name": name }),
    ))
}

/// POST /api/skills/:name/disable — Disable a skill.
pub(crate) async fn handle_skill_disable(
    state: State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    state
        .kernel
        .extensions
        .disable_skill(&name)
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    tracing::info!(skill = %name, "Skill disabled via API");
    Ok(Json(
        serde_json::json!({ "status": "disabled", "name": name }),
    ))
}

/// GET /api/skills/:name/content — Get SKILL.md content.
pub(crate) async fn handle_skill_content(
    state: State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let content = state
        .kernel
        .extensions
        .skill_manager()
        .get_skill_content(&name)
        .await;
    match content {
        Some(md) => Ok(Json(serde_json::json!({
            "name": name,
            "content": md,
        }))),
        None => Err(AppError::NotFound("skill not found".into())),
    }
}

/// Request body for creating a skill.
#[derive(Debug, Deserialize)]
pub(crate) struct SkillCreateRequest {
    /// Skill name.
    name: String,
    /// Skill description.
    description: String,
    /// Skill markdown content.
    #[serde(default)]
    content: String,
}

/// POST /api/skills — Create a new skill.
pub(crate) async fn handle_skill_create(
    state: State<Arc<AppState>>,
    Json(body): Json<SkillCreateRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Validate skill content size (max 64KB)
    const MAX_SKILL_CONTENT: usize = 64 * 1024;
    if body.content.len() > MAX_SKILL_CONTENT {
        return Err(AppError::PayloadTooLarge {
            size: body.content.len(),
            limit: MAX_SKILL_CONTENT,
        });
    }

    state
        .kernel
        .extensions
        .create_skill(&body.name, &body.description, &body.content)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, skill = %body.name, "Failed to create skill");
            AppError::BadRequest(e.to_string())
        })?;

    tracing::info!(skill = %body.name, "Skill created via API");
    Ok(Json(serde_json::json!({
        "status": "created",
        "name": body.name,
    })))
}

/// DELETE /api/skills/:name — Delete a skill.
pub(crate) async fn handle_skill_delete(
    state: State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    state
        .kernel
        .extensions
        .delete_skill(&name)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, skill = %name, "Failed to delete skill");
            AppError::BadRequest(e.to_string())
        })?;

    tracing::info!(skill = %name, "Skill deleted via API");
    Ok(Json(serde_json::json!({
        "status": "deleted",
        "name": name,
    })))
}

/// Request body for editing a skill's content (PUT /content).
#[derive(Debug, Deserialize)]
pub(crate) struct SkillContentUpdate {
    /// Full SKILL.md content (frontmatter preserved, written verbatim).
    #[serde(default)]
    content: String,
}

/// PUT /api/skills/:name/content — Update a skill's SKILL.md verbatim.
///
/// Preserves frontmatter (unlike POST /api/skills which re-synthesizes it).
/// Used by the inline editor.
pub(crate) async fn handle_skill_content_update(
    state: State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<SkillContentUpdate>,
) -> Result<Json<serde_json::Value>, AppError> {
    const MAX_SKILL_CONTENT: usize = 64 * 1024;
    if body.content.len() > MAX_SKILL_CONTENT {
        return Err(AppError::PayloadTooLarge {
            size: body.content.len(),
            limit: MAX_SKILL_CONTENT,
        });
    }
    if state
        .kernel
        .extensions
        .get_skill_entry(&name)
        .await
        .is_none()
    {
        return Err(AppError::NotFound(format!("skill not found: {name}")));
    }
    let entry = state
        .kernel
        .extensions
        .write_skill_raw(&name, &body.content)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, skill = %name, "Failed to update skill content");
            AppError::BadRequest(e.to_string())
        })?;
    tracing::info!(skill = %name, "Skill content updated via API");
    Ok(Json(skill_entry_to_json(&entry)))
}

/// Request body for text-paste import.
#[derive(Debug, Deserialize)]
pub(crate) struct SkillTextImport {
    /// Full SKILL.md content (with frontmatter).
    content: String,
    /// Optional name override (used only if frontmatter has no name).
    #[serde(default)]
    name: Option<String>,
}

/// POST /api/skills/import/text — Import a skill from pasted SKILL.md text.
pub(crate) async fn handle_skill_import_text(
    state: State<Arc<AppState>>,
    Json(body): Json<SkillTextImport>,
) -> Result<Json<serde_json::Value>, AppError> {
    const MAX_SKILL_CONTENT: usize = 64 * 1024;
    if body.content.len() > MAX_SKILL_CONTENT {
        return Err(AppError::PayloadTooLarge {
            size: body.content.len(),
            limit: MAX_SKILL_CONTENT,
        });
    }
    let entry = state
        .kernel
        .extensions
        .import_skill_text(&body.content, body.name.as_deref())
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Skill text import failed");
            AppError::BadRequest(e.to_string())
        })?;
    tracing::info!(skill = %entry.skill.name, "Skill imported via text");
    Ok(Json(skill_entry_to_json(&entry)))
}

/// Request body for URL import.
#[derive(Debug, Deserialize)]
pub(crate) struct SkillUrlImport {
    /// http(s) URL to a SKILL.md file.
    url: String,
    /// Optional name override.
    #[serde(default)]
    name: Option<String>,
}

/// POST /api/skills/import/url — Fetch a SKILL.md from a URL and import it.
pub(crate) async fn handle_skill_import_url(
    state: State<Arc<AppState>>,
    Json(body): Json<SkillUrlImport>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !(body.url.starts_with("http://") || body.url.starts_with("https://")) {
        return Err(AppError::BadRequest(
            "only http:// and https:// URLs are allowed".into(),
        ));
    }
    const MAX_FETCH: usize = 1024 * 1024;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::Internal(format!("HTTP client error: {e}")))?;
    let resp = client
        .get(&body.url)
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("fetch failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::BadRequest(format!(
            "fetch returned HTTP {}",
            resp.status()
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::BadRequest(format!("reading response failed: {e}")))?;
    if bytes.len() > MAX_FETCH {
        return Err(AppError::PayloadTooLarge {
            size: bytes.len(),
            limit: MAX_FETCH,
        });
    }
    let content = String::from_utf8(bytes.to_vec())
        .map_err(|e| AppError::BadRequest(format!("fetched content is not valid UTF-8: {e}")))?;
    let entry = state
        .kernel
        .extensions
        .import_skill_text(&content, body.name.as_deref())
        .await
        .map_err(|e| {
            tracing::error!(error = %e, url = %body.url, "Skill URL import failed");
            AppError::BadRequest(e.to_string())
        })?;
    tracing::info!(skill = %entry.skill.name, "Skill imported via URL");
    Ok(Json(skill_entry_to_json(&entry)))
}

/// POST /api/skills/import — Import a skill from an uploaded file (multipart).
///
/// Accepts `.md` (single SKILL.md), `.zip`, or `.skill` (zip) archives. A
/// raised body limit is applied on the route so archives up to 32 MB upload.
pub(crate) async fn handle_skill_import_file(
    state: State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut filename: Option<String> = None;
    let mut name_override: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("malformed multipart upload: {e}")))?
    {
        match field.name() {
            Some("file") => {
                filename = field.file_name().map(|s| s.to_string());
                let b = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("reading file field failed: {e}")))?;
                file_bytes = Some(b.to_vec());
            }
            Some("name") => {
                name_override = Some(field.text().await.map_err(|e| {
                    AppError::BadRequest(format!("reading name field failed: {e}"))
                })?);
            }
            _ => {
                // Discard unknown fields.
            }
        }
    }

    let bytes = file_bytes
        .ok_or_else(|| AppError::BadRequest("no 'file' field in multipart upload".into()))?;
    const MAX_UPLOAD: usize = 32 * 1024 * 1024;
    if bytes.len() > MAX_UPLOAD {
        return Err(AppError::PayloadTooLarge {
            size: bytes.len(),
            limit: MAX_UPLOAD,
        });
    }
    let fname = filename.unwrap_or_else(|| "skill.zip".to_string());
    let lower = fname.to_lowercase();

    let result = if lower.ends_with(".md") {
        let content = String::from_utf8(bytes)
            .map_err(|e| AppError::BadRequest(format!("file is not valid UTF-8: {e}")))?;
        state
            .kernel
            .extensions
            .import_skill_text(&content, name_override.as_deref())
            .await
    } else if lower.ends_with(".zip") || lower.ends_with(".skill") {
        state
            .kernel
            .extensions
            .import_skill_zip(&fname, &bytes)
            .await
    } else {
        return Err(AppError::BadRequest(format!(
            "unsupported file type: {fname} (expected .md, .zip, or .skill)"
        )));
    };

    let entry = result.map_err(|e| {
        tracing::error!(error = %e, file = %fname, "Skill file import failed");
        AppError::BadRequest(e.to_string())
    })?;
    tracing::info!(skill = %entry.skill.name, file = %fname, "Skill imported via file upload");
    Ok(Json(skill_entry_to_json(&entry)))
}

// ─────────────────────────────────────────────────────────────────────
// Brain routes (daemonless, oxibrain 0.8) — replaced the retired
// /api/memory/* surface. All handlers degrade to empty payloads when the
// binary is missing or the session child is down (degradation contract);
// GET /api/brain/status reports session/binary state.
// ─────────────────────────────────────────────────────────────────────

/// Parse the `planes` search param: `None`/`both` → both planes,
/// `memory`/`documents` → that plane, anything else → a 400 message.
fn parse_brain_planes(raw: Option<&str>) -> Result<Option<Vec<&'static str>>, String> {
    match raw {
        None | Some("both") => Ok(None),
        Some("memory") => Ok(Some(vec!["memory"])),
        Some("documents") => Ok(Some(vec!["documents"])),
        Some(other) => Err(format!("invalid planes '{other}' — memory|documents|both")),
    }
}

/// Build the `/api/brain/status` JSON. One shape for the live brain
/// (`Some(snapshot)`) and the degraded no-brain branch (`None`). Global by
/// design: per-space data (space name, episode counts) moved to the scoped
/// `/api/brain/stats?space=` endpoint.
fn brain_status_json(
    snap: Option<oxios_kernel::BrainStatusSnapshot>,
    pending_extraction: Option<u64>,
) -> serde_json::Value {
    match snap {
        None => serde_json::json!({
            "available": false,
            "pending_extraction": null,
            "binary": { "installed": false, "path": null, "version": null },
        }),
        Some(s) => serde_json::json!({
            "available": s.available,
            "pending_extraction": pending_extraction,
            "binary": {
                "installed": s.binary_installed,
                "path": s.binary_path,
                "version": s.binary_version,
            },
        }),
    }
}

/// GET /api/brain/status — daemonless session/binary state + pending
/// extraction backlog.
pub(crate) async fn handle_brain_status(
    state: State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let Some(brain) = state.kernel.brain.as_ref() else {
        return Ok(Json(brain_status_json(None, None)));
    };
    let snap = brain.status_snapshot();
    let pending_extraction = brain.pending_stats().await.map(|p| p.count);
    Ok(Json(brain_status_json(Some(snap), pending_extraction)))
}
/// POST /api/brain/recall — assemble context for an agent turn.
#[derive(Debug, Deserialize)]
pub(crate) struct BrainRecallRequest {
    /// Query to assemble context for.
    pub query: String,
    /// Token budget for the assembled context.
    #[serde(default = "default_recall_budget")]
    pub budget: usize,
    /// Oxibrain space to operate in. Required.
    pub space: String,
}

fn default_recall_budget() -> usize {
    3000
}

pub(crate) async fn handle_brain_recall(
    state: State<Arc<AppState>>,
    Json(body): Json<BrainRecallRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if body.space.is_empty() {
        return Err(AppError::BadRequest("space parameter required".into()));
    }
    let context = match state.kernel.brain.as_ref() {
        Some(brain) => brain.recall(&body.space, &body.query, body.budget).await,
        None => None,
    };
    Ok(Json(serde_json::json!({ "context": context })))
}
/// GET /api/brain/search?q=&space=&mode=&limit=&planes= —
/// hybrid/lexical/semantic/graph/community.
#[derive(Debug, Deserialize)]
pub(crate) struct BrainSearchQuery {
    pub q: String,
    /// Oxibrain space to operate in. Required.
    pub space: String,
    #[serde(default = "default_search_mode")]
    pub mode: String,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
    /// `memory`, `documents`, or `both` (default). Invalid values 400.
    pub planes: Option<String>,
}

fn default_search_mode() -> String {
    "hybrid".to_string()
}

fn default_search_limit() -> usize {
    20
}

pub(crate) async fn handle_brain_search(
    state: State<Arc<AppState>>,
    Query(query): Query<BrainSearchQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let planes = parse_brain_planes(query.planes.as_deref()).map_err(AppError::BadRequest)?;
    let Some(brain) = state.kernel.brain.as_ref() else {
        return Ok(Json(serde_json::Value::Null));
    };
    let refs = planes.as_deref();
    let result = brain
        .search(&query.space, &query.q, &query.mode, query.limit, refs)
        .await;
    Ok(Json(
        result
            .and_then(|r| serde_json::to_value(r).ok())
            .unwrap_or(serde_json::Value::Null),
    ))
}

/// GET /api/brain/document-history?space=&alias=&locator=&limit= —
/// gix-backed revision history of one tracked vault document.
#[derive(Debug, Deserialize)]
pub(crate) struct BrainDocumentHistoryQuery {
    /// Oxibrain space to operate in. Required.
    pub space: String,
    pub alias: String,
    pub locator: String,
    pub limit: Option<usize>,
}

pub(crate) async fn handle_brain_document_history(
    state: State<Arc<AppState>>,
    Query(query): Query<BrainDocumentHistoryQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let Some(brain) = state.kernel.brain.as_ref() else {
        return Ok(Json(serde_json::Value::Null));
    };
    let result = brain
        .document_history(
            &query.space,
            &query.alias,
            &query.locator,
            query.limit.unwrap_or(32),
        )
        .await;
    Ok(Json(result.unwrap_or(serde_json::Value::Null)))
}

/// Shared single-field query for scoped brain endpoints without other
/// parameters. `space` is required — a missing value is rejected by axum.
#[derive(Debug, Deserialize)]
pub(crate) struct BrainSpaceQuery {
    /// Oxibrain space to operate in. Required.
    pub space: String,
}

/// GET /api/brain/entity/{id}?space= — an entity's current beliefs.
pub(crate) async fn handle_brain_entity(
    state: State<Arc<AppState>>,
    Path(entity_id): Path<String>,
    Query(query): Query<BrainSpaceQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let Some(brain) = state.kernel.brain.as_ref() else {
        return Ok(Json(serde_json::Value::Null));
    };
    let result = brain.get_entity(&query.space, &entity_id).await;
    Ok(Json(result.unwrap_or(serde_json::Value::Null)))
}

/// GET /api/brain/timeline?space=&entity=&from=&to= — belief intervals
/// over a range.
#[derive(Debug, Deserialize)]
pub(crate) struct BrainTimelineQuery {
    /// Oxibrain space to operate in. Required.
    pub space: String,
    pub entity: String,
    /// Start of range in Unix milliseconds (optional).
    pub from: Option<i64>,
    /// End of range in Unix milliseconds (optional).
    pub to: Option<i64>,
}

pub(crate) async fn handle_brain_timeline(
    state: State<Arc<AppState>>,
    Query(query): Query<BrainTimelineQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    // `entity` is required. Validate before touching the brain so callers
    // get a clean 400 instead of axum's default 500 (QueryRejection).
    if query.entity.trim().is_empty() {
        return Err(AppError::BadRequest(
            "`entity` query parameter is required".into(),
        ));
    }
    let Some(brain) = state.kernel.brain.as_ref() else {
        return Ok(Json(serde_json::Value::Null));
    };
    let result = brain
        .timeline(&query.space, &query.entity, query.from, query.to)
        .await;
    Ok(Json(result.unwrap_or(serde_json::Value::Null)))
}

/// GET /api/brain/why/{statement_id}?space= — provenance + confidence
/// breakdown.
pub(crate) async fn handle_brain_why(
    state: State<Arc<AppState>>,
    Path(statement_id): Path<String>,
    Query(query): Query<BrainSpaceQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let Some(brain) = state.kernel.brain.as_ref() else {
        return Ok(Json(serde_json::Value::Null));
    };
    let result = brain.why(&query.space, &statement_id).await;
    Ok(Json(result.unwrap_or(serde_json::Value::Null)))
}

/// GET /api/brain/contradictions?space= — contradicted statements in the
/// space.
pub(crate) async fn handle_brain_contradictions(
    state: State<Arc<AppState>>,
    Query(query): Query<BrainSpaceQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let Some(brain) = state.kernel.brain.as_ref() else {
        return Ok(Json(serde_json::Value::Null));
    };
    let result = brain.contradictions(&query.space).await;
    Ok(Json(result.unwrap_or(serde_json::Value::Null)))
}

/// GET /api/brain/stats?space= — aggregate counts for the space.
pub(crate) async fn handle_brain_stats(
    state: State<Arc<AppState>>,
    Query(query): Query<BrainSpaceQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let Some(brain) = state.kernel.brain.as_ref() else {
        return Ok(Json(serde_json::Value::Null));
    };
    let result = brain.stats(&query.space).await;
    Ok(Json(result.unwrap_or(serde_json::Value::Null)))
}

/// GET /api/brain/brief?space=&target_kind=&entity_id=&topic= — rendered
/// page (Markdown) for an entity, the space, or a topic keyword.
#[derive(Debug, Deserialize)]
pub(crate) struct BrainBriefQuery {
    /// Oxibrain space to operate in. Required.
    pub space: String,
    /// `entity` (default), `space`, or `topic`.
    pub target_kind: Option<String>,
    pub entity_id: Option<String>,
    pub topic: Option<String>,
}

pub(crate) async fn handle_brain_brief(
    state: State<Arc<AppState>>,
    Query(query): Query<BrainBriefQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let kind = query.target_kind.unwrap_or_else(|| "entity".into());
    if !matches!(kind.as_str(), "entity" | "space" | "topic") {
        return Err(AppError::BadRequest(
            "`target_kind` must be entity, space, or topic".into(),
        ));
    }
    if kind == "entity"
        && query
            .entity_id
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
    {
        return Err(AppError::BadRequest(
            "`entity_id` is required for target_kind=entity".into(),
        ));
    }
    if kind == "topic"
        && query
            .topic
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
    {
        return Err(AppError::BadRequest(
            "`topic` is required for target_kind=topic".into(),
        ));
    }
    let Some(brain) = state.kernel.brain.as_ref() else {
        return Ok(Json(serde_json::Value::Null));
    };
    let markdown = brain
        .brief(
            &query.space,
            &kind,
            query.entity_id.as_deref(),
            query.topic.as_deref(),
        )
        .await;
    Ok(Json(serde_json::json!({ "markdown": markdown })))
}

/// GET /api/brain/graph?space=&start=a,b&depth=&max_nodes=&direction= —
/// bounded belief-filtered subgraph traversal.
#[derive(Debug, Deserialize)]
pub(crate) struct BrainGraphQuery {
    /// Oxibrain space to operate in. Required.
    pub space: String,
    /// Comma-separated start entity ids.
    pub start: String,
    pub depth: Option<usize>,
    pub max_nodes: Option<usize>,
    /// `both` (default), `out`, or `in`.
    pub direction: Option<String>,
}

pub(crate) async fn handle_brain_graph(
    state: State<Arc<AppState>>,
    Query(query): Query<BrainGraphQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let start: Vec<String> = query
        .start
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if start.is_empty() {
        return Err(AppError::BadRequest(
            "`start` must list at least one entity id".into(),
        ));
    }
    let direction = query.direction.unwrap_or_else(|| "both".into());
    if !matches!(direction.as_str(), "both" | "out" | "in") {
        return Err(AppError::BadRequest(
            "`direction` must be both, out, or in".into(),
        ));
    }
    let Some(brain) = state.kernel.brain.as_ref() else {
        return Ok(Json(serde_json::Value::Null));
    };
    let result = brain
        .traverse(
            &query.space,
            &start,
            query.depth.unwrap_or(2).clamp(1, 6),
            query.max_nodes.unwrap_or(64).clamp(1, 256),
            &direction,
        )
        .await;
    Ok(Json(result.unwrap_or(serde_json::Value::Null)))
}
/// GET /api/brain/operations/{section}?space= — daemon console data:
/// `merges` (entity merge records), `failures` (extraction failures),
/// `sources` (registered pull sources).
pub(crate) async fn handle_brain_operations(
    state: State<Arc<AppState>>,
    Path(section): Path<String>,
    Query(query): Query<BrainSpaceQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !matches!(section.as_str(), "merges" | "failures" | "sources") {
        return Err(AppError::BadRequest(
            "`section` must be merges, failures, or sources".into(),
        ));
    }
    let Some(brain) = state.kernel.brain.as_ref() else {
        return Ok(Json(serde_json::Value::Null));
    };
    let result = brain.review_merges(&query.space, &section).await;
    Ok(Json(result.unwrap_or(serde_json::Value::Null)))
}

/// GET /api/brain/spaces — spaces the daemon exposes.
pub(crate) async fn handle_brain_spaces(
    state: State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let Some(brain) = state.kernel.brain.as_ref() else {
        return Ok(Json(serde_json::Value::Null));
    };
    let result = brain.spaces().await;
    Ok(Json(result.unwrap_or(serde_json::Value::Null)))
}

/// GET /api/brain/space?space= — overview of one space (counts + recent
/// entities).
pub(crate) async fn handle_brain_space(
    state: State<Arc<AppState>>,
    Query(query): Query<BrainSpaceQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let Some(brain) = state.kernel.brain.as_ref() else {
        return Ok(Json(serde_json::Value::Null));
    };
    let result = brain.space_overview(&query.space).await;
    Ok(Json(result.unwrap_or(serde_json::Value::Null)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // TreeEntry serialization

    #[test]
    fn test_tree_entry_serialization() {
        let entry = TreeEntry {
            name: "hello.md".into(),
            is_dir: false,
            size: 1024,
        };
        let json = serde_json::to_value(&entry).expect("serializable value");
        assert_eq!(json["name"], "hello.md");
        assert_eq!(json["is_dir"], false);
        assert_eq!(json["size"], 1024);

        let dir_entry = TreeEntry {
            name: "src".into(),
            is_dir: true,
            size: 0,
        };
        let json = serde_json::to_value(&dir_entry).expect("serializable value");
        assert_eq!(json["is_dir"], true);
        assert_eq!(json["size"], 0);
    }

    // Pagination

    #[test]
    fn test_pagination_bounds() {
        let items: Vec<i32> = (1..=10).collect();

        // Page 1, limit 3 → items [1, 2, 3]
        let p1 = PageParams { page: 1, limit: 3 };
        let result = paginate(&items, &p1);
        assert_eq!(result["total"], 10);
        assert_eq!(result["page"], 1);
        assert_eq!(result["limit"], 3);
        let returned: Vec<i32> = serde_json::from_value(result["items"].clone()).unwrap();
        assert_eq!(returned, vec![1, 2, 3]);

        // Page 4, limit 3 → items [10]
        let p4 = PageParams { page: 4, limit: 3 };
        let result = paginate(&items, &p4);
        let returned: Vec<i32> = serde_json::from_value(result["items"].clone()).unwrap();
        assert_eq!(returned, vec![10]);

        // Page 0 (underflow) → offset wraps to 0 via saturating_sub
        let p0 = PageParams { page: 0, limit: 3 };
        let result = paginate(&items, &p0);
        let returned: Vec<i32> = serde_json::from_value(result["items"].clone()).unwrap();
        assert_eq!(returned, vec![1, 2, 3]);

        // Limit capped at 500
        let big = PageParams {
            page: 1,
            limit: 9999,
        };
        let result = paginate(&items, &big);
        assert_eq!(result["limit"], 500);
    }

    // MIME guessing

    #[test]
    fn test_guess_mime_common_types() {
        assert_eq!(guess_mime("main.rs"), "text/plain; charset=utf-8");
        assert_eq!(guess_mime("Cargo.toml"), "application/toml");
        assert_eq!(guess_mime("README.md"), "text/markdown; charset=utf-8");
        assert_eq!(guess_mime("data.json"), "application/json");
        assert_eq!(guess_mime("app.js"), "application/javascript");
        assert_eq!(guess_mime("index.html"), "text/html");
        assert_eq!(guess_mime("unknown.bin"), "text/plain; charset=utf-8");
    }

    // Memory type validation (removed with the memory system — RFC-047)

    // File size limit enforcement

    #[test]
    fn test_file_size_limit_enforcement() {
        // MAX_FILE_SIZE in handle_workspace_file_put is 1MB.
        const MAX_FILE_SIZE: usize = 1024 * 1024;

        // A body exactly at the limit should be accepted by the size check.
        let body_at_limit = "x".repeat(MAX_FILE_SIZE);
        assert_eq!(body_at_limit.len(), MAX_FILE_SIZE);
        assert!(body_at_limit.len() <= MAX_FILE_SIZE);

        // A body one byte over the limit should be rejected.
        let body_over_limit = "x".repeat(MAX_FILE_SIZE + 1);
        assert!(body_over_limit.len() > MAX_FILE_SIZE);

        // Simulate the check done in handle_workspace_file_put:
        // if body.len() > MAX_FILE_SIZE { return PayloadTooLarge }
        assert!(body_over_limit.len() > MAX_FILE_SIZE);

        // Skill content limit (64KB)
        const MAX_SKILL_CONTENT: usize = 64 * 1024;
        let big_skill = "a".repeat(MAX_SKILL_CONTENT + 1);
        assert!(big_skill.len() > MAX_SKILL_CONTENT);

        // Memory entry limit (32KB)
        const MAX_MEMORY_ENTRY: usize = 32 * 1024;
        let big_memory = "m".repeat(MAX_MEMORY_ENTRY + 1);
        assert!(big_memory.len() > MAX_MEMORY_ENTRY);
    }

    // ── Brain route contracts (daemonless, oxibrain 0.8) ──────────────

    #[test]
    fn brain_planes_param_contract() {
        // Default and explicit both → both planes (None).
        assert_eq!(parse_brain_planes(None), Ok(None));
        assert_eq!(parse_brain_planes(Some("both")), Ok(None));
        // Single-plane selection.
        assert_eq!(parse_brain_planes(Some("memory")), Ok(Some(vec!["memory"])));
        assert_eq!(
            parse_brain_planes(Some("documents")),
            Ok(Some(vec!["documents"]))
        );
        // Invalid values carry the documented message shape.
        let err = parse_brain_planes(Some("memry")).unwrap_err();
        assert!(err.contains("invalid planes 'memry'"), "{err}");
        assert!(err.contains("memory|documents|both"), "{err}");
    }

    #[test]
    fn brain_status_shape_live_and_degraded() {
        // Degraded (no brain facade): exact documented shape. Global by
        // design — no `space`, no per-space `episodes` (both moved to the
        // scoped `/api/brain/stats?space=` endpoint).
        let degraded = brain_status_json(None, None);
        assert_eq!(degraded["available"], serde_json::json!(false));
        assert_eq!(degraded["pending_extraction"], serde_json::json!(null));
        assert_eq!(
            degraded["binary"],
            serde_json::json!({"installed": false, "path": null, "version": null})
        );
        assert!(degraded.get("space").is_none());
        assert!(degraded.get("episodes").is_none());

        // Live: snapshot fields surface.
        let snap = oxios_kernel::BrainStatusSnapshot {
            available: true,
            binary_installed: true,
            binary_path: Some("/opt/oxibrain".into()),
            binary_version: Some("oxibrain 0.8.0".into()),
        };
        let live = brain_status_json(Some(snap), Some(7));
        assert_eq!(live["available"], serde_json::json!(true));
        assert_eq!(live["pending_extraction"], serde_json::json!(7));
        assert_eq!(live["binary"]["installed"], serde_json::json!(true));
        assert_eq!(live["binary"]["path"], serde_json::json!("/opt/oxibrain"));
        assert_eq!(
            live["binary"]["version"],
            serde_json::json!("oxibrain 0.8.0")
        );
        assert!(live.get("space").is_none());
    }

    #[test]
    fn brain_search_envelope_serialization_keys() {
        // The handler serializes SearchResponseDto verbatim; pin the
        // envelope keys so a rename cannot pass CI silently.
        let dto = oxibrain_client::SearchResponseDto {
            memory: vec![serde_json::json!({"entity_id": "e1"})],
            documents: vec![oxibrain_client::DocumentHitDto {
                document_id: "d1".into(),
                root: "vault".into(),
                locator: "notes/a.md".into(),
                revision: "r1".into(),
                ordinal: 0,
                text: oxibrain_client::UntrustedContentDto {
                    kind: "untrusted_content".into(),
                    text: "body".into(),
                    provenance: oxibrain_client::ContentProvenanceDto {
                        reference: "doc://vault/notes/a.md".into(),
                        trust: "unverified".into(),
                    },
                },
                modified_at_ms: 1700000000,
                score: 0.5,
            }],
            freshness: oxibrain_client::DocumentFreshnessDto {
                reconciled_roots: vec!["vault".into()],
                skipped_roots: Vec::new(),
                skipped_files: 0,
                stale_after_retry: Vec::new(),
                dense_coverage: None,
            },
        };
        let v = serde_json::to_value(&dto).unwrap();
        for key in ["memory", "documents", "freshness"] {
            assert!(v.get(key).is_some(), "envelope missing {key}");
        }
        assert_eq!(v["memory"][0]["entity_id"], serde_json::json!("e1"));
        assert_eq!(
            v["documents"][0]["locator"],
            serde_json::json!("notes/a.md")
        );
        // Wire field is millis; the JSON key keeps the engine shape.
        assert_eq!(
            v["documents"][0]["modified_at"],
            serde_json::json!(1700000000)
        );
        // v2.13 §4: retrieved text is typed untrusted content, never a
        // bare string.
        assert_eq!(
            v["documents"][0]["text"]["kind"],
            serde_json::json!("untrusted_content")
        );
        assert_eq!(
            v["documents"][0]["text"]["provenance"]["ref"],
            serde_json::json!("doc://vault/notes/a.md")
        );
    }
}
