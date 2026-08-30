# Managed Install Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the cargo-only binary update with a rustup-model managed install (version store + launcher symlink + channel-aware updater + PATH shadow audit) and ship a curl|sh installer.

**Architecture:** New `src/managed_install.rs` owns the store (install/flip/prune), channel classification, tarball fetch/verify/extract, and shadow audit. `run_update` dispatches on the detected channel. `DaemonManager` gains an exe override so daemon restarts spawn the launcher path, not the stale resolved binary (root cause of the 8/26 incident). `share/install.sh` bootstraps.

**Tech Stack:** Rust (tokio, reqwest, sha2, tar, flate2 — all already workspace deps), POSIX sh for the installer.

**Spec:** `docs/designs/2026-08-27-managed-install-design.md`

## Global Constraints

- Target: macOS ARM64 only (`aarch64-apple-darwin`). No cross-platform code.
- Release asset names: `oxios-aarch64-apple-darwin.tar.gz` and `.tar.gz.sha256`.
- Canonical GitHub repo constant: `project-oxi/oxios` (single constant; the `a7garden/oxios` literals in `update.rs` are legacy aliases — replace at touched sites).
- Keep-2 version retention (current + previous); never delete the version the launcher targets.
- All CLI-facing strings English (structural/tool output rule, AGENTS.md).
- Workspace lints: `unwrap_used` warn (use `?`/`expect`), `missing_docs` warn on public crates — this file is in the binary crate, document `pub` items anyway.
- Tests: `#[cfg(test)] mod tests` in-file, `tempfile` for fs fixtures (dev-dep already present).
- Commits: `feat(cli): …` / `feat(kernel): …` / `test(cli): …` scopes per repo convention.
- Gates before done: `cargo fmt --all`, `cargo clippy --workspace --all-features --all-targets -- -D warnings`, `cargo nextest run --workspace --all-features --no-fail-fast`.

---

### Task 1: `src/managed_install.rs` — channel classification + store layout (pure)

**Files:**
- Create: `src/managed_install.rs`
- Modify: `src/main.rs` (add `mod managed_install;` next to `mod web_dist;`)

