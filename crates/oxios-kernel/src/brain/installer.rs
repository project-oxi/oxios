//! Brain binary installer — the surviving half of the RFC-049 supervisor.
//!
//! oxibrain has no daemon, so there is nothing to keep alive: this module
//! only puts the `oxibrain` binary on disk (GitHub Releases, sha256-verified)
//! and cleans up daemon-era leftovers once.
//!
//! The managed layout follows the ecosystem binary standard — each app owns
//! `~/.oxi/<app>/bin/<app>` (launcher symlink → `../versions/<v>/<app>`),
//! mirroring oxios's own managed channel (`src/managed_install.rs`). The
//! pre-standard shared location `~/.oxi/bin/oxibrain` still resolves as a
//! fallback and is retired by the next successful install.

use anyhow::{Context, Result};
use futures::future::BoxFuture;
use reqwest::Client;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::BrainSection;

/// launchd label used by the retired daemon; kept only for cleanup.
pub const LEGACY_LAUNCHD_LABEL: &str = "com.oxi.oxibrain";

/// GitHub release JSON endpoint for the latest oxibrain build.
pub const RELEASES_LATEST_URL: &str =
    "https://api.github.com/repos/a7garden/oxibrain/releases/latest";

/// Tarball asset name for aarch64-apple-darwin — sole first-party target.
pub const ASSET_TAR: &str = "oxibrain-aarch64-apple-darwin.tar.gz";

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

/// Release tag → sanitized version directory name (`v1.2.3` → `1.2.3`).
///
/// Rejects empty tags, path separators, and `.`/`..` so a hostile release
/// payload cannot escape the versions directory.
pub(crate) fn version_dir_name(tag: &str) -> Result<String> {
    let v = tag.trim().trim_start_matches('v');
    anyhow::ensure!(
        !v.is_empty() && v != "." && v != "..",
        "unusable release tag {tag:?}"
    );
    anyhow::ensure!(
        !v.contains('/') && !v.contains('\\') && !v.contains('\0'),
        "release tag {tag:?} contains path separators"
    );
    Ok(v.to_owned())
}

/// `tag_name` out of a GitHub release payload, sanitized for a version dir.
pub(crate) fn release_version(release: &serde_json::Value) -> Result<String> {
    version_dir_name(release["tag_name"].as_str().unwrap_or_default())
}

/// Strict SemVer triple for version-dir names — no leading `v`, no
/// pre-release/build metadata (mirrors oxios `managed_install`).
fn parse_version_dir(name: &str) -> Option<(u64, u64, u64)> {
    let mut parts = name.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((major, minor, patch))
}

/// Place a verified, extracted binary at `<versions_dir>/<version>/oxibrain`
/// (temp file + rename, so a partial extract never clobbers a version) and
/// atomically flip the `launcher` symlink at it. Shared by the GitHub
/// pipeline and the layout tests.
pub(crate) fn place_binary(
    versions_dir: &Path,
    launcher: &Path,
    version: &str,
    binary: &[u8],
) -> Result<()> {
    let version_dir = versions_dir.join(version);
    std::fs::create_dir_all(&version_dir)
        .with_context(|| format!("create version dir {}", version_dir.display()))?;
    let target = version_dir.join("oxibrain");
    let tmp = version_dir.join(".oxibrain.tmp");
    std::fs::write(&tmp, binary)
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
    flip_launcher(launcher, version)
}

