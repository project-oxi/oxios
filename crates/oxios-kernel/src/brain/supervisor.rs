//! Brain supervisor (RFC-049): types and pure install helpers.
//!
//! This module is split deliberately. Types live here so they round-trip with
//! `serde` even before the supervisor itself is wired in — tests can construct
//! [`SupervisorStatus`] and serialize without touching `launchd`. The pure
//! helpers ([`build_plist`], [`asset_urls`], [`verify_sha256`],
//! [`extract_single_binary`]) own the deterministic install math: any path
//! through them with the same inputs produces byte-identical outputs, so
//! they're trivially unit-testable without a real GitHub release or
//! filesystem layout.
//!
//! I/O — `launchctl` invocation, plist installation, daemon downloads — lives
//! in the `BrainSupervisor` struct delivered by a follow-up task; keeping it
//! out of this module means tests never spawn a process.

use anyhow::{Context, Result};
use futures::future::BoxFuture;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::config::BrainSection;
use crate::metrics::get_metrics;

/// launchd service label used for the oxibrain daemon.
pub const LAUNCHD_LABEL: &str = "com.oxi.oxibrain";

/// GitHub release JSON endpoint for the latest oxibrain build.
pub const RELEASES_LATEST_URL: &str =
    "https://api.github.com/repos/a7garden/oxibrain/releases/latest";

/// Tarball asset name for aarch64-apple-darwin — sole first-party target.
pub const ASSET_TAR: &str = "oxibrain-aarch64-apple-darwin.tar.gz";

/// Coarse lifecycle state reported to callers of `BrainSupervisor::status`.
///
/// The lifecycle progresses `Disabled` → `NotInstalled` → `Installing` →
/// `Starting` → `Online`. `Failed` is the terminal sink for any state
/// transition that hits an unrecoverable error (e.g. sha256 mismatch,
/// download HTTP 404, daemon crash loop). `Disabled` is the off-switch for
/// users who explicitly want oxios to leave the daemon alone (no install,
/// no auto-start) — distinct from `NotInstalled`, which means "we haven't
/// tried yet".
///
/// Distinct from [`ManagedBy`]: a daemon can be `Online` and still be
/// owned by another app's process tree. The status type composes both
/// fields so downstream code can decide.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorState {
    /// Auto-management explicitly turned off — supervisor must not touch
    /// the daemon. Fresh "off" state for users who want oxios to stay
    /// out of the way.
    Disabled,
    /// No binary is installed and no install is in progress. The
    /// supervisor will pick an install strategy on the next `ensure`.
    /// Also the [`Default`] for [`SupervisorStatus`] — "we know nothing
    /// about a daemon yet" reads more accurately than reporting
    /// `disabled` before the first `ensure()`.
    #[default]
    NotInstalled,
    /// An installer is currently downloading + verifying + extracting
    /// the binary. Concurrent `ensure` calls must join, not race.
    Installing,
    /// The daemon binary is on disk and we are about to start it (via
    /// launchd `bootstrap` or detached-spawn fallback).
    Starting,
    /// The daemon is running and healthy — most recent health probe
    /// succeeded. This is the steady state.
    Online,
    /// Last transition failed and was not retried into success; the
    /// `last_error` field on [`SupervisorStatus`] carries the detail.
    /// Caller can `ensure` again to retry.
    Failed,
}

/// Who is responsible for the daemon's lifecycle at this moment.
///
/// Mirrors the `managed_by` field of [`SupervisorStatus`]. The split exists
/// because oxibrain is a *shared* system service — oxios, oximemo, and
/// oxiline all consume the same daemon. We want to surface who started it
/// rather than silently overwrite that with our own `Supervisor` opinion.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedBy {
    /// Nothing owns the daemon; it's not running.
    #[default]
    None,
    /// launchd owns the daemon via the installed plist (preferred path).
    Launchd,
    /// We started it ourselves via a detached spawn (fallback when
    /// launchd is unavailable — e.g. inside a sandboxed child that
    /// can't write `~/Library/LaunchAgents`).
    Spawn,
    /// Another oxios-family app (oximemo / oxiline) already has the
    /// daemon running and is sharing it with us. We must never try to
    /// own, restart, or upgrade it — only consume.
    External,
}

/// Snapshot of the oxibrain daemon's install + run state.
///
/// Default is `NotInstalled` + `ManagedBy::None` — i.e. "we know nothing
/// about a daemon yet" (as opposed to `Disabled`, which is the
/// user-opt-out signal). Callers compare against `Default` to decide
/// whether to surface the status to the user or silently skip.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupervisorStatus {
    pub state: SupervisorState,
    /// Version of the binary on disk (`None` when nothing's installed).
    pub installed_version: Option<String>,
    /// Version the running daemon reported, if any.
    pub daemon_version: Option<String>,
    pub managed_by: ManagedBy,
    /// Last health-probe failure message; cleared on successful next probe.
    pub last_error: Option<String>,
}

/// Escape characters that are unsafe inside a plist `<string>` value.
///
/// launchd `plutil` is lenient, but a hand-written file may be re-read by
/// anything that uses a strict XML parser (and the file is human-visible in
/// `~/Library/LaunchAgents/`). Keep this in lockstep with the entity set
/// required by the XML 1.0 spec for well-formed text nodes.
fn escape_xml(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Render a launchd plist for the oxibrain daemon.
///
/// The plist is intentionally simple: no `EnvironmentVariables`,
/// `WorkingDirectory`, or `StandardInPath` — those get inherited from the
/// `launchctl bootstrap` caller's environment, which matches how the
/// kernel itself is launched.
pub fn build_plist(binary: &Path, log_path: &Path) -> String {
    let bin = escape_xml(&binary.display().to_string());
    let log = escape_xml(&log_path.display().to_string());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LAUNCHD_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{bin}</string>
        <string>serve</string>
        <string>--daemon</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>10</integer>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#
    )
}