**Interfaces:**
- Produces (used by Tasks 2–7):
  - `pub const GITHUB_REPO: &str = "project-oxi/oxios";`
  - `pub const ASSET_BASE: &str = "oxios-aarch64-apple-darwin.tar.gz";`
  - `pub enum Channel { Managed, Brew, Cargo, Dev, Unmanaged }`
  - `pub fn classify_channel(exe: &Path, oxios_home: &Path) -> Channel`
  - `pub fn versions_dir(oxios_home: &Path) -> PathBuf` → `<home>/versions`
  - `pub fn launcher_path(oxios_home: &Path) -> PathBuf` → `<home>/bin/oxios`
  - `pub fn parse_version_dir(name: &str) -> Option<(u64,u64,u64)>` (strict SemVer, no leading `v`)

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_managed_paths() {
        let home = std::path::Path::new("/fake/home");
        assert!(matches!(classify_channel(Path::new("/fake/home/versions/1.44.0/oxios"), home), Channel::Managed));
        assert!(matches!(classify_channel(Path::new("/fake/home/bin/oxios"), home), Channel::Managed));
    }

    #[test]
    fn classify_foreign_channels() {
        let home = std::path::Path::new("/fake/home");
        assert!(matches!(classify_channel(Path::new("/opt/homebrew/bin/oxios"), home), Channel::Brew));
        assert!(matches!(classify_channel(Path::new("/usr/local/Cellar/oxios/1.44.0/bin/oxios"), home), Channel::Brew));
        assert!(matches!(classify_channel(Path::new("/Users/w/.cargo/bin/oxios"), home), Channel::Cargo));
    }

    #[test]
    fn classify_dev_build_inside_repo_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("crates/oxios-kernel")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"crates/oxios-kernel\"]\n").unwrap();
        let target = root.join("target/release/oxios");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"bin").unwrap();
        // cargo build artifacts live inside the repo that defines the workspace
        assert!(matches!(classify_channel(&target, Path::new("/fake/home")), Channel::Dev));
    }

    #[test]
    fn classify_unmanaged_plain_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("oxios-1.43.1");
        std::fs::write(&bin, b"bin").unwrap();
        assert!(matches!(classify_channel(&bin, Path::new("/fake/home")), Channel::Unmanaged));
    }

    #[test]
    fn version_dir_parsing() {
        assert_eq!(parse_version_dir("1.44.0"), Some((1, 44, 0)));
        assert_eq!(parse_version_dir("v1.44.0"), None);
        assert_eq!(parse_version_dir("1.44"), None);
        assert_eq!(parse_version_dir("1.44.0-livestream"), None);
    }
}
```

- [ ] **Step 2: Run to verify failure** — `cargo nextest run -p oxios managed_install` → compile error (module missing).
- [ ] **Step 3: Implement** — `classify_channel` canonicalizes `exe` (`std::fs::canonicalize`, fall back to the input path on error). Order: Managed (starts_with `home/versions` or `home/bin`) → Brew (`/opt/homebrew/` or `/usr/local/Cellar/` prefix) → Cargo (`$CARGO_HOME/bin` or `$HOME/.cargo/bin` prefix) → Dev (walk ancestors; an ancestor containing `Cargo.toml` AND `crates/oxios-kernel/Cargo.toml` relative to it) → Unmanaged.
- [ ] **Step 4: Run tests** — `cargo nextest run -p oxios managed_install` → PASS.
- [ ] **Step 5: Commit** — `git add src/managed_install.rs src/main.rs && git commit -m "feat(cli): managed install channel classification"`

### Task 2: Store operations — install bytes, atomic flip, prune, rollback pick

**Files:**
- Modify: `src/managed_install.rs`

**Interfaces:**
- Produces:
  - `pub fn install_version_bytes(oxios_home: &Path, version: &str, tar_gz: &[u8]) -> Result<PathBuf>` (extracts the single `oxios` member, sets mode 0755, writes `<home>/versions/<v>/oxios` via temp+rename)
  - `pub fn flip_launcher(oxios_home: &Path, version: &str) -> Result<()>` (temp symlink `bin/.oxios.tmp` → `versions/<v>/oxios`, then `fs::rename` over `bin/oxios` — atomic)
  - `pub fn prune_versions(oxios_home: &Path, keep: usize) -> Result<Vec<String>>` (returns removed dir names; never removes the launcher's current target)
  - `pub fn current_target_version(oxios_home: &Path) -> Option<String>` (readlink of launcher, take parent dir name)
  - `pub fn rollback_target(oxios_home: &Path) -> Option<String>` (highest installed version `< current`)

- [ ] **Step 1: Failing tests** (in `mod tests`; build a fixture tarball in-memory with `flate2::write::GzEncoder` + `tar::Builder` — both workspace deps):

```rust
fn fixture_tar_gz() -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(4);
    header.set_mode(0o755);
    header.set_cksum();
    builder.append_data(&mut header, "oxios", std::io::Cursor::new(b"bin\n")).unwrap();
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
    assert_eq!(removed, vec!["1.42.0".to_string()]);          // keep 1.43.0 + 1.44.0
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
    flip_launcher(home, "1.40.0").unwrap();                    // old but live
    let removed = prune_versions(home, 2).unwrap();
    assert!(!removed.contains(&"1.40.0".to_string()));
    assert!(home.join("versions/1.40.0/oxios").is_file());
}
```

- [ ] **Step 2: Run** — expect compile errors (functions missing).
- [ ] **Step 3: Implement.** Extraction detail: iterate `tar::Archive` entries; accept a member whose path is exactly `oxios` (the tarball contains one entry); reject extra members (`anyhow::bail!("unexpected tar member …")`). Set mode via `fs::set_permissions(0o755)`. Flip must be unix `std::os::unix::fs::symlink` (macOS-only target — acceptable).
- [ ] **Step 4: Run tests** — PASS.
- [ ] **Step 5: Commit** — `feat(cli): managed install store ops (install/flip/prune/rollback)`

### Task 3: Tarball fetch + sha256 verify + disk precheck

**Files:**
- Modify: `src/managed_install.rs`
- Modify: `Cargo.toml` (add to `[dependencies]`: `tar = { workspace = true }`, `flate2 = { workspace = true }` — sha2/reqwest already deps)

**Interfaces:**
- Produces:
  - `pub struct ReleaseInfo { pub tag: String, /* "1.44.0" no leading v */ pub html_url: String, pub body: String }`
  - `pub async fn fetch_release(version: Option<&str>) -> Result<ReleaseInfo>` (GitHub API, `GITHUB_REPO`; reqwest client with the same UA/timeout pattern as `update.rs:92-97`)
  - `pub async fn fetch_tarball(version: &str) -> Result<Vec<u8>>` (downloads `{ASSET_BASE}` and `{ASSET_BASE}.sha256` from the release; verifies lowercase-hex sha256 before returning; on mismatch bail + drop bytes)
  - `pub fn disk_precheck(dir: &Path, needed_bytes: u64) -> Result<()>` (free space via `nix`-free statfs? — no new deps: use `libc::statfs` (libc is a dep) — `f_bavail * f_bsize >= needed`; error message includes both numbers)
  - `pub fn sha256_hex(data: &[u8]) -> String` (sha2, hex — hex crate is a dep)
- [ ] **Step 1: Failing tests** (pure parts only; network fns get thin wrappers tested via Task 6's dry-run test):

```rust
#[test]
fn sha_verify_rejects_mismatch() {
    let data = b"payload".to_vec();
    let good = format!("{}  {}\n", sha256_hex(&data), ASSET_BASE);
    assert!(verify_sha256(&data, &good).is_ok());
    let bad = format!("{}  {}\n", "0".repeat(64), ASSET_BASE);
    assert!(verify_sha256(&data, &bad).is_err());
}