/// Atomically point `launcher` at `../versions/<version>/oxibrain` — build
/// a temp symlink, then `rename` it over the real launcher, so every
/// `execve` observes either the old or the new target, never a missing
/// file.
pub(crate) fn flip_launcher(launcher: &Path, version: &str) -> Result<()> {
    let bin_dir = launcher
        .parent()
        .context("launcher path has no parent directory")?;
    std::fs::create_dir_all(bin_dir)
        .with_context(|| format!("create bin dir {}", bin_dir.display()))?;
    let tmp_link = bin_dir.join(".oxibrain.link.tmp");
    let _ = std::fs::remove_file(&tmp_link);
    #[cfg(unix)]
    {
        let rel_target = PathBuf::from("../versions").join(version).join("oxibrain");
        std::os::unix::fs::symlink(&rel_target, &tmp_link).with_context(|| {
            format!("symlink {} → {}", tmp_link.display(), rel_target.display())
        })?;
    }
    #[cfg(not(unix))]
    {
        // No symlinks: fall back to an atomic copy of the version binary.
        let source = launcher
            .parent()
            .and_then(|bin| bin.parent())
            .context("launcher is not two levels below the app root")?
            .join("versions")
            .join(version)
            .join("oxibrain");
        std::fs::copy(&source, &tmp_link)
            .with_context(|| format!("copy {} → {}", source.display(), tmp_link.display()))?;
    }
    std::fs::rename(&tmp_link, launcher).with_context(|| {
        format!(
            "atomic launcher flip {} → {}",
            tmp_link.display(),
            launcher.display()
        )
    })?;
    Ok(())
}

/// Delete version dirs beyond the newest `keep`, never touching `current`.
/// Non-SemVer directories are left alone — not ours to judge.
pub(crate) fn prune_versions(
    versions_dir: &Path,
    current: &str,
    keep: usize,
) -> Result<Vec<String>> {
    let mut installed: Vec<((u64, u64, u64), String)> = Vec::new();
    let entries = std::fs::read_dir(versions_dir)
        .with_context(|| format!("read_dir {}", versions_dir.display()))?;
    for entry in entries {
        let entry =
            entry.with_context(|| format!("read_dir entry in {}", versions_dir.display()))?;
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == current {
            continue;
        }
        if let Some(v) = parse_version_dir(&name) {
            installed.push((v, name));
        }
    }
    installed.sort();
    let mut removed = Vec::new();
    while installed.len() >= keep {
        let (_, name) = installed.remove(0);
        std::fs::remove_dir_all(versions_dir.join(&name))
            .with_context(|| format!("prune version dir {}", name))?;
        removed.push(name);
    }
    Ok(removed)
}

/// Install the oxibrain release: place the binary under `versions_dir` and
/// flip the `launcher` symlink.
///
/// Implementations are responsible for the full pipeline: locate the
/// artifact, verify its integrity, place an executable at
/// `<versions_dir>/<version>/oxibrain`, and atomically point `launcher` at
/// it. The trait is `async`-aware (returns a boxed future) so callers can
/// plug in either a real GitHub fetcher or a fake without monomorphising
/// its call sites.
pub(crate) trait Installer: Send + Sync {
    /// Download + verify + extract + install into the managed layout.
    /// Returns the absolute path of the installed launcher.
    fn install(&self, versions_dir: &Path, launcher: &Path) -> BoxFuture<'static, Result<PathBuf>>;
}

/// Default [`Installer`]: fetch the latest GitHub release for oxibrain.
pub(crate) struct GithubInstaller {
    /// `releases/latest` JSON endpoint. Set once at construction time and
    /// read by [`Installer::install`]; defaults to [`RELEASES_LATEST_URL`]
    /// (the public constant) — not exposed for runtime override.
    releases_url: String,
}

impl Default for GithubInstaller {
    fn default() -> Self {
        Self {
            releases_url: RELEASES_LATEST_URL.to_string(),
        }
    }
}

impl Installer for GithubInstaller {
    fn install(&self, versions_dir: &Path, launcher: &Path) -> BoxFuture<'static, Result<PathBuf>> {
        let url = self.releases_url.clone();
        let versions_dir = versions_dir.to_path_buf();
        let launcher = launcher.to_path_buf();
        Box::pin(async move {
            std::fs::create_dir_all(&versions_dir)
                .with_context(|| format!("create versions dir {}", versions_dir.display()))?;
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
            let version = release_version(&release)
                .with_context(|| format!("release at {url} carries no usable tag_name"))?;
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
            place_binary(&versions_dir, &launcher, &version, &binary)?;
            prune_versions(&versions_dir, &version, 2)?;
            tracing::info!(
                launcher = %launcher.display(),
                version,
                "installed oxibrain release"
            );
            Ok(launcher)
        })
    }
}

