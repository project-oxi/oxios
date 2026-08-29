//! Session-scoped project workspace API (project-roots workbench, design §4.4).
//!
//! Routes live under `/api/project/workspace/*` so they cannot collide with
//! the kernel state-workspace browser at `/api/workspace/*`.
//!
//! Every endpoint resolves the caller's session to the bound Project's
//! `root_paths` and serves files strictly inside those roots. A session
//! without a project (or with a project that has no roots) has no
//! filesystem scope: all endpoints answer `409` with
//! `"no project filesystem scope for this session"` BEFORE any filesystem
//! access.
//!
//! Surface:
//! - `GET  /api/project/workspace/tree?session_id=&path=`   — directory listing
//! - `GET  /api/project/workspace/file?session_id=&path=`   — file read (≤512 KiB)
//! - `PUT  /api/project/workspace/file`                     — create/overwrite (≤2 MiB)
//! - `GET  /api/project/workspace/status?session_id=&root=` — git branch + dirty count
//!
//! Path safety: requested paths are canonicalized and must resolve to a
//! root itself or stay under a root (byte-boundary check), then pass the
//! kernel `AccessManager` path check. Symlinks that escape the roots are
//! rejected. Every write is audited through the kernel security trail.

use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

use oxios_kernel::access_manager::AgentPermissions;
use oxios_kernel::state_store::SessionId;

use crate::api::error::AppError;
use crate::api::server::AppState;

/// Exact conflict message for scopeless sessions (wire contract).
const NO_SCOPE_MESSAGE: &str = "no project filesystem scope for this session";
/// 403 message for any path that resolves outside the project roots.
const OUT_OF_SCOPE_MESSAGE: &str = "path is outside the project filesystem scope";
/// Maximum file size served by `GET /api/project/workspace/file` (512 KiB).
const MAX_READ_BYTES: u64 = 512 * 1024;
/// Maximum body accepted by `PUT /api/project/workspace/file` (2 MiB).
const MAX_WRITE_BYTES: usize = 2 * 1024 * 1024;
/// Entries never listed in tree output.
const SKIPPED_ENTRIES: [&str; 5] = [".git", "node_modules", "target", "dist", "build"];
/// `git` subprocess timeout for the status endpoint.
const GIT_TIMEOUT: Duration = Duration::from_secs(5);
/// Synthetic agent name under which the AccessManager evaluates workspace
/// paths. Its permissions are re-derived from the session's project roots on
/// every request, so the sandbox always equals the session scope.
const WORKSPACE_AGENT: &str = "web-workspace";

// ─── Request / response DTOs ────────────────────────────────────────────────

/// Query parameters for `GET /api/project/workspace/tree`.
#[derive(Debug, Deserialize)]
pub(crate) struct TreeQuery {
    pub session_id: Option<String>,
    /// Directory to list (absolute within roots). Defaults to `roots[0]`.
    pub path: Option<String>,
}

/// Query parameters for `GET /api/project/workspace/file`.
#[derive(Debug, Deserialize)]
pub(crate) struct FileQuery {
    pub session_id: Option<String>,
    /// File to read (absolute within roots).
    pub path: Option<String>,
}

/// Body for `PUT /api/project/workspace/file`.
#[derive(Debug, Deserialize)]
pub(crate) struct WriteFileBody {
    pub session_id: String,
    /// File to create/overwrite (absolute within roots).
    pub path: String,
    pub content: String,
}

/// Query parameters for `GET /api/project/workspace/status`.
#[derive(Debug, Deserialize)]
pub(crate) struct StatusQuery {
    pub session_id: Option<String>,
    /// Index into the project roots (defaults to 0).
    pub root: Option<usize>,
}

/// One directory-listing entry.
#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Whether `path` is a root itself or stays under one of `roots`.
///
/// `Path::starts_with` compares whole components, so the boundary is exact:
/// `/a/bc` is NOT inside root `/a/b`, and `/a/b/..` never survives
/// canonicalization to begin with.
fn is_within_roots(path: &Path, roots: &[PathBuf]) -> bool {
    roots
        .iter()
        .any(|root| path == root || path.starts_with(root))
}