#[test]
fn disk_precheck_passes_in_tempdir() {
    let tmp = tempfile::tempdir().unwrap();
    // 1 byte must be available in any writable tempdir
    assert!(disk_precheck(tmp.path(), 1).is_ok());
    // 8 EiB is never available
    assert!(disk_precheck(tmp.path(), u64::MAX).is_err());
}
```

- [ ] **Step 2: Run** — compile fail. **Step 3: Implement** `verify_sha256` as `pub` (used by fetch_tarball), plus the rest. **Step 4: PASS.**
- [ ] **Step 5: Commit** — `feat(cli): release tarball fetch with sha256 gate + disk precheck`

### Task 4: PATH shadow audit + migration plan

**Files:**
- Modify: `src/managed_install.rs`

**Interfaces:**
- Produces:
  - `pub struct ShadowEntry { pub path: PathBuf, pub symlink_target: Option<PathBuf>, pub size_bytes: u64 }`
  - `pub fn audit_shadows(oxios_home: &Path, path_env: &str) -> Vec<ShadowEntry>` (dedup entries preserving PATH order; skip `home/bin`; stat `<dir>/oxios` when it exists; size of the *resolved* file)
  - `pub struct MigrationPlan { pub repoint: Vec<PathBuf>, pub preserve_as_bak: Vec<(PathBuf, PathBuf)> }`
  - `pub fn plan_migration(shadows: &[ShadowEntry]) -> MigrationPlan` (symlinks → repoint; regular files → `(<path>, <path>.bak.<YYYYMMDD>)`)

- [ ] **Step 1: Failing tests:**

```rust
#[test]
fn audit_reports_shadows_in_path_order_excluding_home() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let dir_a = tmp.path().join("a"); std::fs::create_dir_all(&dir_a).unwrap();
    let dir_b = tmp.path().join("b"); std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(dir_a.join("oxios"), vec![0u8; 10]).unwrap();
    std::fs::write(dir_b.join("oxios"), vec![0u8; 20]).unwrap();
    std::fs::create_dir_all(home.join("bin")).unwrap();
    std::fs::write(home.join("bin/oxios"), vec![0u8; 5]).unwrap();
    let path_env = format!("{:?}:{:?}", dir_a, dir_b); // no separators needed for the pure fn
    let shadows = audit_shadows(&home, &format!("{}:{:?}", dir_a.to_str().unwrap(), home.join("bin").to_str().unwrap()));
    // first entry of path_env is dir_a only in this test's second call shape —
    // simpler: pass "dirA:dirB" and expect both, in order
    let shadows = audit_shadows(&home, &format!("{}:{}", dir_a.to_str().unwrap(), dir_b.to_str().unwrap()));
    assert_eq!(shadows.len(), 2);
    assert_eq!(shadows[0].path, dir_a.join("oxios"));
}

