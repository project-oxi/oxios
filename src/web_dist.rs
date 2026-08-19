//! Web UI dist resolution.
//!
//! Embedded builds (default release path) serve the SPA compiled into the
//! binary — exclusively. Non-embedded builds (`cargo install` from crates.io)
//! download `web-dist.zip` from GitHub Releases at startup and resolve the
//! downloaded generation via the `~/.oxios/web/.active` marker on restart.
//!
//! There is deliberately NO on-disk override for embedded builds: a manually
//! placed dist used to shadow binary deploys silently (stale-UI incidents,
//! 2026-08-19) and was removed. The resolved path is passed to surfaces via
//! `SurfaceContext.web_dist` so the server never starts listening before
//! the web UI source is known.

use anyhow::{Context, Result};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const GITHUB_REPO: &str = "a7garden/oxios";

/// Returns `~/.oxios/web/` (download staging root + marker location).
fn user_web_root() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".oxios").join("web"))
}

/// Returns the path to the active-dist marker file (`~/.oxios/web/.active`).
///
/// RFC-024 SP3: persists the path the in-memory atomic pointer last pointed
/// at, so a daemon restart resolves the same generation the previous process
/// was serving (the pointer itself does not survive restart).
pub fn active_marker_path() -> Option<PathBuf> {
    user_web_root().map(|r| r.join(".active"))
}

/// Diagnosis of the active web-dist, used by `oxios status` to report UI
/// integrity independently of process liveness. A daemon can be alive while
/// serving a dangling marker (a raced update deleted the active dir) —
/// exactly the "status says Running but the UI 404s" confusion this
/// disambiguates. Reads the *persisted* marker (not another process's
/// in-memory pointer).
pub enum WebUiHealth {
    /// Resolves to a self-consistent directory, optionally with a version.
    Ok {
        /// Absolute path of the served dist.
        path: PathBuf,
        /// Version string from `version.json`, when present.
        version: Option<String>,
    },
    /// Marker present but resolves to no usable directory.
    Broken {
        /// The marker path that would not resolve.
        marker: PathBuf,
    },
    /// Embedded assets baked into the binary; no on-disk dist needed.
    Embedded {
        /// Version from the embedded `version.json`, when present.
        version: Option<String>,
    },
    /// No marker / nothing installed on this machine.
    NotInstalled,
}

