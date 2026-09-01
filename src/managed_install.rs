//! Managed install: channel classification and on-disk store layout.
//!
//! Pure functions only — no I/O beyond `std::fs::canonicalize` for symlink
//! resolution. The store (extract, atomic flip, prune, sha256 verification,
//! PATH audit, migration) is layered on top in later tasks.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// GitHub owner/repo that hosts oxios release tarballs.
pub const GITHUB_REPO: &str = "project-oxi/oxios";

/// Asset filename on each release: a gzipped tarball containing the single
/// `oxios` binary for the current target triple.
pub const ASSET_BASE: &str = "oxios-aarch64-apple-darwin.tar.gz";

/// Where the running `oxios` binary came from, determined from
/// `std::env::current_exe()` (symlinks resolved) plus the configured
/// `oxios_home`. See `docs/designs/2026-08-27-managed-install-design.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Binary lives under `$OXIOS_HOME/versions/` (a released build) or
    /// `$OXIOS_HOME/bin/` (the launcher symlink). `oxios update` flips it
    /// in place.
    Managed,
    /// Homebrew install (`/opt/homebrew/...` on Apple Silicon,
    /// `/usr/local/Cellar/...` on Intel macs and Linuxbrew). Out of scope
    /// for the rustup-style updater; user is told to run `brew upgrade`.
    Brew,
    /// Cargo install (`$CARGO_HOME/bin/oxios`, defaulting to
    /// `$HOME/.cargo/bin`). Treated as unmanaged — `oxios update` offers
    /// migration to the managed channel; `--via cargo` keeps the legacy
    /// `cargo install` path.
    Cargo,
    /// Dev build: the exe lives under a cargo workspace that contains
    /// `crates/oxios-kernel`. Auto-update is refused; user is told to
    /// `cargo build`.
    Dev,
    /// Anything else — a manually-downloaded release tarball in `~/bin`,
    /// a symlink left behind by an old workflow, etc. `oxios update`
    /// offers migration to the managed channel.
    Unmanaged,
}

/// Classify which distribution channel the running exe came from.
///
/// `exe` is the path returned by `std::env::current_exe()` (or any other
/// caller-supplied path — tests use synthetic absolute paths). It is
/// canonicalized to resolve symlinks; on failure the raw input is used as
/// the fallback so synthetic paths in tests still classify correctly.
///
/// Detection order matches the design doc §"Install channels":
/// Managed → Brew → Cargo → Dev → Unmanaged.
pub fn classify_channel(exe: &Path, oxios_home: &Path) -> Channel {
    let canonical = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());

    // Managed — canonical first (the common case for `current_exe()`), then
    // the raw input (covers synthetic paths and unresolvable symlinks).
    if path_has_prefix(&canonical, oxios_home, "versions")
        || path_has_prefix(&canonical, oxios_home, "bin")
        || path_has_prefix(exe, oxios_home, "versions")
        || path_has_prefix(exe, oxios_home, "bin")
    {
        return Channel::Managed;
    }

    // Brew.
    if canonical.starts_with("/opt/homebrew/") || canonical.starts_with("/usr/local/Cellar/") {
        return Channel::Brew;
    }

    // Cargo — `$CARGO_HOME/bin` or `$HOME/.cargo/bin` (env-derived), plus a
    // path-component fallback (`<...>/.cargo/bin/oxios`) so synthetic paths
    // in tests classify correctly when neither env var matches.
    if let Some(cargo_bin) = cargo_bin_dir()
        && canonical.starts_with(&cargo_bin)
    {
        return Channel::Cargo;
    }
    if path_under_cargo_bin(&canonical) {
        return Channel::Cargo;
    }

    // Dev — walk ancestors looking for the oxios-kernel workspace marker.
    if is_dev_build(&canonical) || is_dev_build(exe) {
        return Channel::Dev;
    }

    Channel::Unmanaged
}

/// Directory holding extracted per-version binaries: `<home>/versions`.
pub fn versions_dir(oxios_home: &Path) -> PathBuf {
    oxios_home.join("versions")
}

/// The launcher symlink that PATH points at: `<home>/bin/oxios`.
pub fn launcher_path(oxios_home: &Path) -> PathBuf {
    oxios_home.join("bin").join("oxios")
}

/// Strict SemVer parser for version-dir names. No leading `v`, no
/// pre-release/build metadata — release tags land in `<home>/versions/`
/// under their bare `MAJOR.MINOR.PATCH` form.
pub fn parse_version_dir(name: &str) -> Option<(u64, u64, u64)> {
    let mut parts = name.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// True iff `<oxios_home>/<sub>` is a strict path-prefix of `p`.
///
/// `Path::starts_with` is component-aware (it would not match
/// `home/versions` against `home/versions_old`), but the second arg needs
/// to be a full `Path`, so we rebuild `<oxios_home>/<sub>` rather than
/// splitting the string.
fn path_has_prefix(p: &Path, oxios_home: &Path, sub: &str) -> bool {
    let prefix = oxios_home.join(sub);
    p.starts_with(&prefix)
}

/// Resolved `$CARGO_HOME/bin` (or the `$HOME/.cargo/bin` default).
fn cargo_bin_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CARGO_HOME")
        && !dir.is_empty()
    {
        return Some(PathBuf::from(dir).join("bin"));
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".cargo").join("bin"))
}

/// True iff the path's parent directory is a `<...>/.cargo/bin` (i.e. its
/// immediate parent is named `bin` and *its* parent is named `.cargo`).
/// Catches `cargo install` artifacts regardless of where `$HOME` points.
fn path_under_cargo_bin(p: &Path) -> bool {
    let mut comps = p.components();
    let last = comps.next_back();
    let bin = comps.next_back();
    let cargo = comps.next_back();
    matches!(
        (last, bin, cargo),
        (Some(_), Some(c), Some(p)) if c.as_os_str() == "bin" && p.as_os_str() == ".cargo"
    )
}

/// A path is a dev build when one of its ancestors is a directory that
/// contains both `Cargo.toml` and a `crates/oxios-kernel` member (either
/// a directory or a `Cargo.toml` inside it) — the oxios workspace marker.
/// Cheap and local; we don't stat every ancestor, just probe each one
/// for the two required entries.
fn is_dev_build(p: &Path) -> bool {
    let mut cur: Option<&Path> = Some(p);
    while let Some(dir) = cur {
        let cargo_toml = dir.join("Cargo.toml");
        let kernel_marker = dir.join("crates").join("oxios-kernel");
        if cargo_toml.is_file()
            && (kernel_marker.is_dir() || kernel_marker.join("Cargo.toml").is_file())
        {
            return true;
        }
        cur = dir.parent();
    }
    false
}

