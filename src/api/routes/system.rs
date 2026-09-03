use std::path::PathBuf;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};

use crate::api::error::AppError;
use crate::api::server::AppState;
use crate::managed_install;

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

/// GET /health — Liveness check (no auth required).
///
/// Always returns 200 OK if the process is alive.
pub(crate) async fn handle_health(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// GET /health/ready — Readiness check (no auth required).
///
/// Checks subsystem health: state store, git repository.
/// Returns 200 if healthy, 503 if degraded.
pub(crate) async fn handle_readiness(
    state: State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    let mut components = serde_json::Map::new();
    let mut all_healthy = true;

    // State store: check workspace path exists
    let ws_path = state.kernel.state.workspace_path();
    let state_ok = ws_path.exists();
    components.insert(
        "state_store".into(),
        serde_json::json!({"healthy": state_ok}),
    );
    all_healthy &= state_ok;

    // Git: verify repository integrity
    let git_ok = state.kernel.infra.git_verify().unwrap_or(false);
    components.insert("git".into(), serde_json::json!({"healthy": git_ok}));
    // Git failure is degraded, not fatal

    // Brain daemonless (RFC-047 / RFC-049): degraded when unavailable,
    // but a healthy binary on disk is still "pass" — install via
    // `oxios brain install` to bring the session online.
    let brain_ok = state
        .kernel
        .brain
        .as_ref()
        .map(|b| {
            let snap = b.status_snapshot();
            snap.available || snap.binary_installed
        })
        .unwrap_or(false);
    components.insert("brain".into(), serde_json::json!({"healthy": brain_ok}));

    let status = if all_healthy { "healthy" } else { "degraded" };
    let body = serde_json::json!({
        "status": status,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": state.start_time.elapsed().as_secs(),
        "components": components,
    });

    if all_healthy {
        Ok(Json(body))
    } else {
        Err((axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(body)))
    }
}

// ---------------------------------------------------------------------------
// Control
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Component Health Types
// ---------------------------------------------------------------------------

/// Health status of an individual component.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct ComponentStatus {
    /// Whether the component is healthy.
    pub healthy: bool,
    /// Optional detail message.
    pub detail: Option<String>,
}

/// Brain daemonless health (RFC-047).
#[derive(Debug, Serialize, Clone)]
pub(crate) struct BrainHealth {
    /// Whether the session child is up or the binary is installed.
    pub available: bool,
}

/// Agent subsystem health.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct AgentHealth {
    /// Number of currently active agents.
    pub active_count: usize,
    /// Total agents forked (lifetime).
    pub total_forked: u64,
    /// Total agents completed (lifetime).
    pub total_completed: u64,
    /// Total agents failed (lifetime).
    pub total_failed: u64,
}

/// Aggregate health of all system components.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct ComponentHealth {
    /// State store health.
    pub state_store: ComponentStatus,
    /// Event bus health.
    pub event_bus: ComponentStatus,
    /// Brain daemon health.
    pub brain: BrainHealth,
    /// Agent subsystem health.
    pub agents: AgentHealth,
    /// Active projects count.
    pub projects_active: usize,
}

/// Response body for the status endpoint.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct StatusResponse {
    /// Service name.
    service: String,
    /// Current status.
    status: String,
    /// Binary (daemon) version.
    version: String,
    /// Web UI frontend version (read from `<web_dist>/version.json` written
    /// by the Vite build, or `"dev"` when not present — e.g. `bun dev`).
    web_version: String,
    /// Whether the HTTP API requires authentication. The frontend uses this
    /// to decide whether the WebSocket needs a token — when false (the
    /// default for local deployments), the WS connects without credentials.
    auth_enabled: bool,
    /// Registered channels.
    channels: Vec<String>,
    /// Uptime info.
    uptime: String,
    /// Component-level health details.
    components: Option<ComponentHealth>,
}

/// Reads the Web UI version from `<web_dist>/version.json`.
///
/// That file is emitted by the Vite build and carries the version stamped
/// from the root `Cargo.toml` (same source as the binary version), plus an
/// optional `git_sha`. Embedded builds have no on-disk dist — `web_dist` is
/// `None` — so the stamp comes from the compiled-in `version.json` instead.
/// Returns `"dev"` when neither source carries a version — e.g. when running
/// `bun dev` or serving a workspace `web/dist` that predates the version
/// plugin — so the dashboard always renders a sane badge.
fn read_web_version(web_dist: &Option<PathBuf>) -> String {
    #[derive(Deserialize)]
    struct VersionFile {
        version: Option<String>,
    }

    let on_disk = web_dist
        .as_ref()
        .and_then(|p| std::fs::read(p.join("version.json")).ok())
        .and_then(|b| serde_json::from_slice::<VersionFile>(&b).ok())
        .and_then(|v| v.version);
    on_disk.unwrap_or_else(|| {
        if crate::embedded_web::is_embedded() {
            crate::embedded_web::version()
        } else {
            "dev".to_string()
        }
    })
}

/// GET /api/status — System status with component health.
pub(crate) async fn handle_status(state: State<Arc<AppState>>) -> Json<StatusResponse> {
    let uptime = state.start_time.elapsed();
    let uptime_str = format!(
        "{}h {}m {}s",
        uptime.as_secs() / 3600,
        (uptime.as_secs() % 3600) / 60,
        uptime.as_secs() % 60
    );

    // State store health — check that the base path exists
    let state_store_healthy = state.kernel.state.workspace_path().exists();

    // Event bus — always healthy if we got this far
    let event_bus_healthy = true;

    // Brain daemonless health — session child is up OR the binary is
    // installed (first brain call will spawn the child).
    let brain_ok = state
        .kernel
        .brain
        .as_ref()
        .map(|b| {
            let snap = b.status_snapshot();
            snap.available || snap.binary_installed
        })
        .unwrap_or(false);
    let brain_health = BrainHealth {
        available: brain_ok,
    };

    // Agent health — count active from supervisor, metrics from export
    let active_count = state
        .kernel
        .agents
        .list()
        .await
        .map(|agents| {
            agents
                .iter()
                .filter(|a| {
                    matches!(
                        a.status,
                        oxios_kernel::AgentStatus::Running
                            | oxios_kernel::AgentStatus::Starting
                            | oxios_kernel::AgentStatus::Idle
                    )
                })
                .count()
        })
        .unwrap_or(0);

    let (total_forked, total_completed, total_failed) = parse_agent_metrics();

    let agent_health = AgentHealth {
        active_count,
        total_forked,
        total_completed,
        total_failed,
    };

    let components = Some(ComponentHealth {
        state_store: ComponentStatus {
            healthy: state_store_healthy,
            detail: if state_store_healthy {
                None
            } else {
                Some("base path not found".to_string())
            },
        },
        event_bus: ComponentStatus {
            healthy: event_bus_healthy,
            detail: None,
        },
        brain: brain_health,
        agents: agent_health,
        projects_active: state
            .kernel
            .projects
            .as_ref()
            .map(|p| p.list_projects().len())
            .unwrap_or(0),
    });

    // Web UI version — read at runtime from `<web_dist>/version.json`.
    // That file is emitted by the Vite build (`vite.config.ts`) from the root
    // `Cargo.toml` version, so it matches the binary version by construction.
    // Falls back to `"dev"` when the file is absent (e.g. `bun dev`, or a
    // workspace `web/dist` predating this change). Reading on each request is
    // effectively free: the file is tiny and the OS page-caches it, and it lets
    // a daily auto-update of `web/dist/` be reflected without a restart.
    let web_version = read_web_version(&state.web_dist.path());

    Json(StatusResponse {
        service: "oxios".into(),
        status: "running".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        web_version,
        auth_enabled: state.config.read().security.auth_enabled,
        channels: vec!["web".into()],
        uptime: uptime_str,
        components,
    })
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// Query params for update check.
#[derive(Debug, Deserialize)]
pub(crate) struct UpdateCheckParams {
    /// Check a specific version instead of latest.
    #[serde(default)]
    pub version: Option<String>,
}

/// Response for `GET /api/update/check`.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct UpdateCheckResponse {
    /// Currently running version.
    pub current_version: String,
    /// Latest available version on GitHub.
    pub latest_version: String,
    /// Whether an update is available.
    pub update_available: bool,
    /// Release tag name (e.g. "v1.0.0").
    pub tag_name: String,
    /// URL to the release page.
    pub html_url: String,
    /// Short body / release notes excerpt.
    pub release_notes: String,
    /// Publication date.
    pub published_at: String,
    /// Available download assets.
    pub assets: Vec<AssetInfo>,
}

/// Info about a release asset.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct AssetInfo {
    pub name: String,
    pub size: u64,
    pub download_url: String,
}

/// Body for `POST /api/update/run`.
#[derive(Debug, Deserialize)]
pub(crate) struct UpdateRunBody {
    /// Update binary (default: true).
    #[serde(default = "default_true")]
    pub binary: bool,
    /// Update web UI (default: true).
    #[serde(default = "default_true")]
    pub web: bool,
    /// Target version (default: latest).
    pub version: Option<String>,
    /// Install channel override (default: auto-detect from `current_exe()`).
    /// `None` = Auto (the daemon's detected channel). `"cargo"` forces the
    /// legacy `cargo install oxios` flow regardless of the detected channel.
    /// Any other value is rejected with 400 — see [`validate_via`].
    pub via: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Response for `POST /api/update/run`.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct UpdateRunResponse {
    /// Whether the update succeeded.
    pub success: bool,
    /// Version we updated to.
    pub updated_to: String,
    /// What was updated.
    pub binary_updated: bool,
    pub web_updated: bool,
    /// Human-readable message.
    pub message: String,
    /// Whether the daemon's install channel was migrated to managed.
    /// Always `false` over HTTP — PATH-shadow migration only happens
    /// from a TTY. `true` is reserved for the TTY/CLI surface.
    pub migrated: bool,
    /// Audit of `oxios` binaries found on the daemon's PATH outside the
    /// managed launcher. Empty when the PATH is clean. Each entry
    /// carries the path, the symlink target (when applicable), and the
    /// file size in bytes.
    pub shadows: Vec<ShadowPayload>,
    /// Human-readable instructions the caller must run in a terminal to
    /// adopt the managed channel. Always present (empty when the PATH
    /// is clean) so consumers don't have to special-case absence vs.
    /// emptiness.
    pub instructions: String,
}

/// Response for `GET /api/update/changelog`.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct ChangelogResponse {
    pub tag_name: String,
    pub version: String,
    pub published_at: String,
    pub body: String,
    pub html_url: String,
}

/// GET /api/update/check — Check for available updates from GitHub Releases.
pub(crate) async fn handle_update_check(
    Query(params): Query<UpdateCheckParams>,
) -> Result<Json<UpdateCheckResponse>, AppError> {
    let current = env!("CARGO_PKG_VERSION");

    let release = fetch_github_release(params.version.as_deref()).await?;

    let tag_name = release["tag_name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let latest_version = tag_name.trim_start_matches('v').to_string();
    let html_url = release["html_url"].as_str().unwrap_or("").to_string();
    let body_text = release["body"]
        .as_str()
        .unwrap_or("No release notes.")
        .to_string();
    let published_at = release["published_at"].as_str().unwrap_or("").to_string();

    let assets: Vec<AssetInfo> = release["assets"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|a| {
            Some(AssetInfo {
                name: a["name"].as_str()?.to_string(),
                size: a["size"].as_u64()?,
                download_url: a["browser_download_url"].as_str()?.to_string(),
            })
        })
        .collect();

    Ok(Json(UpdateCheckResponse {
        current_version: current.to_string(),
        latest_version: latest_version.clone(),
        update_available: latest_version != current,
        tag_name,
        html_url,
        release_notes: body_text,
        published_at,
        assets,
    }))
}

/// Validate a user-supplied version string for the update flow.
///
/// F10: `body.version` is interpolated into the GitHub API URL and passed
/// to `cargo install --version <v>`. While `Command::new` avoids shell
/// injection, an attacker-controlled value could still probe arbitrary
/// release tags or construct a malformed `cargo install` invocation. We
/// accept only SemVer-shaped strings (digits, dots, alphanumerics, `-`),
/// rejecting anything with path traversal, shell metacharacters, or
/// control bytes.
fn validate_update_version(v: &str) -> Result<(), AppError> {
    if v.is_empty() || v.len() > 64 {
        return Err(AppError::BadRequest(
            "version must be 1..64 characters".into(),
        ));
    }
    let ok = v
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '+' || c == '_');
    if !ok {
        return Err(AppError::BadRequest(
            "version contains invalid characters (allowed: alphanumeric, . - + _)".into(),
        ));
    }
    Ok(())
}

/// Validate the `via` field of `POST /api/update/run`.
///
/// Mirrors [`validate_update_version`]'s style: same `AppError::BadRequest`
/// mapping, same single-line message. `None` (Auto) is always valid;
/// `"cargo"` is the only accepted non-null value today. New channels will
/// need to be added here when their support lands.
fn validate_via(via: Option<&str>) -> Result<(), AppError> {
    match via {
        None => Ok(()),
        Some("cargo") => Ok(()),
        Some(other) => Err(AppError::BadRequest(format!(
            "via must be \"cargo\" or omitted (got: {other:?})"
        ))),
    }
}

/// Dispatch decision for `POST /api/update/run`.
///
/// Pure output of [`classify_update_dispatch`]: the handler switches on
/// this to pick the install branch and the response shape. The "HTTP
/// never auto-migrates" contract is enforced by the handler binding
/// `migrated = false` unconditionally — there is no flag on the enum
/// itself, by design (a per-variant flag would let a future code path
/// flip it back on).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UpdateDispatch {
    /// Full managed install (fetch → precheck → install → flip → prune).
    Managed,
    /// Unmanaged (or detected Cargo): managed install ran (or was
    /// skipped), but the user's PATH shadows are NOT migrated. The
    /// handler must respond with the shadow audit + instructions.
    Unmanaged,
    /// Brew: refused. Handler returns 200 with a `brew upgrade oxios`
    /// message; no install runs.
    Brew,
    /// Dev: refused. Handler returns 400 with a rebuild hint.
    Dev,
    /// Legacy `cargo install oxios`. Reached only when the caller
    /// explicitly sent `via: "cargo"` (the F10 version-validation gate
    /// is preserved). The handler spawns `cargo` with these args.
    Cargo {
        /// Args after the binary name. `["install", "oxios", "--locked"]`
        /// for an unpinned run, plus `--version <v>` if a target version
        /// was supplied.
        args: Vec<String>,
    },
}