#[derive(Debug)]
pub struct BrainInstaller {
    binary_path: Option<PathBuf>,
    /// `~/.oxi/oxibrain/bin` — launcher directory of the managed layout.
    bin_dir: PathBuf,
    /// `~/.oxi/oxibrain/versions` — per-release binaries.
    versions_dir: PathBuf,
    /// Pre-standard managed location `~/.oxi/bin/oxibrain`: still resolved
    /// by [`locate_binary`](Self::locate_binary), retired by the next
    /// successful [`install`](Self::install).
    legacy_binary: PathBuf,
    plist_path: PathBuf,
    /// Memoized `<binary> --version` probe. Outer `None` = dirty (must probe);
    /// inner `Some(s)` = cached `Ok(Some(s))`; inner `None` = cached
    /// `Ok(None)` (probe returned nothing). Invalidated by `install()` and
    /// `remove_managed()` (RwLock so the hot read path stays lock-free).
    version_cache: std::sync::RwLock<Option<Option<String>>>,
}

impl BrainInstaller {
    /// Build from the `[brain]` section. `home` anchors the `~/.oxi` layout.
    pub fn from_brain_section(home: &Path, section: &BrainSection) -> Self {
        let oxi = home.join(".oxi");
        Self {
            binary_path: (!section.binary_path.is_empty())
                .then(|| PathBuf::from(&section.binary_path)),
            bin_dir: oxi.join("oxibrain").join("bin"),
            versions_dir: oxi.join("oxibrain").join("versions"),
            legacy_binary: oxi.join("bin").join("oxibrain"),
            plist_path: home
                .join("Library")
                .join("LaunchAgents")
                .join(format!("{LEGACY_LAUNCHD_LABEL}.plist")),
            version_cache: std::sync::RwLock::new(None),
        }
    }

    /// Explicit config wins, then the managed launcher, then the
    /// pre-standard shared location, then `PATH`.
    pub fn locate_binary(&self) -> Option<PathBuf> {
        if let Some(explicit) = &self.binary_path
            && explicit.is_file()
        {
            return Some(explicit.clone());
        }
        let managed = self.bin_dir.join("oxibrain");
        if managed.is_file() {
            return Some(managed);
        }
        if self.legacy_binary.is_file() {
            return Some(self.legacy_binary.clone());
        }
        which::which("oxibrain").ok()
    }

    /// Force a fresh download regardless of presence. Installs into the
    /// managed versions layout, flips the launcher, prunes old versions,
    /// and retires the pre-standard `~/.oxi/bin/oxibrain`.
    pub async fn install(&self) -> Result<PathBuf> {
        // Binary on disk is about to be replaced — drop the cached
        // version probe so the next `version_of_cached` re-reads.
        *self.version_cache.write().expect("version_cache poisoned") = None;
        let installer = GithubInstaller::default();
        let launcher = installer
            .install(&self.versions_dir, &self.bin_dir.join("oxibrain"))
            .await?;
        self.retire_legacy();
        Ok(launcher)
    }

    /// Remove the pre-standard `~/.oxi/bin/oxibrain` once the managed
    /// layout serves the binary. Best-effort: the legacy directory is
    /// dropped too when this leaves it empty.
    fn retire_legacy(&self) {
        if self.legacy_binary.exists() {
            match std::fs::remove_file(&self.legacy_binary) {
                Ok(()) => tracing::info!(
                    path = %self.legacy_binary.display(),
                    "retired pre-standard oxibrain binary"
                ),
                Err(e) => tracing::warn!(
                    error = %e,
                    path = %self.legacy_binary.display(),
                    "could not retire pre-standard oxibrain binary"
                ),
            }
        }
        if let Some(dir) = self.legacy_binary.parent() {
            let _ = std::fs::remove_dir(dir); // only succeeds when empty
        }
    }