// --- store ops: install/extract, atomic flip, prune, rollback pick ---

/// Extract a single `oxios` member from a gzipped tarball and write it to
/// `<oxios_home>/versions/<version>/oxios` with mode `0755`. Atomic via
/// temp-file + rename: a partial extract never clobbers an existing
/// version dir.
///
/// Rejects any tar member whose path is not exactly `oxios` (a release
/// tarball contains exactly one entry — the binary itself). Returns the
/// final path of the installed binary.
pub fn install_version_bytes(oxios_home: &Path, version: &str, tar_gz: &[u8]) -> Result<PathBuf> {
    use std::io::Read;

    // Reject non-SemVer version-dir names: prune/rollback/current_target
    // all rely on `parse_version_dir` recognizing the directory, so an
    // unparseable tag (e.g. `1.45.0-rc.1`) would install fine but silently
    // disable launcher protection and rollback. Also blocks hypothetical
    // path-traversal tags from a tampered API response.
    if parse_version_dir(version).is_none() {
        anyhow::bail!("refusing to install non-SemVer version directory: {version:?}");
    }

    let version_dir = versions_dir(oxios_home).join(version);
    std::fs::create_dir_all(&version_dir)
        .with_context(|| format!("create version dir {}", version_dir.display()))?;

    let final_path = version_dir.join("oxios");
    let tmp_path = version_dir.join(".oxios.tmp");

    let decoder = flate2::read::GzDecoder::new(tar_gz);
    let mut archive = tar::Archive::new(decoder);
    let mut found = false;
    for entry in archive.entries().context("open tar entries")? {
        let mut entry = entry.context("read tar entry")?;
        let path = entry.path().context("read tar entry path")?.into_owned();
        // macOS tar stores xattrs as `._<name>` AppleDouble members even
        // with --no-xattrs in older toolchains — noise, not tampering.
        // Skip them; anything else that is not exactly `oxios` still bails.
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("._") && path.parent() == Some(std::path::Path::new("")))
        {
            continue;
        }
        if path != std::path::Path::new("oxios") {
            anyhow::bail!("unexpected tar member {}", path.display());
        }
        if found {
            anyhow::bail!("unexpected tar member {}", path.display());
        }
        found = true;
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .context("read tar entry bytes")?;
        std::fs::write(&tmp_path, &buf).with_context(|| format!("write {}", tmp_path.display()))?;
    }
    if !found {
        anyhow::bail!("tarball contains no `oxios` member");
    }

    std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))
        .with_context(|| format!("chmod 0755 {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, &final_path)
        .with_context(|| format!("rename to {}", final_path.display()))?;

    Ok(final_path)
}

/// Atomically point `<oxios_home>/bin/oxios` at `versions/<version>/oxios`.
/// Build a temp symlink at `bin/.oxios.tmp`, then `rename` it over the
/// real launcher — POSIX rename is atomic on the same filesystem, so any
/// `execve` of the launcher observes either the old or new target, never
/// a half-built path.
pub fn flip_launcher(oxios_home: &Path, version: &str) -> Result<()> {
    use std::os::unix::fs::symlink;

    let bin_dir = oxios_home.join("bin");
    std::fs::create_dir_all(&bin_dir)
        .with_context(|| format!("create bin dir {}", bin_dir.display()))?;

    let target = PathBuf::from("../versions").join(version).join("oxios");
    let tmp_link = bin_dir.join(".oxios.tmp");
    let final_link = launcher_path(oxios_home);

    let _ = std::fs::remove_file(&tmp_link);
    symlink(&target, &tmp_link)
        .with_context(|| format!("symlink {} -> {}", tmp_link.display(), target.display()))?;
    std::fs::rename(&tmp_link, &final_link)
        .with_context(|| format!("rename {} -> {}", tmp_link.display(), final_link.display()))?;

    Ok(())
}

/// Delete old version directories so at most `keep` remain on disk,
/// ordered newest-first by `parse_version_dir`. Returns the names of the
/// removed dirs (not paths). The version the launcher currently points
/// at is never removed, even when it would otherwise be a candidate.
pub fn prune_versions(oxios_home: &Path, keep: usize) -> Result<Vec<String>> {
    let mut installed: Vec<(u64, u64, u64, String)> = Vec::new();
    let dir = versions_dir(oxios_home);
    let entries = std::fs::read_dir(&dir).with_context(|| format!("read_dir {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("read_dir entry in {}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(parsed) = parse_version_dir(&name) {
            installed.push((parsed.0, parsed.1, parsed.2, name));
        }
    }
    installed.sort_by(|a, b| b.cmp(a));

    let protect: Option<String> = current_target_version(oxios_home);
    let mut removed = Vec::new();
    for (_maj, _min, _pat, name) in installed.into_iter().skip(keep) {
        if protect.as_deref() == Some(name.as_str()) {
            continue;
        }
        let p = dir.join(&name);
        std::fs::remove_dir_all(&p).with_context(|| format!("remove_dir_all {}", p.display()))?;
        removed.push(name);
    }
    Ok(removed)
}

/// Name of the version directory the launcher symlink points at, or
/// `None` if the launcher is missing, unreadable, or doesn't resolve to
/// a `<home>/versions/<v>/oxios` path.
pub fn current_target_version(oxios_home: &Path) -> Option<String> {
    let link = launcher_path(oxios_home);
    let target = std::fs::read_link(&link).ok()?;
    // Symlink lives at <home>/bin/oxios and points at ../versions/<v>/oxios
    // (relative). Components: ".." -> "versions" -> version -> "oxios".
    let comps: Vec<_> = target.components().collect();
    if comps.len() != 4 {
        return None;
    }
    let version = comps[2].as_os_str().to_str()?;
    parse_version_dir(version).map(|_| version.to_string())
}

/// Highest installed version strictly less than the launcher's current
/// target — the natural rollback pick. `None` when no such version
/// exists (only one installed, or launcher is missing).
pub fn rollback_target(oxios_home: &Path) -> Option<String> {
    let current = current_target_version(oxios_home)?;
    let current_parsed = parse_version_dir(&current)?;
    let dir = versions_dir(oxios_home);
    let entries = std::fs::read_dir(&dir).ok()?;
    let mut best: Option<((u64, u64, u64), String)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(parsed) = parse_version_dir(&name)
            && parsed < current_parsed
            && best.as_ref().is_none_or(|b| parsed > b.0)
        {
            best = Some((parsed, name));
        }
    }
    best.map(|(_, n)| n)
}

// --- release fetch: GitHub API + tarball download with sha256 gate ---

/// Minimal subset of the GitHub release JSON shape we read.
#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    /// Version string with any leading `v` stripped (e.g. `"1.44.0"`).
    pub tag: String,
    /// Release web page URL.
    pub html_url: String,
    /// Release notes (markdown body).
    pub body: String,
    /// `published_at` timestamp from the GitHub API (raw, unparsed).
    /// `oxios changelog` renders this alongside the tag.
    pub published_at: String,
}