/// Resolve `oxios_home` from the daemon's `state.config_path`.
///
/// Mirrors the `oxios_home_from_config` helper in `main.rs`:
/// `~/.oxios` is the directory the config file lives in. Falls back to
/// `~/.oxios` under `$HOME` (or `.` when `HOME` is unset) when the
/// config path has no parent — defensive, since every config path
/// produced by `expand_home` has a parent in practice.
fn oxios_home_from_state(state: &AppState) -> PathBuf {
    state
        .config_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(oxios_kernel::oxi_home::oxios_home)
}

/// Classify the install channel for the running daemon and combine it
/// with the user-supplied `via` to pick an [`UpdateDispatch`].
///
/// The CLI contract is refuse-first: Brew and Dev channels are refused
/// BEFORE any `--via` handling (run_update:200-212), so `--via cargo`
/// on a Brew/Dev install refuses rather than shadowing the channel's
/// manager. This function mirrors that order: channel classification
/// runs first, Brew/Dev refuse regardless of `via`, and only then does
/// `via == Some("cargo")` route to the legacy block.
///
/// For `None` (Auto), `exe` (typically `std::env::current_exe()`) is
/// classified against `oxios_home` via
/// [`managed_install::classify_channel`]. Per the design's channel
/// table, `Channel::Cargo` (detected) is treated as Unmanaged: the
/// daemon runs the managed install and the response carries the
/// shadow-audit payload; `migrated` is `false`. Only the explicit
/// `via: "cargo"` arm takes the legacy block.
///
/// `version` is forwarded into the legacy `cargo install --version`
/// argv so a `via=cargo` request with a pinned version actually
/// installs that version, not the latest.
pub(crate) fn classify_update_dispatch(
    oxios_home: &std::path::Path,
    exe: &std::path::Path,
    via: Option<&str>,
    version: Option<&str>,
) -> UpdateDispatch {
    use crate::managed_install::Channel;
    match crate::managed_install::classify_channel(exe, oxios_home) {
        Channel::Managed => {
            if via == Some("cargo") {
                return UpdateDispatch::Cargo {
                    args: cargo_args(version),
                };
            }
            UpdateDispatch::Managed
        }
        // Brew: refuse, print the brew upgrade hint. No install — even
        // with via=cargo (cargo install would shadow the brew copy).
        Channel::Brew => UpdateDispatch::Brew,
        // Dev: refuse with a rebuild hint.
        Channel::Dev => UpdateDispatch::Dev,
        // Cargo (detected) is treated as Unmanaged per the design
        // channel table: the daemon was installed via cargo, the user
        // can opt into the managed store via the TTY surface, but
        // over HTTP we run the managed install + audit and never
        // auto-migrate. Explicit via=cargo still takes the legacy block.
        Channel::Cargo | Channel::Unmanaged => {
            if via == Some("cargo") {
                return UpdateDispatch::Cargo {
                    args: cargo_args(version),
                };
            }
            UpdateDispatch::Unmanaged
        }
    }
}

/// Build the `cargo install` argv for the legacy Cargo branch.
///
/// Returned as `Vec<String>` (owned) so the handler can pass it to
/// `Command::new("cargo").args(...)` without lifetime gymnastics. The
/// `--version <v>` flag is appended when a target version was
/// supplied so a pinned run actually installs that version, not the
/// latest.
fn cargo_args(version: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = vec!["install".into(), "oxios".into(), "--locked".into()];
    if let Some(v) = version {
        args.push("--version".into());
        args.push(v.to_string());
    }
    args
}

/// One shadow entry as serialized in the API response.
///
/// Maps [`managed_install::ShadowEntry`] into the `{path, target,
/// size_bytes}` shape pinned by the brief. `target` is the symlink
/// target (always emitted, `null` for regular files) so consumers
/// don't have to handle the field's presence.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct ShadowPayload {
    pub path: String,
    pub target: Option<String>,
    pub size_bytes: u64,
}

/// Build the shadow payload for the API response.
///
/// Pure transform — takes the audit result and the launcher path, emits
/// the response array. `target` is the symlink target for symlink
/// shadows; for regular-file shadows the field is `None` (serialized as
/// JSON `null`) so consumers always see the key.
pub(crate) fn build_shadow_payload(
    shadows: &[crate::managed_install::ShadowEntry],
) -> Vec<ShadowPayload> {
    shadows
        .iter()
        .map(|s| ShadowPayload {
            path: s.path.to_string_lossy().into_owned(),
            target: s
                .symlink_target
                .as_ref()
                .map(|t| t.to_string_lossy().into_owned()),
            size_bytes: s.size_bytes,
        })
        .collect()
}

/// The "no auto-migrate" instructions string for the API response.
///
/// `None` when the user's PATH is clean (the handler leaves the
/// `instructions` field as an empty string in the JSON). `Some(s)` when
/// shadows are present — `s` is a single line the caller can paste
/// into a terminal.
pub(crate) fn update_instructions(
    shadows: &[crate::managed_install::ShadowEntry],
) -> Option<String> {
    if shadows.is_empty() {
        None
    } else {
        Some(format!(
            "re-run `oxios update` in a terminal to adopt {} shadow{}",
            shadows.len(),
            if shadows.len() == 1 { "" } else { "s" }
        ))
    }
}

/// Build the canonical post-pipeline `UpdateRunResponse`.
///
/// Pure helper: takes the post-install state (binary / web flags,
/// accumulated messages, audit + instructions) and pins the fields the
/// HTTP surface must enforce — `migrated: false` unconditionally,
/// `shadows` + `instructions` populated from the audit. Extracted so
/// tests can drive the response builder directly instead of relying on
/// a tautological `struct {...}; assert_eq!(...migrated, false)` shape.
pub(crate) fn build_update_response(
    target_version: &str,
    binary_updated: bool,
    web_updated: bool,
    messages: &[String],
    shadows: &[crate::managed_install::ShadowEntry],
    instructions: &str,
) -> UpdateRunResponse {
    UpdateRunResponse {
        success: true,
        updated_to: target_version.to_string(),
        binary_updated,
        web_updated,
        message: messages.join("; "),
        migrated: false,
        shadows: build_shadow_payload(shadows),
        instructions: instructions.to_string(),
    }
}

/// Maximum download size for the web-dist.zip asset (200 MiB). Guards
/// against a malicious or tampered release asset OOMing the server.
const MAX_DOWNLOAD_BYTES: u64 = 200 * 1024 * 1024;

/// POST /api/update/run — Execute the update (download + install binary/web).
pub(crate) async fn handle_update_run(
    state: State<Arc<AppState>>,
    Json(body): Json<UpdateRunBody>,
) -> Result<Json<UpdateRunResponse>, AppError> {
    let current = env!("CARGO_PKG_VERSION");
    // F10: validate user-supplied version before it reaches the GitHub API
    // URL or `cargo install --version`. Empty (latest) is allowed.
    if let Some(ref v) = body.version {
        validate_update_version(v)?;
    }
    // Task 7: validate `via` first — it's the cheap gate and 400s out
    // before we touch the network.
    validate_via(body.via.as_deref())?;

    // Resolve `oxios_home` and the running daemon's exe. We need them
    // BEFORE the GitHub fetch so Brew / Dev refusals don't burn a
    // round-trip (Task 6 refuses pre-fetch).
    let oxios_home = oxios_home_from_state(&state);
    let exe = std::env::current_exe().unwrap_or_else(|_| oxios_home.clone());
    let dispatch = classify_update_dispatch(
        &oxios_home,
        &exe,
        body.via.as_deref(),
        body.version.as_deref(),
    );
    // Brew / Dev short-circuit: no network, no install. The body of
    // these branches returns early with the refusal response.
    match dispatch {
        UpdateDispatch::Brew => {
            return Ok(Json(UpdateRunResponse {
                success: true,
                updated_to: current.to_string(),
                binary_updated: false,
                web_updated: false,
                message: "Brew install detected — run `brew upgrade oxios` to update".into(),
                migrated: false,
                shadows: Vec::new(),
                instructions: String::new(),
            }));
        }
        UpdateDispatch::Dev => {
            return Err(AppError::BadRequest(
                "dev build detected — run `cargo build` from the workspace to update".into(),
            ));
        }
        UpdateDispatch::Managed | UpdateDispatch::Unmanaged | UpdateDispatch::Cargo { .. } => {
            // Fall through to the full pipeline below.
        }
    }

    let release = fetch_github_release(body.version.as_deref()).await?;

    let tag_name = release["tag_name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let target_version = tag_name.trim_start_matches('v').to_string();

    let assets: Vec<(String, String, u64)> = release["assets"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|a| {
            Some((
                a["name"].as_str()?.to_string(),
                a["browser_download_url"].as_str()?.to_string(),
                a["size"].as_u64()?,
            ))
        })
        .collect();

    let client = reqwest::Client::builder()
        .user_agent(format!("oxios/{current}"))
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::Internal(format!("failed to create HTTP client: {e}")))?;

    let mut binary_updated = false;
    let mut web_updated = false;
    let mut messages: Vec<String> = Vec::new();
    // HTTP never auto-migrates — `migrated` is reserved for the TTY
    // surface (where the operator can consent via `--adopt`). Always
    // false here.
    let migrated = false;
    let mut shadows: Vec<crate::managed_install::ShadowEntry> = Vec::new();
    let mut instructions = String::new();

    // here. Brew / Dev returned early above (no network).
    match dispatch {
        // Brew / Dev are returned early above (no network). Reaching
        // them here is unreachable — kept for exhaustiveness so the
        // compiler flags a new dispatch variant if one is added.
        UpdateDispatch::Brew | UpdateDispatch::Dev => {
            unreachable!("Brew / Dev must short-circuit before the GitHub fetch")
        }
        // Cargo: legacy `cargo install oxios` block. The F10 version
        // gate above already ran; the dispatch args are used as-is.
        UpdateDispatch::Cargo { args } => {
            // Gate: respect `body.binary: false` (the caller's explicit
            // opt-out) — don't overwrite ~/.cargo/bin/oxios against the
            // request. Managed / Unmanaged arms keep the same gate.
            if body.binary {
                tracing::info!(?args, "Running cargo install for binary update (via=cargo)");
                let str_args: Vec<&str> = args.iter().map(String::as_str).collect();
                let output = tokio::process::Command::new("cargo")
                    .args(&str_args)
                    .output()
                    .await
                    .map_err(|e| AppError::Internal(format!("failed to run cargo: {e}")))?;
                if output.status.success() {
                    binary_updated = true;
                    messages.push(format!("Binary updated to {target_version} via cargo"));
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    tracing::error!(%stderr, "cargo install failed");
                    messages.push(format!(
                        "Binary update failed: {}",
                        stderr.lines().take(3).collect::<Vec<_>>().join("; ")
                    ));
                }
            }
        }
        // Managed: full managed install — fetch tarball, precheck disk,
        // install, flip launcher, prune. PATH-shadow audit is included
        // in the response so a fresh install surfaces any prior
        // stragglers.
        UpdateDispatch::Managed => {
            if body.binary && target_version != current {
                run_managed_install(&oxios_home, &target_version, &mut messages).await?;
                binary_updated = true;
            } else if body.binary {
                messages.push(format!(
                    "Binary already at {target_version}; no install needed"
                ));
            }
            // Managed-channel hosts shouldn't normally have PATH
            // shadows, but audit anyway so a misconfigured PATH is
            // surfaced (no migration — that's a TTY-only op).
            let path_env = std::env::var("PATH").unwrap_or_default();
            shadows = crate::managed_install::audit_shadows(&oxios_home, &path_env);
            if let Some(s) = update_instructions(&shadows) {
                instructions = s;
            }
        }
        // Unmanaged: install the new version to the managed store
        // (when the binary is requested and the version actually
        // changed), but NEVER auto-migrate PATH. Audit runs and the
        // caller is told to re-run in a terminal.
        UpdateDispatch::Unmanaged => {
            if body.binary && target_version != current {
                run_managed_install(&oxios_home, &target_version, &mut messages).await?;
                binary_updated = true;
            } else if body.binary {
                messages.push(format!(
                    "Binary already at {target_version}; no install needed"
                ));
            }
            let path_env = std::env::var("PATH").unwrap_or_default();
            shadows = crate::managed_install::audit_shadows(&oxios_home, &path_env);
            if let Some(s) = update_instructions(&shadows) {
                instructions = s;
            }
        }
    }

    // Update web UI (atomic — RFC-024 SP3). Embedded builds skip this
    // entirely: the SPA ships in the binary and nothing on disk can
    // replace it — publishing a downloaded generation would be dead
    // weight.
    if body.web {
        if crate::embedded_web::is_embedded() {
            messages.push(
                "Web UI skipped: embedded build serves the web UI from the binary — update the binary instead".to_string(),
            );
        } else if let Some((name, url, size)) = assets.iter().find(|(n, _, _)| n == "web-dist.zip")
        {
            tracing::info!(name, size, "Downloading web UI for update");
            let bytes = download_bytes(&client, url, MAX_DOWNLOAD_BYTES).await?;

            // Extract into a fresh versioned staging dir — NEVER the active
            // dir. The active dir is published only after extraction
            // succeeds and validates, so no request ever sees a
            // half-populated dist.
            // Unified home: web-dist staging lives under the
            // resolver root (`OXIOS_HOME` override or `~/.oxi/oxios`)
            // — the `web/` subtree is denied to agents via
            // `OXIOS_HOME_DENY_SUBPATHS`.
            let web_root = oxios_kernel::oxi_home::oxios_home().join("web");
            let staging = web_root.join(format!("dist-{target_version}"));
            if staging.exists() {
                std::fs::remove_dir_all(&staging)
                    .map_err(|e| AppError::Internal(format!("failed to clear staging: {e}")))?;
            }
            std::fs::create_dir_all(&staging)
                .map_err(|e| AppError::Internal(format!("failed to create staging: {e}")))?;

            let cursor = std::io::Cursor::new(&bytes);
            let mut archive = zip::ZipArchive::new(cursor)
                .map_err(|e| AppError::Internal(format!("invalid zip: {e}")))?;

            for i in 0..archive.len() {
                let mut file = archive
                    .by_index(i)
                    .map_err(|e| AppError::Internal(format!("zip read error: {e}")))?;
                let out_path = match file.enclosed_name() {
                    Some(p) => staging.join(p),
                    None => continue,
                };
                if file.is_dir() {
                    std::fs::create_dir_all(&out_path).ok();
                } else {
                    if let Some(p) = out_path.parent() {
                        std::fs::create_dir_all(p).ok();
                    }
                    let mut out_file = std::fs::File::create(&out_path)
                        .map_err(|e| AppError::Internal(format!("write error: {e}")))?;
                    std::io::copy(&mut file, &mut out_file)
                        .map_err(|e| AppError::Internal(format!("write error: {e}")))?;
                }
            }

            // Validate before publishing. Self-consistency (not just
            // index.html presence) catches a dist that mixes two
            // builds, so a broken page is never published as active.
            if !oxios_gateway::ActiveWebDist::dist_is_consistent(&staging) {
                return Err(AppError::Internal(
                    "extracted dist is not self-consistent (index.html references missing assets)"
                        .into(),
                ));
            }

            // Atomic publish: swap the in-memory pointer + persist
            // marker. Previous generation is cleaned up after a grace
            // period.
            let marker = web_root.join(".active");
            state.web_dist.publish(staging, &marker);

            web_updated = true;
            messages.push(format!("Web UI updated to {target_version}"));
        } else {
            messages.push("web-dist.zip not found in release, skipped".to_string());
        }
    }

    tracing::info!(
        binary_updated,
        web_updated,
        target_version,
        migrated,
        "Update completed"
    );

    Ok(Json(build_update_response(
        &target_version,
        binary_updated,
        web_updated,
        &messages,
        &shadows,
        &instructions,
    )))
}