#[test]
fn migration_plan_splits_symlinks_and_copies() {
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("oxios-1.43.1"); std::fs::write(&real, b"x").unwrap();
    let link = tmp.path().join("oxios"); std::os::unix::fs::symlink(&real, &link).unwrap();
    let shadows = vec![
        ShadowEntry { path: link.clone(), symlink_target: Some(real.clone()), size_bytes: 1 },
        ShadowEntry { path: real.clone(), symlink_target: None, size_bytes: 1 },
    ];
    let plan = plan_migration(&shadows);
    assert_eq!(plan.repoint, vec![link]);
    assert_eq!(plan.preserve_as_bak.len(), 1);
    assert!(plan.preserve_as_bak[0].1.to_str().unwrap().contains(".bak."));
}
```

(Clean the first test to a single `audit_shadows` call + assertion set before implementing — the sketch above double-calls; the implementer should keep the final two-line version.)

- [ ] **Step 2: Run** — fail. **Step 3: Implement.** `symlink_target` via `fs::read_link` (Ok only). `plan_migration` uses `chrono::Local` (dep present) for the date suffix. **Step 4: PASS.**
- [ ] **Step 5: Commit** — `feat(cli): PATH shadow audit + migration planner`

### Task 5: `DaemonManager::with_exe` — restart via launcher path

**Files:**
- Modify: `crates/oxios-kernel/src/daemon.rs` (add builder; replace both `std::env::current_exe()` call sites at lines ~160 and ~394 with the override when set)
- Modify: `src/main.rs:1940-1967` (update flow: when channel is Managed, build the manager with `.with_exe(managed_install::launcher_path(&oxios_home))`)

**Interfaces:**
- Produces: `impl DaemonManager { pub fn with_exe(mut self, exe: PathBuf) -> Self }` (field `exe_override: Option<PathBuf>` defaulting `None`; `None` keeps `current_exe()` behavior — all existing callers unchanged)
- Consumes: `managed_install::launcher_path` (Task 1)

- [ ] **Step 1: Failing test** in `daemon.rs` `#[cfg(test)]`:

```rust
#[test]
fn with_exe_overrides_spawn_target() {
    let dm = DaemonManager::new("/tmp/oxios-with-exe-test.pid", "/tmp")
        .with_exe(std::path::PathBuf::from("/nonexistent/launcher"));
    assert_eq!(dm.exe_override.as_deref(), Some(std::path::Path::new("/nonexistent/launcher")));
    // spawn would fail on nonexistent path; the unit pins the plumbing only
}
```