/// Pull the (tarball, checksum) URLs out of a GitHub `releases/latest`
/// payload.
///
/// `tar` is `oxibrain-<target>.tar.gz` and `sha` is the same with
/// `.sha256` appended. We pair them by stem so a release with extra
/// unrelated assets (signature files, source tarballs) doesn't fool us into
/// downloading the wrong artifact.
pub fn asset_urls(release: &serde_json::Value) -> Option<(String, String)> {
    let assets = release.get("assets")?.as_array()?;
    let mut tar: Option<String> = None;
    let mut sha: Option<String> = None;
    for asset in assets {
        let name = asset.get("name")?.as_str()?;
        let url = asset.get("browser_download_url")?.as_str()?.to_string();
        if name == ASSET_TAR {
            tar = Some(url);
        } else if name == format!("{ASSET_TAR}.sha256") {
            sha = Some(url);
        }
    }
    match (tar, sha) {
        (Some(t), Some(s)) => Some((t, s)),
        _ => None,
    }
}

/// Verify a SHA-256 digest against raw bytes.
///
/// `expected` is either a bare hex digest (`"b94d…efcde9"`) or the BSD-style
/// `digest  filename` form some release scripts emit — we tokenize on
/// whitespace and take the first token. Anything that isn't exactly 64
/// lowercase hex chars (after `eq_ignore_ascii_case`) fails closed.
pub fn verify_sha256(bytes: &[u8], expected: &str) -> bool {
    use sha2::{Digest, Sha256};
    let first = expected.split_whitespace().next().unwrap_or("");
    if first.len() != 64 || !first.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = hasher.finalize();
    let actual_hex = hex::encode(actual);
    actual_hex.eq_ignore_ascii_case(first)
}
/// Extract the `oxibrain` binary out of a `.tar.gz` asset.
///
/// The release tarball contains exactly one entry we care about — the
/// statically-linked binary named `oxibrain` (no version suffix, per
/// design amendment 31223a68). Any other content (man pages, signatures,
/// `LICENSE`) is ignored; if the entry isn't present we return an error
/// rather than guessing, since extracting the wrong file would silently
/// ship a non-functional daemon.
///
/// Returning `anyhow::Result` (not `Option`) keeps the install pipeline
/// uniform: every failure along the way — network, sha256, archive
/// decode, missing entry — chains through `?` with `.context(...)`.
pub fn extract_single_binary(archive: &[u8]) -> Result<Vec<u8>> {
    let decoder = flate2::read::GzDecoder::new(archive);
    let mut tar_reader = tar::Archive::new(decoder);
    for entry in tar_reader.entries().context("iterate tar entries")? {
        let mut entry = entry.context("read tar entry header")?;
        let path = entry.path().context("decode tar entry path")?.into_owned();
        if path.file_name().and_then(|s| s.to_str()) == Some("oxibrain") {
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut buf)
                .context("read `oxibrain` entry body")?;
            return Ok(buf);
        }
    }
    anyhow::bail!("no `oxibrain` entry in archive")
}

/// Install the oxibrain daemon binary into `install_root`.
///
/// Implementations are responsible for the full pipeline: locate the
/// artifact, verify its integrity, and place an executable binary at
/// `<install_root>/oxibrain`. The trait is `async`-aware (returns a
/// boxed future) so the supervisor can plug in either a real GitHub
/// fetcher or a fake without monomorphising its call sites.
pub(crate) trait Installer: Send + Sync {
    /// Download + verify + extract + atomically install the binary.
    /// Returns the absolute path of the installed executable.
    fn install(&self, install_root: &Path) -> BoxFuture<'static, Result<PathBuf>>;
}

/// Default [`Installer`]: fetch the latest GitHub release for oxibrain,
pub(crate) struct GithubInstaller {
    /// `releases/latest` JSON endpoint. Overridable so tests can point at a
    /// local fixture; defaults to the production oxibrain repo.
    pub releases_url: String,
}

impl Default for GithubInstaller {
    fn default() -> Self {
        Self {
            releases_url: RELEASES_LATEST_URL.to_string(),
        }
    }
}

impl Installer for GithubInstaller {
    fn install(&self, install_root: &Path) -> BoxFuture<'static, Result<PathBuf>> {
        let url = self.releases_url.clone();
        let root = install_root.to_path_buf();
        Box::pin(async move {
            std::fs::create_dir_all(&root)
                .with_context(|| format!("create install root {}", root.display()))?;
            let client = Client::builder()
                .user_agent("oxios-brain-supervisor")
                .timeout(Duration::from_secs(180))
                .connect_timeout(Duration::from_secs(10))
                .build()
                .context("build reqwest client for github installer")?;
            let body = client
                .get(&url)
                .send()
                .await
                .with_context(|| format!("GET {url}"))?
                .error_for_status()
                .with_context(|| format!("{url} returned non-2xx"))?
                .text()
                .await
                .with_context(|| format!("read {url} body"))?;
            let release: serde_json::Value = serde_json::from_str(&body)
                .with_context(|| format!("parse release JSON from {url}"))?;
            let (tar_url, sha_url) = asset_urls(&release).with_context(|| {
                format!("release has no {ASSET_TAR} asset (pre-artifact release?)")
            })?;
            let tar_bytes = client
                .get(&tar_url)
                .send()
                .await
                .with_context(|| format!("GET {tar_url}"))?
                .error_for_status()
                .with_context(|| format!("{tar_url} returned non-2xx"))?
                .bytes()
                .await
                .with_context(|| format!("read {tar_url} body"))?;
            let sha_text = client
                .get(&sha_url)
                .send()
                .await
                .with_context(|| format!("GET {sha_url}"))?
                .error_for_status()
                .with_context(|| format!("{sha_url} returned non-2xx"))?
                .text()
                .await
                .with_context(|| format!("read {sha_url} body"))?;
            anyhow::ensure!(
                verify_sha256(&tar_bytes, sha_text.trim()),
                "sha256 mismatch for {tar_url} — refusing to install"
            );
            let binary = extract_single_binary(&tar_bytes)
                .with_context(|| format!("{tar_url} did not contain an `oxibrain` binary entry"))?;
            let target = root.join("oxibrain");
            let tmp = root.join(".oxibrain.tmp");
            std::fs::write(&tmp, &binary)
                .with_context(|| format!("write staged binary to {}", tmp.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&tmp)
                    .with_context(|| format!("stat {}", tmp.display()))?
                    .permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&tmp, perms)
                    .with_context(|| format!("chmod 0755 {}", tmp.display()))?;
            }
            std::fs::rename(&tmp, &target).with_context(|| {
                format!(
                    "atomic install of {} → {} (same volume required)",
                    tmp.display(),
                    target.display()
                )
            })?;
            tracing::info!(path = %target.display(), "installed oxibrain binary");
            Ok(target)
        })
    }
}
/// Look up the current user's POSIX UID by shelling out to `id -u`.
///
/// macOS launchd uses the `gui/<uid>/<label>` domain for per-user agents
/// (the `gui` domain is the only one that reliably allows bootstrap from
/// a non-root session). Reading `/etc/passwd` would require parsing on
/// top of libc's `getpwuid`; shelling out to `id -u` is the
/// always-present `/usr/bin` utility and produces a single decimal
/// integer on stdout. On any failure (missing binary, non-UTF-8,
/// non-numeric, permission denied) we fall back to `501` — the first
/// non-system macOS UID. The fallback keeps `ensure_launchd` from
/// hard-failing in a degraded environment; the supervisor will still
/// get a launchd attempt and fall back to detached spawn if launchctl
/// rejects the target.
fn nix_uid() -> u32 {
    // `id -u` is always present on macOS.
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(501)
}