/// Resolve the active web-dist health for status reporting.
pub fn diagnose_active() -> WebUiHealth {
    // Embedded builds always serve the compiled-in SPA — nothing on disk can
    // override that, so report it without consulting the marker.
    if crate::embedded_web::is_embedded() {
        return WebUiHealth::Embedded {
            version: Some(crate::embedded_web::version()),
        };
    }
    let Some(marker) = active_marker_path() else {
        return WebUiHealth::NotInstalled;
    };
    match oxios_gateway::ActiveWebDist::resolve(&marker) {
        Some(p) => {
            let version = std::fs::read(p.join("version.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .and_then(|v| v["version"].as_str().map(str::to_string));
            WebUiHealth::Ok { path: p, version }
        }
        None => WebUiHealth::Broken { marker },
    }
}

/// Result of ensuring web UI availability.
#[derive(Debug)]
pub enum WebDistResult {
    /// Non-embedded build: generation resolved from the persisted
    /// `~/.oxios/web/.active` marker (survives restarts).
    Marker(PathBuf),
    /// Non-embedded build: downloaded from GitHub Releases.
    Downloaded { path: PathBuf, version: String },
    /// Embedded build: the SPA compiled into the binary. Authoritative and
    /// exclusive — no on-disk dist can override or shadow it.
    Embedded,
    /// Download failed — nothing to serve for the web surface (embedded
    /// builds never reach this variant).
    DownloadFailed { reason: String },
}

impl WebDistResult {
    /// Returns the version tag without the leading 'v' prefix (for display).
    pub fn version_display(&self) -> Option<&str> {
        match self {
            WebDistResult::Downloaded { version, .. } => Some(version.trim_start_matches('v')),
            _ => None,
        }
    }
}

/// Format bytes into human-readable string.
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Fetches the latest release tag from GitHub API.
async fn fetch_latest_release_tag() -> Result<String> {
    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    let client = reqwest::Client::builder()
        .user_agent("oxios-web")
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .context("failed to create HTTP client")?;
    let resp: serde_json::Value = client
        .get(&url)
        .send()
        .await
        .context("failed to fetch GitHub release info")?
        .json()
        .await
        .context("failed to parse GitHub response")?;
    let tag = resp["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("tag_name not found in GitHub response"))?;
    Ok(tag.to_string())
}

/// Extract a `web-dist.zip` byte slice into `dest` (created if missing).
///
/// Atomic: extracts into a temp sibling first, then renames into `dest`, so
/// a crash mid-extract never leaves a half-populated target. Returns the
/// number of files extracted. `dest` is cleared first if it already exists.
///
/// Currently the building block for the RFC-042 Tauri seed installer (a
/// future caller passes pre-seeded bytes through the unified web installer);
/// the daemon's own download path uses [`download_and_extract_web_dist`]
/// (same staging convention, with a progress bar). Kept `pub` so the seed
/// path composes extraction + [`ActiveWebDist::persist_marker`] without
/// re-implementing atomicity.
#[allow(dead_code)]
pub fn extract_zip_into(dest: &std::path::Path, bytes: &[u8]) -> Result<usize> {
    // Extract into a temp sibling first, then rename atomically into `dest`.
    // A crash mid-extract therefore never leaves a half-populated `dest`
    // that a later health check might publish, and two extracts to the same
    // deterministic staging path can't interleave (each owns its own temp
    // dir). `dest` is overwritten only once the new tree is fully written.
    let parent = dest.parent().context("staging dir has no parent")?;
    let tmp_name = format!(
        ".{}-extract.tmp",
        dest.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("staging")
    );
    let tmp = parent.join(tmp_name);
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    std::fs::create_dir_all(&tmp)?;

    let reader = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).context("invalid zip file")?;
    let mut count = 0usize;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).context("zip read error")?;
        let outpath = match file.enclosed_name() {
            Some(path) => tmp.join(path),
            None => continue,
        };
        if file.is_dir() {
            std::fs::create_dir_all(&outpath)?;
        } else {
            if let Some(p) = outpath.parent()
                && !p.exists()
            {
                std::fs::create_dir_all(p)?;
            }
            let mut outfile = std::fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
            count += 1;
        }
    }

    // Publish the complete tree into place. Rename is atomic on the same
    // filesystem (parent is the same dir); remove a prior dest first so the
    // rename target never pre-exists.
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }
    std::fs::rename(&tmp, dest)
        .with_context(|| format!("failed to publish staging dir into {}", dest.display()))?;
    Ok(count)
}

/// Path for a versioned staging directory under `~/.oxios/web/`.
pub fn staging_dir_for(version_tag: &str) -> Option<PathBuf> {
    let id = version_tag.trim_start_matches('v');
    user_web_root().map(|r| r.join(format!("dist-{id}")))
}