- [ ] **Step 2: Run** — `cargo nextest run -p oxios-kernel with_exe` → fail.
- [ ] **Step 3: Implement** — private field + builder; `start()` and `install_service()` use `self.exe_override.clone().map_or_else(|| std::env::current_exe(), Ok)`.
- [ ] **Step 4: PASS** (kernel suite green).
- [ ] **Step 5: Commit** — `feat(kernel): DaemonManager exe override for launcher-path restarts`

### Task 6: CLI flags + `run_update` channel dispatch

**Files:**
- Modify: `src/cli.rs:181-205` (Update variant gains `via: Option<String>`, `adopt: bool`, `rollback: bool`)
- Modify: `src/commands/update.rs` (rewrite binary-update section, lines 186-251)
- Modify: `src/main.rs:1920-1934` (pass `oxios_home`, new args; destructuring update)

**Interfaces:**
- Consumes: Tasks 1–5 (`classify_channel`, `install_version_bytes`, `flip_launcher`, `prune_versions`, `rollback_target`, `fetch_release`, `fetch_tarball`, `disk_precheck`, `audit_shadows`, `plan_migration`, `launcher_path`)
- Produces: `run_update(web_only, binary_only, version, dry_run, yes, no_restart→kept-in-main, oxios_home: &Path, via: Via, adopt: bool, rollback: bool) -> Result<UpdateOutcome>` where `pub enum Via { Auto, Cargo }`
  - main.rs already gates restart on `outcome.any()`; Managed restart uses Task 5 override.

Dispatch (replaces the `cargo install` block):

```rust
// pseudo-shape of the new section; real code mirrors the existing print/style helpers
match classify_channel(&std::env::current_exe()?, oxios_home) {
    Channel::Brew => { println!("Installed via Homebrew — run `brew upgrade oxios`."); return unchanged; }
    Channel::Dev => { bail!("refusing to update a dev build ({exe:?}) — rebuild with cargo build"); }
    Channel::Managed | Channel::Cargo | Channel::Unmanaged if via == Via::Cargo => { /* legacy cargo install block, moved verbatim */ }
    Channel::Managed | Channel::Unmanaged | Channel::Cargo => {
        let tar = fetch_tarball(&tag_name).await?;
        disk_precheck(&versions_dir(oxios_home), tar.len() as u64 * 3)?;
        install_version_bytes(oxios_home, &tag_name, &tar)?;
        flip_launcher(oxios_home, &tag_name)?;
        let removed = prune_versions(oxios_home, 2)?;
        outcome.binary_updated = true;
        // migration for Unmanaged/Cargo: audit + plan
        let shadows = audit_shadows(oxios_home, &std::env::var("PATH").unwrap_or_default());
        if !shadows.is_empty() {
            let plan = plan_migration(&shadows);
            // TTY && (yes || prompt) or adopt → apply: fs::rename each repoint symlink
            //   to `<path>.pre-oxios` then symlink to launcher; rename each preserve_as_bak.
            // Non-TTY without adopt → print exact commands, warn shadow may win PATH.
        }
    }
}
```

Rollback: `if rollback { match rollback_target(home) { Some(v) => { flip_launcher(home,&v)?; outcome.binary_updated=true; } None => bail!("no previous version to roll back to") } }` — runs before the fetch path; `--rollback` skips release fetch entirely.

Dry-run prints: channel, planned install dir, flip target, prune candidates, migration plan (no fs mutation).

- [ ] **Step 1: Failing tests** (update.rs `mod tests`):
  - `dry_run_managed_prints_plan`: tempdir home, set `PATH` to include a tempdir containing a fake `oxios` file; call `run_update(true /*binary_only semantics inverted? — use binary_only=true*/, …, dry_run=true, via=Via::Auto, …)`; assert stdout contains `versions/` and `flip` and the shadow path. (Use `std::env::set_var("PATH", …)` — tests run serially in this module; alternatively thread `path_env` through an internal `fn plan_update(...) -> String` pure core and test that; **prefer the pure core**: `fn render_update_plan(channel, tag, home, path_env) -> String` — the async `run_update` becomes a thin shell. Test `render_update_plan` only.)
  - `rollback_picks_previous`: install two fixture versions + flip to newer via Task 2 fns, then `run_update(…, rollback=true, …)` with fetch stubbed out — rollback path must not touch the network; assert `current_target_version` == older.
