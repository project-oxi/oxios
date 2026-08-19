//! Web surface plugin.
//!
//! Factory for creating the web control surface. Implements
//! [`Surface`](crate::surface::Surface) so the main binary can
//! activate the web dashboard with full kernel access.
//!
//! The web surface is both a control plane (kernel management,
//! monitoring, configuration) and a message interface (chat via gateway).
//!
//! **Auto-update**: The web UI can be updated by placing a new build in
//! `~/.oxios/web/dist/` (checked first) or `<workspace>/web/dist/` (fallback).
//! If neither exists, the web UI is automatically downloaded from GitHub Releases.
//! The server reads from filesystem on every request — no restart needed.

use anyhow::Result;
use async_trait::async_trait;
use axum::{Router, body::Body, response::Response, routing::get};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use oxios_gateway::surface::{Surface, SurfaceContext, SurfaceHandle};

use crate::api::api_docs;
use crate::api::bridge::{WebBridge, WebBridgeHandle};
use crate::api::middleware::RateLimiter;
use crate::api::routes;
use crate::api::server::AppState;
use oxios_gateway::ReliabilityLayer;
use tower_http::compression::CompressionLayer;

// Web UI serving. When the binary is compiled with a built `web/dist/`
// present, `build.rs` emits `web_embedded` and `src/embedded_web.rs` bakes
// the SPA into the binary — served EXCLUSIVELY: no first-run download, and
// no on-disk dist can override it (a manual override used to silently
// shadow binary deploys with stale UIs — removed 2026-08-19). When the
// cfg is absent (`cargo install` from crates.io), `ensure_web_dist`
// downloads `web-dist.zip` from GitHub Releases at startup and serving
// goes through the active-dist pointer (RFC-024 SP3/C3).

// ---------------------------------------------------------------------------
// Filesystem serving (RFC-024 SP3: atomic pointer + immutable cache)
// ---------------------------------------------------------------------------

/// Reads a file from the filesystem dist directory.
/// Returns `None` if the file doesn't exist.
fn fs_read(dist: &std::path::Path, path: &str) -> Option<Vec<u8>> {
    let clean = path.trim_start_matches('/');
    // Reject path traversal (.., absolute, drive prefix) before joining.
    let p = std::path::Path::new(clean);
    if p.components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        return None;
    }
    let file_path = dist.join(clean);
    // Confirm the resolved path stays inside dist — defends against symlinks
    // and any residual traversal the component filter missed.
    let canon_file = file_path.canonicalize().ok()?;
    let canon_dist = dist.canonicalize().ok()?;
    if !canon_file.starts_with(&canon_dist) {
        return None;
    }
    std::fs::read(&canon_file).ok()
}

/// Determines MIME type from file path.
fn mime_type(path: &str) -> axum::http::HeaderValue {
    let clean = path.trim_start_matches('/');
    mime_guess::from_path(clean)
        .first_or_octet_stream()
        .to_string()
        .parse()
        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("application/octet-stream"))
}

/// Whether `path` is a content-addressed asset (hashed filename under
/// `assets/`). Such files are safe to cache as immutable because a new
/// build emits new hashes, so the URL itself is the cache key.
fn is_immutable_asset(path: &str) -> bool {
    let clean = path.trim_start_matches('/');
    clean.starts_with("assets/")
}

/// Compute a weak ETag from file content using SipHash-1-3.
///
/// Not cryptographically strong, but deterministic within a process
/// lifetime — sufficient for cache validation (browser re-requests
/// after restart get a full response, which is fine).
fn compute_etag(data: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    format!("\"{:x}\"", hasher.finish())
}