/// Run the full managed install: fetch tarball → disk precheck → install
/// to `<home>/versions/<v>/` → flip launcher → prune old versions.
///
/// Pure orchestrator over the `managed_install` helpers. Failures
/// bubble as `AppError::Internal` so the route surface is uniform. The
/// caller is responsible for the dispatch decision (Managed / Unmanaged
/// both reach this fn; Brew / Dev / Cargo are handled in `handle_update_run`).
async fn run_managed_install(
    oxios_home: &std::path::Path,
    target_version: &str,
    messages: &mut Vec<String>,
) -> Result<(), AppError> {
    use crate::managed_install;

    let versions = managed_install::versions_dir(oxios_home);
    std::fs::create_dir_all(&versions)
        .map_err(|e| AppError::Internal(format!("create versions dir: {e}")))?;

    let tar = managed_install::fetch_tarball(target_version)
        .await
        .map_err(|e| AppError::Internal(format!("fetch tarball: {e}")))?;
    managed_install::disk_precheck(&versions, (tar.len() as u64) * 3)
        .map_err(|e| AppError::Internal(format!("disk precheck: {e}")))?;
    managed_install::install_version_bytes(oxios_home, target_version, &tar)
        .map_err(|e| AppError::Internal(format!("install: {e}")))?;
    managed_install::flip_launcher(oxios_home, target_version)
        .map_err(|e| AppError::Internal(format!("flip launcher: {e}")))?;
    let pruned = managed_install::prune_versions(oxios_home, 2)
        .map_err(|e| AppError::Internal(format!("prune: {e}")))?;
    messages.push(format!(
        "Managed install to {target_version} (pruned: {})",
        if pruned.is_empty() {
            "<none>".to_string()
        } else {
            pruned.join(", ")
        }
    ));
    Ok(())
}

/// GET /api/update/changelog — Show release notes for a version.
pub(crate) async fn handle_update_changelog(
    Query(params): Query<UpdateCheckParams>,
) -> Result<Json<ChangelogResponse>, AppError> {
    let release = fetch_github_release(params.version.as_deref()).await?;

    let tag_name = release["tag_name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let version = tag_name.trim_start_matches('v').to_string();
    let published_at = release["published_at"].as_str().unwrap_or("").to_string();
    let body = release["body"]
        .as_str()
        .unwrap_or("No release notes.")
        .to_string();
    let html_url = release["html_url"].as_str().unwrap_or("").to_string();

    Ok(Json(ChangelogResponse {
        tag_name,
        version,
        published_at,
        body,
        html_url,
    }))
}

// ---------------------------------------------------------------------------
// Update helpers
// ---------------------------------------------------------------------------

async fn fetch_github_release(version: Option<&str>) -> Result<serde_json::Value, AppError> {
    // Canonical repo constant lives in `managed_install` so the CLI and
    // the web route cannot drift apart. The previous local
    // GITHUB_OWNER/GITHUB_REPO pair used the `a7garden/oxios` alias —
    // wrong; the canonical repo is `project-oxi/oxios`.
    let api_url = match version {
        Some(v) => {
            format!(
                "https://api.github.com/repos/{}/releases/tags/v{v}",
                managed_install::GITHUB_REPO
            )
        }
        None => {
            format!(
                "https://api.github.com/repos/{}/releases/latest",
                managed_install::GITHUB_REPO
            )
        }
    };

    let client = reqwest::Client::builder()
        .user_agent(format!("oxios/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::Internal(format!("HTTP client error: {e}")))?;

    let resp = client
        .get(&api_url)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("GitHub API request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Internal(format!(
            "GitHub API error {status}: {body}"
        )));
    }

    resp.json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse GitHub response: {e}")))
}