- [ ] **Step 2: Run** — fail. **Step 3: Implement** per dispatch shape. **Step 4: PASS.**
- [ ] **Step 5: Commit** — `feat(cli): channel-aware oxios update with managed install + rollback`

### Task 7: API route — `via` field + audit payload, no HTTP auto-migrate

**Files:**
- Modify: `src/api/routes/system.rs:327-336` (`UpdateRunBody` gains `pub via: Option<String>` — `"cargo"` accepted, anything else rejected 400) and the handler body to mirror Task 6 dispatch; **unmanaged over HTTP never migrates** — respond with `{"migrated": false, "shadows": [{path, target, size_bytes}...], "instructions": "re-run `oxios update` in a terminal to adopt"}` and `binary: true` semantics unchanged for Managed.

- [ ] **Step 1: Failing test**: extend the existing route tests in `system.rs` (`#[cfg(test)]` bottom of file): POST with `via: "cargo"` hits the cargo branch (assert the spawned args include `install`, mocked via the same seam the current tests use — follow the existing test pattern around `UpdateRunBody`); POST managed returns the shadows array shape.
- [ ] **Step 2: Run** — fail. **Step 3: Implement.** **Step 4: PASS.**
- [ ] **Step 5: Commit** — `feat(web): update route channel awareness + shadow audit payload`

### Task 8: `share/install.sh` + shellcheck gate

**Files:**
- Create: `share/install.sh` (0755)
- Modify: `.github/workflows/ci.yml` (fmt job: add shellcheck step — `command -v shellcheck >/dev/null || brew install shellcheck; shellcheck share/install.sh`)

**Script (complete, this IS the deliverable):**

```sh
#!/bin/sh
# oxios managed installer — macOS ARM64.
# curl -fsSL https://raw.githubusercontent.com/project-oxi/oxios/main/share/install.sh | sh
set -eu

REPO="project-oxi/oxios"
ASSET="oxios-aarch64-apple-darwin.tar.gz"
OXIOS_HOME="${OXIOS_HOME:-$HOME/.oxios}"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) ;;
  *) echo "unsupported platform: $(uname -s) $(uname -m) (macOS ARM64 only)" >&2; exit 1 ;;
esac

tag=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | tr -d '\r')
[ -n "$tag" ] || { echo "failed to resolve latest release" >&2; exit 1; }
ver=${tag#v}

tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
base="https://github.com/$REPO/releases/download/$tag"
curl -fsSL -o "$tmp/$ASSET"        "$base/$ASSET"
curl -fsSL -o "$tmp/$ASSET.sha256" "$base/$ASSET.sha256"
(cd "$tmp" && shasum -a 256 -c "$ASSET.sha256" >/dev/null) || { echo "sha256 mismatch" >&2; exit 1; }

dest="$OXIOS_HOME/versions/$ver"
mkdir -p "$dest" "$OXIOS_HOME/bin"
tar -xzf "$tmp/$ASSET" -C "$dest"
chmod 755 "$dest/oxios"

ln -sfn "$dest/oxios" "$OXIOS_HOME/bin/.oxios.tmp"
mv -f "$OXIOS_HOME/bin/.oxios.tmp" "$OXIOS_HOME/bin/oxios"

# keep-2
( cd "$OXIOS_HOME/versions" && ls -1 | grep -v "^$ver$" | sort -V | sed -e '$d' | while read -r old; do rm -rf "$OXIOS_HOME/versions/$old"; done )

rc="$HOME/.zshrc"; [ -f "$rc" ] || rc="$HOME/.zprofile"
if ! grep -q 'BEGIN oxios (managed)' "$rc" 2>/dev/null; then
  printf '\n# BEGIN oxios (managed)\nexport PATH="%s/bin:\$PATH"\n# END oxios (managed)\n' "$OXIOS_HOME" >> "$rc"
  echo "PATH entry added to $rc — restart your shell or: export PATH=\"$OXIOS_HOME/bin:\$PATH\""
fi

echo "installed oxios $ver -> $OXIOS_HOME/bin/oxios"
echo "shadow check:"
found=0
oldifs=$IFS; IFS=:
for d in $PATH; do
  [ -x "$d/oxios" ] || continue
  case "$(cd "$d" && pwd -P)" in "$OXIOS_HOME/bin") continue ;; esac
  echo "  note: another oxios at $d/oxios (may shadow; run 'oxios doctor' after install)"
  found=1
done
IFS=$oldifs
[ "$found" = 0 ] && echo "  none"
```