/// Lowercase-hex SHA-256 of `data` (64 chars).
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Verify `data` against a `sha256sum`-style sidecar file.
///
/// Accepts any line in `<hex>` or `<hex>  <filename>` form (two spaces,
/// single space, or asterisk-binary-mode separator all parsed); the first
/// 64-char lowercase hex token on the first non-empty line is compared
/// against the recomputed digest. Returns `Err` on any mismatch or
/// unparseable sidecar so the caller can drop the untrusted bytes.
pub fn verify_sha256(data: &[u8], sha_text: &str) -> Result<()> {
    let expected = sha_text
        .lines()
        .find(|l| !l.trim().is_empty())
        .and_then(|line| line.split_whitespace().next())
        .ok_or_else(|| anyhow::anyhow!("sha256 sidecar is empty"))?;
    let expected = expected.trim().to_ascii_lowercase();
    if expected.len() != 64 || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("malformed sha256 digest: {expected}");
    }
    let actual = sha256_hex(data);
    if actual != expected {
        anyhow::bail!("sha256 mismatch: expected {expected}, got {actual}");
    }
    Ok(())
}

/// Fetch the latest (or pinned) GitHub release metadata for `GITHUB_REPO`.
///
/// When `version` is `None`, hits `/releases/latest`; otherwise hits
/// `/releases/tags/v{version}`. Uses a 30s overall / 10s connect timeout
/// (matching `commands/update.rs:92-97`) and a `oxios-updater` UA so
/// GitHub rate-limits by user rather than dropping the request.
pub async fn fetch_release(version: Option<&str>) -> Result<ReleaseInfo> {
    let url = match version {
        Some(v) => format!("https://api.github.com/repos/{GITHUB_REPO}/releases/tags/v{v}"),
        None => format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest"),
    };

    let client = reqwest::Client::builder()
        .user_agent("oxios-updater")
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .context("build HTTP client")?;

    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;

    if !resp.status().is_success() {
        anyhow::bail!("GitHub API {url} returned {}", resp.status());
    }

    let v: serde_json::Value = resp
        .json()
        .await
        .with_context(|| format!("parse JSON from {url}"))?;

    let tag = v["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("release JSON missing tag_name"))?
        .trim_start_matches('v')
        .to_string();
    let html_url = v["html_url"].as_str().unwrap_or("").to_string();
    let body = v["body"].as_str().unwrap_or("").to_string();
    let published_at = v["published_at"].as_str().unwrap_or("?").to_string();
    Ok(ReleaseInfo {
        tag,
        html_url,
        body,
        published_at,
    })
}

/// Maximum bytes we'll buffer for a single release tarball download.
/// Parity with the web route's `MAX_DOWNLOAD_BYTES` (200 MiB) so both
/// surfaces reject oversized payloads with the same cap. The actual
/// binary tarball is ~80 MB; 200 MiB leaves ample headroom for future
/// asset growth without inviting OOM from a tampered release.
pub const MAX_TARBALL_BYTES: u64 = 200 * 1024 * 1024;

/// Download the release tarball for `version` and verify its sha256
/// sidecar before returning the bytes. Any digest mismatch bails and
/// drops the buffer so a partially-trusted download never reaches
/// `install_version_bytes`. The download is size-capped via
/// `MAX_TARBALL_BYTES` to prevent a tampered release from OOMing the
/// updater; the cap is checked up-front from `Content-Length` (when
/// the server advertises it) so we never buffer more than the cap.
pub async fn fetch_tarball(version: &str) -> Result<Vec<u8>> {
    let tar_url =
        format!("https://github.com/{GITHUB_REPO}/releases/download/v{version}/{ASSET_BASE}");
    let sha_url = format!("{tar_url}.sha256");

    let client = reqwest::Client::builder()
        .user_agent("oxios-updater")
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .context("build HTTP client")?;

    let tar_resp = client
        .get(&tar_url)
        .send()
        .await
        .with_context(|| format!("GET {tar_url}"))?
        .error_for_status()
        .with_context(|| format!("GET {tar_url}"))?;

    // F11: reject oversized payloads before buffering. Mirrors the web
    // route's `MAX_DOWNLOAD_BYTES` cap. Servers that omit
    // Content-Length (chunked) are not pre-checked; the post-read
    // assertion below catches them. Either path keeps us below the
    // cap.
    if let Some(cl) = tar_resp.content_length()
        && cl > MAX_TARBALL_BYTES
    {
        anyhow::bail!(
            "release tarball {} too large: {} bytes (cap {})",
            tar_url,
            cl,
            MAX_TARBALL_BYTES
        );
    }

    let tar_bytes = tar_resp
        .bytes()
        .await
        .with_context(|| format!("read body {tar_url}"))?
        .to_vec();

    if (tar_bytes.len() as u64) > MAX_TARBALL_BYTES {
        anyhow::bail!(
            "release tarball {} exceeded size cap: {} bytes (cap {})",
            tar_url,
            tar_bytes.len(),
            MAX_TARBALL_BYTES
        );
    }

    let sha_text = client
        .get(&sha_url)
        .send()
        .await
        .with_context(|| format!("GET {sha_url}"))?
        .error_for_status()
        .with_context(|| format!("GET {sha_url}"))?
        .text()
        .await
        .with_context(|| format!("read body {sha_url}"))?;

    verify_sha256(&tar_bytes, &sha_text)?;
    Ok(tar_bytes)
}

/// Reject the install when the filesystem holding `dir` cannot fit
/// `needed_bytes` after reservation. `f_bavail` (free blocks for the
/// unprivileged caller) times `f_bsize` gives the realistic headroom
/// without double-counting root-reserved blocks.
pub fn disk_precheck(dir: &Path, needed_bytes: u64) -> Result<()> {
    let cpath = std::ffi::CString::new(dir.as_os_str().as_bytes())
        .with_context(|| format!("path contains NUL: {}", dir.display()))?;
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statfs(cpath.as_ptr(), &mut stat) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        anyhow::bail!("statfs({}): {err}", dir.display());
    }
    let free = (stat.f_bavail as u64).saturating_mul(stat.f_bsize as u64);
    if free < needed_bytes {
        anyhow::bail!(
            "not enough free space at {}: need {} bytes, have {} bytes",
            dir.display(),
            needed_bytes,
            free
        );
    }
    Ok(())
}