/// Install + bootstrap the oxibrain agent via launchd (macOS only).
///
/// Returns `Ok(true)` when launchd was asked to take over the daemon
/// (plist written, bootstrap attempted, kickstart issued). Returns
/// `Ok(false)` when the launchd path is not available — caller should
/// fall back to [`spawn_detached`]. Never returns an error for the
/// "not applicable" cases (non-macOS, opt-out env var, no `$HOME`).
/// Hard errors are reserved for problems that prevent the file write
/// or both `bootstrap` and legacy `load` invocations failing.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn ensure_launchd(binary: &Path, log_path: &Path) -> Result<bool> {
    // Non-macOS: no launchd, no agent.
    if !cfg!(target_os = "macos") {
        return Ok(false);
    }
    // Explicit opt-out for tests and users debugging the spawn path.
    if std::env::var_os("OXIOS_BRAIN_NO_LAUNCHD").is_some() {
        tracing::debug!("OXIOS_BRAIN_NO_LAUNCHD set — skipping launchd bootstrap");
        return Ok(false);
    }
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            tracing::warn!("no $HOME — cannot place LaunchAgent plist");
            return Ok(false);
        }
    };
    let agents_dir = home.join("Library").join("LaunchAgents");
    std::fs::create_dir_all(&agents_dir)
        .with_context(|| format!("create {}", agents_dir.display()))?;
    let plist = agents_dir.join(format!("{LAUNCHD_LABEL}.plist"));
    let desired = build_plist(binary, log_path);
    let unchanged = std::fs::read_to_string(&plist)
        .map(|existing| existing == desired)
        .unwrap_or(false);
    let uid = nix_uid();
    let target = format!("gui/{uid}/{LAUNCHD_LABEL}");
    if !unchanged {
        std::fs::write(&plist, desired)
            .with_context(|| format!("write plist {}", plist.display()))?;
        // bootout is best-effort: failure means the agent wasn't loaded,
        // which is the common case on first install.
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", &target])
            .output();
        let bootstrap = std::process::Command::new("launchctl")
            .args([
                "bootstrap",
                &format!("gui/{uid}"),
                plist.to_string_lossy().as_ref(),
            ])
            .output();
        let bootstrap_ok = matches!(&bootstrap, Ok(o) if o.status.success());
        if !bootstrap_ok {
            // Legacy syntax (10.11-10.15-ish) fallback for older macOS.
            let load = std::process::Command::new("launchctl")
                .args(["load", "-w", plist.to_string_lossy().as_ref()])
                .output();
            if !matches!(&load, Ok(o) if o.status.success()) {
                let bb = bootstrap
                    .as_ref()
                    .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
                    .unwrap_or_default();
                let ll = load
                    .as_ref()
                    .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
                    .unwrap_or_default();
                tracing::warn!(
                    bootstrap_err = %bb,
                    load_err = %ll,
                    "launchctl bootstrap + load both failed; falling back to detached spawn"
                );
                return Ok(false);
            }
        }
    }
    // kickstart is best-effort: a fresh agent will start on its own
    // when bootstrapped, and an already-running agent is fine too.
    let _ = std::process::Command::new("launchctl")
        .args(["kickstart", &target])
        .output();
    Ok(true)
}

/// Spawn the oxibrain daemon as a detached child of the supervisor.
///
/// The child is placed in its own process group (Unix) so it can
/// outlive oxios without inheriting the kernel's terminal or signal
/// disposition. stdout + stderr are appended to `log_path`; stdin is
/// closed. The PID is written to `pidfile` for callers that want to
/// observe the daemon across oxios restarts. The returned PID matches
/// the value written to `pidfile`.
///
/// IMPORTANT: the `tokio::process::Child` is intentionally leaked
/// (`std::mem::forget`) — `kill_on_drop` is *not* set, so dropping the
/// handle would normally let the runtime reap a long-lived daemon. The
/// daemon is shared by oximemo and oxiline and must outlive oxios; this
/// function returns the PID precisely so callers can manage its
/// lifecycle, not the supervisor.
pub(crate) fn spawn_detached(binary: &Path, log_path: &Path, pidfile: &Path) -> Result<u32> {
    if let Some(parent) = log_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create log dir {}", parent.display()))?;
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .with_context(|| format!("open log file {}", log_path.display()))?;
    let log_for_err = log.try_clone().context("clone log fd for stderr")?;
    let mut cmd = tokio::process::Command::new(binary);
    cmd.arg("serve").arg("--daemon");
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::from(log));
    cmd.stderr(std::process::Stdio::from(log_for_err));
    #[cfg(unix)]
    {
        // SAFETY: `setsid` creates a new session; documented async-signal-safe.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    let child = cmd
        .spawn()
        .with_context(|| format!("spawn detached oxibrain at {}", binary.display()))?;
    let pid = child
        .id()
        .context("oxibrain child has no pid (already reaped?)")?;
    if let Some(parent) = pidfile.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create pidfile dir {}", parent.display()))?;
    }
    if let Err(e) = std::fs::write(pidfile, pid.to_string()) {
        // The daemon is already running; a pidfile write failure must not
        // report a spawn failure (RFC-047: never hard-fail the boot path).
        tracing::warn!(error = %e, path = %pidfile.display(), "could not write spawn pidfile");
    }
    // Intentionally leak: see doc comment above. We do NOT call
    // child.wait() and we do NOT enable kill_on_drop; the daemon must
    // outlive oxios.
    std::mem::forget(child);
    tracing::info!(pid, binary = %binary.display(), "spawned oxibrain daemon (detached)");
    Ok(pid)
}