/// Resolve a session to its project's canonical roots.
///
/// Missing session, unbound project, unknown project, unparsable project id,
/// or empty roots all mean "no filesystem scope" → `409` with
/// [`NO_SCOPE_MESSAGE`].
async fn session_roots(state: &AppState, session_id: &str) -> Result<Vec<PathBuf>, AppError> {
    let conflict = || AppError::Conflict(NO_SCOPE_MESSAGE.to_string());

    let session = state
        .kernel
        .state
        .load_session(&SessionId(session_id.to_string()))
        .await
        .map_err(|e| AppError::Internal(format!("failed to load session: {e}")))?
        .ok_or_else(conflict)?;

    // RFC-025: top-level binding first, legacy metadata fallbacks.
    let project_id = session
        .project_id
        .or_else(|| {
            session
                .metadata
                .get("project_id")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .or_else(|| {
            session
                .metadata
                .get("project_ids")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .ok_or_else(conflict)?;

    let projects = state.kernel.projects.as_ref().ok_or_else(conflict)?;
    let project = projects.get_project(&project_id).ok_or_else(conflict)?;

    let roots: Vec<PathBuf> = project.root_paths.iter().map(PathBuf::from).collect();
    if roots.is_empty() {
        return Err(conflict());
    }
    Ok(roots)
}

/// Re-derive the workspace agent's path permissions from the session roots.
///
/// The permission set is replaced (never merged) so the AccessManager sandbox
/// always equals exactly the session's project scope — stale or ambient
/// grants cannot accumulate.
fn grant_scope(state: &AppState, roots: &[PathBuf]) {
    let mut mgr = state.kernel.exec.access_manager().lock();
    let mut perms = AgentPermissions::for_new_agent(WORKSPACE_AGENT);
    for root in roots {
        perms.allow_path(&root.display().to_string());
        perms.allow_path(&format!("{}/**", root.display()));
    }
    mgr.set_permissions(perms);
}

/// AccessManager verdict for a (canonicalized) path.
fn path_allowed(state: &AppState, path: &Path) -> bool {
    state
        .kernel
        .exec
        .access_manager()
        .lock()
        .can_access_path(WORKSPACE_AGENT, &path.to_string_lossy())
}

/// Resolve the requested path against the roots: absolute paths are used
/// verbatim, relative paths anchor at `roots[0]`.
fn resolve_candidate(requested: &str, roots: &[PathBuf]) -> PathBuf {
    let requested = Path::new(requested);
    if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        roots[0].join(requested)
    }
}

fn require_session_id(session_id: &Option<String>) -> Result<String, AppError> {
    session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| AppError::BadRequest("session_id query parameter is required".into()))
}

/// `git -C <root> branch --show-current` — `None` on any failure (including
/// detached HEAD, where git prints an empty branch).
async fn git_current_branch(root: &Path) -> Option<String> {
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["branch", "--show-current"])
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        None
    } else {
        Some(branch)
    }
}
async fn git_dirty_count(root: &Path) -> Option<u64> {
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain"])
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count() as u64,
    )
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// `GET /api/project/workspace/tree` — list a directory inside the session scope.
pub(crate) async fn handle_workspace_tree(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TreeQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let session_id = require_session_id(&query.session_id)?;
    let roots = session_roots(&state, &session_id).await?;

    let candidate = match query.path.as_deref().filter(|p| !p.trim().is_empty()) {
        Some(requested) => resolve_candidate(requested, &roots),
        None => roots[0].clone(),
    };
    let dir = candidate
        .canonicalize()
        .map_err(|_| AppError::NotFound("directory not found".into()))?;
    if !is_within_roots(&dir, &roots) {
        return Err(AppError::Forbidden(OUT_OF_SCOPE_MESSAGE.into()));
    }
    if !dir.is_dir() {
        return Err(AppError::BadRequest("path is not a directory".into()));
    }
    grant_scope(&state, &roots);
    if !path_allowed(&state, &dir) {
        return Err(AppError::Forbidden(OUT_OF_SCOPE_MESSAGE.into()));
    }

    let mut entries: Vec<WorkspaceEntry> = Vec::new();
    let mut read_dir = match tokio::fs::read_dir(&dir).await {
        Ok(rd) => rd,
        Err(e) => return Err(AppError::Internal(format!("failed to read directory: {e}"))),
    };
    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if SKIPPED_ENTRIES.contains(&name.as_str()) {
            continue;
        }
        let Ok(metadata) = entry.metadata().await else {
            continue; // raced deletion / broken symlink — skip silently
        };
        let is_dir = metadata.is_dir();
        entries.push(WorkspaceEntry {
            name,
            path: entry.path().to_string_lossy().into_owned(),
            is_dir,
            size: if is_dir { 0 } else { metadata.len() },
        });
    }
    // Dirs first (alphabetical), then files (alphabetical).
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));

    Ok(Json(json!({
        "roots": roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>(),
        "path": dir.display().to_string(),
        "entries": entries,
    })))
}