// --- PATH shadow audit + migration planner ---

/// What kind of shadow an `audit_shadows` entry is.
///
/// - `Canonical`: a file named exactly `oxios` (`<dir>/oxios`). The
///   installer repoints these — they shadow the launcher when they live
///   ahead of `$OXIOS_HOME/bin` in PATH.
/// - `Versioned`: a file named `oxios-<version>…` in a PATH dir (the
///   suffix after `oxios-` starts with a digit — sibling tools like
///   `oxios-migrate-vault` never match). Stale
///   copies left behind by old workflows (`oxios-1.42.0-livestream*`,
///   `oxios-1.43.1` after a `cargo install --force`, etc.). Not
///   eligible for repointing — they are the file, not a link. Cleanup
///   treats them as deletion candidates.
/// - `Backup`: a file named `oxios.bak.<date>` (the migration plan's
///   backup target for a previously-preserved copy). Deletion target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowKind {
    Canonical,
    Versioned,
    Backup,
}

/// One `oxios` binary found on the user's PATH other than `<home>/bin/oxios`.
///
/// `path` is the shadow location as discovered (e.g. `/usr/local/bin/oxios`).
/// `symlink_target` is `Some(t)` if the entry is a symlink and `read_link`
/// succeeded; `None` for regular files or when the link target cannot be
/// resolved. `size_bytes` is the resolved file's length, falling back to the
/// shadow entry's own length when canonicalization fails. `kind` classifies
/// the entry (canonical name, versioned `oxios-*`, or `oxios.bak.<date>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowEntry {
    /// Absolute path of the shadow entry, `<dir>/oxios` or `<dir>/oxios-*`.
    pub path: PathBuf,
    /// Symlink target if the entry is a symlink, otherwise `None`.
    pub symlink_target: Option<PathBuf>,
    /// Size of the resolved file in bytes (resolved via `canonicalize`).
    pub size_bytes: u64,
    /// What kind of shadow this is.
    pub kind: ShadowKind,
}
/// Walk `path_env` (a colon-separated `PATH`-style list) and report every
/// `oxios` binary found, in PATH order.
///
/// Pure: takes the raw `PATH` string and `oxios_home`; performs no env
/// reads. Skips `<oxios_home>/bin` — that's the launcher we manage, not a
/// shadow. Deduplicates entries that appear in multiple PATH components
/// while preserving first-seen order. Entries whose `<dir>/oxios` does not
/// exist on disk are silently skipped (the typical "no such binary here"
/// case). `size_bytes` reflects the resolved file's length; if
/// `canonicalize` fails, the entry's own metadata length is used as the
/// fallback.
///
/// In addition to the canonical `<dir>/oxios` entries, this audit also
/// reports version-named stale copies (`<dir>/oxios-<version>…`, e.g.
/// `oxios-1.42.0-livestream*`, `oxios-1.43.1` — the suffix must start
/// with a digit so sibling tools like `oxios-migrate-vault` are never
/// matched) and pre-existing backup renames (`<dir>/oxios.bak.<date>`).
/// Those are stale leftovers from previous install workflows;
/// `oxios doctor --cleanup` reclaims them. `oxios update`'s
/// migration repoint path only touches the canonical `<dir>/oxios`
/// link (handled separately because it's still pointing at the launcher
/// once the swap completes).
pub fn audit_shadows(oxios_home: &Path, path_env: &str) -> Vec<ShadowEntry> {
    let launcher = launcher_path(oxios_home);
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut out: Vec<ShadowEntry> = Vec::new();
    for dir in std::env::split_paths(path_env) {
        // Canonical `oxios` entry — always reported first (one per dir).
        push_shadow(
            &mut seen,
            &mut out,
            dir.join("oxios"),
            &launcher,
            ShadowKind::Canonical,
        );
        // Sibling entries: version-named `oxios-<version>…` drops and
        // `oxios.bak.<date>` renames. Read the directory once; `read_dir`
        // is cheap and the iteration is in directory order (stable, but we
        // dedup by absolute path to be safe across PATH re-entries).
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = match name.to_str() {
                    Some(s) => s,
                    None => continue,
                };
                let kind = if let Some(rest) = name_str.strip_prefix("oxios-") {
                    // `oxios-<rest>` — versioned stale copy. The suffix must
                    // start with a digit so we only match version-named drops
                    // (`oxios-1.42.0-livestream*`, `oxios-1.43.1`, `oxios-1.43.1.keep`)
                    // and never sibling tools like `oxios-migrate-vault`, which is a
                    // different binary shipped in the same release.
                    if !rest.starts_with(|c: char| c.is_ascii_digit()) {
                        continue;
                    }
                    ShadowKind::Versioned
                } else if let Some(rest) = name_str.strip_prefix("oxios.bak.") {
                    // Backup rename from a prior migration. Only accept
                    // non-empty suffixes that look like the YYYYMMDD form
                    // we generate; anything else isn't ours to delete.
                    if rest.len() != 8 || !rest.chars().all(|c| c.is_ascii_digit()) {
                        continue;
                    }
                    ShadowKind::Backup
                } else if name_str == "oxios.pre-oxios" {
                    // Aside from our own migration repoint: apply_migration_plan
                    // renames the old symlink here before replacing it. Folded
                    // into Backup so doctor reports it and --cleanup reclaims
                    // it (deleting the aside symlink never touches its target).
                    ShadowKind::Backup
                } else {
                    continue;
                };
                push_shadow(&mut seen, &mut out, dir.join(&name), &launcher, kind);
            }
        }
    }
    out
}