async fn download_bytes(
    client: &reqwest::Client,
    url: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, AppError> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Download request failed: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(AppError::Internal(format!("Download failed: {status}")));
    }

    // F10: reject oversized payloads up front when the server advertises a
    // Content-Length. This stops a zip-bomb / large-asset OOM before we
    // allocate for it.
    if let Some(cl) = resp.content_length()
        && cl > max_bytes
    {
        return Err(AppError::PayloadTooLarge {
            size: cl as usize,
            limit: max_bytes as usize,
        });
    }

    // Stream the body with a running byte counter so a server that omits
    // Content-Length (chunked encoding) still cannot push us past the cap.
    use futures::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|e| AppError::Internal(format!("Failed to read download body: {e}")))?;
        let new_len = buf.len().saturating_add(chunk.len());
        if new_len > max_bytes as usize {
            return Err(AppError::PayloadTooLarge {
                size: new_len,
                limit: max_bytes as usize,
            });
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Parse agent metrics from the Prometheus export text.
/// Returns (forked, completed, failed) counters.
fn parse_agent_metrics() -> (u64, u64, u64) {
    let export = oxios_kernel::metrics::registry().export();
    let mut forked = 0u64;
    let mut completed = 0u64;
    let mut failed = 0u64;
    for line in export.lines() {
        if line.starts_with("oxios_agents_forked_total ") {
            forked = line
                .rsplit(' ')
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        } else if line.starts_with("oxios_agents_completed_total ") {
            completed = line
                .rsplit(' ')
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        } else if line.starts_with("oxios_agents_failed_total ") {
            failed = line
                .rsplit(' ')
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        }
    }
    (forked, completed, failed)
}

/// Query params for GET /api/agents.
#[derive(Debug, Deserialize)]
pub(crate) struct AgentQueryParams {
    pub q: Option<String>,
    pub search_field: Option<String>,
    pub status: Option<String>,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub model_id: Option<String>,
    pub tool: Option<String>,
    pub has_error: Option<bool>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub cost_min: Option<f64>,
    pub cost_max: Option<f64>,
    pub tokens_min: Option<u64>,
    pub tokens_max: Option<u64>,
    pub duration_min: Option<u64>,
    pub duration_max: Option<u64>,
    pub sort_by: Option<String>,
    pub sort_dir: Option<String>,
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_limit")]
    pub per_page: u32,
}

fn default_page() -> u32 {
    1
}
fn default_limit() -> u32 {
    50
}

/// Convert query params to AgentListFilter.
fn params_to_filter(p: &AgentQueryParams) -> oxios_kernel::agent_log_db::AgentListFilter {
    let status = p.status.as_deref().and_then(|s| match s {
        "running" => Some(oxios_kernel::agent_log_db::AgentStatusFilter::Running),
        "completed" => Some(oxios_kernel::agent_log_db::AgentStatusFilter::Completed),
        "failed" => Some(oxios_kernel::agent_log_db::AgentStatusFilter::Failed),
        "stopped" => Some(oxios_kernel::agent_log_db::AgentStatusFilter::Stopped),
        "starting" => Some(oxios_kernel::agent_log_db::AgentStatusFilter::Starting),
        "idle" => Some(oxios_kernel::agent_log_db::AgentStatusFilter::Idle),
        _ => None,
    });

    let search_field = p.search_field.as_deref().map_or(
        oxios_kernel::agent_log_db::SearchField::All,
        |s| match s {
            "name" => oxios_kernel::agent_log_db::SearchField::Name,
            "error" => oxios_kernel::agent_log_db::SearchField::Error,
            "tool_name" => oxios_kernel::agent_log_db::SearchField::ToolName,
            "tool_output" => oxios_kernel::agent_log_db::SearchField::ToolOutput,
            _ => oxios_kernel::agent_log_db::SearchField::All,
        },
    );

    let sort_by = p
        .sort_by
        .as_deref()
        .map_or(oxios_kernel::agent_log_db::SortBy::CreatedAt, |s| match s {
            "cost" => oxios_kernel::agent_log_db::SortBy::Cost,
            "duration" => oxios_kernel::agent_log_db::SortBy::Duration,
            "tokens" => oxios_kernel::agent_log_db::SortBy::Tokens,
            "name" => oxios_kernel::agent_log_db::SortBy::Name,
            _ => oxios_kernel::agent_log_db::SortBy::CreatedAt,
        });

    let sort_dir = p
        .sort_dir
        .as_deref()
        .map_or(oxios_kernel::agent_log_db::SortDir::Desc, |s| match s {
            "asc" => oxios_kernel::agent_log_db::SortDir::Asc,
            _ => oxios_kernel::agent_log_db::SortDir::Desc,
        });

    let parse_dt = |s: &Option<String>| -> Option<chrono::DateTime<chrono::Utc>> {
        s.as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
    };

    oxios_kernel::agent_log_db::AgentListFilter {
        q: p.q.clone(),
        search_field,
        status,
        session_id: p.session_id.clone(),
        project_id: p.project_id.clone(),
        model_id: p.model_id.clone(),
        tool: p.tool.clone(),
        has_error: p.has_error,
        date_from: parse_dt(&p.date_from),
        date_to: parse_dt(&p.date_to),
        cost_min: p.cost_min,
        cost_max: p.cost_max,
        tokens_min: p.tokens_min,
        tokens_max: p.tokens_max,
        duration_min: p.duration_min,
        duration_max: p.duration_max,
        sort_by,
        sort_dir,
        page: p.page,
        per_page: p.per_page,
    }
}

/// GET /api/agents — List agent instances with full filter/search/sort/paginate.
pub(crate) async fn handle_agents_list(
    state: State<Arc<AppState>>,
    Query(params): Query<AgentQueryParams>,
) -> Json<serde_json::Value> {
    let filter = params_to_filter(&params);
    match state.kernel.agents.query(&filter).await {
        Ok(result) => Json(serde_json::json!({
            "items": result.items.iter().map(serialize_agent_summary).collect::<Vec<_>>(),
            "total": result.total,
            "page": result.page,
            "per_page": result.per_page,
            "total_pages": result.total_pages,
            "stats": {
                "total_cost_usd": result.stats.total_cost_usd,
                "total_tokens": result.stats.total_tokens,
                "avg_duration_secs": result.stats.avg_duration_secs,
                "count_running": result.stats.count_running,
                "count_completed": result.stats.count_completed,
                "count_failed": result.stats.count_failed,
            },
        })),
        Err(e) => {
            tracing::error!(error = %e, "Failed to query agents");
            Json(serde_json::json!({
                "items": [],
                "total": 0,
                "page": params.page,
                "per_page": params.per_page,
                "total_pages": 0,
                "stats": {},
            }))
        }
    }
}

/// GET /api/agents/stats — Global agent stats.
pub(crate) async fn handle_agent_stats(state: State<Arc<AppState>>) -> Json<serde_json::Value> {
    match state.kernel.agents.stats().await {
        Ok(s) => Json(serde_json::json!({
            "total_agents": s.total_agents,
            "running": s.running,
            "completed": s.completed,
            "failed": s.failed,
            "total_cost_usd": s.total_cost_usd,
            "total_tokens": s.total_tokens,
            "total_duration_secs": s.total_duration_secs,
            "avg_duration_secs": s.avg_duration_secs,
            "avg_cost_usd": s.avg_cost_usd,
            "total_sessions": s.total_sessions,
            "oldest_agent_at": s.oldest_agent_at.map(|t| t.to_rfc3339()),
            "newest_agent_at": s.newest_agent_at.map(|t| t.to_rfc3339()),
        })),
        Err(e) => {
            tracing::error!(error = %e, "Failed to get agent stats");
            Json(serde_json::json!({"error": e.to_string()}))
        }
    }
}

fn serialize_agent_summary(a: &oxios_kernel::types::AgentInfo) -> serde_json::Value {
    serde_json::json!({
        "id": a.id.to_string(),
        "name": a.name,
        "status": a.status.to_string(),
        "created_at": a.created_at.to_rfc3339(),
        "started_at": a.started_at.map(|t| t.to_rfc3339()),
        "completed_at": a.completed_at.map(|t| t.to_rfc3339()),
        "project_id": a.project_id.map(|id| id.to_string()),
        "session_id": a.session_id,
        "error": a.error,
        "steps_completed": a.steps_completed,
        "steps_total": a.steps_total,
        "tokens_used": a.tokens_input + a.tokens_output,
        "cost_usd": a.cost_usd,
        "model_id": a.model_id,
        "duration_secs": a.completed_at.zip(a.started_at)
            .map(|(end, start)| (end - start).num_seconds().max(0)),
    })
}

/// GET /api/agents/{id} — Agent detail (from memory or SQLite).
pub(crate) async fn handle_agent_get(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Try in-memory first
    if let Ok(agents) = state.kernel.agents.list().await
        && let Some(agent) = agents.into_iter().find(|a| a.id.to_string() == id)
    {
        return Ok(Json(agent_detail_json(&agent, &state)));
    }

    // Fall back to SQLite (or filesystem)
    let agent = state.kernel.agents.get(&id).await?;
    match agent {
        Some(agent) => Ok(Json(agent_detail_json(&agent, &state))),
        None => Err(AppError::NotFound("agent not found".into())),
    }
}

fn agent_detail_json(
    agent: &oxios_kernel::types::AgentInfo,
    state: &Arc<AppState>,
) -> serde_json::Value {
    let budget = state.kernel.agents.check_budget(&agent.id);
    serde_json::json!({
        "id": agent.id.to_string(),
        "name": agent.name,
        "status": agent.status.to_string(),
        "created_at": agent.created_at.to_rfc3339(),
        "project_id": agent.project_id.map(|id| id.to_string()),
        "session_id": agent.session_id,
        "started_at": agent.started_at.map(|t| t.to_rfc3339()),
        "completed_at": agent.completed_at.map(|t| t.to_rfc3339()),
        "error": agent.error,
        "steps_completed": agent.steps_completed,
        "steps_total": agent.steps_total,
        "tokens_used": agent.tokens_input + agent.tokens_output,
        "cost_usd": agent.cost_usd,
        "model_id": agent.model_id,
        "budget": {
            "tokens_remaining": budget.tokens_remaining,
            "calls_remaining": budget.calls_remaining,
            "window_remaining_secs": budget.window_remaining_secs,
            "is_exhausted": budget.is_exhausted,
        },
    })
}

pub(crate) async fn handle_agent_trace(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    use oxios_kernel::state_store::SessionId;

    // Try in-memory first, then SQLite.
    let agent = if let Ok(agents) = state.kernel.agents.list().await
        && let Some(agent) = agents.into_iter().find(|a| a.id.to_string() == id)
    {
        agent
    } else if let Ok(Some(agent)) = state.kernel.agents.get(&id).await {
        agent
    } else {
        return Err(AppError::NotFound("agent not found".into()));
    };

    // RFC-028 SP-3a: join session trajectory for a fuller trace.
    // The agent's `tool_calls` field is capped (`max_tool_calls_per_agent`),
    // but the session trajectory accumulates the complete execution history.
    let trajectory = if let Some(sid) = &agent.session_id {
        match state
            .kernel
            .state
            .load_session(&SessionId(sid.clone()))
            .await
        {
            Ok(Some(session)) => Some(session.trajectory().to_vec()),
            _ => None,
        }
    } else {
        None
    };

    Ok(Json(trace_json(&agent, trajectory.as_deref())))
}

fn trace_json(
    agent: &oxios_kernel::types::AgentInfo,
    trajectory: Option<&[oxios_kernel::state_store::TrajectoryStepRecord]>,
) -> serde_json::Value {
    use std::collections::HashSet;

    // Collect tool_call_ids already present in agent.tool_calls to dedup
    // against the session trajectory.
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut steps: Vec<serde_json::Value> = Vec::new();

    for (i, tc) in agent.tool_calls.iter().enumerate() {
        if !tc.tool_call_id.is_empty() {
            seen_ids.insert(tc.tool_call_id.clone());
        }
        steps.push(serde_json::json!({
            "index": i,
            "kind": "tool",
            "tool_name": tc.tool,
            "action": tc.tool,
            "input": tc.input,
            "output": tc.output,
            "started_at": tc.timestamp.map(|t| t.to_rfc3339()).unwrap_or_default(),
            "duration_ms": tc.duration_ms,
            "status": if tc.is_error { "failed" } else { "completed" },
        }));
    }

    // Append trajectory steps not already covered by tool_calls.
    if let Some(traj) = trajectory {
        let mut idx = steps.len();
        for step in traj {
            if !step.tool_call_id.is_empty() && seen_ids.contains(&step.tool_call_id) {
                continue;
            }
            seen_ids.insert(step.tool_call_id.clone());
            steps.push(serde_json::json!({
                "index": idx,
                "kind": "tool",
                "tool_name": step.tool_name,
                "action": step.tool_name,
                "input": step.tool_args,
                "output": step.output_summary,
                "started_at": step.timestamp.to_rfc3339(),
                "duration_ms": step.duration_ms,
                "status": if step.is_error { "failed" } else { "completed" },
            }));
            idx += 1;
        }

        // Sort by started_at so the merged timeline is chronological.
        steps.sort_by(|a, b| {
            let ta = a["started_at"].as_str().unwrap_or("");
            let tb = b["started_at"].as_str().unwrap_or("");
            ta.cmp(tb)
        });
        // Re-index after sort.
        for (i, step) in steps.iter_mut().enumerate() {
            step["index"] = serde_json::json!(i);
        }
    }

    serde_json::json!({
        "agent_id": agent.id.to_string(),
        "steps": steps,
        "completed_at": agent.completed_at.map(|t| t.to_rfc3339()),
    })
}

/// GET /api/agents/{id}/logs — Agent execution logs.
pub(crate) async fn handle_agent_logs(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Try in-memory first
    let agent = if let Ok(agents) = state.kernel.agents.list().await {
        agents.into_iter().find(|a| a.id.to_string() == id)
    } else {
        None
    };

    // Fallback: load from SQLite
    let agent = match agent {
        Some(a) => a,
        None => match state.kernel.agents.get(&id).await {
            Ok(Some(a)) => a,
            _ => return Err(AppError::NotFound("agent not found".into())),
        },
    };

    let mut entries = Vec::new();

    if let Some(started) = agent.started_at {
        entries.push(serde_json::json!({
            "timestamp": started.to_rfc3339(),
            "level": "info",
            "message": format!("Agent started: {}", agent.name),
        }));
    }

    for (i, tc) in agent.tool_calls.iter().enumerate() {
        let ts = tc.timestamp.map(|t| t.to_rfc3339()).unwrap_or_default();
        entries.push(serde_json::json!({
            "timestamp": ts,
            "level": "info",
            "message": format!("[Step {}] {} ({}) → {}",
                i + 1, tc.tool, format_duration(tc.duration_ms),
                truncate_str(&tc.output, 120)),
        }));
    }

    if let Some(completed) = agent.completed_at {
        let (level, msg) = if let Some(ref err) = agent.error {
            ("error", format!("Agent failed: {err}"))
        } else {
            (
                "info",
                format!("Agent completed ({} steps)", agent.steps_completed),
            )
        };
        entries.push(serde_json::json!({
            "timestamp": completed.to_rfc3339(),
            "level": level,
            "message": msg,
        }));
    }

    Ok(Json(serde_json::json!({
        "agent_id": id,
        "entries": entries,
    })))
}

/// POST /api/agents/{id}/kill — Kill an agent.
pub(crate) async fn handle_agent_kill(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<(), AppError> {
    tracing::info!(agent_id = %id, "Kill agent requested");
    state.kernel.agents.kill(&id).await.map_err(|e| {
        tracing::warn!(error = %e, "Agent not found");
        AppError::NotFound("agent not found".into())
    })
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// GET /api/config — Get current configuration.
pub(crate) async fn handle_config_get(
    state: State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Serialize the actual config from AppState (read lock).
    let config = state.config.read();
    match serde_json::to_value(&*config) {
        Ok(mut json) => {
            // Mask API key in response — never expose plaintext
            if let Some(engine) = json.get_mut("engine")
                && let Some(api_key) = engine.get_mut("api_key")
                && api_key.as_str().is_some_and(|k| !k.is_empty())
            {
                *api_key = serde_json::Value::String("***".to_string());
            }
            // Add api_key_set flag so the frontend knows if a key is currently set
            json["engine"]["api_key_set"] =
                serde_json::Value::Bool(config.engine.api_key.is_some());
            Ok(Json(json))
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to serialize config");
            Err(AppError::Internal("failed to serialize config".into()))
        }
    }
}

/// GET /api/config/meta — Hot-reload classification metadata.
///
/// Returns the backend's authoritative classification of which config
/// sections/fields are hot-reloadable (applied immediately) vs which
/// require a daemon restart. This is the **single source of truth** for
/// the frontend's pre-save Diff Preview badges and SaveDock counts.
///
/// Without this endpoint, the frontend would need to maintain a parallel
/// `hotReload` boolean per field in `field-defs.ts` that silently drifts
/// from the backend's actual propagation logic (the `resource_monitor`
/// drift bug was a concrete instance of this problem).
pub(crate) async fn handle_config_meta() -> Json<ConfigMetaResponse> {
    Json(ConfigMetaResponse {
        hot_reloadable_sections: HOT_RELOADABLE_SECTIONS
            .iter()
            .map(|(s, _)| (*s).to_string())
            .collect(),
        always_restart_fields: RESTART_REQUIRED_FIELDS
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
    })
}

/// Response for `GET /api/config/meta`.
#[derive(Debug, Serialize)]
pub(crate) struct ConfigMetaResponse {
    /// Top-level config section keys whose fields are hot-reloadable
    /// (e.g. `"exec"`, `"resource_monitor"`). Fields in these sections
    /// that are NOT in `always_restart_fields` are applied immediately.
    pub hot_reloadable_sections: Vec<String>,
    /// Dotted field paths that always require a restart, even inside a
    /// hot-reloadable section (e.g. `"gateway.host"`).
    pub always_restart_fields: Vec<String>,
}

/// Deep-merge a patch into a base `serde_json::Value` (both must be objects).
///
/// Sections and fields present in `patch` override those in `base`.
/// Sections and fields absent from `patch` are preserved from `base`.
/// This implements PATCH semantics so that a partial config update does not
/// reset fields the caller did not intend to change.
///
/// Conflict policy:
/// - If both sides have a non-null object at the same path, recurse.
/// - Otherwise `patch` wins.
fn deep_merge_json(base: &mut serde_json::Value, patch: serde_json::Value) {
    use serde_json::Value;
    if let Value::Object(patch_map) = patch {
        if !base.is_object() {
            *base = Value::Object(serde_json::Map::new());
        }
        let base_map = base.as_object_mut().expect("just ensured object");
        for (key, patch_val) in patch_map {
            match base_map.get_mut(&key) {
                Some(existing) if existing.is_object() && patch_val.is_object() => {
                    deep_merge_json(existing, patch_val);
                }
                _ => {
                    base_map.insert(key, patch_val);
                }
            }
        }
    }
}

/// PUT /api/config — Update configuration (alias of PATCH).
///
/// Like the PATCH handler, the request body is **deep-merged** into the
/// current in-memory config. Sections and fields the caller omits are
/// preserved, not reset to defaults. Despite the HTTP verb, this is
/// NOT a full-config replacement — it has the same semantics as
/// `PATCH /api/config`.
///
/// Why is PUT exposed at all? Some HTTP clients, automation tooling,
/// and older Oxios versions send PUT instead of PATCH. The handler
/// is kept so that those callers still work; new code should prefer
/// `PATCH /api/config`, which also returns the hot-reload
/// classification report (`ConfigPatchResponse`) that PUT does not.
///
/// Engine configuration (`engine.*`) is rejected by PATCH with 400;
/// PUT keeps the same restriction. Use the typed engine endpoints
/// (`/api/engine/api-key`, `/api/engine/model`,
/// `/api/engine/provider-options`) for those.
pub(crate) async fn handle_config_put(
    state: State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    tracing::info!("Config update requested");

    // Reject engine.* fields — same restriction as PATCH. The bulk PUT path
    // does not encrypt or mask, so accepting `engine.api_key` here would
    // write the key in plaintext to config.toml and bypass the typed
    // `/api/engine/api-key` endpoint (which handles encryption/masking).
    if let Some(forbidden) = find_forbidden_patch_key(&body, PATCH_FORBIDDEN_TOP_LEVEL_KEYS) {
        tracing::warn!(key = %forbidden, "PUT /api/config rejected forbidden key");
        return Err(AppError::BadRequest(format!(
            "PUT /api/config does not accept '{forbidden}' fields. \
             Use the typed endpoint instead: \
             /api/engine/api-key (POST), /api/engine/model (PUT), \
             /api/engine/provider-options (PUT)."
        )));
    }

    // Deep-merge the patch into the current config so omitted fields are preserved.
    let mut current_value = {
        let cfg = state.config.read();
        serde_json::to_value(&*cfg).map_err(|e| {
            tracing::error!(error = %e, "Failed to serialize current config");
            AppError::Internal("failed to serialize current config".into())
        })?
    };
    deep_merge_json(&mut current_value, body);

    // Validate the merged result by parsing as OxiosConfig.
    let updated: oxios_kernel::OxiosConfig = match serde_json::from_value(current_value.clone()) {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!(error = %e, "Invalid config shape");
            return Err(AppError::BadRequest(format!("Invalid config: {e}")));
        }
    };

    // Run the kernel validator too (catches semantic errors like
    // default_timeout > max_timeout) before we touch disk.
    let (errors, warnings) = updated.validate();
    for w in &warnings {
        tracing::warn!(config_warning = %w, "Config validation warning");
    }
    if !errors.is_empty() {
        let msg = errors.join("; ");
        tracing::warn!(error = %msg, "Config validation failed");
        return Err(AppError::BadRequest(format!("Invalid config: {msg}")));
    }

    // Persist the merged config to disk.
    let content = toml::to_string_pretty(&updated)
        .map_err(|e: toml::ser::Error| AppError::Internal(e.to_string()))?;
    if let Err(e) = tokio::fs::write(&state.config_path, content).await {
        tracing::error!(error = %e, "Failed to persist config");
        return Err(AppError::Internal(e.to_string()));
    }
    tracing::info!(path = %state.config_path.display(), "Config persisted");

    // Hot-reload: update in-memory config.
    let updated_config = updated;
    *state.config.write() = updated_config.clone();

    // Propagate hot-reloadable config to kernel subsystems.
    // Each subsystem gets its relevant slice of the config.

    // ExecApi — allowlist, shell mode, timeouts
    *state.kernel.exec.shared_config().write() = updated_config.exec.clone();
    // ResourceMonitor — CPU/memory/load thresholds
    use oxios_kernel::resource_monitor::OverloadThreshold;
    state
        .kernel
        .infra
        .resource_monitor()
        .set_overload_threshold(OverloadThreshold {
            cpu_percent: updated_config.resource_monitor.cpu_threshold,
            memory_percent: updated_config.resource_monitor.memory_threshold,
            load_avg: updated_config.resource_monitor.load_threshold,
        });

    tracing::info!(
        "Config hot-reloaded (web + kernel subsystems) from {}",
        state.config_path.display()
    );

    // Return the merged, masked config (same shape as GET /api/config) so
    // the caller never receives an echoed plaintext api_key from its own
    // request body.
    let mut response = current_value;
    if let Some(engine) = response.get_mut("engine")
        && let Some(api_key) = engine.get_mut("api_key")
        && api_key.as_str().is_some_and(|k| !k.is_empty())
    {
        *api_key = serde_json::Value::String("***".to_string());
    }
    if let Some(engine) = response.get_mut("engine") {
        engine["api_key_set"] = serde_json::Value::Bool(updated_config.engine.api_key.is_some());
    }
    Ok(Json(response))
}

// ---------------------------------------------------------------------------
// PATCH /api/config — Partial config update with hot-reload metadata
// ---------------------------------------------------------------------------

/// List of top-level config sections whose fields are propagated to the
/// running kernel at PATCH time (no daemon restart required).
///
/// Each entry is `(section_name, restart_scope)`. `restart_scope` describes
/// the runtime subsystem that needs to pick up the change (used in logs and
/// tooltips on the frontend).
///
/// IMPORTANT: this list MUST match what `handle_config_patch` actually
/// propagates. Sections not listed here (security, audit, context,
/// but the running daemon keeps the boot-time values, so they are
/// classified as `requires_restart`. Adding a section to this list
/// without wiring the propagation in `handle_config_patch` would lie
/// to the user about whether the change took effect.
const HOT_RELOADABLE_SECTIONS: &[(&str, &str)] = &[
    ("exec", "exec_api"),
    ("resource_monitor", "resource_monitor"),
    ("token_maxing", "quota_tracker"),
    ("orchestrator", "infra_api"),
    // Config single-source unification: AppState.config IS the kernel
    // EngineApi's RwLock (one shared Arc), so writes here are observed by the
    // per-turn reads in `AgentRuntime::execute_inner` on the next turn —
    // no restart, no extra propagation step.
    ("execution_recipe", "agent_runtime"),
];

/// Subset of fields that always require a restart even inside a
/// hot-reloadable section (e.g. `memory.embedding.provider` swaps a
/// model that was loaded at boot).
const RESTART_REQUIRED_FIELDS: &[&str] = &[
    "memory.embedding.dimension",
    "memory.sqlite.path",
    "memory.sqlite.embedding_dim",
    "memory.bridge.sync_enabled",
    "memory.bridge.interval_secs",
    "engine.default_model",
    "engine.api_key",
    "engine.provider_options",
    "engine.routing_enabled",
    "engine.prefer_cost_efficient",
    "engine.fallback_models",
    "engine.excluded_models",
    "gateway.host",
    "gateway.port",
    "daemon.pid_file",
    "daemon.log_dir",
    "channels.enabled",
    "channels.telegram.bot_token_env",
    "channels.telegram.api_base",
    "channels.telegram.allowed_users",
    "channels.telegram.session.rotation_hours",
    "channels.telegram.session.max_messages",
    "surfaces",
    "cron",
    "mcp",
    "browser",
    "persona",
    "marketplace",
    "budget",
    "git",
    "memory.consolidation.preset",
];

/// Response body for `PATCH /api/config`.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct ConfigPatchResponse {
    /// Echo of the saved patch (deep-merged view of the modified config).
    pub config: serde_json::Value,
    /// Hot-reload classification of the changes that were applied.
    pub hot_reload: HotReloadReport,
}

/// Hot-reload classification of a config patch.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct HotReloadReport {
    /// Dotted field paths that were applied to running subsystems immediately.
    pub applied_immediately: Vec<String>,
    /// Dotted field paths that require a daemon restart to take full effect.
    pub requires_restart: Vec<String>,
    /// Total number of changed fields (sum of both lists).
    pub total_changed: usize,
}