/// Read the active web version from `<dist>/version.json` (for the
/// `X-Web-Version` header). Returns `"dev"` when not present.
fn read_active_version(dist: &std::path::Path) -> String {
    #[derive(serde::Deserialize)]
    struct VersionFile {
        version: Option<String>,
    }
    std::fs::read(dist.join("version.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<VersionFile>(&b).ok())
        .and_then(|v| v.version)
        .unwrap_or_else(|| "dev".to_string())
}

/// Whether a gateway `host` string binds to a loopback interface.
///
/// Used by the F2 non-loopback + auth-disabled warning. Accepts the
/// literal loopback addresses (127.0.0.0/8 spelled out as 127.0.0.1,
/// the IPv6 loopback `::1`, and the `localhost` name) plus an empty
/// host (axum binds to 127.0.0.1 by default when given an empty string).
fn is_loopback_host(host: &str) -> bool {
    let h = host.trim().to_ascii_lowercase();
    if h.is_empty() || h == "localhost" {
        return true;
    }
    // IPv6 loopback (strip optional brackets/zone).
    let h = h.trim_start_matches('[').trim_end_matches(']');
    if h == "::1" {
        return true;
    }
    // IPv4 127.0.0.0/8.
    if let Some(rest) = h.strip_prefix("127.")
        && let Some(first) = rest.split('.').next()
        && first.bytes().all(|b| b.is_ascii_digit())
    {
        return true;
    }
    false
}

/// Build a cache-correct response for asset bytes. Shared by the active-dist
/// and embedded serving paths so MIME, ETag, and immutability semantics stay
/// identical (RFC-024 C3).
fn asset_response(data: Vec<u8>, clean: &str, if_none_match: Option<&str>) -> Response {
    // Dist files live either at root or under assets/ — normalize so
    // is_immutable_asset and mime_type resolve the same regardless of which
    // lookup path found the bytes.
    let lookup = if clean.starts_with("assets/") {
        clean.to_string()
    } else {
        format!("assets/{clean}")
    };
    let immutable = is_immutable_asset(&lookup);
    let cache = if immutable {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    // ETag + conditional request for non-immutable assets. Immutable (hashed)
    // assets skip ETag — their URL changes when content changes, and the
    // Cache-Control: immutable directive forbids revalidation.
    if !immutable {
        let etag = compute_etag(&data);
        if let Some(client_etag) = if_none_match {
            // Accept both weak and strong comparison (RFC 7232 §2.3.2).
            let client_etag = client_etag.trim().trim_start_matches("W/");
            let our_etag = etag.trim_matches('"');
            if client_etag.trim_matches('"') == our_etag {
                return Response::builder()
                    .status(304)
                    .header("Cache-Control", cache)
                    .header("ETag", &etag)
                    .body(Body::empty())
                    .expect("static response build is infallible");
            }
        }
        return Response::builder()
            .status(200)
            .header("Content-Type", mime_type(&lookup))
            .header("Cache-Control", cache)
            .header("ETag", &etag)
            .body(Body::from(data))
            .expect("static response build is infallible");
    }
    Response::builder()
        .status(200)
        .header("Content-Type", mime_type(&lookup))
        .header("Cache-Control", cache)
        .body(Body::from(data))
        .expect("static response build is infallible")
}

/// Serve a static file.
///
/// **Embedded-first:** when the binary has the SPA compiled in, it is served
/// exclusively — an active-dist pointer is never consulted, so nothing on
/// disk can shadow the binary's UI. Otherwise (crates.io builds) an active
/// dist is served *only* from itself with no embedded fallback, so a request
/// never mixes two build hashes (RFC-024 C3).
fn serve_file(dist: Option<&std::path::Path>, path: &str, if_none_match: Option<&str>) -> Response {
    let clean = path.trim_start_matches('/');

    // Embedded assets: authoritative and exclusive when compiled in.
    if crate::embedded_web::is_embedded() {
        if let Some(data) = crate::embedded_web::get(clean)
            .or_else(|| crate::embedded_web::get(&format!("assets/{clean}")))
        {
            return asset_response(data.to_vec(), clean, if_none_match);
        }
        return Response::builder()
            .status(404)
            .body(Body::empty())
            .expect("static response build is infallible");
    }

    // Non-embedded: active dist is served ONLY from it (C3 — no fallback on
    // miss).
    if let Some(d) = dist {
        if let Some(data) = fs_read(d, clean).or_else(|| fs_read(d, &format!("assets/{clean}"))) {
            return asset_response(data, clean, if_none_match);
        }
        return Response::builder()
            .status(404)
            .body(Body::empty())
            .expect("static response build is infallible");
    }

    // No active dist and no embedded assets (crates.io build, download not
    // yet complete) → 503 so the client retries. `ensure_web_dist` makes
    // this transient.
    Response::builder()
        .status(503)
        .header("Retry-After", "5")
        .body(Body::from("Web UI dist not available yet — retry shortly"))
        .expect("static response build is infallible")
}

/// Static asset handler.
///
/// Serves wildcard `assets/*` paths AND fixed root files (`/favicon.png`,
/// `/apple-touch-icon.png`, …). Fixed routes carry no `{*path}` capture, so
/// the `Path` extractor is absent there — falling back to the request URI
/// avoids axum's "Expected 1 path argument but got 0" rejection, which
/// previously 500'd every fixed asset request (incl. the old `/favicon.svg`).
async fn static_handler(
    path: Option<axum::extract::Path<String>>,
    uri: axum::extract::OriginalUri,
    headers: axum::http::HeaderMap,
    state: axum::extract::State<Arc<AppState>>,
) -> Response {
    // RFC-024 SP3: load the atomic pointer per request (O(1)).
    let dist = state.web_dist.path();
    let if_none_match = headers
        .get(axum::http::header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());
    let path = path
        .map(|p| p.0)
        .unwrap_or_else(|| uri.path().trim_start_matches('/').to_string());
    serve_file(dist.as_deref(), &path, if_none_match)
}

/// SPA fallback — serves index.html for client-side routing.
async fn spa_handler(
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    headers: axum::http::HeaderMap,
    state: axum::extract::State<Arc<AppState>>,
) -> Response {
    // Never serve the SPA fallback for API paths. An unmatched /api/* route
    // means the endpoint is missing or the backend binary is stale. Return a
    // 404 JSON so the client's apiClient throws ApiError (instead of silently
    // parsing the HTML index page as the response type and crashing on .map).
    if uri.path().starts_with("/api/") {
        return Response::builder()
            .status(404)
            .header("Content-Type", "application/json")
            .body(Body::from(
                r#"{"error":"Not Found","detail":"API route not registered"}"#,
            ))
            .expect("static response build is infallible");
    }
    // RFC-024 SP3: load the atomic pointer per request.
    let dist = state.web_dist.path();

    // Active dist: serve its index.html, annotated with the active version
    // so clients can detect a version switch (3-source consistency). index.html
    // is never cached immutably — it is the pointer to the hashed assets.
    if let Some(ref d) = dist
        && let Some(data) = fs_read(d, "index.html")
    {
        let version = read_active_version(d);
        let etag = compute_etag(&data);

        // Check If-None-Match for conditional request.
        if let Some(client_etag) = headers
            .get(axum::http::header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
        {
            let client_etag = client_etag.trim().trim_start_matches("W/");
            if client_etag.trim_matches('"') == etag.trim_matches('"') {
                return Response::builder()
                    .status(304)
                    .header("Cache-Control", "no-cache")
                    .header("ETag", &etag)
                    .header("X-Web-Version", version)
                    .body(Body::empty())
                    .expect("static response build is infallible");
            }
        }

        return Response::builder()
            .status(200)
            .header("Content-Type", "text/html; charset=utf-8")
            .header("Cache-Control", "no-cache")
            .header("ETag", &etag)
            .header("X-Web-Version", version)
            .body(Body::from(data))
            .expect("static response build is infallible");
    }

    // No active dist → embedded assets (authoritative when compiled in).
    if let Some(data) = crate::embedded_web::get("index.html") {
        let version = crate::embedded_web::version();
        let etag = compute_etag(data);
        if let Some(client_etag) = headers
            .get(axum::http::header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
        {
            let client_etag = client_etag.trim().trim_start_matches("W/");
            if client_etag.trim_matches('"') == etag.trim_matches('"') {
                return Response::builder()
                    .status(304)
                    .header("Cache-Control", "no-cache")
                    .header("ETag", &etag)
                    .header("X-Web-Version", version)
                    .body(Body::empty())
                    .expect("static response build is infallible");
            }
        }
        return Response::builder()
            .status(200)
            .header("Content-Type", "text/html; charset=utf-8")
            .header("Cache-Control", "no-cache")
            .header("ETag", &etag)
            .header("X-Web-Version", version)
            .body(Body::from(data.to_vec()))
            .expect("static response build is infallible");
    }

    // No active dist and no embedded assets (crates.io build whose startup
    // download hasn't completed). `web_dist.rs` makes this transient; 503.
    Response::builder()
        .status(503)
        .header("Retry-After", "5")
        .body(Body::from("Web UI dist not available yet — retry shortly"))
        .expect("static response build is infallible")
}

/// Web surface — kernel-connected control dashboard.
pub struct WebSurface;

impl WebSurface {
    /// Create a new web surface instance.
    pub fn new() -> Self {
        Self
    }
}

impl Default for WebSurface {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Surface for WebSurface {
    fn name(&self) -> &str {
        "web"
    }

    async fn start(&self, ctx: SurfaceContext) -> Result<SurfaceHandle> {
        let config = ctx.config.read().clone();
        let host = config.gateway.host.clone();
        let port = config.gateway.port;
        // F2: surface the dangerous "non-loopback bind + auth disabled"
        // combination. When auth_enabled is false the API has no defense
        // for destructive endpoints (POST /api/mcp/servers, POST
        // /api/update/run, PUT /api/config, POST /api/system/backup, …).
        // Binding those to a public interface is remote code execution.
        let auth_enabled = config.security.auth_enabled;
        let is_loopback = is_loopback_host(&host);
        if !auth_enabled && !is_loopback {
            tracing::error!(
                host = %host,
                port,
                "SECURITY: HTTP API is binding to a non-loopback interface \
                 ({host}) with auth_enabled=false. Destructive endpoints \
                 (MCP server spawn, update/run, config write, backup) will \
                 be reachable WITHOUT authentication. Set \
                 [security].auth_enabled=true, or bind to 127.0.0.1/::1/localhost."
            );
            // Also print to stderr so it is visible even when logs are not
            // being tailed (e.g. systemd journalctl, container stdout).
            eprintln!(
                "⚠️  Oxios: HTTP API binding to {host}:{port} with auth_enabled=false.\n\
                 ⚠️  Destructive endpoints are UNAUTHENTICATED. Set auth_enabled=true \
                 or bind to a loopback address."
            );
        }

        let rate_limit = config.security.rate_limit_per_minute;

        // Use the pre-resolved web dist path from SurfaceContext.
        // `web_dist.rs` in the binary already downloaded it before this surface starts.
        // `None` here means we'll fall back to embedded assets.
        let web_dist = ctx.web_dist;

        // Create web channel for gateway message routing. Each web bridge
        // owns its own reliability layer (RFC-024 SP2): the gateway's
        // global layer is the source of truth, but the bridge layer is
        // what WS resume handlers query for replay.
        let web_channel = WebBridge::new(256, Arc::new(ReliabilityLayer::new(Default::default())));
        // RFC-024 SP1 / C1: pull the response timeout from config so
        // operators can tune the HTTP→gateway ceiling per environment.
        let response_timeout = std::time::Duration::from_secs(config.gateway.response_timeout_secs);
        let bridge_handle =
            WebBridgeHandle::from_bridge(&web_channel).with_response_timeout(response_timeout);

        // Build app state — all knowledge access goes through kernel.knowledge
        // Task store (RFC-043) — attached by the kernel assembler; the web
        // surface hard-requires it.
        let task_store = ctx
            .kernel
            .task_store
            .clone()
            .expect("task store not attached to kernel handle");

        let state = Arc::new(AppState {
            base_url: format!("http://{host}:{port}"),
            kernel: ctx.kernel.clone(),
            bridge: bridge_handle,
            config: ctx.config.clone(),
            config_path: ctx.config_path.clone(),
            start_time: ctx.kernel.start_time(),
            rate_limiter: RateLimiter::new(rate_limit),
            web_dist,
            readiness: ctx.kernel.readiness.clone(),
            gateway: ctx.gateway.clone(),
            task_store,
        });

        // Build API routes
        let api_routes = routes::build_routes(state.clone());

        // CORS layer
        let cors_origins: Vec<_> = config
            .security
            .cors_origins
            .iter()
            .filter_map(|o| o.parse::<axum::http::HeaderValue>().ok())
            .collect();
        let cors = tower_http::cors::CorsLayer::new()
            .allow_origin(cors_origins)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any);

        // OpenAPI / Swagger UI — gated by `gateway.expose_api_docs` AND
        // a loopback bind. See `GatewayConfig::should_expose_api_docs`.
        let should_expose_docs = state.config.read().gateway.should_expose_api_docs();

        // SPA routes (defined first so we can merge them into `app`).
        let spa_routes: Router<Arc<AppState>> = Router::new()
            .route("/assets/{*path}", get(static_handler))
            .route("/favicon.png", get(static_handler))
            .route("/apple-touch-icon.png", get(static_handler))
            .route("/icons.svg", get(static_handler))
            .route("/{*path}", get(spa_handler))
            .route("/", get(spa_handler));

        let mut app = Router::new()
            .merge(api_routes)
            .merge(spa_routes)
            .layer(CompressionLayer::new())
            .layer(cors);

        if should_expose_docs {
            let openapi = api_docs::build_openapi();
            let swagger: Router<()> = utoipa_swagger_ui::SwaggerUi::new("/api-docs")
                .url("/openapi.json", openapi)
                .into();
            app = app.nest_service("/api-docs", swagger);
            tracing::info!("API docs exposed at /api-docs and /openapi.json");
        } else {
            tracing::info!(
                "API docs disabled (set gateway.expose_api_docs=true on a loopback bind to enable)"
            );
        }
        // Spawn the task auto-run tick loop BEFORE `state` is moved into the
        // router. It clones the Arc internally.
        let task_tick = spawn_task_auto_run(state.clone());

        let app = app.with_state(state);

        // Bind listener
        let addr = format!("{host}:{port}");
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!(addr = %addr, "Web server listening");

        // RFC-030 A5: use the shared shutdown token rather than an independent
        // ctrl_c consumer. The supervisor owns the single shutdown signal; on
        // graceful shutdown it cancels the root token, which cascades here so
        // axum drains in-flight requests before stopping (no task.abort()).
        let shutdown = ctx.shutdown.clone();

        // Spawn server. Returns () — the supervisor observes the JoinHandle's
        // completion and applies its policy (scoped restart on unexpected exit).
        // Under panic=abort in release, a panic aborts the process directly and
        // the OS supervisor restarts; the non-panic Err path is what the
        // supervisor intercepts here.
        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    shutdown.cancelled().await;
                    tracing::info!("Web server shutting down (graceful)");
                })
                .await
            {
                tracing::error!(error = %e, "Web server error");
            }
        });

        Ok(SurfaceHandle {
            channel: Some(Box::new(web_channel)),
            tasks: vec![handle, task_tick],
        })
    }
}

/// Background loop that drives scheduled/heartbeat tasks.
///
/// Every 60 s, polls `list_due_tasks` and executes each due task on its own
/// task (bounded concurrency via spawn; a stuck LLM call doesn't block the
/// tick). Each run goes through the shared `execute_task_run` helper, so the
/// manual run endpoint and this loop share ONE execution path.
///
/// On boot it first recovers tasks stranded at `running` by a prior crash
/// (the Task model persists status to SQLite, unlike the CronScheduler's
/// in-memory set).
fn spawn_task_auto_run(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    use oxios_kernel::task::{TaskAutomationMode, TaskRunTrigger, execute_task_run};

    tokio::spawn(async move {
        let task_store = state.task_store.clone();
        let kernel = state.kernel.clone();

        // Boot recovery: close orphaned runs + reset stranded tasks.
        if let Err(e) = task_store.lock().await.recover_stranded().await {
            tracing::warn!(error = %e, "Task stranded-recovery failed");
        }

        // First tick fires immediately (interval semantics); consume it so we
        // don't double-fire right after recovery, then poll every 60 s.
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        interval.tick().await;

        loop {
            interval.tick().await;
            let now = chrono::Utc::now();
            let due = match task_store.lock().await.list_due_tasks().await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(error = %e, "list_due_tasks failed");
                    continue;
                }
            };
            for task in due {
                // Safety: skip exhausted tasks (list_due_tasks shouldn't
                // return them, but guard against a race).
                if task.is_exhausted() {
                    continue;
                }

                // Dependency gate (RFC-043): defer until all dependencies
                // reach `completed`. Manual runs bypass this — explicit user
                // intent. Push next_run_at to the next eligible slot so the
                // task doesn't pop up on every tick.
                let unsatisfied = match task_store
                    .lock()
                    .await
                    .unsatisfied_dependencies(&task.id)
                    .await
                {
                    Ok(u) => u,
                    Err(e) => {
                        tracing::warn!(task = %task.id, error = %e, "unsatisfied_dependencies failed");
                        Vec::new()
                    }
                };
                if !unsatisfied.is_empty() {
                    tracing::debug!(task = %task.id, ?unsatisfied, "deferring: deps not completed");
                    let next = match task.automation_mode {
                        Some(TaskAutomationMode::Schedule) => task
                            .schedule_pattern
                            .as_deref()
                            .and_then(|p| oxios_kernel::task::cron_next_after(p, &now).ok()),
                        Some(TaskAutomationMode::Heartbeat) => task
                            .heartbeat_interval_secs
                            .map(|s| (now + chrono::Duration::seconds(s as i64)).to_rfc3339()),
                        None => None,
                    };
                    let _ = task_store
                        .lock()
                        .await
                        .set_next_run(&task.id, next.as_deref())
                        .await;
                    continue;
                }

                let trigger = match task.automation_mode {
                    Some(TaskAutomationMode::Schedule) => TaskRunTrigger::Schedule,
                    _ => TaskRunTrigger::Heartbeat,
                };
                let ts = task_store.clone();
                let kh = kernel.clone();
                let id = task.id.clone();
                tokio::spawn(async move {
                    // Scheduled runs use the longer cron-style ceiling (600 s).
                    let (_, success, _summary) = execute_task_run(ts, kh, &id, trigger, 600).await;
                    tracing::info!(%id, success, "Scheduled task run completed");
                });
            }
        }
    })
}