/// Downloads `web-dist.zip` from a GitHub release and extracts it into a
/// **fresh, versioned staging directory** (`~/.oxios/web/dist-<version>/`).
///
/// RFC-024 SP3: never deletes the canonical `dist/` here — the caller
/// publishes the staging dir atomically via the in-memory pointer + marker
/// so concurrent requests never observe a half-extracted directory.
async fn download_and_extract_web_dist(version_tag: &str) -> Result<PathBuf> {
    let dist_dir = staging_dir_for(version_tag)
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;

    let url =
        format!("https://github.com/{GITHUB_REPO}/releases/download/{version_tag}/web-dist.zip");

    let client = reqwest::Client::builder()
        .user_agent("oxios-web")
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .context("failed to create HTTP client")?;

    // ── Download with progress bar ─────────────────────────────────────────
    let resp = client
        .get(&url)
        .send()
        .await
        .context("download request failed")?;

    if !resp.status().is_success() {
        anyhow::bail!("Failed to download web-dist.zip: HTTP {}", resp.status());
    }

    let total_size = resp.content_length().unwrap_or(0);
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("  {spinner} {msg}  [{bar:>.dim}] {bytes}/{total_bytes} ({bytes_per_sec})")
            .expect("valid progress-bar template")
            .progress_chars("█▉▊▋▌▍▎▏  "),
    );
    let tag_label = style(version_tag).cyan().to_string();
    pb.set_message(format!("Downloading web UI {tag_label}"));

    let bytes = resp.bytes().await.context("failed to read response body")?;

    let ok = style("✓").green().to_string();
    let downloaded = style("Downloaded").green().to_string();
    let done_msg = format!(
        "  {} {} ({})",
        ok,
        downloaded,
        format_size(bytes.len() as u64)
    );
    pb.finish_with_message(done_msg);

    // ── Extract with progress ─────────────────────────────────────────────
    let reader = std::io::Cursor::new(bytes.as_ref());
    let mut archive = zip::ZipArchive::new(reader).context("invalid zip file")?;
    let file_count = archive.len();

    let extract_pb = ProgressBar::new(file_count as u64);
    extract_pb.set_style(
        ProgressStyle::default_bar()
            .template("  {spinner} {msg}  [{bar:>.dim}] {pos}/{len}")
            .expect("valid progress-bar template")
            .progress_chars("█▉▊▋▌▍▎▏  "),
    );
    extract_pb.set_message("Extracting files".to_string());

    // Clear any pre-existing staging dir for this exact version (interrupted
    // prior run), then create fresh. The canonical `dist/` is left untouched.
    if dist_dir.exists() {
        std::fs::remove_dir_all(&dist_dir)?;
    }
    std::fs::create_dir_all(&dist_dir)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = match file.enclosed_name() {
            Some(path) => dist_dir.join(path),
            None => continue,
        };
        if file.is_dir() {
            std::fs::create_dir_all(&outpath)?;
        } else {
            if let Some(p) = outpath.parent()
                && !p.exists()
            {
                std::fs::create_dir_all(p)?;
            }
            let mut outfile = std::fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
        }
        extract_pb.inc(1);
    }

    let ok = style("✓").green().to_string();
    let done_msg = format!("  {ok} {file_count} files extracted");
    extract_pb.finish_with_message(done_msg);

    tracing::info!(
        path = ?dist_dir,
        version = %version_tag,
        "Web UI downloaded and extracted"
    );

    Ok(dist_dir)
}