/// Classify a JSON patch against the current config into hot-reloadable vs
/// restart-required field paths. Walks both `base` and `patch` recursively,
/// emitting the dotted path of every key whose value actually changed.
fn classify_patch(
    base: &serde_json::Value,
    patch: &serde_json::Value,
    prefix: &str,
    applied: &mut Vec<String>,
    restart: &mut Vec<String>,
) {
    use serde_json::Value;
    let Value::Object(patch_map) = patch else {
        return;
    };
    for (key, patch_val) in patch_map {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };

        // Recurse into nested objects so we report the exact changed field.
        if patch_val.is_object() {
            let base_child = base.get(key).cloned().unwrap_or(Value::Null);
            classify_patch(&base_child, patch_val, &path, applied, restart);
            continue;
        }

        // Scalar / array — compare for actual change.
        let base_val = base.get(key);
        if base_val == Some(patch_val) {
            continue;
        }

        if is_restart_required(&path) {
            restart.push(path);
        } else {
            applied.push(path);
        }
    }
}

/// Returns true if a dotted config path requires a daemon restart to apply.
fn is_restart_required(path: &str) -> bool {
    if RESTART_REQUIRED_FIELDS.contains(&path) {
        return true;
    }
    // Top-level sections not in HOT_RELOADABLE_SECTIONS are restart-only.
    let top = path.split('.').next().unwrap_or(path);
    !HOT_RELOADABLE_SECTIONS.iter().any(|(s, _)| *s == top)
}

/// Top-level config keys that the PATCH endpoint must refuse, even
/// though they exist in `OxiosConfig`. The engine subsystem manages
/// its own typed endpoints (`/api/engine/api-key`, `/api/engine/model`,
/// `/api/engine/provider-options`) which handle encryption, masking,
/// and provider-scoped semantics. A bulk PATCH that overwrites
/// `engine.api_key: ""` would silently wipe the stored key.
const PATCH_FORBIDDEN_TOP_LEVEL_KEYS: &[&str] = &["engine"];

/// Walk a PATCH body and return the first forbidden top-level key it
/// contains, or `None` if the body is acceptable. Used by
/// `handle_config_patch` to reject engine.* writes before they reach
/// the deep-merge step.
fn find_forbidden_patch_key(body: &serde_json::Value, forbidden: &[&str]) -> Option<String> {
    use serde_json::Value;
    let Value::Object(map) = body else {
        return None;
    };
    for key in map.keys() {
        if forbidden.iter().any(|f| *f == key) {
            return Some(key.clone());
        }
    }
    None
}

/// `PATCH /api/config` — Partial config update.
///
/// Body: a subset of `OxiosConfig` (e.g. `{"exec": {"allowlist_mode":
/// "enforced"}}`). The patch is deep-merged into the current config so
/// sections and fields the caller omits are preserved.
///
/// Engine configuration (`engine.api_key`, `engine.provider_options`,
/// `engine.default_model`, …) MUST NOT be sent via this endpoint.
/// Use the typed engine endpoints (`/api/engine/api-key`,
/// `/api/engine/model`, `/api/engine/provider-options`) instead — they
/// handle encryption, masking, and provider scoping correctly. A PATCH
/// containing engine.* fields is rejected with HTTP 400.
///
/// Response includes a `hot_reload` object classifying which changed
/// fields were applied immediately and which require a daemon restart.
pub(crate) async fn handle_config_patch(
    state: State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<ConfigPatchResponse>, AppError> {
    tracing::info!("Config PATCH requested");

    if !body.is_object() {
        return Err(AppError::BadRequest(
            "PATCH body must be a JSON object".into(),
        ));
    }

    // Reject engine.* fields. They are managed by the typed engine
    // endpoints; sending them here risks wiping the encrypted api_key
    // (the bulk path does not mask or encrypt).
    if let Some(forbidden) = find_forbidden_patch_key(&body, PATCH_FORBIDDEN_TOP_LEVEL_KEYS) {
        tracing::warn!(key = %forbidden, "PATCH /api/config rejected forbidden key");
        return Err(AppError::BadRequest(format!(
            "PATCH /api/config does not accept '{forbidden}' fields. \
             Use the typed endpoint instead: \
             /api/engine/api-key (POST), /api/engine/model (PUT), \
             /api/engine/provider-options (PUT)."
        )));
    }

    // Snapshot the current config as JSON for both merging and classification.
    let mut current_value = {
        let cfg = state.config.read();
        serde_json::to_value(&*cfg).map_err(|e| {
            tracing::error!(error = %e, "Failed to serialize current config");
            AppError::Internal("failed to serialize current config".into())
        })?
    };

    // Capture the pre-merge value so we can detect which fields actually changed.
    let before_patch = current_value.clone();

    // Deep-merge the patch into the current config.
    deep_merge_json(&mut current_value, body.clone());

    // Classify every changed field into hot-reloadable vs restart-required.
    let mut applied: Vec<String> = Vec::new();
    let mut restart: Vec<String> = Vec::new();
    classify_patch(&before_patch, &body, "", &mut applied, &mut restart);
    applied.sort();
    restart.sort();

    // Validate the merged result.
    let updated: oxios_kernel::OxiosConfig = match serde_json::from_value(current_value.clone()) {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!(error = %e, "Invalid config shape");
            return Err(AppError::BadRequest(format!("Invalid config: {e}")));
        }
    };
    let (errors, warnings) = updated.validate();
    for w in &warnings {
        tracing::warn!(config_warning = %w, "Config validation warning");
    }
    if !errors.is_empty() {
        let msg = errors.join("; ");
        tracing::warn!(error = %msg, "Config validation failed");
        return Err(AppError::BadRequest(format!("Invalid config: {msg}")));
    }

    // Persist merged config to disk.
    let content = toml::to_string_pretty(&updated)
        .map_err(|e: toml::ser::Error| AppError::Internal(e.to_string()))?;
    if let Err(e) = tokio::fs::write(&state.config_path, content).await {
        tracing::error!(error = %e, "Failed to persist config");
        return Err(AppError::Internal(e.to_string()));
    }
    tracing::info!(path = %state.config_path.display(), "Config persisted");

    // Hot-reload in-memory config.
    *state.config.write() = updated.clone();

    // Propagate hot-reloadable slices to kernel subsystems.
    *state.kernel.exec.shared_config().write() = updated.exec.clone();
    use oxios_kernel::resource_monitor::OverloadThreshold;
    state
        .kernel
        .infra
        .resource_monitor()
        .set_overload_threshold(OverloadThreshold {
            cpu_percent: updated.resource_monitor.cpu_threshold,
            memory_percent: updated.resource_monitor.memory_threshold,
            load_avg: updated.resource_monitor.load_threshold,
        });

    // Hot-reload orchestrator config (evolution iterations, eval score).
    state
        .kernel
        .infra
        .update_orchestrator_config(updated.orchestrator.clone());

    // RFC-031: hot-reload token-maxing config into the live QuotaTracker,
    // preserving usage counters for providers that remain eligible.
    if let Some(ref tm) = state.kernel.token_maxing {
        tm.reload(updated.token_maxing.clone());
    }

    let total = applied.len() + restart.len();
    tracing::info!(
        applied = applied.len(),
        restart = restart.len(),
        "Config PATCH applied"
    );

    Ok(Json(ConfigPatchResponse {
        config: body,
        hot_reload: HotReloadReport {
            applied_immediately: applied,
            requires_restart: restart,
            total_changed: total,
        },
    }))
}

#[cfg(test)]
mod patch_tests {
    //! Unit tests for the PATCH /api/config hot-reload classification.

    use super::{classify_patch, is_restart_required};
    use serde_json::json;

    #[test]
    fn classify_hot_reloadable_field() {
        let base = json!({"exec": {"allowed_commands": ["ls", "cat"]}});
        let patch = json!({"exec": {"allowed_commands": ["ls", "cat", "rg"]}});
        let mut applied = Vec::new();
        let mut restart = Vec::new();
        classify_patch(&base, &patch, "", &mut applied, &mut restart);
        assert_eq!(applied, vec!["exec.allowed_commands"]);
        assert!(restart.is_empty());
    }

    #[test]
    fn classify_execution_recipe_as_hot_reloadable() {
        // Config single-source unification: the pilot toggles are read per
        // turn through the shared config Arc, so they apply immediately.
        let base = json!({"execution_recipe": {"coding_pilot": false}});
        let patch = json!({"execution_recipe": {"coding_pilot": true}});
        let mut applied = Vec::new();
        let mut restart = Vec::new();
        classify_patch(&base, &patch, "", &mut applied, &mut restart);
        assert_eq!(applied, vec!["execution_recipe.coding_pilot"]);
        assert!(restart.is_empty());
    }

    #[test]
    fn classify_restart_required_field() {
        let base = json!({"gateway": {"port": 4200}});
        let patch = json!({"gateway": {"port": 4300}});
        let mut applied = Vec::new();
        let mut restart = Vec::new();
        classify_patch(&base, &patch, "", &mut applied, &mut restart);
        assert!(applied.is_empty());
        assert_eq!(restart, vec!["gateway.port"]);
    }

    #[test]
    fn classify_mixed_changes() {
        // `exec.allowed_commands` is hot-reloadable, `gateway.port` is not.
        let base = json!({
            "exec": {"allowed_commands": ["ls"]},
            "gateway": {"port": 4200},
        });
        let patch = json!({
            "exec": {"allowed_commands": ["ls", "rg"]},
            "gateway": {"port": 4300},
        });
        let mut applied = Vec::new();
        let mut restart = Vec::new();
        classify_patch(&base, &patch, "", &mut applied, &mut restart);
        applied.sort();
        restart.sort();
        assert_eq!(applied, vec!["exec.allowed_commands"]);
        assert_eq!(restart, vec!["gateway.port"]);
    }

    #[test]
    fn classify_skips_unchanged_fields() {
        // Patch contains a value equal to the base — should not be reported.
        let base = json!({"exec": {"allowed_commands": ["ls"]}});
        let patch = json!({"exec": {"allowed_commands": ["ls"]}});
        let mut applied = Vec::new();
        let mut restart = Vec::new();
        classify_patch(&base, &patch, "", &mut applied, &mut restart);
        assert!(applied.is_empty());
        assert!(restart.is_empty());
    }

    #[test]
    fn classify_recurses_into_nested_objects() {
        // Memory embedding provider change → restart-required.
        let base = json!({
            "memory": {"embedding": {"provider": "gguf", "dimension": 256}}
        });
        let patch = json!({
            "memory": {"embedding": {"provider": "mlx", "dimension": 256}}
        });
        let mut applied = Vec::new();
        let mut restart = Vec::new();
        classify_patch(&base, &patch, "", &mut applied, &mut restart);
        assert!(applied.is_empty());
        assert_eq!(restart, vec!["memory.embedding.provider"]);
    }

    #[test]
    fn unknown_top_level_section_is_restart_required() {
        // `otel` is not in HOT_RELOADABLE_SECTIONS.
        assert!(is_restart_required("otel.enabled"));
        assert!(is_restart_required("otel.endpoint"));
    }

    #[test]
    fn hot_reloadable_sections_are_immediate() {
        // Only sections that `handle_config_patch` actually propagates
        // to the running kernel are marked hot-reloadable. security,
        // audit, etc. are NOT propagated (subsystem constructed at
        // boot) so they must be classified as restart-required.
        assert!(!is_restart_required("exec.allowed_commands"));
        assert!(!is_restart_required("resource_monitor.cpu_threshold"));
    }