/// Shared `stat` + `seen` + `kind` push for one candidate path. Skips
/// missing files, duplicates, the launcher itself, and Canonical symlink
/// shadows whose target has already converged onto the launcher (e.g.
/// `~/bin/oxios -> ~/.oxios/bin/oxios`, or `~/.cargo/bin/oxios` after
/// `apply_migration_plan` repointed it). Without this skip the audit
/// would list an already-converged entry forever, `oxios doctor` would
/// never go clean, and `--adopt` would loop a no-op repoint. Other
/// kinds (Versioned stale copies, Backup renames) keep their full
/// treatment because they're stale files, not converged aliases.
fn push_shadow(
    seen: &mut Vec<PathBuf>,
    out: &mut Vec<ShadowEntry>,
    candidate: PathBuf,
    launcher: &Path,
    kind: ShadowKind,
) {
    if candidate == launcher {
        return;
    }
    if seen.contains(&candidate) {
        return;
    }
    seen.push(candidate.clone());
    let meta = match std::fs::symlink_metadata(&candidate) {
        Ok(m) => m,
        Err(_) => return, // not present in this PATH dir
    };
    let symlink_target = if meta.file_type().is_symlink() {
        std::fs::read_link(&candidate).ok()
    } else {
        None
    };
    // Fix #3: a Canonical symlink whose target resolves to the launcher
    // (absolute path canonicalized, or relative path resolved against the
    // candidate's own directory) has already converged onto the managed
    // store — it's an alias, not a shadow. Skip it so `oxios doctor`
    // reports a clean PATH. Versioned/Backup kinds still report because
    // they're stale files (not aliases) that should be cleaned up.
    if matches!(kind, ShadowKind::Canonical)
        && let Some(t) = symlink_target.as_ref()
    {
        let absolute_target = if t.is_absolute() {
            std::fs::canonicalize(t).ok()
        } else {
            candidate
                .parent()
                .and_then(|d| std::fs::canonicalize(d.join(t)).ok())
        };
        let launcher_abs = std::fs::canonicalize(launcher).ok();
        if absolute_target.is_some() && launcher_abs.is_some() && absolute_target == launcher_abs {
            return;
        }
    }
    let size_bytes = std::fs::canonicalize(&candidate)
        .ok()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .unwrap_or_else(|| meta.len());
    out.push(ShadowEntry {
        path: candidate,
        symlink_target,
        size_bytes,
        kind,
    });
}

/// Plan for migrating shadow entries off the user's PATH.
///
/// `repoint` lists symlink shadows that the installer can safely overwrite
/// (its target was inside the managed store or was otherwise a previous
/// launcher symlink). `preserve_as_bak` lists regular-file shadows with
/// their backup target `<path>.bak.<YYYYMMDD>` so a stray copy can be
/// restored on rollback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPlan {
    /// Symlink shadows that should be replaced by the managed launcher.
    pub repoint: Vec<PathBuf>,
    /// Regular-file shadows paired with the `.bak.<YYYYMMDD>` path they
    /// should be moved to before the installer's own launcher takes over.
    pub preserve_as_bak: Vec<(PathBuf, PathBuf)>,
}

/// Build a migration plan from a list of `ShadowEntry`s.
///
/// Symlinks (entries whose `symlink_target` is `Some`) are added to
/// `repoint` — the installer can replace them atomically with the managed
/// launcher symlink. Regular files (no symlink target) are paired with a
/// `.bak.<YYYYMMDD>` backup path under `Local::now().date()`; if multiple
/// regular-file shadows share the same date the same suffix is used.
pub fn plan_migration(shadows: &[ShadowEntry]) -> MigrationPlan {
    let suffix = chrono::Local::now().format("%Y%m%d").to_string();
    let mut repoint = Vec::new();
    let mut preserve_as_bak = Vec::new();
    for entry in shadows {
        match &entry.symlink_target {
            Some(_) => repoint.push(entry.path.clone()),
            None => {
                let mut bak = entry.path.clone();
                let name = bak
                    .file_name()
                    .map(|n| n.to_os_string())
                    .unwrap_or_default();
                let mut new_name = name;
                new_name.push(format!(".bak.{suffix}"));
                bak.set_file_name(new_name);
                preserve_as_bak.push((entry.path.clone(), bak));
            }
        }
    }
    MigrationPlan {
        repoint,
        preserve_as_bak,
    }
}

/// Ensure the idempotent managed PATH block exists in the user's shell
/// rc file, mirroring `share/install.sh` step 4 exactly:
///
/// ```text
/// # BEGIN oxios (managed)
/// export PATH="$OXIOS_HOME/bin:$PATH"
/// # END oxios (managed)
/// ```
///
/// Design (PATH integration): `$OXIOS_HOME/bin` is prepended "by
/// `install.sh` AND by `oxios update` migration". Migration renames
/// plain-file shadows (e.g. `~/.cargo/bin/oxios`) aside to
/// `.bak.<date>`, so without this block a user who never ran install.sh
/// would lose `oxios` from PATH entirely after confirming a migration.
///
/// rc selection mirrors install.sh: `~/.zshrc` when it exists, else
/// `~/.zprofile` (created by the append when missing). Idempotent: skips
/// when a previous managed block marker is already present. Never
/// touches any other line (the 8/26 incident showed user profiles
/// prepend `~/bin` last — we must not reorder user lines, only ensure
/// ours exists).
///
/// Returns the rc file written to, or `None` when the block already
/// existed or the home directory could not be resolved.
pub fn ensure_path_block(oxios_home: &Path) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let rc = if home.join(".zshrc").exists() {
        home.join(".zshrc")
    } else {
        home.join(".zprofile")
    };
    let existing = std::fs::read_to_string(&rc).unwrap_or_default();
    if existing.contains("BEGIN oxios (managed)") {
        return None;
    }
    // `$PATH` stays literal in the rc file — the shell expands it at
    // startup, exactly like the `printf` in install.sh (SC2016).
    let block = format!(
        "\n# BEGIN oxios (managed)\nexport PATH=\"{}/bin:$PATH\"\n# END oxios (managed)\n",
        oxios_home.display()
    );
    std::fs::write(&rc, format!("{existing}{block}")).ok()?;
    Some(rc)
}
/// Render the `audit_shadows` table for `oxios doctor`.
///
/// One line per shadow entry: `path -> target (size)` for symlinks, or
/// `path (size)` for plain files. Pure: takes ownership of nothing, the
/// output string is built from the input slice. A trailing newline is
/// not appended; callers decide.
pub fn render_doctor_report(shadows: &[ShadowEntry]) -> String {
    let mut out = String::new();
    for entry in shadows {
        match &entry.symlink_target {
            Some(target) => {
                out.push_str(&format!(
                    "{} -> {} ({})\n",
                    entry.path.display(),
                    target.display(),
                    entry.size_bytes
                ));
            }
            None => {
                out.push_str(&format!(
                    "{} ({})\n",
                    entry.path.display(),
                    entry.size_bytes
                ));
            }
        }
    }
    out
}