/// Ensures the web UI source is known before surfaces start.
///
/// Resolution order:
///  1. Embedded assets (`src/embedded_web.rs`) — authoritative and
///     EXCLUSIVE when compiled in. Nothing on disk is consulted; no download
///     runs; no on-disk dist can shadow the binary's UI.
///  2. `~/.oxios/web/.active` marker → generation last served (non-embedded
///     builds only; survives restart).
///  3. Download from GitHub Releases into a fresh versioned staging dir,
///     then publish via marker so restarts resolve it (non-embedded only).
///
/// The former `~/.oxios/web/dist/` (user override) and
/// `<workspace>/web/dist/` tiers were removed: they silently shadowed
/// embedded binaries and served stale UIs across deploys (2026-08-19).
pub async fn ensure_web_dist() -> WebDistResult {
    // 1. Embedded assets (compiled in via `build.rs`). Authoritative and
    //    exclusive — short-circuits every on-disk path and the download.
    if crate::embedded_web::is_embedded() {
        tracing::info!("Serving web UI from embedded assets (exclusive)");
        return WebDistResult::Embedded;
    }

    // 2. Marker (RFC-024): generation the previous process was serving.
    //    Also guards pre-1.29.0 upgraders whose marker still points at a
    //    stale `~/.oxios/web/dist-*/` download from an older version —
    //    resolve() validates self-consistency and refuses broken dirs.
    if let Some(m) = active_marker_path()
        && let Some(p) = oxios_gateway::ActiveWebDist::resolve(&m)
    {
        tracing::info!(path = ?p, "Serving web UI from active marker");
        return WebDistResult::Marker(p);
    }

    // 3. Auto-download from GitHub Releases (with bounded retry so a transient
    //    network blip or rate-limit doesn't strand the daemon serving 503
    //    until a manual `oxios update --web-only`). Each attempt retries the
    //    full tag-lookup + download pair.
    tracing::info!("No web UI found locally, downloading from GitHub Releases...");

    const MAX_ATTEMPTS: u32 = 3;
    let mut last_reason = String::from("unknown error");
    for attempt in 1..=MAX_ATTEMPTS {
        let outcome = match fetch_latest_release_tag().await {
            Ok(tag) => match download_and_extract_web_dist(&tag).await {
                Ok(path) => Some((tag, path)),
                Err(e) => {
                    last_reason = e.to_string();
                    None
                }
            },
            Err(e) => {
                last_reason = e.to_string();
                None
            }
        };

        if let Some((tag, path)) = outcome {
            // Validate the freshly-extracted dist is internally consistent
            // before honoring it — a corrupt release asset (or a zip that
            // drops the entry chunk) must not strand the daemon serving a
            // broken page. Treat a failed check like a download failure so
            // the bounded retry loop kicks in.
            if !oxios_gateway::ActiveWebDist::dist_is_consistent(&path) {
                last_reason = format!(
                    "extracted dist for {tag} is not self-consistent \
                     (index.html references missing assets)"
                );
                tracing::warn!(
                    attempt,
                    tag = %tag,
                    "extracted web dist is not self-consistent; retrying"
                );
            } else {
                // Publish the freshly-extracted staging dir so restarts
                // resolve it via the marker.
                if let Some(m) = active_marker_path() {
                    let _ = std::fs::write(m, path.to_string_lossy().as_bytes());
                }
                return WebDistResult::Downloaded { path, version: tag };
            }
        }

        if attempt < MAX_ATTEMPTS {
            tracing::warn!(
                attempt,
                max = MAX_ATTEMPTS,
                reason = %last_reason,
                "Web UI download failed, retrying"
            );
            tokio::time::sleep(std::time::Duration::from_secs(2 * attempt as u64)).await;
        } else {
            tracing::warn!(reason = %last_reason, "Web UI download failed (no retries left)");
        }
    }
    WebDistResult::DownloadFailed {
        reason: last_reason,
    }
}

// ── Web UI sync (latest / pinned) ────────────────────────────────────────
//
// `ensure_web_dist` only fetches a dist when nothing usable exists locally;
// it never compares the installed version to GitHub's latest. The sync API
// below closes that gap: compare → download → atomic publish. It is shared
// by three callers:
//   • the daily health check (kernel.rs) — periodic catch-up,
//   • the eager startup check (kernel.rs) — so a frequently-restarted host
//     still updates (the old code slept to 03:00 before its first check),
//   • `oxios update --web-only` (commands/update.rs) — manual / pinned.

/// Which release to sync the web UI to.
#[derive(Debug, Clone)]
pub enum SyncTarget {
    /// GitHub `releases/latest`.
    Latest,
    /// A specific tag, e.g. `v1.28.0` or `1.28.0` (leading `v` optional).
    Version(String),
}

/// Outcome of a [`sync`] / [`sync_to_disk`] attempt.
#[derive(Debug, Clone)]
pub enum SyncOutcome {
    /// Active dist already reports `version.json` equal to the target.
    UpToDate {
        /// Version the active dist currently reports.
        active: String,
        /// Version the caller asked to sync to.
        target: String,
    },
    /// Downloaded and published a new generation atomically.
    Updated {
        /// Tag (with leading `v`) that was published.
        to: String,
    },
    /// Active dist is consistent but its `version.json` is blank/unstamped.
    /// Left untouched to avoid a download storm when version stamping
    /// regresses (mirrors the original daily-check guard).
    Unstamped,
    /// Check failed (network, API, extraction, inconsistent dist). The
    /// daemon keeps serving whatever it had.
    Failed { reason: String },
}