    /// Locate the binary, installing when absent and `auto_install` is set.
    pub async fn ensure_binary(&self, auto_install: bool) -> Option<PathBuf> {
        if let Some(b) = self.locate_binary() {
            return Some(b);
        }
        if !auto_install {
            return None;
        }
        match self.install().await {
            Ok(b) => Some(b),
            Err(e) => {
                tracing::warn!(error = %e, "oxibrain install failed — degraded");
                None
            }
        }
    }

    /// Remove the managed install — launcher symlink, version dirs, and the
    /// pre-standard managed binary. Explicit and `PATH` binaries are not
    /// ours.
    pub fn remove_managed(&self) -> Result<()> {
        let managed = self.bin_dir.join("oxibrain");
        if managed.symlink_metadata().is_ok() {
            std::fs::remove_file(&managed)
                .with_context(|| format!("remove {}", managed.display()))?;
        }
        if self.versions_dir.exists() {
            std::fs::remove_dir_all(&self.versions_dir)
                .with_context(|| format!("remove {}", self.versions_dir.display()))?;
        }
        if self.legacy_binary.symlink_metadata().is_ok() {
            std::fs::remove_file(&self.legacy_binary)
                .with_context(|| format!("remove {}", self.legacy_binary.display()))?;
            if let Some(dir) = self.legacy_binary.parent() {
                let _ = std::fs::remove_dir(dir);
            }
        }
        *self.version_cache.write().expect("version_cache poisoned") = None;
        Ok(())
    }

    /// One-time daemon-era cleanup: bootout the legacy launchd job and drop
    /// stale socket/PID/log files. Best-effort — every failure only logs.
    ///
    /// The `launchctl bootout` step is skipped when `OXIOS_BRAIN_NO_LAUNCHD`
    /// is set in the environment (tests set this to keep the host's launchd
    /// untouched).
    pub async fn legacy_cleanup(&self, brain_dir: &Path) {
        #[cfg(target_os = "macos")]
        {
            if std::env::var_os("OXIOS_BRAIN_NO_LAUNCHD").is_some() {
                tracing::debug!("OXIOS_BRAIN_NO_LAUNCHD set — skipping launchctl bootout");
            } else {
                let uid = nix_uid();
                let _ = tokio::process::Command::new("launchctl")
                    .args(["bootout", &format!("gui/{uid}/{LEGACY_LAUNCHD_LABEL}")])
                    .output()
                    .await;
            }
        }
        let _ = tokio::fs::remove_file(&self.plist_path).await;
        for name in [
            "oxibrain.sock",
            ".oxibrain.pid",
            "oxibrain.spawn.pid",
            "daemon.log",
        ] {
            let _ = tokio::fs::remove_file(brain_dir.join(name)).await;
        }
        tracing::debug!("brain legacy daemon cleanup done");
    }