/// Build the list of filesystem paths that `oxios doctor --cleanup` should
/// remove.
///
/// `.bak.<date>` projections produced by the migration plan (so a later
/// `--cleanup` reclaims what a previous migration preserved), plus the
/// stale versioned copies (`oxios-1.43.1`…) and existing backup renames
/// (`oxios.bak.<date>`) the audit found. This matches the design scope:
/// ".bak.* copies and stale versioned files listed by the audit".
///
/// Symlinks are intentionally excluded: the installer repoints them to
/// the managed launcher rather than deleting them. Plain-file Canonical
/// shadows (e.g. a live `~/.cargo/bin/oxios` from `cargo install`) are
/// also NOT deletion targets: deleting them outright could remove the
/// user's only `oxios` on PATH. Migration renames them to `.bak.<date>`
/// first, and that `.bak` then becomes reclaimable here.
pub fn doctor_cleanup_targets(shadows: &[ShadowEntry]) -> Vec<PathBuf> {
    let plan = plan_migration(shadows);
    let mut targets: Vec<PathBuf> = plan
        .preserve_as_bak
        .iter()
        .map(|(_, bak)| bak.clone())
        .collect();
    for entry in shadows {
        let deletion_candidate = matches!(entry.kind, ShadowKind::Versioned | ShadowKind::Backup);
        if deletion_candidate {
            targets.push(entry.path.clone());
        }
    }
    targets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_managed_paths() {
        let home = std::path::Path::new("/fake/home");
        assert!(matches!(
            classify_channel(Path::new("/fake/home/versions/1.44.0/oxios"), home),
            Channel::Managed
        ));
        assert!(matches!(
            classify_channel(Path::new("/fake/home/bin/oxios"), home),
            Channel::Managed
        ));
    }

    #[test]
    fn classify_foreign_channels() {
        let home = std::path::Path::new("/fake/home");
        assert!(matches!(
            classify_channel(Path::new("/opt/homebrew/bin/oxios"), home),
            Channel::Brew
        ));
        assert!(matches!(
            classify_channel(Path::new("/usr/local/Cellar/oxios/1.44.0/bin/oxios"), home),
            Channel::Brew
        ));
        assert!(matches!(
            classify_channel(Path::new("/Users/w/.cargo/bin/oxios"), home),
            Channel::Cargo
        ));
    }

    #[test]
    fn classify_dev_build_inside_repo_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("crates/oxios-kernel")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/oxios-kernel\"]\n",
        )
        .unwrap();
        let target = root.join("target/release/oxios");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"bin").unwrap();
        // cargo build artifacts live inside the repo that defines the workspace
        assert!(matches!(
            classify_channel(&target, Path::new("/fake/home")),
            Channel::Dev
        ));
    }

    #[test]
    fn classify_unmanaged_plain_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("oxios-1.43.1");
        std::fs::write(&bin, b"bin").unwrap();
        assert!(matches!(
            classify_channel(&bin, std::path::Path::new("/fake/home")),
            Channel::Unmanaged
        ));
    }

    #[test]
    fn version_dir_parsing() {
        assert_eq!(parse_version_dir("1.44.0"), Some((1, 44, 0)));
        assert_eq!(parse_version_dir("v1.44.0"), None);
        assert_eq!(parse_version_dir("1.44"), None);
        assert_eq!(parse_version_dir("1.44.0-livestream"), None);
    }

    // ---- fixture tarball: single `oxios` member gzipped ----
    fn fixture_tar_gz() -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "oxios", std::io::Cursor::new(b"bin\n"))
            .unwrap();
        let bytes = builder.into_inner().unwrap();
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut enc, &bytes).unwrap();
        enc.finish().unwrap()
    }

    #[test]
    fn install_flip_prune_keep_two() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        for v in ["1.42.0", "1.43.0", "1.44.0"] {
            install_version_bytes(home, v, &fixture_tar_gz()).unwrap();
        }
        assert!(home.join("versions/1.44.0/oxios").is_file());
        flip_launcher(home, "1.44.0").unwrap();
        assert_eq!(current_target_version(home).as_deref(), Some("1.44.0"));
        let removed = prune_versions(home, 2).unwrap();
        assert_eq!(removed, vec!["1.42.0".to_string()]); // keep 1.43.0 + 1.44.0
        assert!(!home.join("versions/1.42.0").exists());
        assert_eq!(rollback_target(home).as_deref(), Some("1.43.0"));
    }

    #[test]
    fn prune_never_removes_launcher_target() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        install_version_bytes(home, "1.40.0", &fixture_tar_gz()).unwrap();
        install_version_bytes(home, "1.41.0", &fixture_tar_gz()).unwrap();
        install_version_bytes(home, "1.42.0", &fixture_tar_gz()).unwrap();
        flip_launcher(home, "1.40.0").unwrap(); // old but live
        let removed = prune_versions(home, 2).unwrap();
        assert!(!removed.contains(&"1.40.0".to_string()));
        assert!(home.join("versions/1.40.0/oxios").is_file());
    }

    #[test]
    fn sha_verify_rejects_mismatch() {
        let data = b"payload".to_vec();
        let good = format!("{}  {}\n", sha256_hex(&data), ASSET_BASE);
        assert!(verify_sha256(&data, &good).is_ok());
        let bad = format!("{}  {}\n", "0".repeat(64), ASSET_BASE);
        assert!(verify_sha256(&data, &bad).is_err());
    }

    #[test]
    fn audit_reports_shadows_in_path_order_excluding_home() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let dir_a = tmp.path().join("a");
        std::fs::create_dir_all(&dir_a).unwrap();
        let dir_b = tmp.path().join("b");
        std::fs::create_dir_all(&dir_b).unwrap();
        std::fs::write(dir_a.join("oxios"), vec![0u8; 10]).unwrap();
        std::fs::write(dir_b.join("oxios"), vec![0u8; 20]).unwrap();
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::write(home.join("bin/oxios"), vec![0u8; 5]).unwrap();
        let path_env = format!("{}:{}", dir_a.to_str().unwrap(), dir_b.to_str().unwrap());
        let shadows = audit_shadows(&home, &path_env);
        assert_eq!(shadows.len(), 2);
        assert_eq!(shadows[0].path, dir_a.join("oxios"));
        assert_eq!(shadows[1].path, dir_b.join("oxios"));
        // home/bin is excluded even when present in PATH
        let with_home = format!(
            "{}:{}:{}",
            dir_a.to_str().unwrap(),
            home.join("bin").to_str().unwrap(),
            dir_b.to_str().unwrap(),
        );
        let shadows = audit_shadows(&home, &with_home);
        assert_eq!(shadows.len(), 2);
        assert_eq!(shadows[0].path, dir_a.join("oxios"));
        assert_eq!(shadows[1].path, dir_b.join("oxios"));
    }

    #[test]
    fn audit_skips_canonical_symlinks_already_pointing_at_launcher() {
        // Fix #3: a Canonical symlink shadow whose target resolves to
        // the managed launcher is already converged. The audit must
        // skip it so `oxios doctor` reports clean and `--adopt`
        // doesn't loop no-op repoints. The smoke machine exhibits
        // `~/bin/oxios -> ~/.oxios/bin/oxios` as a permanent false
        // positive without this filter.
        //
        // Layout: two foreign dirs on PATH, each with its own `oxios`
        // symlink. The first dir's symlink points at the managed
        // launcher (converged — must be skipped); the second dir's
        // symlink points at an unrelated file (NOT converged — must
        // still be reported).
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let converged_dir = tmp.path().join("converged-foreign");
        let unrelated_dir = tmp.path().join("unrelated-foreign");
        std::fs::create_dir_all(&converged_dir).unwrap();
        std::fs::create_dir_all(&unrelated_dir).unwrap();
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::write(home.join("bin/oxios"), vec![0u8; 4]).unwrap();

        // (a) foreign symlink that points at the launcher (converged).
        let converged = converged_dir.join("oxios");
        std::os::unix::fs::symlink(home.join("bin/oxios"), &converged).unwrap();

        // (b) foreign symlink that points somewhere else (unrelated).
        let unrelated_target = unrelated_dir.join("old-copy");
        std::fs::write(&unrelated_target, vec![0u8; 3]).unwrap();
        let unrelated = unrelated_dir.join("oxios");
        std::os::unix::fs::symlink(&unrelated_target, &unrelated).unwrap();

        let path_env = format!(
            "{}:{}:{}",
            converged_dir.to_str().unwrap(),
            unrelated_dir.to_str().unwrap(),
            home.join("bin").to_str().unwrap()
        );
        let shadows = audit_shadows(&home, &path_env);
        // Only the unrelated symlink comes back; the converged one is
        // skipped. (The launcher itself is also excluded.)
        assert_eq!(shadows.len(), 1, "got: {:?}", shadows);
        assert_eq!(shadows[0].path, unrelated);
    }

    #[test]
    fn audit_skips_converged_canonical_symlink_with_relative_target() {
        // Same shape as the absolute case but using a relative symlink
        // target — `flip_launcher` writes the launcher as a relative
        // symlink, so this is the production form. The relative-target
        // branch of the converged filter must work too.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let foreign_dir = tmp.path().join("foreign");
        std::fs::create_dir_all(&foreign_dir).unwrap();
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::write(home.join("bin/oxios"), vec![0u8; 4]).unwrap();

        // foreign/oxios -> ../home/bin/oxios (relative path back to launcher)
        let launcher_path = std::path::Path::new("../home/bin/oxios");
        std::os::unix::fs::symlink(launcher_path, foreign_dir.join("oxios")).unwrap();

        let path_env = format!(
            "{}:{}",
            foreign_dir.to_str().unwrap(),
            home.join("bin").to_str().unwrap()
        );
        let shadows = audit_shadows(&home, &path_env);
        assert!(
            shadows.is_empty(),
            "converged relative symlink must not appear, got: {:?}",
            shadows
        );
    }

    #[test]
    fn migration_plan_splits_symlinks_and_copies() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("oxios-1.43.1");
        std::fs::write(&real, b"x").unwrap();
        let link = tmp.path().join("oxios");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let shadows = vec![
            ShadowEntry {
                path: link.clone(),
                symlink_target: Some(real.clone()),
                size_bytes: 1,
                kind: ShadowKind::Canonical,
            },
            ShadowEntry {
                path: real.clone(),
                symlink_target: None,
                size_bytes: 1,
                kind: ShadowKind::Canonical,
            },
        ];
        let plan = plan_migration(&shadows);
        assert_eq!(plan.repoint, vec![link]);
        assert_eq!(plan.preserve_as_bak.len(), 1);
        assert!(
            plan.preserve_as_bak[0]
                .1
                .to_str()
                .unwrap()
                .contains(".bak.")
        );
    }
    #[test]
    fn render_doctor_report_two_entries_arrow_for_symlink() {
        let shadows = vec![
            ShadowEntry {
                path: PathBuf::from("/usr/local/bin/oxios"),
                symlink_target: Some(PathBuf::from("/Users/me/.oxios/versions/1.44.0/oxios")),
                size_bytes: 12_345,
                kind: ShadowKind::Canonical,
            },
            ShadowEntry {
                path: PathBuf::from("/opt/homebrew/bin/oxios"),
                symlink_target: None,
                size_bytes: 6_789,
                kind: ShadowKind::Canonical,
            },
        ];
        let report = render_doctor_report(&shadows);
        let lines: Vec<&str> = report.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("/usr/local/bin/oxios"));
        assert!(lines[0].contains("->"));
        assert!(lines[0].contains("1.44.0/oxios"));
        assert!(lines[0].contains("12345"));
        assert!(lines[1].contains("/opt/homebrew/bin/oxios"));
        assert!(lines[1].contains("6789"));
        assert!(!lines[1].contains("->"));
    }

    #[test]
    fn doctor_cleanup_targets_excludes_symlinks_and_plain_files_includes_baks() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("oxios-1.43.1");
        std::fs::write(&real, b"x").unwrap();
        let link = tmp.path().join("oxios");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let plain = tmp.path().join("plain-oxios");
        std::fs::write(&plain, b"y").unwrap();
        let shadows = vec![
            ShadowEntry {
                path: link.clone(),
                symlink_target: Some(real.clone()),
                size_bytes: 1,
                kind: ShadowKind::Canonical,
            },
            ShadowEntry {
                path: plain.clone(),
                symlink_target: None,
                size_bytes: 1,
                kind: ShadowKind::Canonical,
            },
        ];
        let targets = doctor_cleanup_targets(&shadows);
        // Symlink is excluded (installer repoints it, doesn't delete).
        assert!(!targets.contains(&link));
        // Plain-file Canonical shadow is NOT a direct deletion target —
        // deleting it could remove the user's only `oxios` on PATH. The
        // migration renames it to .bak.<date> first; only that .bak is
        // reclaimable.
        assert!(!targets.contains(&plain));
        // The .bak.<date> projection from plan_migration is reclaimable.
        assert!(targets.iter().any(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("plain-oxios.bak."))
        }));
    }

    #[test]
    fn disk_precheck_passes_in_tempdir() {
        let tmp = tempfile::tempdir().unwrap();
        // 1 byte must be available in any writable tempdir
        assert!(disk_precheck(tmp.path(), 1).is_ok());
        // 8 EiB is never available
        assert!(disk_precheck(tmp.path(), u64::MAX).is_err());
    }

    #[test]
    fn audit_includes_versioned_copies_and_backup_renames() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let dir = tmp.path().join("p");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(home.join("bin")).unwrap();
        // canonical + versioned + backup siblings
        std::fs::write(dir.join("oxios"), vec![0u8; 4]).unwrap();
        std::fs::write(dir.join("oxios-1.42.0-livestream"), vec![0u8; 5]).unwrap();
        std::fs::write(dir.join("oxios-1.43.1"), vec![0u8; 6]).unwrap();
        std::fs::write(dir.join("oxios.bak.20260101"), vec![0u8; 7]).unwrap();
        // out-of-scope: should NOT be picked up
        std::fs::write(dir.join("oxios-migrate-vault"), vec![0u8; 8]).unwrap();
        std::fs::write(dir.join("oxios.sh"), vec![0u8; 9]).unwrap();
        let path_env = format!(
            "{}:{}",
            dir.to_str().unwrap(),
            home.join("bin").to_str().unwrap()
        );
        let shadows = audit_shadows(&home, &path_env);
        let paths: Vec<_> = shadows
            .iter()
            .map(|s| {
                (
                    s.path.file_name().unwrap().to_str().unwrap().to_string(),
                    s.kind,
                )
            })
            .collect();
        assert!(paths.contains(&("oxios".to_string(), ShadowKind::Canonical)));
        assert!(paths.contains(&("oxios-1.42.0-livestream".to_string(), ShadowKind::Versioned)));
        assert!(paths.contains(&("oxios-1.43.1".to_string(), ShadowKind::Versioned)));
        assert!(paths.contains(&("oxios.bak.20260101".to_string(), ShadowKind::Backup)));
        // Negative matches — these are not "oxios" copies, the audit must skip them.
        assert!(!paths.iter().any(|(n, _)| n == "oxios-migrate-vault"));
        assert!(!paths.iter().any(|(n, _)| n == "oxios.sh"));
    }

    #[test]
    fn audit_classifies_pre_oxios_aside_as_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let dir = tmp.path().join("p");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(home.join("bin")).unwrap();
        // A migration aside: apply_migration_plan renamed an old symlink here.
        std::os::unix::fs::symlink("/nonexistent/target", dir.join("oxios.pre-oxios")).unwrap();
        let path_env = dir.to_str().unwrap().to_string();
        let shadows = audit_shadows(&home, &path_env);
        let aside = shadows
            .iter()
            .find(|s| s.path.file_name().unwrap() == "oxios.pre-oxios");
        assert!(aside.is_some(), ".pre-oxios aside must be audited");
        assert_eq!(aside.unwrap().kind, ShadowKind::Backup);
        // And doctor --cleanup must reclaim it (it's our own litter).
        let targets = doctor_cleanup_targets(&shadows);
        assert!(targets.iter().any(|p| p.ends_with("oxios.pre-oxios")));
    }

    #[test]
    fn install_rejects_non_semver_version_dirs() {
        // F7: a non-SemVer tag (e.g. `1.45.0-rc.1`) would install fine but
        // silently disable prune's launcher protection and rollback, since
        // both rely on parse_version_dir recognizing the directory.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        // Minimal single-member tar.gz containing just `oxios`.
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "oxios", std::io::Cursor::new(b"bin\n"))
            .unwrap();
        let tar = builder.into_inner().unwrap();
        let mut gz = Vec::new();
        {
            let mut enc = flate2::write::GzEncoder::new(&mut gz, flate2::Compression::default());
            std::io::Write::write_all(&mut enc, &tar).unwrap();
            enc.finish().unwrap();
        }
        for bad in ["1.45.0-rc.1", "1.45", "abc", "../escape"] {
            let err = install_version_bytes(&home, bad, &gz).unwrap_err();
            assert!(
                format!("{err}").contains("non-SemVer"),
                "version {bad:?} must be rejected, got: {err}"
            );
        }
    }
    #[test]
    fn cleanup_targets_includes_versioned_and_backup() {
        let dir = std::env::temp_dir();
        let canonical_symlink = dir.join("oxios-cleanup-test-canonical");
        let versioned = dir.join("oxios-cleanup-test-1.42.0");
        let backup = dir.join("oxios-cleanup-test.bak.20260101");
        let _ = std::fs::remove_file(&canonical_symlink);
        let _ = std::fs::remove_file(&versioned);
        let _ = std::fs::remove_file(&backup);
        std::fs::write(&versioned, vec![0u8; 5]).unwrap();
        std::fs::write(&backup, vec![0u8; 7]).unwrap();
        std::os::unix::fs::symlink(&versioned, &canonical_symlink).unwrap();
        let shadows = vec![
            ShadowEntry {
                path: canonical_symlink.clone(),
                symlink_target: Some(versioned.clone()),
                size_bytes: 5,
                kind: ShadowKind::Canonical,
            },
            ShadowEntry {
                path: versioned.clone(),
                symlink_target: None,
                size_bytes: 5,
                kind: ShadowKind::Versioned,
            },
            ShadowEntry {
                path: backup.clone(),
                symlink_target: None,
                size_bytes: 7,
                kind: ShadowKind::Backup,
            },
        ];
        let targets = doctor_cleanup_targets(&shadows);
        assert!(
            targets.contains(&versioned),
            "versioned {versioned:?} must be a target"
        );
        assert!(
            targets.contains(&backup),
            "backup {backup:?} must be a target"
        );
        // Canonical symlink stays excluded — installer repoints it.
        assert!(!targets.contains(&canonical_symlink));
        // cleanup
        let _ = std::fs::remove_file(&versioned);
        let _ = std::fs::remove_file(&backup);
        let _ = std::fs::remove_file(&canonical_symlink);
    }
}