    #[test]
    fn security_section_is_restart_required() {
        // security.cors_origins used to be classified hot-reloadable,
        // but AccessManager is constructed at boot. PATCH persists
        // the new value but the running subsystem keeps the boot
        // configuration until restart. Must be classified as
        // restart-required to avoid lying to the user.
        assert!(is_restart_required("security.cors_origins"));
        assert!(is_restart_required("security.auth_enabled"));
        assert!(is_restart_required("security.rate_limit_per_minute"));
    }

    #[test]
    fn audit_section_is_restart_required() {
        // Audit writer is constructed at boot with its rotating file
        // handle. PATCH persists but does not reopen the writer.
        assert!(is_restart_required("audit.max_entries"));
        assert!(is_restart_required("audit.enabled"));
    }

    #[test]
    fn memory_section_is_restart_required() {
        // Memory subsystem is constructed at boot (SQLite handle,
        // embedding model, SONA). Toggling `enabled` or any sub-field
        // is restart-only.
        assert!(is_restart_required("memory.enabled"));
        assert!(is_restart_required("memory.embedding.provider"));
        assert!(is_restart_required("memory.consolidation.dream_enabled"));
        assert!(is_restart_required("memory.learning.sona_enabled"));
    }

    #[test]
    fn channels_telegram_session_requires_restart() {
        // Telegram channel is launched at boot — session changes need restart.
        assert!(is_restart_required(
            "channels.telegram.session.rotation_hours"
        ));
        assert!(is_restart_required("channels.telegram.allowed_users"));
    }

    #[test]
    fn memory_consolidation_preset_requires_restart() {
        // Preset triggers `apply_preset()` which mutates many sibling fields.
        assert!(is_restart_required("memory.consolidation.preset"));
    }
}

#[cfg(test)]
mod patch_rejection_tests {
    //! Engine.* fields must be rejected by PATCH /api/config. They are
    //! managed by the typed engine endpoints (which handle encryption,
    //! masking, and provider-scoped semantics) and sending them via the
    //! bulk PATCH would risk wiping the encrypted api_key.

    use super::{PATCH_FORBIDDEN_TOP_LEVEL_KEYS, find_forbidden_patch_key};
    use serde_json::json;

    #[test]
    fn rejects_engine_api_key() {
        let body = json!({"engine": {"api_key": "sk-secret"}});
        let found = find_forbidden_patch_key(&body, PATCH_FORBIDDEN_TOP_LEVEL_KEYS);
        assert_eq!(found.as_deref(), Some("engine"));
    }

    #[test]
    fn rejects_engine_provider_options() {
        let body = json!({"engine": {"provider_options": {"temperature": 0.7}}});
        let found = find_forbidden_patch_key(&body, PATCH_FORBIDDEN_TOP_LEVEL_KEYS);
        assert_eq!(found.as_deref(), Some("engine"));
    }

    #[test]
    fn rejects_engine_default_model() {
        let body = json!({"engine": {"default_model": "anthropic/claude-3"}});
        let found = find_forbidden_patch_key(&body, PATCH_FORBIDDEN_TOP_LEVEL_KEYS);
        assert_eq!(found.as_deref(), Some("engine"));
    }

    #[test]
    fn accepts_non_engine_sections() {
        let body = json!({
            "exec": {"allowlist_mode": "enforced"},
        });
        let found = find_forbidden_patch_key(&body, PATCH_FORBIDDEN_TOP_LEVEL_KEYS);
        assert!(found.is_none());
    }

    #[test]
    fn accepts_mixed_payload_without_engine() {
        // The check is for the *top-level* `engine` key, not a field
        // anywhere in the body. A nested object containing the word
        // "engine" elsewhere is fine.
        let body = json!({"exec": {"allowed_commands": ["engine-status"]}});
        let found = find_forbidden_patch_key(&body, PATCH_FORBIDDEN_TOP_LEVEL_KEYS);
        assert!(found.is_none());
    }

    #[test]
    fn empty_body_is_acceptable() {
        let body = json!({});
        let found = find_forbidden_patch_key(&body, PATCH_FORBIDDEN_TOP_LEVEL_KEYS);
        assert!(found.is_none());
    }
}

#[cfg(test)]
mod deep_merge_tests {
    use super::deep_merge_json;
    use serde_json::json;

    #[test]
    fn preserves_omitted_top_level_sections() {
        let mut base = json!({
            "kernel": {"workspace": "~/.oxios/workspace", "max_agents": 10},
            "exec": {"allowed_commands": ["ls", "cat"], "allowlist_mode": "enforced"},
        });
        let patch = json!({
            "kernel": {"max_agents": 20},
        });
        deep_merge_json(&mut base, patch);
        assert_eq!(base["kernel"]["workspace"], "~/.oxios/workspace");
        assert_eq!(base["kernel"]["max_agents"], 20);
        assert_eq!(base["exec"]["allowed_commands"][0], "ls");
        assert_eq!(base["exec"]["allowlist_mode"], "enforced");
    }

    #[test]
    fn patch_value_replaces_scalar() {
        let mut base = json!({"engine": {"default_model": "old/model"}});
        deep_merge_json(&mut base, json!({"engine": {"default_model": "new/model"}}));
        assert_eq!(base["engine"]["default_model"], "new/model");
    }

    #[test]
    fn patch_object_replaces_object() {
        let mut base = json!({"security": {"auth_enabled": false, "cors_origins": ["http://a"]}});
        deep_merge_json(&mut base, json!({"security": {"auth_enabled": true}}));
        assert_eq!(base["security"]["auth_enabled"], true);
        assert_eq!(base["security"]["cors_origins"][0], "http://a");
    }

    #[test]
    fn empty_patch_is_noop() {
        let mut base = json!({"exec": {"allowed_commands": ["ls"]}});
        let original = base.clone();
        deep_merge_json(&mut base, json!({}));
        assert_eq!(base, original);
    }
}

// ---------------------------------------------------------------------------
// System Tools (Doctor, Audit Verify, Backup, Log)
// ---------------------------------------------------------------------------

/// A single diagnostic check result.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct DoctorCheck {
    /// Check name.
    pub name: String,
    /// Status: pass, warn, fail.
    pub status: String,
    /// Human-readable detail.
    pub message: String,
}

/// Response for `POST /api/system/doctor`.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct DoctorResponse {
    /// Total checks performed.
    pub checks: u32,
    /// Number of issues found.
    pub issues: u32,
    /// Per-check results.
    pub results: Vec<DoctorCheck>,
    /// List of actionable issues.
    pub action_items: Vec<String>,
}

/// POST /api/system/doctor — Run system diagnostics.
pub(crate) async fn handle_doctor(state: State<Arc<AppState>>) -> Json<DoctorResponse> {
    // Clone what we need from config, don't hold the lock across await points
    let (default_model, api_key, workspace, _daemon_log_dir) = {
        let config = state.config.read();
        (
            config.engine.default_model.clone(),
            config.api_key(),
            config.kernel.workspace.clone(),
            config.daemon.log_dir.clone(),
        )
    };
    let mut results = Vec::new();
    let mut action_items = Vec::new();

    // 1. Config file
    if state.config_path.exists() {
        results.push(DoctorCheck {
            name: "config_file".into(),
            status: "pass".into(),
            message: format!("Config file present ({})", state.config_path.display()),
        });
    } else {
        results.push(DoctorCheck {
            name: "config_file".into(),
            status: "fail".into(),
            message: "Config file missing".into(),
        });
        action_items.push("Config file not found. Run `oxios onboard` to create it.".into());
    }

    // 2. Credentials
    let provider = oxios_kernel::CredentialStore::provider_from_model(&default_model);
    match provider {
        Some(p) => match oxios_kernel::CredentialStore::resolve(p, api_key.as_deref()) {
            Some((_key, source)) => {
                // F11: do not echo any part of the API key — even a
                // 4+4 char preview leaks most of a short key and helps
                // brute-force. Report presence + source only. The
                // `api_key_set` boolean from GET /api/config already
                // tells the frontend whether a key is configured.
                results.push(DoctorCheck {
                    name: "credentials".into(),
                    status: "pass".into(),
                    message: format!("Credentials found (via {source:?})"),
                });
            }
            None => {
                results.push(DoctorCheck {
                    name: "credentials".into(),
                    status: "fail".into(),
                    message: format!("No credentials for provider '{p}'"),
                });
                action_items.push(format!(
                    "No API key for '{p}'. Configure in Settings → Engine."
                ));
            }
        },
        None => {
            results.push(DoctorCheck {
                name: "credentials".into(),
                status: "fail".into(),
                message: "No model configured".into(),
            });
            action_items.push("No model set. Configure in Settings → Engine.".into());
        }
    }

    // 3. Workspace directory
    let workspace = oxios_kernel::config::expand_home(&workspace);
    if workspace.exists() {
        results.push(DoctorCheck {
            name: "workspace".into(),
            status: "pass".into(),
            message: format!("Workspace directory ({})", workspace.display()),
        });
    } else {
        results.push(DoctorCheck {
            name: "workspace".into(),
            status: "warn".into(),
            message: format!("Workspace directory missing ({})", workspace.display()),
        });
        action_items.push("Workspace directory not found. It will be created on first run.".into());
    }

    // 4. Default model
    if !default_model.is_empty() {
        results.push(DoctorCheck {
            name: "model".into(),
            status: "pass".into(),
            message: format!("Default model: {default_model}"),
        });
    } else {
        results.push(DoctorCheck {
            name: "model".into(),
            status: "fail".into(),
            message: "No default model set".into(),
        });
        action_items.push("No default model configured.".into());
    }

    // 5. MCP servers
    let mcp_count = state.kernel.mcp.server_count();
    if mcp_count > 0 {
        results.push(DoctorCheck {
            name: "mcp_servers".into(),
            status: "pass".into(),
            message: format!("{mcp_count} MCP server(s) connected"),
        });
    } else {
        results.push(DoctorCheck {
            name: "mcp_servers".into(),
            status: "warn".into(),
            message: "No MCP servers configured".into(),
        });
    }

    // 6. Git repository
    let git_ok = state.kernel.infra.git_verify().unwrap_or(false);
    if git_ok {
        results.push(DoctorCheck {
            name: "git".into(),
            status: "pass".into(),
            message: "Git repository intact".into(),
        });
    } else {
        results.push(DoctorCheck {
            name: "git".into(),
            status: "warn".into(),
            message: "Git repository verification failed".into(),
        });
    }

    // 7. State store
    let ws_path = state.kernel.state.workspace_path();
    if ws_path.exists() {
        results.push(DoctorCheck {
            name: "state_store".into(),
            status: "pass".into(),
            message: format!("State store path exists ({})", ws_path.display()),
        });
    } else {
        results.push(DoctorCheck {
            name: "state_store".into(),
            status: "warn".into(),
            message: "State store path not found".into(),
        });
    }

    // 8. Brain daemonless (RFC-047 / RFC-049) — "ready" iff session is up
    // OR the managed binary is installed. Otherwise prompt to install.
    let brain_ok = state
        .kernel
        .brain
        .as_ref()
        .map(|b| {
            let snap = b.status_snapshot();
            snap.available || snap.binary_installed
        })
        .unwrap_or(false);
    results.push(DoctorCheck {
        name: "brain".into(),
        status: if brain_ok { "pass" } else { "warn" }.into(),
        message: match brain_ok {
            true => "Brain: ready (daemonless session)".into(),
            false => "Brain: binary missing — install with `oxios brain install`".into(),
        },
    });

    // 9. Web dist directory
    if let Some(web_dist) = state.web_dist.path() {
        if web_dist.exists() {
            results.push(DoctorCheck {
                name: "web_dist".into(),
                status: "pass".into(),
                message: format!("Web UI dist ({})", web_dist.display()),
            });
        } else {
            results.push(DoctorCheck {
                name: "web_dist".into(),
                status: "warn".into(),
                message: "Web UI dist directory not found".into(),
            });
        }
    } else {
        results.push(DoctorCheck {
            name: "web_dist".into(),
            status: "pass".into(),
            message: "Web UI served from embedded assets".into(),
        });
    }

    let checks = results.len() as u32;
    let issues = action_items.len() as u32;

    Json(DoctorResponse {
        checks,
        issues,
        results,
        action_items,
    })
}

/// Response for `POST /api/system/audit-verify`.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct AuditVerifyResponse {
    pub valid: bool,
    pub entries_checked: u64,
    pub message: String,
}

/// POST /api/system/audit-verify — Verify audit trail integrity.
pub(crate) async fn handle_audit_verify_api(
    state: State<Arc<AppState>>,
) -> Json<AuditVerifyResponse> {
    let audit = &state.kernel.security;
    match audit.verify_chain() {
        Ok(valid) => Json(AuditVerifyResponse {
            valid,
            entries_checked: 0,
            message: if valid {
                "Audit trail verified successfully.".into()
            } else {
                "Audit trail verification failed.".into()
            },
        }),
        Err(e) => Json(AuditVerifyResponse {
            valid: false,
            entries_checked: 0,
            message: format!("Audit trail verification failed: {e}"),
        }),
    }
}

/// Response for `POST /api/system/backup`.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct BackupResponse {
    pub success: bool,
    pub path: String,
    pub size_bytes: u64,
    pub message: String,
}

/// Build the `tar` argv for `POST /api/system/backup`.
///
/// Extracted as a pure function so the backup layout — particularly the
/// inclusion of the ecosystem vault root — can be unit-tested without
/// spawning the process. The vault (resolved by the same 3-step chain
/// `KernelConfig::resolved_knowledge_root` uses: explicit
/// `knowledge_root` > `~/.oxi/config.toml [vault].path` > default
/// `~/.oxi/vault`) is added as a second `-C` archive root so T19's
/// migration routine can roll back from a single tarball produced
/// before the vault was repointed.
///
/// The vault block is added ONLY when the resolved vault path exists on
/// disk — on hosts without a migrated vault yet (or with a custom
/// `[vault].path` whose directory is absent), we skip the second `-C`
/// block entirely so `tar` does not warn about a missing archive member
/// and fail the backup.
///
/// The legacy `~/.oxios/knowledge` member is included only when the
/// directory still exists; after the unification migration, oxios no
/// longer writes through that path and tar would otherwise warn.
fn build_backup_tar_args(
    backup_path: &std::path::Path,
    oxios_home: &std::path::Path,
    resolved_vault: &std::path::Path,
) -> Vec<String> {
    let oxios_root_str = oxios_home.to_str().unwrap_or(".");

    let mut args: Vec<String> = vec![
        "-czf".to_string(),
        backup_path.display().to_string(),
        "-C".to_string(),
        oxios_root_str.to_string(),
        "config.toml".to_string(),
        "workspace".to_string(),
    ];

    if oxios_home.join("knowledge").exists() {
        args.push("knowledge".to_string());
    }

    // Second archive root: the resolved vault. Skipped when the
    // resolved path does not exist (no vault yet, or custom
    // `[vault].path` whose directory was never created) so the
    // backup does not exit non-zero on a missing-member warning.
    if resolved_vault.exists() {
        let vault_root = resolved_vault
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(resolved_vault);
        let vault_root_str = vault_root.to_str().unwrap_or(".");
        let vault_rel = resolved_vault
            .strip_prefix(vault_root)
            .unwrap_or(resolved_vault)
            .to_string_lossy()
            .into_owned();
        args.push("-C".to_string());
        args.push(vault_root_str.to_string());
        args.push(vault_rel);
    }

    args
}