/// `GET /api/project/workspace/file` — read a file inside the session scope.
pub(crate) async fn handle_workspace_file_get(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FileQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let session_id = require_session_id(&query.session_id)?;
    let path = query
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| AppError::BadRequest("path query parameter is required".into()))?;
    let roots = session_roots(&state, &session_id).await?;

    let candidate = resolve_candidate(path, &roots);
    let file = candidate
        .canonicalize()
        .map_err(|_| AppError::NotFound("file not found".into()))?;
    if !is_within_roots(&file, &roots) {
        return Err(AppError::Forbidden(OUT_OF_SCOPE_MESSAGE.into()));
    }
    grant_scope(&state, &roots);
    if !path_allowed(&state, &file) {
        return Err(AppError::Forbidden(OUT_OF_SCOPE_MESSAGE.into()));
    }

    let metadata = tokio::fs::metadata(&file)
        .await
        .map_err(|_| AppError::NotFound("file not found".into()))?;
    if metadata.is_dir() {
        return Err(AppError::BadRequest("path is a directory".into()));
    }

    let size = metadata.len();
    if size > MAX_READ_BYTES {
        // Contract: oversized reads are flagged, never streamed.
        return Ok(Json(json!({
            "path": file.display().to_string(),
            "content": "",
            "size": size,
            "truncated": true,
        })));
    }
    let bytes = tokio::fs::read(&file)
        .await
        .map_err(|e| AppError::Internal(format!("failed to read file: {e}")))?;
    Ok(Json(json!({
        "path": file.display().to_string(),
        "content": String::from_utf8_lossy(&bytes),
        "size": size,
        "truncated": false,
    })))
}

/// `PUT /api/project/workspace/file` — create/overwrite a file inside the scope.
pub(crate) async fn handle_workspace_file_put(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WriteFileBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let roots = session_roots(&state, &body.session_id).await?;

    if body.content.len() > MAX_WRITE_BYTES {
        return Err(AppError::PayloadTooLarge {
            size: body.content.len(),
            limit: MAX_WRITE_BYTES,
        });
    }

    let candidate = resolve_candidate(&body.path, &roots);
    let Some(name) = candidate.file_name() else {
        return Err(AppError::BadRequest("invalid file path".into()));
    };
    let Some(parent) = candidate.parent() else {
        return Err(AppError::BadRequest("invalid file path".into()));
    };
    let parent_canon = parent
        .canonicalize()
        .map_err(|_| AppError::NotFound("parent directory not found".into()))?;
    if !is_within_roots(&parent_canon, &roots) {
        return Err(AppError::Forbidden(OUT_OF_SCOPE_MESSAGE.into()));
    }

    // A pre-existing symlink at the target must not redirect the write
    // outside the roots (tokio::fs::write follows symlinks).
    if let Ok(meta) = tokio::fs::symlink_metadata(&candidate).await
        && meta.file_type().is_symlink()
    {
        let resolved = candidate
            .canonicalize()
            .map_err(|_| AppError::Forbidden(OUT_OF_SCOPE_MESSAGE.into()))?;
        if !is_within_roots(&resolved, &roots) {
            return Err(AppError::Forbidden(OUT_OF_SCOPE_MESSAGE.into()));
        }
    }

    let target = parent_canon.join(name);
    grant_scope(&state, &roots);
    if !path_allowed(&state, &target) {
        return Err(AppError::Forbidden(OUT_OF_SCOPE_MESSAGE.into()));
    }

    tokio::fs::write(&target, &body.content)
        .await
        .map_err(|e| AppError::Internal(format!("failed to write file: {e}")))?;

    // Audit the write through the kernel security trail.
    state.kernel.security.audit(
        WORKSPACE_AGENT,
        oxicode_sdk::AuditAction::Other {
            detail: "workspace_file_write".into(),
        },
        &target.display().to_string(),
    );

    Ok(Json(json!({
        "path": target.display().to_string(),
        "size": body.content.len(),
    })))
}