/// Poll `probe(socket)` every 250ms until it returns `true` or the
/// deadline elapses. Returns the probe's final value.
///
/// `probe` is responsible for any transport-level check (Unix socket
/// connect, HTTP health endpoint, etc.) and MUST be cheap — the caller
/// is expected to fail fast inside `probe` (e.g. timeout=0 connect)
/// because this loop will run it up to `timeout/250ms + 1` times.
pub(crate) async fn wait_ready<F>(socket: &Path, timeout: Duration, probe: &F) -> bool
where
    F: Fn(&Path) -> BoxFuture<'static, bool>,
{
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if probe(socket).await {
            return true;
        }
        let now = std::time::Instant::now();
        if now >= deadline {
            return false;
        }
        let remaining = deadline.saturating_duration_since(now);
        let sleep = std::cmp::min(remaining, Duration::from_millis(250));
        tokio::time::sleep(sleep).await;
    }
}
// ============================================================================
// Task 5: BrainSupervisor orchestration
// ============================================================================

/// Send `sig` to `pid` via libc::kill. Unix-only.
#[cfg(unix)]
fn kill(pid: i32, sig: i32) -> i32 {
    // SAFETY: `libc::kill` is async-signal-safe; arguments are POD.
    unsafe { libc::kill(pid, sig) }
}

/// Resolved paths and feature flags for [`BrainSupervisor`].
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    /// When `false`, [`BrainSupervisor::ensure`] is a no-op and the
    /// supervisor reports [`SupervisorState::Disabled`] — the explicit
    /// "leave the daemon alone" opt-out.
    pub auto_manage: bool,
    /// Unix-domain socket the daemon listens on; defaults to the Oxi
    /// Foundation `~/.oxi/brain/oxibrain.sock` when not explicitly set.
    pub socket_path: PathBuf,
    /// Explicit daemon binary. When `Some(_)` and present on disk, the
    /// download step is skipped.
    pub binary_path: Option<PathBuf>,
    /// Where the installer drops the binary (`~/.oxi/bin`).
    pub install_root: PathBuf,
    /// Combined stdout/stderr log (`~/.oxi/brain/daemon.log`).
    pub log_path: PathBuf,
    /// PID file written by [`spawn_detached`] for our spawn-managed
    /// daemon — `stop()` reads this to know which pid to signal.
    pub pidfile: PathBuf,
    /// `~/Library/LaunchAgents/<LAUNCHD_LABEL>.plist`.
    pub plist_path: PathBuf,
}

impl SupervisorConfig {
    /// Build a [`SupervisorConfig`] from a parsed `[brain]` section.
    /// `home` is the user's home directory — the Oxi Foundation layout
    /// hangs off `~/.oxi/`.
    pub fn from_brain_section(home: &Path, section: &BrainSection) -> Self {
        let brain_dir = home.join(".oxi").join("brain");
        Self {
            auto_manage: section.auto_manage,
            socket_path: super::config::resolved_socket_path(home, &section.socket_path),
            binary_path: (!section.binary_path.is_empty())
                .then(|| PathBuf::from(&section.binary_path)),
            install_root: home.join(".oxi").join("bin"),
            log_path: brain_dir.join("daemon.log"),
            pidfile: brain_dir.join("oxibrain.spawn.pid"),
            plist_path: home
                .join("Library")
                .join("LaunchAgents")
                .join(format!("{LAUNCHD_LABEL}.plist")),
        }
    }
}

/// Type alias for the pluggable readiness probe. The supervisor calls
/// `probe(socket)` and treats a `true` return as "daemon is up".
pub(crate) type ProbeFn = Arc<dyn Fn(&Path) -> BoxFuture<'static, bool> + Send + Sync>;

/// Default probe: connect to the daemon via oxibrain-client and issue a
/// single `ping()`. Returns `false` for any error.
async fn client_probe(socket: &Path) -> bool {
    let path = socket.to_path_buf();
    match oxibrain_client::BrainClient::connect(path).await {
        Ok(mut c) => c.ping().await.is_ok(),
        Err(_) => false,
    }
}

/// Long-lived supervisor for the oxibrain daemon.
///
/// The supervisor owns a [`SupervisorStatus`] snapshot (cached for fast
/// reads from the web API) and orchestrates the install + launchd/spawn
/// dance on `ensure()`. All public methods are safe to call repeatedly
/// — `ensure()` is idempotent and `respawn_if_needed()` is rate-limited
/// to one attempt / 30 s so a flapping daemon cannot spin the supervisor.
pub struct BrainSupervisor {
    cfg: SupervisorConfig,
    installer: Arc<dyn Installer>,
    probe: ProbeFn,
    status: RwLock<SupervisorStatus>,
    last_respawn: Mutex<Option<Instant>>,
}