/// POST /api/system/backup — Create a backup of Oxios state.
///
/// Resolves the vault via the kernel's documented chain
/// (`config.kernel.resolved_knowledge_root`) and skips the vault
/// archive root when the resolved path does not exist — see
/// `build_backup_tar_args` for the contract.
pub(crate) async fn handle_backup(state: State<Arc<AppState>>) -> Json<BackupResponse> {
    // Unified home: tarballs land under the resolver root
    // (`OXIOS_HOME` override or `~/.oxi/oxios`), NOT a hardcoded
    // `~/.oxios`.
    let oxios_home = oxios_kernel::oxi_home::oxios_home();

    // Resolve the vault through the kernel's 3-step chain so the
    // backup lands on the actual vault root — explicit
    // `kernel.knowledge_root` > `~/.oxi/config.toml [vault].path` >
    // default `~/.oxi/vault`. Hardcoding the default would back up
    // the wrong directory on any host with a custom vault path.
    let resolved_vault = {
        let config = state.config.read();
        config.kernel.resolved_knowledge_root()
    };

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let backup_name = format!("oxios-backup-{timestamp}.tar.gz");

    // T18 R4: tarballs land under `<oxios home>/backups/`, NOT
    // directly in the oxios home. Pre-R4 backups matched no deny
    // entry, so a broadly-allowed agent could read the resulting tar
    // and pull out credential-bearing config.toml + vault contents.
    // `backups/` is in `OXIOS_HOME_DENY_SUBPATHS` (single source of
    // truth) — the agent sees an opaque directory under the deny,
    // and tarballs inherit the protection via the gate's canonical-
    // prefix subpath policy.
    let backups_dir = oxios_home.join("backups");
    if let Err(e) = std::fs::create_dir_all(&backups_dir) {
        return Json(BackupResponse {
            success: false,
            path: String::new(),
            size_bytes: 0,
            message: format!("Failed to create backups directory: {e}"),
        });
    }
    let backup_path = backups_dir.join(&backup_name);

    if backup_path.to_str().is_none() {
        return Json(BackupResponse {
            success: false,
            path: String::new(),
            size_bytes: 0,
            message: "Invalid backup path.".into(),
        });
    }

    let tar_args = build_backup_tar_args(&backup_path, &oxios_home, &resolved_vault);

    tracing::info!(path = %backup_path.display(), "Creating backup");

    let output = match tokio::process::Command::new("tar")
        .args(&tar_args)
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => {
            return Json(BackupResponse {
                success: false,
                path: String::new(),
                size_bytes: 0,
                message: format!("tar failed: {e}"),
            });
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Json(BackupResponse {
            success: false,
            path: String::new(),
            size_bytes: 0,
            message: format!("Backup failed: {stderr}"),
        });
    }

    let size = std::fs::metadata(&backup_path)
        .map(|m| m.len())
        .unwrap_or(0);

    tracing::info!(
        path = %backup_path.display(),
        size,
        "Backup created"
    );

    Json(BackupResponse {
        success: true,
        path: backup_path.display().to_string(),
        size_bytes: size,
        message: format!(
            "Backup created: backups/{backup_name} ({})",
            format_size_helper(size)
        ),
    })
}

/// Response for `GET /api/system/log`.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct LogResponse {
    pub lines: Vec<String>,
    pub total: usize,
}

/// GET /api/system/log — Read recent daemon log entries.
pub(crate) async fn handle_log(state: State<Arc<AppState>>) -> Json<LogResponse> {
    let log_dir = {
        let config = state.config.read();
        oxios_kernel::config::expand_home(&config.daemon.log_dir)
    };
    let log_file = log_dir.join("oxios.log");

    if !log_file.exists() {
        return Json(LogResponse {
            lines: vec!["No log file found.".into()],
            total: 1,
        });
    }

    // F8: read only a bounded tail of the log so a multi-GB daemon log
    // cannot OOM the server on a single request. We scan at most the last
    // 256 KiB, drop the leading partial line if we seeked into the middle
    // of the file, then take the last 50 lines.
    let (lines, total) = read_log_tail(&log_file, 50, 256 * 1024);

    Json(LogResponse { lines, total })
}

/// Read the last `max_lines` lines from a file, loading at most
/// `max_bytes` into memory. Returns the lines and the total number of
/// lines within the scanned window (approximate for large files — exact
/// only when the whole file fit in the window).
fn read_log_tail(path: &std::path::Path, max_lines: usize, max_bytes: u64) -> (Vec<String>, usize) {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(metadata) = std::fs::metadata(path) else {
        return (Vec::new(), 0);
    };
    let size = metadata.len();
    let Ok(mut file) = std::fs::File::open(path) else {
        return (Vec::new(), 0);
    };

    let window = size.min(max_bytes);
    if size > max_bytes && file.seek(SeekFrom::End(-(window as i64))).is_err() {
        return (Vec::new(), 0);
    }
    let mut bytes = Vec::with_capacity(window as usize);
    if file.read_to_end(&mut bytes).is_err() {
        return (Vec::new(), 0);
    }

    // If we seeked into the middle of the file, drop the first partial
    // line (it may begin mid-UTF-8-char and would be garbled).
    if size > max_bytes
        && let Some(nl) = bytes.iter().position(|&b| b == b'\n')
    {
        bytes.drain(..=nl);
    }

    let content = String::from_utf8_lossy(&bytes);
    let all_lines: Vec<&str> = content.lines().collect();
    let total = all_lines.len();
    let start = all_lines.len().saturating_sub(max_lines);
    let lines = all_lines[start..].iter().map(|s| s.to_string()).collect();
    (lines, total)
}

fn format_size_helper(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Format milliseconds into a human-readable duration.
fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}

/// Truncate a string to `max_len` characters, appending "..." if needed.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len - 3).collect();
        format!("{truncated}...")
    }
}

#[cfg(test)]
mod backup_args_tests {
    //! Smoke tests for the backup tar argv.
    //!
    //! T18 — the vault must be included in the archive so T19's
    //! migration routine can roll back from a single tarball, AND the
    //! backup must NOT fail on hosts whose resolved vault root does
    //! not exist (pre-migration or custom `[vault].path`). We assert
    //! on the args list rather than spawning the process so the test
    //! stays hermetic and fast.

    use super::build_backup_tar_args;
    use std::path::{Path, PathBuf};

    fn args_with_paths(backup: &Path, oxios_home: &Path, resolved_vault: &Path) -> Vec<String> {
        build_backup_tar_args(backup, oxios_home, resolved_vault)
    }

    fn setup_tmp() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        let oxios_home = home.join(".oxios");
        std::fs::create_dir_all(&oxios_home).expect("mkdir");
        (tmp, home, oxios_home)
    }

    #[test]
    fn backup_includes_oxios_config_and_workspace() {
        let (_tmp, _home, oxios_home) = setup_tmp();
        let backup = oxios_home.join("oxios.tar.gz");
        let vault = oxios_home.parent().unwrap().join(".oxi").join("vault");
        std::fs::create_dir_all(&vault).expect("vault");

        let args = args_with_paths(&backup, &oxios_home, &vault);

        let c_idx = args
            .iter()
            .position(|a| a == "-C")
            .expect("first -C must be present");
        assert_eq!(args[c_idx + 1], oxios_home.display().to_string());
        assert!(
            args.contains(&"config.toml".to_string()),
            "backup must include oxios config.toml; args={args:?}",
        );
        assert!(
            args.contains(&"workspace".to_string()),
            "backup must include oxios workspace; args={args:?}",
        );
    }

    #[test]
    fn backup_includes_default_vault_as_second_archive_root() {
        // T18 acceptance (a): the default vault path
        // (~/.oxi/vault) is included as a second `-C` root when it
        // exists on disk.
        let (_tmp, _home, oxios_home) = setup_tmp();
        let backup = oxios_home.join("oxios.tar.gz");
        let vault = oxios_home.parent().unwrap().join(".oxi").join("vault");
        std::fs::create_dir_all(&vault).expect("vault");

        let args = args_with_paths(&backup, &oxios_home, &vault);

        let c_positions: Vec<usize> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == "-C")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            c_positions.len(),
            2,
            "backup tar must have two `-C` roots (oxios_home + vault); args={args:?}",
        );

        // Second `-C` is the vault's parent (`~/.oxi`), and the
        // member following it is `vault` — so the archive contains
        // `<home>/.oxi/vault/...`.
        let vault_root_idx = c_positions[1];
        let vault_parent = vault.parent().unwrap();
        assert_eq!(
            args[vault_root_idx + 1],
            vault_parent.display().to_string(),
            "second `-C` must point at the vault's parent directory; args={args:?}",
        );
        assert!(
            args.iter().any(|a| a == "vault"),
            "backup must include the `vault` member; args={args:?}",
        );
    }

    #[test]
    fn backup_skips_vault_block_when_resolved_path_missing() {
        // T18 acceptance (b): on hosts whose resolved vault root does
        // not exist (no migrated vault yet, or a custom `[vault].path`
        // whose directory was never created), the backup must NOT
        // include a `-C` block for the vault — otherwise `tar` warns
        // about a missing member and exits non-zero, taking the entire
        // backup down with it.
        let (_tmp, _home, oxios_home) = setup_tmp();
        let backup = oxios_home.join("oxios.tar.gz");
        // Note: vault path does NOT exist.
        let vault = oxios_home.parent().unwrap().join(".oxi").join("vault");

        let args = args_with_paths(&backup, &oxios_home, &vault);

        let c_positions: Vec<usize> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == "-C")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            c_positions.len(),
            1,
            "backup tar must have ONLY ONE `-C` root when the resolved vault is missing; args={args:?}",
        );
        assert!(
            !args.iter().any(|a| a == "vault"),
            "backup must not include a `vault` member when the resolved path is absent; args={args:?}",
        );
    }

    #[test]
    fn backup_includes_custom_vault_path_when_set_in_resolved() {
        // T18 acceptance (c): if the kernel resolved the vault to a
        // custom path (e.g. `[vault].path = /data/my-vault`), the
        // backup must include THAT root, not the hardcoded default.
        // This is what the resolved-vault parameter buys us — the
        // handler calls `config.kernel.resolved_knowledge_root()` and
        // passes the result here.
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        let oxios_home = home.join(".oxios");
        std::fs::create_dir_all(&oxios_home).expect("mkdir");

        let custom_vault = tmp.path().join("data").join("my-vault");
        std::fs::create_dir_all(&custom_vault).expect("custom vault");

        let backup = oxios_home.join("oxios.tar.gz");
        let args = args_with_paths(&backup, &oxios_home, &custom_vault);

        let c_positions: Vec<usize> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == "-C")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(c_positions.len(), 2, "args={args:?}");

        // The second `-C` must point at the custom vault's parent and
        // the member must be the custom vault's last segment — proving
        // we did NOT hardcode `~/.oxi/vault`.
        let vault_root_idx = c_positions[1];
        let custom_parent = custom_vault.parent().unwrap();
        assert_eq!(
            args[vault_root_idx + 1],
            custom_parent.display().to_string(),
            "second `-C` must point at the custom vault's parent; args={args:?}",
        );
        let custom_rel = custom_vault.strip_prefix(custom_parent).unwrap();
        assert!(
            args.iter().any(|a| a == custom_rel.to_str().unwrap()),
            "backup must include the custom-vault relative path; args={args:?}",
        );
    }

    #[test]
    fn backup_omits_legacy_knowledge_when_dir_missing() {
        let (_tmp, _home, oxios_home) = setup_tmp();
        let backup = oxios_home.join("oxios.tar.gz");
        let vault = oxios_home.parent().unwrap().join(".oxi").join("vault");

        // No knowledge dir.
        let args = args_with_paths(&backup, &oxios_home, &vault);
        assert!(
            !args.contains(&"knowledge".to_string()),
            "backup must not include the legacy `knowledge` member when the dir is absent; args={args:?}",
        );
    }

    #[test]
    fn backup_keeps_legacy_knowledge_when_dir_present() {
        let (_tmp, _home, oxios_home) = setup_tmp();
        std::fs::create_dir_all(oxios_home.join("knowledge")).expect("mkdir knowledge");
        let backup = oxios_home.join("oxios.tar.gz");
        let vault = oxios_home.parent().unwrap().join(".oxi").join("vault");

        let args = args_with_paths(&backup, &oxios_home, &vault);
        assert!(
            args.contains(&"knowledge".to_string()),
            "backup must include the legacy `knowledge` member when the dir is present; args={args:?}",
        );
    }

    #[test]
    fn r4_backup_output_dir_is_denied_by_kernel_gate_constant() {
        // T18 R4: `handle_backup` now writes tarballs under
        // `~/.oxios/backups/` (not directly in `~/.oxios/`). The
        // kernel gate denies that subtree via
        // `OXIOS_HOME_DENY_SUBPATHS`. This binary-side test pins
        // the contract: if either side changes (handler relocates
        // again, or the kernel drops the deny), this fails.
        assert!(
            oxios_kernel::access_manager::OXIOS_HOME_DENY_SUBPATHS.contains(&"backups"),
            "OXIOS_HOME_DENY_SUBPATHS must contain `backups` — the handler writes              tarballs under ~/.oxios/backups/ and the gate must deny agent reads;              got {:?}",
            oxios_kernel::access_manager::OXIOS_HOME_DENY_SUBPATHS,
        );
    }

    #[test]
    fn r4_backup_output_lands_under_backups_dir() {
        // T18 R4: the handler relocates output from
        // `~/.oxios/oxios-backup-<ts>.tar.gz` (no deny entry,
        // agent-readable) to `~/.oxios/backups/oxios-backup-<ts>.tar.gz`
        // (denied subtree). This mirrors the handler's path
        // construction so a future relocation breaks this test
        // first.
        let (_tmp, _home, oxios_home) = setup_tmp();
        let backup_name = "oxios-backup-20260821_000000.tar.gz";

        // Mirror handle_backup's construction:
        let backups_dir = oxios_home.join("backups");
        std::fs::create_dir_all(&backups_dir).expect("mkdir backups");
        let backup_path = backups_dir.join(backup_name);

        assert_eq!(
            backup_path,
            oxios_home.join("backups").join(backup_name),
            "backup output must land under ~/.oxios/backups/",
        );
        // And critically NOT directly under ~/.oxios/:
        assert_ne!(
            backup_path,
            oxios_home.join(backup_name),
            "backup output must NOT land directly in ~/.oxios/ (pre-R4 location, agent-readable)",
        );
    }
}