- [ ] **Step 1:** `sh -n share/install.sh` (syntax) + `shellcheck share/install.sh` locally (`brew install shellcheck` if absent) — fix findings until clean.
- [ ] **Step 2:** Local smoke against the real v1.44.0 release with `OXIOS_HOME=$HOME/.oxios-smoke` (throwaway home; delete after): run script, assert `~/.oxios-smoke/bin/oxios --version` prints `oxios 1.44.0`, then `rm -rf ~/.oxios-smoke`.
- [ ] **Step 3: Commit** — `feat(cli): curl|sh managed installer + shellcheck CI gate`

### Task 9: `oxios doctor` integration — shadow report + `--cleanup`

**Files:**
- Modify: `src/main.rs` (doctor command; locate `Command::Doctor` handler) and/or its command module — report `audit_shadows` table; `--cleanup` flag (cli.rs Doctor variant): applies migration-plan deletions for `.bak.*` and stale versioned copies after a TTY confirm; non-TTY prints `rm` lines.

- [ ] **Step 1: Failing test** — pure fn `render_doctor_report(shadows: &[ShadowEntry]) -> String` in `managed_install.rs`: table line per shadow `path -> target (size)`. Test: two entries → two lines, arrow rendered for symlink, plain for file.
- [ ] **Step 2/3/4:** fail → implement → pass.
- [ ] **Step 5: Commit** — `feat(cli): doctor shadow report + cleanup`

### Task 10: Docs, changelog, gates, real-machine smoke

**Files:**
- Modify: `docs/USER-GUIDE.md` (Install/Update section: curl installer primary; brew/cargo alternatives; `oxios update --rollback`; `oxios doctor --cleanup`)
- Modify: `CHANGELOG.md` (new `## [Unreleased]` block above 1.44.0: Added — managed install, installer, audit, rollback; Changed — update binary strategy cargo → release tarball with `--via cargo` fallback)
- [ ] Write docs; [ ] append changelog; [ ] commit `docs: managed install user guide + changelog`
- [ ] **Gates**: `cargo fmt --all && cargo clippy --workspace --all-features --all-targets -- -D warnings && cargo nextest run --workspace --all-features --no-fail-fast` — all green.
- [ ] **Real-machine smoke (this Mac, in order):**
  1. `share/install.sh` → `oxios --version` shows new build from `~/.oxios/bin`
  2. Migrate the real `~/bin/oxios`: `ln -sfn ~/.oxios/bin/oxios ~/bin/oxios`; keep `~/bin/oxios-1.43.1` as the rollback copy; `oxios doctor` reports remaining livestream copies
  3. `oxios update --dry-run` → channel=Managed, plan prints
  4. After next release tag exists: `oxios update` → flip + daemon restart via launcher (verify `ps` shows `~/.oxios/bin/oxios`), `oxios update --rollback` returns to previous
  5. `oxios doctor --cleanup` clears `.bak`/livestream copies (~700 MB reclaim) after confirm