impl BrainSupervisor {
    /// Build a supervisor with the production dependencies
    /// ([`GithubInstaller`] + `oxibrain-client` connect+ping probe).
    pub fn new(cfg: SupervisorConfig) -> Self {
        Self::with_deps(
            cfg,
            Arc::new(GithubInstaller::default()),
            Arc::new(|socket: &Path| {
                let socket = socket.to_path_buf();
                Box::pin(async move { client_probe(&socket).await })
            }),
        )
    }

    /// Build a supervisor with injected dependencies (test fake).
    pub(crate) fn with_deps(
        cfg: SupervisorConfig,
        installer: Arc<dyn Installer>,
        probe: ProbeFn,
    ) -> Self {
        Self {
            cfg,
            installer,
            probe,
            status: RwLock::new(SupervisorStatus::default()),
            last_respawn: Mutex::new(None),
        }
    }

    /// Update the cached status + the `oxibrain_available` gauge.
    ///
    /// Every state transition flows through this helper so the metric
    /// and the cached snapshot never drift. `managed_by` and `err` are
    /// optional — `None` leaves that field untouched.
    fn set(&self, state: SupervisorState, managed_by: Option<ManagedBy>, err: Option<String>) {
        let mut st = self
            .status
            .write()
            .expect("supervisor status lock poisoned");
        st.state = state;
        if let Some(m) = managed_by {
            st.managed_by = m;
        }
        st.last_error = err;
        get_metrics()
            .oxibrain_available
            .set(if state == SupervisorState::Online {
                1.0
            } else {
                0.0
            });
    }