/// Read the `version` field from `<dist>/version.json`, if present.
fn read_version_json(dist: &Path) -> Option<String> {
    std::fs::read(dist.join("version.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v["version"].as_str().map(str::to_string))
}

/// Normalize a user-supplied version to a `v`-prefixed tag.
fn normalize_tag(v: &str) -> String {
    let v = v.trim();
    let core = if let Some(rest) = v.strip_prefix('v').or_else(|| v.strip_prefix('V')) {
        rest.trim()
    } else {
        v
    };
    format!("v{core}")
}

/// Resolve a [`SyncTarget`] to a concrete GitHub release tag. `Latest` does
/// an API lookup; `Version` is normalized locally (a bad tag surfaces as a
/// 404 at download time).
async fn resolve_target_tag(target: &SyncTarget) -> Result<String> {
    match target {
        SyncTarget::Latest => fetch_latest_release_tag().await,
        SyncTarget::Version(v) => Ok(normalize_tag(v)),
    }
}

/// Resolve the currently-active dist path from the persisted marker.
/// Used by the CLI disk-only path, which has no in-memory pointer.
fn current_active_path() -> Option<PathBuf> {
    let marker = active_marker_path()?;
    oxios_gateway::ActiveWebDist::resolve(&marker)
}

/// Internal outcome of the compare+download stage, before the publish
/// strategy (in-memory vs disk-only) is chosen.
enum PrepareOutcome {
    /// Active dist already matches the target.
    UpToDate { active: String, target: String },
    /// Active dist is consistent but unstamped.
    Unstamped,
    /// Downloaded + extracted + validated; ready to publish.
    Ready { tag: String, staging: PathBuf },
    /// Something went wrong.
    Failed(String),
}

/// Shared core: resolve target, compare to the active dist, download into a
/// fresh versioned staging dir, validate self-consistency. `active_path` is
/// the dist currently being served (`None` when unknown). This does NOT
/// publish — the caller decides how (in-memory pointer vs disk marker).
async fn prepare_sync(target: &SyncTarget, active_path: Option<&Path>) -> PrepareOutcome {
    let tag = match resolve_target_tag(target).await {
        Ok(t) => t,
        Err(e) => return PrepareOutcome::Failed(e.to_string()),
    };
    let target_version = tag.trim_start_matches('v').to_string();

    let (consistent, current_version) = match active_path {
        Some(p) => (
            oxios_gateway::ActiveWebDist::dist_is_consistent(p),
            read_version_json(p).unwrap_or_default(),
        ),
        None => (false, String::new()),
    };

    // Up-to-date: consistent, stamped, and equal to target.
    if consistent && !current_version.is_empty() && current_version == target_version {
        return PrepareOutcome::UpToDate {
            active: current_version,
            target: target_version,
        };
    }
    // Consistent but unstamped → leave alone (avoids a re-download storm
    // when version stamping regresses — see web/vite.config.ts).
    if consistent && current_version.is_empty() {
        return PrepareOutcome::Unstamped;
    }
    // Missing / inconsistent / different version → download into a fresh
    // versioned staging dir. `download_and_extract_web_dist` clears any
    // pre-existing staging dir for this exact version first.
    let staging = match download_and_extract_web_dist(&tag).await {
        Ok(p) => p,
        Err(e) => return PrepareOutcome::Failed(e.to_string()),
    };
    // Validate the freshly-extracted dist before honoring it — a corrupt or
    // partial extraction (entry chunk missing) must never become active.
    if !oxios_gateway::ActiveWebDist::dist_is_consistent(&staging) {
        return PrepareOutcome::Failed(format!(
            "extracted dist for {tag} is not self-consistent \
             (index.html references missing assets)"
        ));
    }
    PrepareOutcome::Ready { tag, staging }
}

/// Daemon entry: sync the active web-dist to `target`, atomically publishing
/// via the in-memory pointer + persisted marker. A running daemon swaps to
/// the new generation without restart. Non-fatal on failure — the daemon
/// keeps serving whatever it had.
///
/// Used by the daily health check and the eager startup check.
pub async fn sync(web_dist: &oxios_gateway::ActiveWebDist, target: SyncTarget) -> SyncOutcome {
    // Embedded builds serve the compiled-in SPA exclusively (see
    // `embedded_web`). Skip the compare+download entirely — a downloaded
    // generation can never shadow it. `sync_to_disk` below carries the same
    // gate.
    if crate::embedded_web::is_embedded() {
        return SyncOutcome::UpToDate {
            active: "embedded".into(),
            target: "embedded".into(),
        };
    }
    let active = web_dist.path();
    match prepare_sync(&target, active.as_deref()).await {
        PrepareOutcome::UpToDate { active, target } => SyncOutcome::UpToDate { active, target },
        PrepareOutcome::Unstamped => SyncOutcome::Unstamped,
        PrepareOutcome::Failed(reason) => SyncOutcome::Failed { reason },
        PrepareOutcome::Ready { tag, staging } => {
            let Some(marker) = active_marker_path() else {
                return SyncOutcome::Failed {
                    reason: "cannot determine home directory".into(),
                };
            };
            web_dist.publish(staging, &marker);
            SyncOutcome::Updated { to: tag }
        }
    }
}

/// CLI / disk-only entry: sync to `target` by downloading into a versioned
/// staging dir and persisting the marker — WITHOUT touching an in-memory
/// pointer (the CLI runs in its own process; the running daemon picks the
/// new generation up on restart via `resolve`).
///
/// Used by `oxios update --web-only`. Gated for embedded builds: the
/// compiled-in SPA is exclusive, so there is nothing to sync — the CLI
/// prints an explicit skip message instead.
pub async fn sync_to_disk(target: SyncTarget) -> SyncOutcome {
    if crate::embedded_web::is_embedded() {
        return SyncOutcome::UpToDate {
            active: "embedded".into(),
            target: "embedded".into(),
        };
    }
    let active = current_active_path();
    match prepare_sync(&target, active.as_deref()).await {
        PrepareOutcome::UpToDate { active, target } => SyncOutcome::UpToDate { active, target },
        PrepareOutcome::Unstamped => SyncOutcome::Unstamped,
        PrepareOutcome::Failed(reason) => SyncOutcome::Failed { reason },
        PrepareOutcome::Ready { tag, staging } => {
            let Some(marker) = active_marker_path() else {
                return SyncOutcome::Failed {
                    reason: "cannot determine home directory".into(),
                };
            };
            oxios_gateway::ActiveWebDist::persist_marker(&marker, &staging);
            SyncOutcome::Updated { to: tag }
        }
    }
}

// ── Eager-startup throttle ───────────────────────────────────────────────
//
// The eager startup check runs `sync(Latest)` once on daemon boot so a host
// that never survives until 03:00 still gets web UI updates. To keep a crash
// loop from hammering GitHub (unauth limit 60/hr), it is throttled to once
// per hour via the mtime of `~/.oxios/web/.last-check`.

/// Minimum spacing between eager startup checks (crash-loop protection).
const EAGER_THROTTLE: Duration = Duration::from_secs(3600);

/// Path to the throttle sentinel (`~/.oxios/web/.last-check`).
fn last_check_path() -> Option<PathBuf> {
    user_web_root().map(|r| r.join(".last-check"))
}

/// True when no eager check has run in the last hour (or none ever ran).
pub(crate) fn eager_check_allowed() -> bool {
    let Some(p) = last_check_path() else {
        return true;
    };
    let Ok(meta) = std::fs::metadata(&p) else {
        return true;
    };
    match meta.modified() {
        Ok(t) => SystemTime::now()
            .duration_since(t)
            .map(|elapsed| elapsed >= EAGER_THROTTLE)
            .unwrap_or(true),
        Err(_) => true,
    }
}

/// Record that an eager check just ran (updates the sentinel's mtime).
pub(crate) fn touch_last_check() {
    if let Some(p) = last_check_path() {
        // `write` creates the file if absent and bumps mtime either way.
        let _ = std::fs::write(&p, b"");
    }
}