    /// Other recognized locations where an `oxibrain` binary also exists —
    /// shadow diagnostics for `oxios brain status`. The winning path
    /// (from [`locate_binary`](Self::locate_binary)) is excluded; the
    /// pre-standard shared location and every `PATH` hit are listed when
    /// they exist on disk. Pure filesystem probe — no daemon, no spawn.
    pub fn shadow_roots(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut seen: Vec<PathBuf> = Vec::new();
        let winner = self.locate_binary();
        let push = |path: PathBuf, out: &mut Vec<PathBuf>, seen: &mut Vec<PathBuf>| {
            if path.is_file() && Some(&path) != winner.as_ref() && !seen.contains(&path) {
                seen.push(path.clone());
                out.push(path);
            }
        };
        push(self.legacy_binary.clone(), &mut out, &mut seen);
        if let Some(home) = std::env::var_os("HOME") {
            let cargo_bin = PathBuf::from(home)
                .join(".cargo")
                .join("bin")
                .join("oxibrain");
            push(cargo_bin, &mut out, &mut seen);
        }
        if let Some(path_var) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_var) {
                push(dir.join("oxibrain"), &mut out, &mut seen);
            }
        }
        out
    }

    /// Read `<binary> --version` stdout (trimmed); `None` when it fails.
    pub fn version_of(binary: &Path) -> Option<String> {
        let out = std::process::Command::new(binary)
            .arg("--version")
            .output()
            .ok()?;
        let s = String::from_utf8_lossy(&out.stdout).into_owned();
        let t = s.trim();
        (!t.is_empty()).then(|| t.to_string())
    }

    /// Cached variant of [`version_of`] (instance-method form). The first
    /// call probes the binary; subsequent calls reuse the cached result
    /// until [`install`] or [`remove_managed`] invalidates it. Hot path:
    /// `/health/ready`, `/api/status`, `/api/doctor`, and the web UI's
    /// 30 s `/api/brain/status` poll — no need to spawn `oxibrain --version`
    /// on every poll.
    pub fn version_of_cached(&self, binary: &Path) -> Option<String> {
        // Fast path: already cached — clone the inner Option out.
        {
            let guard = self.version_cache.read().expect("version_cache poisoned");
            if let Some(cached) = guard.as_ref() {
                return cached.clone();
            }
        }
        // Slow path: probe and store.
        let probed = Self::version_of(binary);
        let mut guard = self.version_cache.write().expect("version_cache poisoned");
        *guard = Some(probed.clone());
        probed
    }

    /// Test-only: drop the cached version probe. Production callers go
    /// through `install` / `remove_managed`.
    #[cfg(test)]
    pub fn invalidate_version_cache(&self) {
        *self.version_cache.write().expect("version_cache poisoned") = None;
    }
}