    /// Idempotent: probe → install (if no binary) → launchd/spawn →
    /// wait ready. Never returns `Err` — every failure path lands in
    /// `status.last_error` with state [`SupervisorState::Failed`] and
    /// the supervisor remains callable (RFC-047 degradation contract).
    pub async fn ensure(&self) -> SupervisorStatus {
        if !self.cfg.auto_manage {
            self.set(SupervisorState::Disabled, None, None);
            return self.status();
        }
        // Honor a daemon that someone else already owns.
        if (self.probe)(&self.cfg.socket_path).await {
            self.set(SupervisorState::Online, Some(ManagedBy::External), None);
            return self.status();
        }
        // Make sure a binary is on disk.
        let binary = match self.locate_binary() {
            Some(b) => b,
            None => {
                self.set(SupervisorState::Installing, None, None);
                match self.installer.install(&self.cfg.install_root).await {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::warn!(error = %e, "oxibrain install failed — degraded");
                        self.set(SupervisorState::Failed, None, Some(e.to_string()));
                        return self.status();
                    }
                }
            }
        };
        // Read `--version` and tag the install so the UI can show "what
        // version is on disk right now". Empty output → "<unknown>".
        let version_out = std::process::Command::new(&binary)
            .arg("--version")
            .output()
            .ok();
        let version_str = version_out
            .as_ref()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let version_tag = if version_str.trim().is_empty() {
            "<unknown>".to_string()
        } else {
            version_str
        };
        match self.launch_and_wait(&binary).await {
            Ok(()) => {
                {
                    let mut st = self
                        .status
                        .write()
                        .expect("supervisor status lock poisoned");
                    st.installed_version = Some(version_tag);
                }
                self.status()
            }
            Err(e) => {
                self.set(SupervisorState::Failed, None, Some(e.to_string()));
                self.status()
            }
        }
    }

    /// Locate an existing daemon binary. Explicit config wins, then the
    /// managed install root, then PATH via `which`.
    fn locate_binary(&self) -> Option<PathBuf> {
        if let Some(explicit) = &self.cfg.binary_path
            && explicit.is_file()
        {
            return Some(explicit.clone());
        }
        let managed = self.cfg.install_root.join("oxibrain");
        if managed.is_file() {
            return Some(managed);
        }
        which::which("oxibrain").ok()
    }

    /// launchd (preferred) → detached-spawn fallback → wait for the
    /// probe to flip. `Ok(())` only when the daemon actually answers.
    async fn launch_and_wait(&self, binary: &Path) -> Result<()> {
        self.set(SupervisorState::Starting, None, None);
        // ensure_launchd signals "not applicable" via `Ok(false)` (e.g. no
        // $HOME, opt-out env, non-macOS). Any `Err` is a hard failure —
        // a plist write rejected by the FS, launchctl bootstrap unable
        // to find the binary, etc. — and the contract is "spawn_detached
        // fallback". Treat the hard error as "launchd unavailable" and
        // fall through rather than tearing the supervisor down.
        let by_launchd = match ensure_launchd(binary, &self.cfg.log_path) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "launchd keeper unavailable — falling back to detached spawn"
                );
                false
            }
        };
        if !by_launchd {
            spawn_detached(binary, &self.cfg.log_path, &self.cfg.pidfile)?;
        }
        if wait_ready(
            &self.cfg.socket_path,
            Duration::from_secs(30),
            &|s: &Path| {
                let p = self.probe.clone();
                let s = s.to_path_buf();
                Box::pin(async move { (p)(&s).await })
            },
        )
        .await
        {
            let by = if by_launchd {
                ManagedBy::Launchd
            } else {
                ManagedBy::Spawn
            };
            self.set(SupervisorState::Online, Some(by), None);
            Ok(())
        } else {
            anyhow::bail!(
                "daemon socket not ready within 30s at {}",
                self.cfg.socket_path.display()
            );
        }
    }

    /// Lazy respawn hook for `BrainConnection`'s reconnect-failure path.
    /// Returns the probe's final value (`true` once the daemon is alive
    /// again, `false` otherwise). Rate-limited to one attempt / 30 s.
    pub async fn respawn_if_needed(&self) -> bool {
        if (self.probe)(&self.cfg.socket_path).await {
            self.set(SupervisorState::Online, None, None);
            return true;
        }
        {
            let mut last = self.last_respawn.lock().expect("respawn lock poisoned");
            if last
                .map(|t| t.elapsed() < Duration::from_secs(30))
                .unwrap_or(false)
            {
                return false; // rate-limited; launchd KeepAlive usually wins
            }
            *last = Some(Instant::now());
        }
        if !self.cfg.auto_manage {
            return false;
        }
        if let Some(binary) = self.locate_binary() {
            let _ = self.launch_and_wait(&binary).await;
        }
        (self.probe)(&self.cfg.socket_path).await
    }

    /// CLI helper: force a fresh install (download regardless of presence).
    pub async fn install(&self) -> Result<PathBuf> {
        self.installer.install(&self.cfg.install_root).await
    }

    /// CLI helper: start with an existing binary (no install).
    pub async fn start(&self) -> Result<()> {
        let binary = self
            .locate_binary()
            .with_context(|| "oxibrain binary not found — run `oxios brain install` first")?;
        self.launch_and_wait(&binary).await
    }

    /// CLI helper: bootout launchd + kill the spawn-managed pid. We
    /// never touch a daemon we didn't start — if the socket answers,
    /// `ensure()` reports `managed_by = external` and `stop()` is a no-op
    /// against that daemon.
    pub async fn stop(&self) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let uid = nix_uid();
            let _ = std::process::Command::new("launchctl")
                .args(["bootout", &format!("gui/{uid}/{LAUNCHD_LABEL}")])
                .output();
        }
        if let Ok(pid_text) = std::fs::read_to_string(&self.cfg.pidfile)
            && let Ok(pid) = pid_text.trim().parse::<i32>()
        {
            #[cfg(unix)]
            {
                let _ = kill(pid, 15);
            }
            let _ = std::fs::remove_file(&self.cfg.pidfile);
        }
        self.set(SupervisorState::NotInstalled, Some(ManagedBy::None), None);
        Ok(())
    }

    /// CLI helper: `stop()` + remove the plist (binary + data stay).
    pub async fn uninstall(&self) -> Result<()> {
        self.stop().await?;
        let _ = std::fs::remove_file(&self.cfg.plist_path);
        Ok(())
    }

    /// Cached status snapshot (cheap clone — for the web API hot path).
    pub fn status(&self) -> SupervisorStatus {
        self.status
            .read()
            .expect("supervisor status lock poisoned")
            .clone()
    }

    /// CLI helper: fresh look at the world — `<binary> --version`,
    /// launchd `print`, and a current probe — without touching the
    /// cached `status` field on disk.
    pub async fn refresh_status_from_fs(&self) -> SupervisorStatus {
        let mut st = self
            .status
            .read()
            .expect("supervisor status lock poisoned")
            .clone();
        if let Some(bin) = self.locate_binary()
            && let Ok(out) = std::process::Command::new(&bin).arg("--version").output()
        {
            let raw = String::from_utf8_lossy(&out.stdout).into_owned();
            st.installed_version = Some(if raw.trim().is_empty() {
                "<unknown>".to_string()
            } else {
                raw
            });
        }
        #[cfg(target_os = "macos")]
        {
            let uid = nix_uid();
            let loaded = std::process::Command::new("launchctl")
                .args(["print", &format!("gui/{uid}/{LAUNCHD_LABEL}")])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if loaded && st.managed_by == ManagedBy::None {
                st.managed_by = ManagedBy::Launchd;
            }
        }
        if (self.probe)(&self.cfg.socket_path).await {
            st.state = SupervisorState::Online;
            get_metrics().oxibrain_available.set(1.0);
        } else if st.state == SupervisorState::Online {
            st.state = SupervisorState::NotInstalled;
            get_metrics().oxibrain_available.set(0.0);
        }
        st
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[test]
    fn plist_contains_label_and_daemon_flags() {
        let xml = build_plist(
            Path::new("/Users/x/.oxi/bin/oxibrain"),
            Path::new("/Users/x/.oxi/brain/daemon.log"),
        );
        assert!(xml.contains("<string>com.oxi.oxibrain</string>"));
        assert!(xml.contains("<string>serve</string>"));
        assert!(xml.contains("<string>--daemon</string>"));
        assert!(xml.contains("<key>KeepAlive</key>"));
        assert!(xml.contains("<true/>")); // RunAtLoad
        let evil = build_plist(Path::new("/tmp/a<b>&c"), Path::new("/tmp/l"));
        assert!(evil.contains("&lt;b&gt;&amp;c"));
    }

    #[test]
    fn asset_urls_picks_tarball_and_checksum() {
        let release = serde_json::json!({
            "assets": [
                { "name": "oxibrain-aarch64-apple-darwin.tar.gz",
                  "browser_download_url": "https://x/t.tar.gz" },
                { "name": "oxibrain-aarch64-apple-darwin.tar.gz.sha256",
                  "browser_download_url": "https://x/t.sha256" },
                { "name": "Source code.zip", "browser_download_url": "https://x/src" }
            ]
        });
        let (tar, sha) = asset_urls(&release).expect("urls");
        assert_eq!(tar, "https://x/t.tar.gz");
        assert_eq!(sha, "https://x/t.sha256");
        assert!(asset_urls(&serde_json::json!({ "assets": [] })).is_none());
    }

    #[test]
    fn sha256_verify_matches_and_rejects() {
        let bytes = b"hello world";
        let digest = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        assert!(verify_sha256(bytes, digest));
        assert!(!verify_sha256(bytes, "deadbeef"));
        assert!(verify_sha256(bytes, &format!("{digest}  t.tar.gz")));
        assert!(!verify_sha256(bytes, "zz"));
    }

    #[test]
    fn extract_single_binary_reads_oxibrain_entry() {
        let mut buf = Vec::new();
        let data = b"#!/bin/sh\ntrue\n".to_vec();
        {
            let enc = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
            let mut ar = tar::Builder::new(enc);
            let mut hdr = tar::Header::new_gnu();
            hdr.set_size(data.len() as u64);
            hdr.set_mode(0o755);
            hdr.set_cksum();
            ar.append_data(&mut hdr, "oxibrain", data.as_slice())
                .unwrap();
            ar.into_inner().unwrap().finish().unwrap();
        }
        assert_eq!(extract_single_binary(&buf).unwrap(), data);
    }

    /// Run the per-request loop on a pre-bound listener. The release JSON
    /// is supplied by the caller so its `browser_download_url` values can
    /// target this listener's address.
    ///
    /// Each connection gets exactly one response body — the downloader
    /// must not retry any asset. Three connections are expected:
    /// release JSON, tarball, sha sidecar.
    async fn serve_release_on(
        listener: tokio::net::TcpListener,
        tar: Vec<u8>,
        release_json: String,
    ) {
        let sha_line = {
            let digest = Sha256::digest(&tar);
            format!("{}  {ASSET_TAR}\n", hex::encode(digest))
        };
        for _ in 0..3 {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            let body = if request.starts_with("GET /releases/latest") {
                release_json.as_bytes().to_vec()
            } else if request.starts_with(&format!("GET /{ASSET_TAR}.sha256 ")) {
                sha_line.as_bytes().to_vec()
            } else if request.starts_with(&format!("GET /{ASSET_TAR} ")) {
                tar.clone()
            } else {
                b"not found".to_vec()
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(&body).await.unwrap();
        }
    }

    fn fixture_tar() -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let enc = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
            let mut ar = tar::Builder::new(enc);
            let mut hdr = tar::Header::new_gnu();
            let data = b"#!/bin/sh\nsleep 60\n".to_vec();
            hdr.set_size(data.len() as u64);
            hdr.set_mode(0o755);
            hdr.set_cksum();
            ar.append_data(&mut hdr, "oxibrain", data.as_slice())
                .unwrap();
            ar.into_inner().unwrap().finish().unwrap();
        }
        buf
    }

    #[tokio::test]
    async fn github_installer_downloads_verifies_installs_atomically() {
        // Bind a listener up front so the JSON asset URLs can point at the
        // fixture's real port. Then hand the bound listener to the fixture.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let tar = fixture_tar();
        let release_json = serde_json::json!({
            "tag_name": "v1.2.3",
            "assets": [
                { "name": ASSET_TAR,
                  "browser_download_url": format!("{base}/{ASSET_TAR}") },
                { "name": format!("{ASSET_TAR}.sha256"),
                  "browser_download_url": format!("{base}/{ASSET_TAR}.sha256") }
            ]
        })
        .to_string();
        let _srv = tokio::spawn(async move { serve_release_on(listener, tar, release_json).await });
        let dir = tempfile::tempdir().unwrap();
        let inst = GithubInstaller {
            releases_url: format!("{base}/releases/latest"),
        };
        let bin = inst.install(dir.path()).await.unwrap();
        assert_eq!(bin, dir.path().join("oxibrain"));
        assert_eq!(std::fs::read(&bin).unwrap(), b"#!/bin/sh\nsleep 60\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&bin).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
    }
    // -------- Task 4: keeper tests (launchd / spawn / wait_ready) --------

    #[tokio::test]
    async fn spawn_detached_runs_script_and_writes_pidfile() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("oxibrain");
        std::fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let log = dir.path().join("daemon.log");
        let pidfile = dir.path().join("oxibrain.spawn.pid");
        let pid = spawn_detached(&script, &log, &pidfile).unwrap();
        assert!(pid > 0);
        assert_eq!(
            std::fs::read_to_string(&pidfile).unwrap().trim(),
            pid.to_string()
        );
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let alive = kill(pid as i32, 0) == 0;
        assert!(alive, "stub daemon should still be running");
        kill(pid as i32, 15); // cleanup
        let _ = std::fs::remove_file(&pidfile);
    }

    #[cfg(unix)]
    fn kill(pid: i32, sig: i32) -> i32 {
        unsafe { libc::kill(pid, sig) }
    }

    #[tokio::test]
    async fn wait_ready_times_out_on_dead_socket() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("missing.sock");
        let started = std::time::Instant::now();
        let ok = wait_ready(&sock, Duration::from_millis(400), &|_p| {
            Box::pin(async { false })
        })
        .await;
        assert!(!ok);
        assert!(started.elapsed() >= Duration::from_millis(350));
    }

    #[tokio::test]
    async fn wait_ready_returns_when_probe_flips() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("s.sock");
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let f = flag.clone();
        let probe = move |_p: &Path| -> futures::future::BoxFuture<'static, bool> {
            let f = f.clone();
            Box::pin(async move { f.load(std::sync::atomic::Ordering::Relaxed) })
        };
        let ok = tokio::join!(wait_ready(&sock, Duration::from_secs(5), &probe), async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        })
        .0;
        assert!(ok);
    }
    // -------- Task 5: BrainSupervisor orchestration (full-flow) --------

    /// Test-only installer: copies `<root>/source-oxibrain` into
    /// `<install_root>/oxibrain` without touching the network. The source
    /// is owned by the test (not created here) so each test can pick a
    /// stub script that satisfies its own scenario (sleep, --version, etc).
    struct FakeInstaller {
        root: std::path::PathBuf,
    }
    impl Installer for FakeInstaller {
        fn install(&self, install_root: &Path) -> BoxFuture<'static, Result<PathBuf>> {
            let root = install_root.to_path_buf();
            let script = self.root.join("source-oxibrain");
            Box::pin(async move {
                std::fs::create_dir_all(&root)?;
                std::fs::copy(&script, root.join("oxibrain"))?;
                Ok(root.join("oxibrain"))
            })
        }
    }

    /// Full ensure() flow with launchd skipped: install (fake) →
    /// spawn_detached → wait_ready (probe flips once the pidfile exists)
    /// → Online/Spawn. `installed_version` is set to the "<unknown>"
    /// marker because the stub script prints nothing on `--version`.
    #[tokio::test]
    async fn ensure_installs_spawns_and_reaches_online() {
        // Empty PATH so `locate_binary()` doesn't find the host's real
        // `~/.cargo/bin/oxibrain`; otherwise the FakeInstaller is never
        // exercised and we spawn the real binary against a tempdir
        // log_path that gets deleted post-test.
        let saved_path = std::env::var_os("PATH");
        // SAFETY: tokio test body hasn't spawned anything that races on
        // $PATH or `OXIOS_BRAIN_NO_LAUNCHD` at this point.
        unsafe {
            std::env::set_var("PATH", "");
            std::env::set_var("OXIOS_BRAIN_NO_LAUNCHD", "1");
        }
        let home = tempfile::tempdir().unwrap();
        // Stub script: answer `--version` (empty stdout → "<unknown>"
        // marker) then sleep so the spawn stays alive long enough for
        // the pidfile probe to flip. The brief's `sleep 30` alone would
        // make every ensure() run block 30 s on `<binary> --version`
        // before wait_ready even gets a chance.
        let src = home.path().join("source-oxibrain");
        std::fs::write(
            &src,
            "#!/bin/sh\ncase \"$1\" in --version) exit 0;; esac\nsleep 30\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&src, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let cfg = SupervisorConfig {
            auto_manage: true,
            socket_path: home.path().join("brain.sock"),
            binary_path: None,
            install_root: home.path().join("bin"),
            log_path: home.path().join("daemon.log"),
            pidfile: home.path().join("spawn.pid"),
            plist_path: home.path().join("agent.plist"),
        };
        let pidfile = cfg.pidfile.clone();
        let sup = BrainSupervisor::with_deps(
            cfg,
            Arc::new(FakeInstaller {
                root: home.path().to_path_buf(),
            }),
            Arc::new(move |_socket| {
                let pidfile = pidfile.clone();
                Box::pin(async move { pidfile.exists() })
            }),
        );
        let st = sup.ensure().await;
        // Cleanup before asserts so a panic leaves no zombie process.
        if let Ok(pid) = std::fs::read_to_string(home.path().join("spawn.pid")) {
            #[cfg(unix)]
            kill(pid.trim().parse().unwrap(), 15);
        }
        // Restore env before asserting so a panic leaves the env clean.
        // SAFETY: same as above.
        unsafe {
            std::env::remove_var("OXIOS_BRAIN_NO_LAUNCHD");
            match saved_path {
                Some(p) => std::env::set_var("PATH", p),
                None => std::env::remove_var("PATH"),
            }
        }
        assert_eq!(st.state, SupervisorState::Online);
        assert_eq!(st.managed_by, ManagedBy::Spawn);
        assert!(st.installed_version.is_some());
    }

    /// If the probe already answers true (daemon reachable), ensure() must
    /// short-circuit to Online/External without installing or killing
    /// anything — oxibrain is a shared system service.
    #[tokio::test]
    async fn ensure_respects_live_external_daemon() {
        let home = tempfile::tempdir().unwrap();
        let cfg = SupervisorConfig {
            auto_manage: true,
            socket_path: home.path().join("brain.sock"),
            binary_path: None,
            install_root: home.path().join("bin"),
            log_path: home.path().join("daemon.log"),
            pidfile: home.path().join("spawn.pid"),
            plist_path: home.path().join("agent.plist"),
        };
        let sup = BrainSupervisor::with_deps(
            cfg,
            Arc::new(FakeInstaller {
                root: home.path().to_path_buf(),
            }),
            Arc::new(|_s| Box::pin(async { true })),
        );
        let st = sup.ensure().await;
        assert_eq!(st.state, SupervisorState::Online);
        assert_eq!(st.managed_by, ManagedBy::External);
    }

    /// `auto_manage = false` is the explicit user opt-out — ensure() must
    /// never touch the daemon (no probe, no install, no kill).
    #[tokio::test]
    async fn ensure_disabled_when_auto_manage_off() {
        let home = tempfile::tempdir().unwrap();
        let cfg = SupervisorConfig {
            auto_manage: false,
            socket_path: home.path().join("brain.sock"),
            binary_path: None,
            install_root: home.path().join("bin"),
            log_path: home.path().join("daemon.log"),
            pidfile: home.path().join("spawn.pid"),
            plist_path: home.path().join("agent.plist"),
        };
        let sup = BrainSupervisor::with_deps(
            cfg,
            Arc::new(FakeInstaller {
                root: home.path().to_path_buf(),
            }),
            Arc::new(|_s| Box::pin(async { false })),
        );
        assert_eq!(sup.ensure().await.state, SupervisorState::Disabled);
    }

    /// The first respawn call attempts to recover; an immediate second call
    /// is rate-limited to one attempt / 30 s. We use `BrainSupervisor::new`
    /// (real GithubInstaller + probe) but with no binary on disk, so the
    /// attempt fails silently — only the rate-limit matters here.
    #[tokio::test]
    async fn respawn_is_rate_limited() {
        // `locate_binary()` falls through to `which::which("oxibrain")`,
        // which on this dev host resolves to a real install at
        // `~/.cargo/bin/oxibrain`. If we let it find that, the test
        // would call `ensure_launchd` against our real plist and rewrite
        // it with this tempdir's `log_path`. Empty PATH isolates the
        // test from any host-installed binary.
        let saved_path = std::env::var_os("PATH");
        // SAFETY: tokio test body hasn't spawned anything that races on
        // $PATH at this point; the test runs single-threaded.
        unsafe {
            std::env::set_var("PATH", "");
        }
        let home = tempfile::tempdir().unwrap();
        let cfg = SupervisorConfig {
            auto_manage: true,
            socket_path: home.path().join("brain.sock"),
            binary_path: None,
            install_root: home.path().join("bin"),
            log_path: home.path().join("daemon.log"),
            pidfile: home.path().join("spawn.pid"),
            plist_path: home.path().join("agent.plist"),
        };
        let sup = BrainSupervisor::new(cfg);
        let _ = sup.respawn_if_needed().await;
        let second = sup.respawn_if_needed().await;
        // Restore PATH before asserting so a panic leaves the env clean.
        // SAFETY: same as above.
        unsafe {
            match saved_path {
                Some(p) => std::env::set_var("PATH", p),
                None => std::env::remove_var("PATH"),
            }
        }
        assert!(!second);
    }
}