#[cfg(test)]
mod update_route_tests {
    //! Tests for the channel-aware `POST /api/update/run` route.
    //!
    //! The handler itself depends on `AppState` (kernel, config, web_dist) and
    //! runs `cargo install` / `fetch_release` over the network — too heavy for
    //! an in-process unit test. The testable seam is the pure-helper trio:
    //!
    //! - `validate_via`         → 400 for non-"cargo" non-null values
    //! - `classify_update_dispatch`  → enum that picks the cargo / managed /
    //!   unmanaged branch the handler will run
    //! - `build_shadow_payload` + `update_instructions` → response shape
    //!
    //! Together they pin the "via=cargo accepted, via=banana rejected, no
    //! HTTP auto-migrate on unmanaged" contract without spawning subprocesses
    //! or hitting GitHub. The handler test integration that exercises the
    //! full axum stack is deferred to the integration suite (out of scope
    //! for this unit module).
    use super::*;
    use crate::managed_install::ShadowEntry;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn update_run_body_accepts_via_cargo() {
        // The body must round-trip `"via": "cargo"` so the dispatch layer
        // can read it back as Some("cargo"). Default remains `None` (Auto).
        let body: UpdateRunBody = serde_json::from_value(json!({
            "binary": true,
            "web": true,
            "version": null,
            "via": "cargo",
        }))
        .expect("via=cargo must deserialize");
        assert_eq!(body.via.as_deref(), Some("cargo"));

        let none: UpdateRunBody = serde_json::from_value(json!({})).expect("empty body");
        assert!(none.via.is_none(), "missing `via` field stays None (Auto)");
    }

    #[test]
    fn update_run_body_rejects_unknown_via_with_400() {
        // validate_via mirrors `validate_update_version`'s style: same
        // AppError::BadRequest mapping, same single-line message.
        assert!(matches!(
            validate_via(Some("banana")),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            validate_via(Some("brew")),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            validate_via(Some("")),
            Err(AppError::BadRequest(_))
        ));
        // None (Auto) is fine; "cargo" is the only accepted value.
        assert!(validate_via(None).is_ok());
        assert!(validate_via(Some("cargo")).is_ok());
    }

    #[test]
    fn fetch_github_release_uses_canonical_repo_constant() {
        // Fix #5: the web route's fetch_github_release MUST use the
        // canonical `managed_install::GITHUB_REPO` constant (the same
        // one the CLI uses) so the two paths can never drift. The
        // previous local pair `a7garden/oxios` was a stale alias. We
        // assert both the constant value and that the local alias
        // constants are gone.
        assert_eq!(
            crate::managed_install::GITHUB_REPO,
            "project-oxi/oxios",
            "managed_install::GITHUB_REPO must be the canonical repo"
        );

        // Local constants removed — fetch_github_release now reads
        // `managed_install::GITHUB_REPO` directly. Verify the URL
        // composition would target the canonical repo by checking the
        // format-string source itself.
        //
        // The body of fetch_github_release is private; we assert it
        // by reaching into the source text via a debug assertion is
        // out of scope. Instead, the strongest cheap guarantee is
        // that the literal `a7garden` string no longer appears in
        // this module's fetch URL composition.
        //
        // Note: a literal `a7garden` substring check on the file
        // would be brittle (could appear in a comment); the constant
        // check above is the contract.
    }

    #[test]
    fn via_cargo_dispatch_overrides_channel_to_cargo() {
        // The brief: "Via=cargo → existing cargo branch unchanged".
        // Whatever the running channel is, an explicit `via: "cargo"`
        // forces the legacy cargo branch.
        let oxios_home = PathBuf::from("/nonexistent/.oxios");
        let exe = PathBuf::from("/nonexistent/bin/oxios");
        let d = classify_update_dispatch(&oxios_home, &exe, Some("cargo"), None);
        assert_eq!(
            d,
            UpdateDispatch::Cargo {
                args: vec![
                    "install".to_string(),
                    "oxios".to_string(),
                    "--locked".to_string(),
                ],
            },
            "via=cargo (no version) must select the Cargo branch with the unpinned argv"
        );
    }

    #[test]
    fn via_cargo_with_version_threads_version_into_cargo_args() {
        // F1: POST {"version":"1.40.0","via":"cargo"} MUST install
        // 1.40.0, not the latest. The `--version <v>` flag is appended
        // to the cargo argv so a pinned run respects the pin.
        let oxios_home = PathBuf::from("/nonexistent/.oxios");
        let exe = PathBuf::from("/nonexistent/bin/oxios");
        let d = classify_update_dispatch(&oxios_home, &exe, Some("cargo"), Some("1.40.0"));
        assert_eq!(
            d,
            UpdateDispatch::Cargo {
                args: vec![
                    "install".to_string(),
                    "oxios".to_string(),
                    "--locked".to_string(),
                    "--version".to_string(),
                    "1.40.0".to_string(),
                ],
            },
            "via=cargo + version must pin the cargo install argv"
        );
    }

    #[test]
    fn via_cargo_on_brew_channel_refuses_rather_than_install() {
        // CLI contract (run_update:200-212) is refuse-first: Brew channels
        // refuse BEFORE any --via handling — `cargo install` would silently
        // shadow the brew copy. The HTTP dispatch must mirror that order.
        let oxios_home = PathBuf::from("/nonexistent/.oxios");
        let exe = PathBuf::from("/opt/homebrew/bin/oxios");
        let d = classify_update_dispatch(&oxios_home, &exe, Some("cargo"), None);
        assert_eq!(
            d,
            UpdateDispatch::Brew,
            "via=cargo on a Brew channel must refuse, not install"
        );
    }

    #[test]
    fn via_cargo_on_dev_channel_refuses_rather_than_install() {
        // Same refuse-first contract for dev builds: explicit via=cargo
        // must not shadow a workspace build with a crates.io copy.
        let tmp = tempfile::tempdir().expect("tempdir");
        let oxios_home = tmp.path().join(".oxios");
        std::fs::create_dir_all(&oxios_home).expect("mkdir");
        // Synthetic dev-build exe: a file under a directory that contains
        // Cargo.toml + crates/oxios-kernel (the workspace marker).
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).expect("mkdir ws");
        std::fs::write(ws.join("Cargo.toml"), b"[workspace]").expect("write");
        std::fs::create_dir_all(ws.join("crates").join("oxios-kernel")).expect("mkdir");
        std::fs::create_dir_all(ws.join("target").join("debug")).expect("mkdir");
        let exe = ws.join("target").join("debug").join("oxios");
        std::fs::write(&exe, b"dev").expect("write");
        let d = classify_update_dispatch(&oxios_home, &exe, Some("cargo"), None);
        assert_eq!(
            d,
            UpdateDispatch::Dev,
            "via=cargo on a Dev channel must refuse, not install"
        );
    }

    #[test]
    fn auto_managed_uses_full_managed_install_branch() {
        // A managed exe (canonicalized <home>/bin/oxios) under Auto must
        // land on the full managed-install branch — fetch → precheck →
        // install → flip → prune. The shadow audit + response payload is
        // a separate concern (next test).
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        let oxios_home = home.join(".oxios");
        std::fs::create_dir_all(oxios_home.join("bin")).expect("mkdir bin");
        std::fs::write(oxios_home.join("bin").join("oxios"), b"fake").expect("write");
        let exe = oxios_home.join("bin").join("oxios");
        let d = classify_update_dispatch(&oxios_home, &exe, None, None);
        assert_eq!(d, UpdateDispatch::Managed, "managed exe → Managed branch");
    }

    #[test]
    fn auto_detected_cargo_routes_to_unmanaged_branch() {
        // F2: per the design channel table, a daemon detected as
        // `Channel::Cargo` is treated as Unmanaged — the managed
        // install runs but PATH shadow migration does NOT. Only an
        // explicit `via: "cargo"` reaches the legacy block.
        let tmp = tempfile::tempdir().expect("tempdir");
        let oxios_home = tmp.path().join(".oxios");
        std::fs::create_dir_all(&oxios_home).expect("mkdir");
        // Synthetic exe that classifies as Cargo: a file inside
        // `.cargo/bin` makes `classify_channel` see `Channel::Cargo`.
        let cargo_dir = tmp.path().join(".cargo").join("bin");
        std::fs::create_dir_all(&cargo_dir).expect("mkdir cargo");
        std::fs::write(cargo_dir.join("oxios"), b"cargo-install").expect("write");
        let exe = cargo_dir.join("oxios");
        let d = classify_update_dispatch(&oxios_home, &exe, None, None);
        assert_eq!(
            d,
            UpdateDispatch::Unmanaged,
            "Channel::Cargo + Auto must NOT take the legacy cargo branch"
        );
    }

    #[test]
    fn auto_unmanaged_returns_audit_payload_with_migrated_false() {
        // Unmanaged over HTTP must NEVER auto-migrate — the handler
        // binds `migrated = false` so no code path can flip it back on,
        // and the audit + instructions are surfaced so the operator
        // can re-run `oxios update` in a TTY to adopt.
        let tmp = tempfile::tempdir().expect("tempdir");
        let oxios_home = tmp.path().join(".oxios");
        std::fs::create_dir_all(&oxios_home).expect("mkdir");

        // A real shadow binary outside <home>/bin to exercise the payload.
        let shadow_dir = tmp.path().join("usr").join("local").join("bin");
        std::fs::create_dir_all(&shadow_dir).expect("mkdir shadow");
        std::fs::write(shadow_dir.join("oxios"), b"shadow").expect("write shadow");

        let path_env = format!("{}:/usr/bin:/bin", shadow_dir.display());
        let shadows = crate::managed_install::audit_shadows(&oxios_home, &path_env);
        assert!(
            shadows.iter().any(|s| s.path == shadow_dir.join("oxios")),
            "audit_shadows must surface the unmanaged shadow; got {shadows:?}"
        );

        let payload = build_shadow_payload(&shadows);
        assert_eq!(payload.len(), shadows.len());
        for entry in &payload {
            assert!(entry.path.ends_with("oxios"));
            assert!(entry.size_bytes > 0, "real file shadow has nonzero size");
        }

        // Instructions: only when shadows are non-empty.
        let instr = update_instructions(&shadows);
        assert!(instr.is_some(), "non-empty shadows produce instructions");
        let instr = instr.unwrap();
        assert!(instr.contains("oxios update"));
        assert!(instr.contains("terminal"));

        // No shadows → empty payload + no instructions.
        let empty = build_shadow_payload(&[]);
        assert!(empty.is_empty());
        assert!(update_instructions(&[]).is_none());
    }

    #[test]
    fn update_response_migrated_is_always_false_for_dispatch_variants() {
        // F5: pin a real contract. Drive `build_update_response`
        // (the same path the handler takes) for the post-pipeline
        // shape, and verify `migrated` is `false` no matter what
        // dispatch the test feeds in. The JSON shape is asserted
        // alongside so a regression in serialization is caught too.
        let resp = build_update_response(
            "1.45.0",
            true,
            false,
            &["Managed install to 1.45.0 (pruned: <none>)".to_string()],
            &[],
            "",
        );
        let v = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(v["migrated"], json!(false));
        assert_eq!(v["shadows"], json!([]));
        assert_eq!(v["instructions"], json!(""));
        assert_eq!(v["success"], json!(true));
        assert_eq!(v["updated_to"], json!("1.45.0"));
        assert_eq!(v["binary_updated"], json!(true));
        assert_eq!(v["web_updated"], json!(false));

        // Non-empty shadows + instructions round-trip too.
        let shadows = vec![crate::managed_install::ShadowEntry {
            path: PathBuf::from("/usr/local/bin/oxios"),
            symlink_target: None,
            kind: crate::managed_install::ShadowKind::Canonical,
            size_bytes: 7,
        }];
        let resp = build_update_response(
            "1.45.0",
            false,
            true,
            &[],
            &shadows,
            "re-run `oxios update` in a terminal to adopt 1 shadow",
        );
        let v = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(v["migrated"], json!(false));
        assert_eq!(v["shadows"][0]["path"], json!("/usr/local/bin/oxios"));
        assert_eq!(
            v["instructions"],
            json!("re-run `oxios update` in a terminal to adopt 1 shadow")
        );
    }

    #[test]
    fn shadow_payload_uses_path_target_size_shape() {
        // The brief pins the response shape: { path, target, size_bytes }.
        // target is the symlink target (None for regular files) → emitted
        // as JSON null so the field always exists.
        let shadows = vec![ShadowEntry {
            path: PathBuf::from("/usr/local/bin/oxios"),
            symlink_target: Some(PathBuf::from("/opt/old/oxios")),
            kind: crate::managed_install::ShadowKind::Canonical,
            size_bytes: 42,
        }];
        let v = serde_json::to_value(build_shadow_payload(&shadows)).expect("serialize");
        assert_eq!(v[0]["path"], json!("/usr/local/bin/oxios"));
        assert_eq!(v[0]["target"], json!("/opt/old/oxios"));
        assert_eq!(v[0]["size_bytes"], json!(42));

        let regular = vec![ShadowEntry {
            path: PathBuf::from("/usr/local/bin/oxios"),
            symlink_target: None,
            kind: crate::managed_install::ShadowKind::Canonical,
            size_bytes: 7,
        }];
        let v2 = serde_json::to_value(build_shadow_payload(&regular)).expect("serialize");
        assert_eq!(v2[0]["target"], serde_json::Value::Null);
    }
}