#[cfg(target_os = "macos")]
fn nix_uid() -> u32 {
    // `id -u` is always present on macOS.
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(501)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Save the current `PATH`, replace it with an empty string, and restore
    /// the original on `Drop`. Lets `locate_order_explicit_then_managed_then_path`
    /// mask any host-installed `oxibrain` without leaking the empty PATH to
    /// other tests if the test body panics.
    struct EmptyPathGuard {
        saved: Option<std::ffi::OsString>,
    }
    impl EmptyPathGuard {
        fn install() -> Self {
            let saved = std::env::var_os("PATH");
            // SAFETY: single-threaded test runner for env access.
            unsafe { std::env::set_var("PATH", "") };
            Self { saved }
        }
    }
    impl Drop for EmptyPathGuard {
        fn drop(&mut self) {
            // SAFETY: this test's only mutator of PATH; runs on unwind too.
            unsafe {
                match self.saved.take() {
                    Some(p) => std::env::set_var("PATH", p),
                    None => std::env::remove_var("PATH"),
                }
            }
        }
    }

    #[tokio::test]
    async fn locate_order_explicit_then_managed_then_path() {
        // Mask host-installed `oxibrain` so locate_binary is deterministic.
        // The guard restores PATH on Drop, including on panic.
        let tmp = tempfile::tempdir().unwrap();
        let explicit = tmp.path().join("custom-oxibrain");
        std::fs::write(&explicit, b"#!/bin/sh\n").unwrap();
        let section = crate::config::BrainSection {
            binary_path: explicit.display().to_string(),
            ..crate::config::BrainSection::default()
        };
        let inst = BrainInstaller::from_brain_section(tmp.path(), &section);
        {
            let _path_guard = EmptyPathGuard::install();
            assert_eq!(inst.locate_binary(), Some(explicit));
        }

        // No explicit → managed launcher wins over the legacy location and
        // PATH; the pre-standard shared location resolves as a fallback.
        let section = crate::config::BrainSection {
            binary_path: String::new(),
            ..crate::config::BrainSection::default()
        };
        let inst = BrainInstaller::from_brain_section(tmp.path(), &section);
        {
            let _path_guard = EmptyPathGuard::install();
            assert_eq!(inst.locate_binary(), None);
            std::fs::create_dir_all(tmp.path().join(".oxi/bin")).unwrap();
            let legacy = tmp.path().join(".oxi/bin/oxibrain");
            std::fs::write(&legacy, b"#!/bin/sh\n").unwrap();
            assert_eq!(inst.locate_binary(), Some(legacy));
            std::fs::create_dir_all(tmp.path().join(".oxi/oxibrain/bin")).unwrap();
            let managed = tmp.path().join(".oxi/oxibrain/bin/oxibrain");
            std::fs::write(&managed, b"#!/bin/sh\n").unwrap();
            assert_eq!(inst.locate_binary(), Some(managed));
        }
    }

    #[tokio::test]
    async fn legacy_cleanup_removes_stale_daemon_files() {
        // Guard against mutating the host's launchd when `launchctl bootout`
        // would otherwise run. The BrainInstaller::legacy_cleanup body checks
        // this same var before issuing the bootout (P2 follow-up; macOS only).
        // SAFETY: single-threaded env access in this test body.
        unsafe { std::env::set_var("OXIOS_BRAIN_NO_LAUNCHD", "1") };
        let tmp = tempfile::tempdir().unwrap();
        let brain_dir = tmp.path().join(".oxi/brain");
        std::fs::create_dir_all(&brain_dir).unwrap();
        std::fs::write(brain_dir.join("oxibrain.sock"), b"").unwrap();
        std::fs::write(brain_dir.join(".oxibrain.pid"), b"1\n").unwrap();
        std::fs::write(brain_dir.join("oxibrain.spawn.pid"), b"1\n").unwrap();
        let section = crate::config::BrainSection::default();
        let inst = BrainInstaller::from_brain_section(tmp.path(), &section);
        inst.legacy_cleanup(&brain_dir).await;
        // SAFETY: restore env state.
        unsafe { std::env::remove_var("OXIOS_BRAIN_NO_LAUNCHD") };
        for f in [
            "oxibrain.sock",
            ".oxibrain.pid",
            "oxibrain.spawn.pid",
            "daemon.log",
        ] {
            assert!(!brain_dir.join(f).exists(), "{f} should be removed");
        }
    }

    #[test]
    fn version_dir_name_sanitizes_tags() {
        assert_eq!(version_dir_name("v1.2.3").unwrap(), "1.2.3");
        assert_eq!(version_dir_name("0.12.1").unwrap(), "0.12.1");
        assert_eq!(version_dir_name(" v0.9.0 ").unwrap(), "0.9.0");
        assert!(version_dir_name("").is_err());
        assert!(version_dir_name("v").is_err());
        assert!(version_dir_name("..").is_err());
        assert!(version_dir_name("a/b").is_err());
    }

    #[test]
    fn place_binary_builds_versions_layout_and_flips_launcher() {
        let tmp = tempfile::tempdir().unwrap();
        let versions = tmp.path().join(".oxi/oxibrain/versions");
        let launcher = tmp.path().join(".oxi/oxibrain/bin/oxibrain");
        place_binary(&versions, &launcher, "1.2.3", b"#!/bin/sh\n").unwrap();
        assert!(versions.join("1.2.3/oxibrain").is_file());
        assert_eq!(
            std::fs::read_link(&launcher).unwrap(),
            std::path::Path::new("../versions/1.2.3/oxibrain")
        );
        assert!(
            launcher.is_file(),
            "launcher must resolve through the symlink"
        );

        // A second release flips the launcher; pruning keeps two versions.
        place_binary(&versions, &launcher, "1.3.0", b"#!/bin/sh\n").unwrap();
        assert_eq!(
            std::fs::read_link(&launcher).unwrap(),
            std::path::Path::new("../versions/1.3.0/oxibrain")
        );
        place_binary(&versions, &launcher, "1.4.0", b"#!/bin/sh\n").unwrap();
        let removed = prune_versions(&versions, "1.4.0", 2).unwrap();
        assert_eq!(removed, vec!["1.2.3".to_string()]);
        assert!(!versions.join("1.2.3").exists());
        assert!(versions.join("1.3.0").exists());
        assert!(versions.join("1.4.0").exists());
    }

    #[test]
    fn retire_legacy_drops_the_pre_standard_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy_dir = tmp.path().join(".oxi/bin");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        let legacy = legacy_dir.join("oxibrain");
        std::fs::write(&legacy, b"#!/bin/sh\n").unwrap();
        let inst =
            BrainInstaller::from_brain_section(tmp.path(), &crate::config::BrainSection::default());
        assert_eq!(inst.legacy_binary, legacy);
        inst.retire_legacy();
        assert!(!legacy.exists());
        assert!(!legacy_dir.exists(), "empty legacy bin dir is dropped");
    }

    #[test]
    fn version_of_reads_stdout() {
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("oxibrain");
        std::fs::write(&script, "#!/bin/sh\necho 'oxibrain 0.8.0'\n").unwrap();
        make_executable(&script);
        assert_eq!(
            BrainInstaller::version_of(&script).as_deref(),
            Some("oxibrain 0.8.0")
        );
        assert_eq!(
            BrainInstaller::version_of(&tmp.path().join("missing")),
            None
        );
    }

    #[test]
    fn version_of_cached_probes_once_then_reuses() {
        // Wrap `oxibrain` with a shell script that bumps an invocation
        // counter and prints a fixed version line. The test asserts the
        // counter never moves beyond the first probe (cache hits reuse
        // the stored value without spawning the child again).
        let tmp = tempfile::tempdir().unwrap();
        let count_file = tmp.path().join("invocations");
        std::fs::write(&count_file, "0").unwrap();
        let wrapper = tmp.path().join("oxibrain");
        let script = format!(
            "#!/bin/sh\nn=$(cat {count})\nn=$((n + 1))\necho $n > {count}\necho oxibrain 0.8.0\n",
            count = count_file.display(),
        );
        std::fs::write(&wrapper, script).unwrap();
        make_executable(&wrapper);

        let section = crate::config::BrainSection {
            binary_path: wrapper.display().to_string(),
            ..crate::config::BrainSection::default()
        };
        let inst = BrainInstaller::from_brain_section(tmp.path(), &section);

        // First call: probes.
        let v1 = inst.version_of_cached(&wrapper);
        assert_eq!(v1.as_deref(), Some("oxibrain 0.8.0"));
        assert_eq!(
            std::fs::read_to_string(&count_file).unwrap().trim(),
            "1",
            "first call should spawn once"
        );
        // Second call: cached -> no spawn.
        let v2 = inst.version_of_cached(&wrapper);
        assert_eq!(v2.as_deref(), Some("oxibrain 0.8.0"));
        assert_eq!(
            std::fs::read_to_string(&count_file).unwrap().trim(),
            "1",
            "second call must not spawn"
        );
        // invalidate_version_cache (tests only; production goes through
        // install/remove_managed) drops the cached value -> next call
        // re-probes.
        inst.invalidate_version_cache();
        let v3 = inst.version_of_cached(&wrapper);
        assert_eq!(v3.as_deref(), Some("oxibrain 0.8.0"));
        assert_eq!(
            std::fs::read_to_string(&count_file).unwrap().trim(),
            "2",
            "post-invalidate call should spawn again"
        );
    }

    fn make_executable(p: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(p).unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(p, perm).unwrap();
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
}