/// `GET /api/project/workspace/status` — git branch + dirty count for one root.
pub(crate) async fn handle_workspace_status(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StatusQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let session_id = require_session_id(&query.session_id)?;
    let roots = session_roots(&state, &session_id).await?;

    let index = query.root.unwrap_or(0);
    let Some(root) = roots.get(index) else {
        return Err(AppError::BadRequest("root index out of range".into()));
    };

    let branch = tokio::time::timeout(GIT_TIMEOUT, git_current_branch(root))
        .await
        .unwrap_or_default();
    let dirty_count = tokio::time::timeout(GIT_TIMEOUT, git_dirty_count(root))
        .await
        .unwrap_or_default();

    Ok(Json(json!({
        "branch": branch,
        "dirty_count": dirty_count,
    })))
}
// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // `.unwrap()` on setup ops is idiomatic in tests

    use super::*;
    use crate::api::routes::{build_routes, test_support};
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::Service;

    /// Full registered router over the fixture state (auth disabled in the
    /// fixture config), so tests exercise the real route table.
    fn router(app: &test_support::TestApp) -> Router {
        build_routes(app.state.clone()).with_state(app.state.clone())
    }

    async fn send(
        svc: &mut Router,
        method: &str,
        uri: &str,
        body: Option<String>,
    ) -> (StatusCode, serde_json::Value) {
        let builder = Request::builder().method(method).uri(uri);
        let request = match body {
            Some(text) => builder
                .header("content-type", "application/json")
                .body(Body::from(text))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let response = svc.call(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024 * 1024)
            .await
            .unwrap();
        let json = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, json)
    }

    /// Seed a project workspace: `<root>/{a.txt, sub/note.md}` plus ignored
    /// entries (`node_modules/`, `.git/`, `target/`).
    fn seed_workspace(root: &Path) {
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.txt"), "alpha").unwrap();
        std::fs::write(root.join("sub").join("note.md"), "note body").unwrap();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::fs::write(root.join("node_modules").join("pkg.js"), "x").unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
    }

    fn tmpdir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    // ── unit: boundary helper ──

    #[test]
    fn is_within_roots_accepts_root_and_children() {
        let root = PathBuf::from("/tmp/ws");
        let roots = vec![root.clone()];
        assert!(is_within_roots(&root, &roots));
        assert!(is_within_roots(&root.join("sub").join("f.txt"), &roots));
    }

    #[test]
    fn is_within_roots_enforces_byte_boundary() {
        // `/tmp/ws-other` shares a prefix with `/tmp/ws` but must not match.
        let roots = vec![PathBuf::from("/tmp/ws")];
        assert!(!is_within_roots(Path::new("/tmp/ws-other"), &roots));
        assert!(!is_within_roots(Path::new("/tmp"), &roots));
        assert!(!is_within_roots(Path::new("/etc/passwd"), &roots));
    }

    // ── tree ──

    #[tokio::test]
    async fn tree_lists_seeded_dir_skipping_ignored_entries() {
        let tmp = tmpdir();
        let root = tmp.path().join("repo");
        seed_workspace(&root);
        let app = test_support::test_app(tmp.path());
        let sid = test_support::session_for_project(&app, "tree-project", vec![root.clone()]).await;

        let mut svc = router(&app);
        let (status, body) = send(
            &mut svc,
            "GET",
            &format!("/api/project/workspace/tree?session_id={sid}"),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK, "body: {body}");
        let canon = root.canonicalize().unwrap();
        assert_eq!(body["roots"], serde_json::json!([canon.to_string_lossy()]));
        assert_eq!(body["path"], canon.to_string_lossy().to_string());

        let entries = body["entries"].as_array().unwrap();
        let names: Vec<&str> = entries
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        // Dirs first (alphabetical), then files (alphabetical); ignored
        // entries never appear.
        assert_eq!(names, vec!["sub", "a.txt"], "entries: {entries:?}");
        assert_eq!(entries[0]["is_dir"], serde_json::json!(true));
        assert_eq!(entries[0]["size"], serde_json::json!(0));
        assert_eq!(entries[1]["is_dir"], serde_json::json!(false));
        assert_eq!(entries[1]["size"], serde_json::json!(5));
        assert_eq!(
            entries[1]["path"],
            canon.join("a.txt").to_string_lossy().to_string()
        );
    }

    #[tokio::test]
    async fn tree_lists_explicit_subdirectory() {
        let tmp = tmpdir();
        let root = tmp.path().join("repo");
        seed_workspace(&root);
        let app = test_support::test_app(tmp.path());
        let sid = test_support::session_for_project(&app, "tree-project", vec![root.clone()]).await;

        let canon = root.canonicalize().unwrap();
        let mut svc = router(&app);
        let (status, body) = send(
            &mut svc,
            "GET",
            &format!(
                "/api/project/workspace/tree?session_id={sid}&path={}",
                canon.join("sub").to_string_lossy()
            ),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(
            body["path"],
            serde_json::json!(canon.join("sub").to_string_lossy().to_string())
        );
        let names: Vec<&str> = body["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["note.md"]);
    }

    #[tokio::test]
    async fn tree_rejects_path_outside_roots() {
        let tmp = tmpdir();
        let root = tmp.path().join("repo");
        seed_workspace(&root);
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();

        let app = test_support::test_app(tmp.path());
        let sid = test_support::session_for_project(&app, "tree-project", vec![root]).await;

        let mut svc = router(&app);
        let (status, body) = send(
            &mut svc,
            "GET",
            &format!(
                "/api/project/workspace/tree?session_id={sid}&path={}",
                outside.canonicalize().unwrap().display()
            ),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
        let message = body["error"].as_str().unwrap();
        assert!(
            message.contains("outside the project"),
            "English error expected, got: {message}"
        );
    }

    // ── file read ──

    #[tokio::test]
    async fn file_read_roundtrips_content() {
        let tmp = tmpdir();
        let root = tmp.path().join("repo");
        seed_workspace(&root);
        let app = test_support::test_app(tmp.path());
        let sid = test_support::session_for_project(&app, "read-project", vec![root.clone()]).await;

        let canon = root.canonicalize().unwrap();
        let mut svc = router(&app);
        let (status, body) = send(
            &mut svc,
            "GET",
            &format!(
                "/api/project/workspace/file?session_id={sid}&path={}",
                canon.join("a.txt").display()
            ),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(
            body["path"],
            canon.join("a.txt").to_string_lossy().to_string()
        );
        assert_eq!(body["content"], serde_json::json!("alpha"));
        assert_eq!(body["size"], serde_json::json!(5));
        assert_eq!(body["truncated"], serde_json::json!(false));
    }

    #[tokio::test]
    async fn file_read_flags_oversized_file_truncated_with_empty_content() {
        let tmp = tmpdir();
        let root = tmp.path().join("repo");
        seed_workspace(&root);
        let big = root.join("big.bin");
        std::fs::write(&big, vec![b'x'; (MAX_READ_BYTES + 1) as usize]).unwrap();

        let app = test_support::test_app(tmp.path());
        let sid = test_support::session_for_project(&app, "read-project", vec![root.clone()]).await;

        let canon = root.canonicalize().unwrap();
        let mut svc = router(&app);
        let (status, body) = send(
            &mut svc,
            "GET",
            &format!(
                "/api/project/workspace/file?session_id={sid}&path={}",
                canon.join("big.bin").display()
            ),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(body["truncated"], serde_json::json!(true));
        assert_eq!(body["content"], serde_json::json!(""));
        assert_eq!(
            body["size"],
            serde_json::json!(MAX_READ_BYTES + 1),
            "size reports the on-disk size even when truncated"
        );
    }

    #[tokio::test]
    async fn file_read_outside_roots_is_rejected_in_english() {
        let tmp = tmpdir();
        let root = tmp.path().join("repo");
        seed_workspace(&root);
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let outside_file = outside.join("secret.txt");
        std::fs::write(&outside_file, "nope").unwrap();

        let app = test_support::test_app(tmp.path());
        let sid = test_support::session_for_project(&app, "read-project", vec![root.clone()]).await;

        let mut svc = router(&app);
        let (status, body) = send(
            &mut svc,
            "GET",
            &format!(
                "/api/project/workspace/file?session_id={sid}&path={}",
                outside_file.canonicalize().unwrap().display()
            ),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
        assert!(body["error"].as_str().unwrap().contains("outside"));

        // A path inside the roots that does not exist → 404, English.
        let canon = root.canonicalize().unwrap();
        let (status, body) = send(
            &mut svc,
            "GET",
            &format!(
                "/api/project/workspace/file?session_id={sid}&path={}",
                canon.join("missing.txt").display()
            ),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
        assert!(body["error"].as_str().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn file_read_via_symlink_escape_is_rejected() {
        let tmp = tmpdir();
        let root = tmp.path().join("repo");
        seed_workspace(&root);
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let outside_file = outside.join("secret.txt");
        std::fs::write(&outside_file, "nope").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside_file, root.join("escape.lnk")).unwrap();

        let app = test_support::test_app(tmp.path());
        let sid = test_support::session_for_project(&app, "read-project", vec![root.clone()]).await;

        let canon = root.canonicalize().unwrap();
        let mut svc = router(&app);
        let (status, body) = send(
            &mut svc,
            "GET",
            &format!(
                "/api/project/workspace/file?session_id={sid}&path={}",
                canon.join("escape.lnk").display()
            ),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
        assert!(body["error"].as_str().unwrap().contains("outside"));
    }

    // ── file write ──

    #[tokio::test]
    async fn put_creates_new_file() {
        let tmp = tmpdir();
        let root = tmp.path().join("repo");
        seed_workspace(&root);
        let app = test_support::test_app(tmp.path());
        let sid =
            test_support::session_for_project(&app, "write-project", vec![root.clone()]).await;

        let canon = root.canonicalize().unwrap();
        let target = canon.join("created").join("new.txt");
        std::fs::create_dir_all(canon.join("created")).unwrap();

        let mut svc = router(&app);
        let payload = serde_json::json!({
            "session_id": sid,
            "path": target.display().to_string(),
            "content": "written by the workbench",
        });
        let (status, body) = send(
            &mut svc,
            "PUT",
            "/api/project/workspace/file",
            Some(payload.to_string()),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "body: {body}");
        let written = std::fs::read_to_string(&target).unwrap();
        assert_eq!(written, "written by the workbench");
        assert_eq!(body["path"], target.to_string_lossy().to_string());
        assert_eq!(body["size"], serde_json::json!(written.len()));

        // Overwrite works too.
        let payload = serde_json::json!({
            "session_id": sid,
            "path": target.display().to_string(),
            "content": "overwritten",
        });
        let (status, _) = send(
            &mut svc,
            "PUT",
            "/api/project/workspace/file",
            Some(payload.to_string()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "overwritten");
    }

    #[tokio::test]
    async fn put_rejects_oversized_content_in_english() {
        let tmp = tmpdir();
        let root = tmp.path().join("repo");
        seed_workspace(&root);
        let app = test_support::test_app(tmp.path());
        let sid =
            test_support::session_for_project(&app, "write-project", vec![root.clone()]).await;

        let canon = root.canonicalize().unwrap();
        let mut svc = router(&app);
        let payload = serde_json::json!({
            "session_id": sid,
            "path": canon.join("too-big.txt").display().to_string(),
            "content": "x".repeat(MAX_WRITE_BYTES + 1),
        });
        let (status, body) = send(
            &mut svc,
            "PUT",
            "/api/project/workspace/file",
            Some(payload.to_string()),
        )
        .await;

        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "body: {body}");
        let message = body["error"].as_str().unwrap();
        assert!(
            message.contains("exceeds limit"),
            "English error expected, got: {message}"
        );
        assert!(!canon.join("too-big.txt").exists());
    }

    #[tokio::test]
    async fn put_via_symlink_escape_is_rejected() {
        let tmp = tmpdir();
        let root = tmp.path().join("repo");
        seed_workspace(&root);
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let outside_file = outside.join("victim.txt");
        std::fs::write(&outside_file, "original").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside_file, root.join("escape.lnk")).unwrap();

        let app = test_support::test_app(tmp.path());
        let sid =
            test_support::session_for_project(&app, "write-project", vec![root.clone()]).await;

        let canon = root.canonicalize().unwrap();
        let mut svc = router(&app);
        let payload = serde_json::json!({
            "session_id": sid,
            "path": canon.join("escape.lnk").display().to_string(),
            "content": "pwned",
        });
        let (status, body) = send(
            &mut svc,
            "PUT",
            "/api/project/workspace/file",
            Some(payload.to_string()),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
        assert_eq!(
            std::fs::read_to_string(&outside_file).unwrap(),
            "original",
            "the symlink target must be untouched"
        );
    }

    // ── scopeless sessions ──

    #[tokio::test]
    async fn scopeless_session_conflicts_on_every_endpoint() {
        let tmp = tmpdir();
        let root = tmp.path().join("repo");
        seed_workspace(&root);
        let app = test_support::test_app(tmp.path());
        let sid = test_support::scopeless_session(&app).await;

        let mut svc = router(&app);
        let canon = root.canonicalize().unwrap();
        let cases: Vec<(&str, String, Option<String>)> = vec![
            (
                "GET",
                format!("/api/project/workspace/tree?session_id={sid}"),
                None,
            ),
            (
                "GET",
                format!(
                    "/api/project/workspace/file?session_id={sid}&path={}",
                    canon.join("a.txt").display()
                ),
                None,
            ),
            (
                "PUT",
                "/api/project/workspace/file".to_string(),
                Some(
                    serde_json::json!({
                        "session_id": sid,
                        "path": canon.join("a.txt").display().to_string(),
                        "content": "hi",
                    })
                    .to_string(),
                ),
            ),
            (
                "GET",
                format!("/api/project/workspace/status?session_id={sid}"),
                None,
            ),
        ];

        for (method, uri, body) in cases {
            let (status, json) = send(&mut svc, method, &uri, body).await;
            assert_eq!(status, StatusCode::CONFLICT, "{method} {uri}: {json}");
            assert_eq!(
                json["error"],
                serde_json::json!("no project filesystem scope for this session"),
                "{method} {uri}"
            );
        }

        // An unknown session id behaves like a scopeless session.
        let (status, json) = send(
            &mut svc,
            "GET",
            &format!(
                "/api/project/workspace/tree?session_id={}",
                uuid::Uuid::new_v4()
            ),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(json["error"], serde_json::json!(NO_SCOPE_MESSAGE));
    }

    // ── status ──

    #[tokio::test]
    async fn status_reports_git_branch_and_dirty_count() {
        let tmp = tmpdir();
        let root = tmp.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("tracked.txt"), "dirty").unwrap();
        init_git_repo(&root);

        let app = test_support::test_app(tmp.path());
        let sid = test_support::session_for_project(&app, "git-project", vec![root.clone()]).await;

        let mut svc = router(&app);
        let (status, body) = send(
            &mut svc,
            "GET",
            &format!("/api/project/workspace/status?session_id={sid}"),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(body["branch"], serde_json::json!("main"));
        assert_eq!(body["dirty_count"], serde_json::json!(1));
    }

    #[tokio::test]
    async fn status_degrades_to_nulls_outside_git() {
        let tmp = tmpdir();
        let root = tmp.path().join("plain");
        std::fs::create_dir_all(&root).unwrap();

        let app = test_support::test_app(tmp.path());
        let sid = test_support::session_for_project(&app, "plain-project", vec![root]).await;

        let mut svc = router(&app);
        let (status, body) = send(
            &mut svc,
            "GET",
            &format!("/api/project/workspace/status?session_id={sid}"),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(body["branch"], serde_json::Value::Null);
        assert_eq!(body["dirty_count"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn status_rejects_out_of_range_root_index() {
        let tmp = tmpdir();
        let root = tmp.path().join("repo");
        seed_workspace(&root);
        let app = test_support::test_app(tmp.path());
        let sid = test_support::session_for_project(&app, "idx-project", vec![root.clone()]).await;

        let mut svc = router(&app);
        let (status, body) = send(
            &mut svc,
            "GET",
            &format!("/api/project/workspace/status?session_id={sid}&root=5"),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
        assert!(body["error"].as_str().unwrap().contains("out of range"));
    }

    /// `git init -b main` without touching the developer's global config.
    fn init_git_repo(root: &Path) {
        let output = std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .arg(root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .unwrap();
        assert!(output.status.success(), "git init failed: {output:?}");
    }
}
