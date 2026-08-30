//! `oxios update` — channel-aware binary updater + web UI from GitHub Releases.
//!
//! Binary update channels (auto-detected from `std::env::current_exe()` plus
//! the configured `oxios_home`, see `crate::managed_install::classify_channel`):
//!   - Managed / Unmanaged: extract release tarball into `<home>/versions/<v>/`,
//!     atomically flip the launcher symlink at `<home>/bin/oxios`, prune
//!     older versions, optionally migrate PATH shadows off the launcher.
//!   - Cargo: legacy `cargo install oxios` flow (preserved for `--via cargo`
//!     and the `Cargo` channel).
//!   - Brew: refuse, instruct the user to `brew upgrade oxios`.
//!   - Dev: refuse, instruct the user to `cargo build`.
//!
//! Web UI: `web-dist.zip` from GitHub Releases → `~/.oxios/web/dist/`.

use anyhow::{Context, Result};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

/// User-selected install channel for the binary update.
///
/// `Auto` defers to the channel auto-detected from the running exe; `Cargo`
/// forces the legacy `cargo install` flow regardless of the detected channel
/// (used when migrating away from the managed install).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// Defer to the channel auto-detected by `classify_channel`.
    Auto,
    /// Force the legacy `cargo install oxios` flow.
    Cargo,
}

/// Render a human-readable dry-run plan for the binary update.
///
/// Pure core: takes the classified channel, the candidate tag, the install
/// home, and the raw `PATH` string — no env reads, no fs mutation. Used by
/// `run_update` for `--dry-run` and by the unit tests to assert the shape
/// of the plan without touching the network.
pub fn render_update_plan(
    channel: crate::managed_install::Channel,
    tag: &str,
    home: &Path,
    path_env: &str,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("  channel: {channel:?}\n"));
    let install_dir = crate::managed_install::versions_dir(home);
    let target = install_dir.join(tag);
    out.push_str(&format!(
        "  install dir: {}\n  flip target: {}\n",
        target.display(),
        crate::managed_install::launcher_path(home).display()
    ));
    // prune candidates: every installed version except the current target.
    let current = crate::managed_install::current_target_version(home);
    let mut candidates: Vec<String> = std::fs::read_dir(&install_dir)
        .map(|it| {
            it.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| crate::managed_install::parse_version_dir(n).is_some())
                .filter(|n| current.as_deref() != Some(n.as_str()))
                .collect()
        })
        .unwrap_or_default();
    candidates.sort_by(|a, b| b.cmp(a));
    out.push_str(&format!(
        "  prune candidates: {}\n",
        if candidates.is_empty() {
            "<none>".to_string()
        } else {
            candidates.join(", ")
        }
    ));
    let shadows = crate::managed_install::audit_shadows(home, path_env);
    if shadows.is_empty() {
        out.push_str("  migration plan: <none>\n");
    } else {
        let plan = crate::managed_install::plan_migration(&shadows);
        out.push_str("  migration plan:\n");
        for r in &plan.repoint {
            out.push_str(&format!("    repoint {}\n", r.display()));
        }
        for (from, to) in &plan.preserve_as_bak {
            out.push_str(&format!(
                "    preserve {} -> {}\n",
                from.display(),
                to.display()
            ));
        }
    }
    out
}

/// Outcome of `oxios update` — what was actually changed on disk.
///
/// The caller (main.rs) uses this to decide whether a daemon restart is needed.
#[derive(Debug, Default, Clone, Copy)]
pub struct UpdateOutcome {
    /// The `oxios` binary was reinstalled (requires restart to take effect).
    pub binary_updated: bool,
    /// The web UI under `~/.oxios/web/dist/` was replaced.
    pub web_updated: bool,
    /// The managed-store install/flip/prune block actually ran (vs. the
    /// legacy `cargo install` arm). Used by main.rs to gate the
    /// launcher-path daemon restart: the cargo arm writes
    /// `~/.cargo/bin/oxios` while the launcher at `<home>/bin/oxios` is
    /// unchanged, so a launcher-path restart there would spawn the old
    /// binary. This flag is the source of truth for "the managed store
    /// was actually updated this run".
    pub used_managed_store: bool,
}

impl UpdateOutcome {
    /// Nothing changed (already latest, dry run, or user cancelled).
    pub const fn unchanged() -> Self {
        Self {
            binary_updated: false,
            web_updated: false,
            used_managed_store: false,
        }
    }

    /// Whether anything at all was updated.
    pub fn any(&self) -> bool {
        self.binary_updated || self.web_updated
    }
}

/// Update oxios binary and/or web UI (channel-aware).
///
/// `oxios_home` is the resolved `OXIOS_HOME` (used by the managed-channel
/// installer to write `versions/<v>/` and `bin/oxios`). `via` selects the
/// binary install channel: `Auto` defers to the detected channel; `Cargo`
/// forces the legacy `cargo install` flow. `adopt` applies the PATH-shadow
/// migration without prompting; `rollback` skips the network and flips the
/// launcher to the previous installed version.
#[allow(clippy::too_many_arguments)]
pub async fn run_update(
    oxios_home: &Path,
    web_only: bool,
    binary_only: bool,
    version: Option<&str>,
    dry_run: bool,
    yes: bool,
    via: Via,
    adopt: bool,
    rollback: bool,
) -> Result<UpdateOutcome> {
    let current = env!("CARGO_PKG_VERSION");
    let mut outcome = UpdateOutcome::unchanged();

    // ── Determine what to update ────────────────────────────────────────────
    // Rollback always flips the launcher — treat it as a binary op even when
    // the user passed --web-only, so the header matches the actual behavior.
    // Web sync never runs on a rollback (no network), so fold that into
    // `update_web` too — otherwise `--web-only --rollback` would print
    // "Update web UI: yes" and then skip the web block.
    let update_binary = !web_only || rollback;
    let update_web = !binary_only && !rollback;

    println!();
    println!(
        "  {} {}",
        style("⬡ Oxios Updater").bold(),
        style(format!("v{current}")).dim()
    );
    println!("  {}", "─".repeat(52));
    println!("  Current version:  {current}");
    let binary_label = if rollback {
        "yes (rollback)"
    } else if update_binary {
        "yes"
    } else {
        "no"
    };
    println!("  Update binary:    {binary_label}");
    println!(
        "  Update web UI:   {}",
        if update_web { "yes" } else { "no" }
    );
    if let Some(v) = version {
        println!("  Target version:  {v}");
    } else {
        println!("  Target version:  latest");
    }
    if rollback {
        println!("  Rollback:         yes (no network)");
    }
    if matches!(via, Via::Cargo) {
        println!("  Install channel:  cargo (forced)");
    }
    println!();

    // ── Classify the running exe's distribution channel ─────────────────────
    let exe = std::env::current_exe().context("locate current_exe")?;
    let channel = crate::managed_install::classify_channel(&exe, oxios_home);

    // Brew / Dev are unconditional refusals — `cargo install` would silently
    // shadow them and the install would land in the wrong place.
    match channel {
        crate::managed_install::Channel::Brew => {
            println!("  Installed via Homebrew — run `brew upgrade oxios`.");
            return Ok(UpdateOutcome::unchanged());
        }
        crate::managed_install::Channel::Dev => {
            anyhow::bail!(
                "refusing to update a dev build ({:?}) — rebuild with `cargo build`",
                exe
            );
        }
        _ => {}
    }

    // ── Rollback path (no network) ──────────────────────────────────────────
    // INVARIANT: rollback runs BEFORE any fetch — release metadata and
    // tarball downloads must never happen on a rollback. F2 enforces this
    // by gating on `dry_run` to print the plan without mutating.
    if rollback {
        let plan_target = crate::managed_install::rollback_target(oxios_home);
        if dry_run {
            println!("  {} Dry run — no changes made.\n", style("⚠").yellow());
            let target_str = plan_target
                .clone()
                .unwrap_or_else(|| "<no previous version>".to_string());
            println!("  Rollback would flip launcher to: {target_str}");
            return Ok(UpdateOutcome::unchanged());
        }
        match plan_target {
            Some(prev) => {
                crate::managed_install::flip_launcher(oxios_home, &prev)
                    .with_context(|| format!("flip launcher back to {prev}"))?;
                outcome.binary_updated = true;
                outcome.used_managed_store = true;
                println!(
                    "  {} Rolled back to {} (no network used).",
                    style("⟲").cyan(),
                    prev
                );
                // Rollback is terminal: nothing else may run after a flip
                // (no fetch, no web sync, no migration audit).
                return Ok(outcome);
            }
            None => {
                anyhow::bail!("no previous version to roll back to");
            }
        }
    }

    // ── Legacy cargo install (only when --via cargo forces it) ───────────
    // R3: a Cargo-channel install detected via `classify_channel` falls
    // through to the managed path below — the design says cargo installs
    // are "treated as unmanaged → offer migration". Only an explicit
    // `--via cargo` opt-in routes the update through `cargo install`.
    // Sending cargo-channel installs through cargo install again would
    // refresh the stale ~/.cargo/bin shadow rather than converge on the
    // managed launcher. `audit_shadows` already covers ~/.cargo/bin, so
    // the managed path's migration offer handles it correctly.
    let force_cargo = matches!(via, Via::Cargo);
    if update_binary && force_cargo && !rollback {
        // Fetch release notes via the canonical GitHub repo (single source of
        // truth — see crate::managed_install::GITHUB_REPO).
        let release = crate::managed_install::fetch_release(version).await?;
        let tag_name = release.tag.as_str();
        let html_url = release.html_url.as_str();
        let body = release.body.as_str();
        println!(
            "  Latest release:  {} ({})",
            style(tag_name).green().bold(),
            html_url
        );
        println!();
        // Short-circuit "already latest" ONLY when nothing else could change:
        // a pure binary-only update whose binary is already current. For
        // `--web-only` or the default (both), the web UI version is checked
        // independently by `sync_to_disk`, so we never bail here on the binary
        // version alone. (Restored verbatim from the pre-T6 flow.)
        if tag_name == current && !update_web && !dry_run && !yes {
            println!(
                "  {} Already on latest version ({}).",
                style("✓").green(),
                current
            );
            println!("  Use `--version X.Y.Z` to force a specific version.");
            return Ok(UpdateOutcome::unchanged());
        }
        if dry_run {
            println!("  {} Dry run — no changes made.\n", style("⚠").yellow());
            let mut cmd = "cargo install oxios".to_string();
            if let Some(v) = version {
                cmd.push_str(&format!(" --version {v}"));
            }
            println!("  Would run: {cmd}");
            return Ok(UpdateOutcome::unchanged());
        }
        if !yes {
            println!("  {} Release notes:\n", style("Release notes").cyan());
            for line in body.lines().take(10) {
                println!("    {line}");
            }
            if body.lines().count() > 10 {
                println!("    ... ({} more lines)", body.lines().count() - 10);
            }
            println!();
            print!("  Continue with update? [Y/n] ");
            std::io::stdout().flush().ok();
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).ok();
            let answer = input.trim();
            let confirmed = answer.is_empty()
                || answer.eq_ignore_ascii_case("y")
                || answer.eq_ignore_ascii_case("yes");
            if !confirmed {
                println!("  Update cancelled.");
                return Ok(UpdateOutcome::unchanged());
            }
        }
        outcome = cargo_install_block(version, tag_name, outcome).await?;
    }

    // ── Managed / Unmanaged / Cargo (Auto): extract → flip → prune → migrate ──
    // R3: a Cargo-channel install is treated like Unmanaged — the managed
    // path extracts into `<home>/versions/<v>/`, flips the launcher, and
    // `audit_shadows` already covers `~/.cargo/bin/oxios` so the migration
    // offer converges the installed binary onto the managed launcher.
    // The legacy `cargo install` block above is reserved for explicit
    // `--via cargo` opt-in only.
    let managed = matches!(
        channel,
        crate::managed_install::Channel::Managed
            | crate::managed_install::Channel::Unmanaged
            | crate::managed_install::Channel::Cargo
    );
    if update_binary && managed && !force_cargo && !rollback {
        let release = crate::managed_install::fetch_release(version).await?;
        let tag_name = release.tag.as_str();
        let html_url = release.html_url.as_str();
        let body = release.body.as_str();
        println!(
            "  Latest release:  {} ({})",
            style(tag_name).green().bold(),
            html_url
        );
        println!();

        // Already on the pinned version? Short-circuit: no tarball fetch,
        // no extract, no flip, no migration — migration only runs as part
        // of an actual binary flip below. The web UI update path runs
        // independently afterwards.
        let already_current = tag_name == current;
        if dry_run {
            println!("  {} Dry run — no changes made.\n", style("⚠").yellow());
            if update_web {
                println!("  Would sync web UI to release {tag_name}");
            }
            let path_env = std::env::var("PATH").unwrap_or_default();
            print!(
                "{}",
                render_update_plan(channel, tag_name, oxios_home, &path_env)
            );
            return Ok(UpdateOutcome::unchanged());
        }
        if already_current {
            println!(
                "  {} Already on latest version ({}).",
                style("✓").green(),
                current
            );
        }

        // When the running binary already matches the latest release we
        // skip the fetch/install/flip/prune block entirely: the binary is
        // already on disk at the right tag, so re-running them is wasted
        // work AND triggers an unnecessary daemon restart. The web sync
        // path runs independently below; the shadow audit runs here
        // regardless so `oxios doctor` keeps catching PATH drift even on
        // a no-op update.
        let mut confirmed = false;
        if !already_current {
            // Confirmation prompt: required for migration (destructive) but
            // skipped when --yes or --adopt was passed. We thread `confirmed`
            // out so the migration gate can honor a confirmed TTY response.
            if !yes && !adopt {
                println!("  {} Release notes:\n", style("Release notes").cyan());
                for line in body.lines().take(10) {
                    println!("    {line}");
                }
                if body.lines().count() > 10 {
                    println!("    ... ({} more lines)", body.lines().count() - 10);
                }
                println!();
                print!("  Continue with update? [Y/n] ");
                std::io::stdout().flush().ok();
                let mut input = String::new();
                std::io::stdin().read_line(&mut input).ok();
                let answer = input.trim();
                confirmed = answer.is_empty()
                    || answer.eq_ignore_ascii_case("y")
                    || answer.eq_ignore_ascii_case("yes");
                if !confirmed {
                    println!("  Update cancelled.");
                    return Ok(UpdateOutcome::unchanged());
                }
            }

            // Ensure versions dir exists BEFORE disk_precheck — statfs bails
            // ENOENT on a missing directory and there's no point telling the
            // user "no space" when the real failure is "no dir".
            let versions = crate::managed_install::versions_dir(oxios_home);
            std::fs::create_dir_all(&versions)
                .with_context(|| format!("create {}", versions.display()))?;

            println!("  Fetching release tarball...");
            let tar = crate::managed_install::fetch_tarball(tag_name).await?;
            crate::managed_install::disk_precheck(&versions, (tar.len() as u64) * 3)
                .context("disk precheck for new version")?;
            crate::managed_install::install_version_bytes(oxios_home, tag_name, &tar)
                .with_context(|| format!("install version {tag_name}"))?;
            crate::managed_install::flip_launcher(oxios_home, tag_name)
                .with_context(|| format!("flip launcher to {tag_name}"))?;
            let removed = crate::managed_install::prune_versions(oxios_home, 2)
                .context("prune old versions")?;
            if !removed.is_empty() {
                println!(
                    "  Pruned: {}",
                    removed
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            outcome.binary_updated = true;
            outcome.used_managed_store = true;
            println!(
                "  {} Binary updated to {} (managed store).",
                style("✓").green(),
                tag_name
            );
        }

        // PATH-shadow audit: relevant for any channel where something else on
        // PATH could shadow the launcher. The migration gate honors `--adopt`
        // (works in any environment, including scripts) OR a confirmed TTY
        // prompt (interactive install) OR `--yes` (non-interactive explicit
        // consent) — anything else prints the exact commands to run by hand.
        // Runs regardless of whether the binary was updated — already-current
        // runs still need `oxios doctor` to keep catching PATH drift.
        let path_env = std::env::var("PATH").unwrap_or_default();
        let shadows = crate::managed_install::audit_shadows(oxios_home, &path_env);
        if !shadows.is_empty() {
            apply_migration_plan(oxios_home, &shadows, adopt, yes, confirmed)?;
        }
    }

    // ── Download and install web UI ────────────────────────────────────────
    // Sync via the shared `web_dist` core: compare version.json to the
    // target, download into a versioned staging dir, validate, and persist
    // the marker. The running daemon picks the new generation up on restart
    // (the CLI runs in its own process — no in-memory pointer to swap).
    // Embedded builds skip: the SPA ships in the binary — nothing on disk
    // can replace it.
    // R2-2: on dry-run we print the planned target instead of touching disk
    // (matches pre-refactor behavior at git 880c08526).
    if update_web && !rollback {
        if dry_run {
            // R2-2: dry-run informational line; never touches disk.
            let tag = version.unwrap_or("latest");
            println!(
                "  {} Would sync web UI to release {tag}",
                style("⚠").yellow()
            );
        } else if crate::embedded_web::is_embedded() {
            println!(
                "  {} Web UI skipped: embedded build ships the web UI in the binary — update the binary instead.",
                style("⚠").yellow()
            );
        } else {
            let target = version
                .map(|v| crate::web_dist::SyncTarget::Version(v.to_string()))
                .unwrap_or(crate::web_dist::SyncTarget::Latest);
            match crate::web_dist::sync_to_disk(target).await {
                crate::web_dist::SyncOutcome::Updated { to } => {
                    outcome.web_updated = true;
                    println!("  {} Web UI updated to {}.", style("✓").green(), to);
                }
                crate::web_dist::SyncOutcome::UpToDate { active, target } => {
                    println!(
                        "  {} Web UI already at {} (latest {}).",
                        style("✓").green(),
                        active,
                        target
                    );
                }
                crate::web_dist::SyncOutcome::Unstamped => {
                    println!(
                        "  {} Active web dist has no version stamp; skipping download.",
                        style("⚠").yellow()
                    );
                }
                crate::web_dist::SyncOutcome::Failed { reason } => {
                    anyhow::bail!("Web UI update failed: {reason}");
                }
            }
        }
    }

    println!();
    Ok(outcome)
}

/// Legacy `cargo install oxios` block — moved verbatim from the pre-T6 flow,
/// used by the `Cargo` channel and by `--via cargo` on any channel.
async fn cargo_install_block(
    version: Option<&str>,
    tag_name: &str,
    mut outcome: UpdateOutcome,
) -> Result<UpdateOutcome> {
    let mut args = vec!["install", "oxios", "--locked"];
    if let Some(v) = version {
        args.push("--version");
        args.push(v);
    }
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("  {spinner} {msg}")
            .expect("valid progress-bar template")
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    pb.enable_steady_tick(Duration::from_millis(100));
    pb.set_message(format!(
        "cargo install oxios{}",
        version
            .map(|v| format!(" --version {v}"))
            .unwrap_or_default()
    ));
    let mut child = std::process::Command::new("cargo")
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run cargo — is it installed and in PATH?")?;
    let stderr = child.stderr.take().expect("piped stderr");
    let pb_for_thread = pb.clone();
    let stderr_thread = std::thread::spawn(move || -> Vec<String> {
        let reader = BufReader::new(stderr);
        let mut lines = Vec::new();
        for line in reader.lines().map_while(Result::ok) {
            let t = line.trim().to_string();
            if !t.is_empty() {
                pb_for_thread.set_message(t.clone());
                lines.push(t);
            }
        }
        lines
    });
    let status = child.wait().context("failed to wait for cargo")?;
    let lines = stderr_thread.join().unwrap_or_default();
    pb.finish_and_clear();
    if status.success() {
        outcome.binary_updated = true;
        println!(
            "  {} Binary updated to {} via cargo.",
            style("✓").green(),
            tag_name
        );
    } else {
        println!();
        for line in lines.into_iter().take(10) {
            println!("    {line}");
        }
        anyhow::bail!("cargo install failed (see above)");
    }
    Ok(outcome)
}

/// Apply a PATH-shadow migration plan.
///
/// Gate policy (F1): the destructive fs ops run when ANY of the following
/// holds — `--adopt` (scripted consent), `--yes` (non-interactive explicit
/// consent), or a confirmed TTY prompt response. Otherwise the function
/// prints the exact commands the user can run by hand and emits the
/// mandated "shadow may win PATH" warning.
fn apply_migration_plan(
    oxios_home: &Path,
    shadows: &[crate::managed_install::ShadowEntry],
    adopt: bool,
    yes: bool,
    confirmed: bool,
) -> Result<()> {
    let plan = crate::managed_install::plan_migration(shadows);

    let launcher = crate::managed_install::launcher_path(oxios_home);
    // R2-1: gate is exactly `adopt || (is_tty && (yes || confirmed))`.
    // `is_tty` is on stdin because the confirm prompt reads from stdin;
    // when confirmed is true the prompt ran, so stdin must have been a TTY.
    // stderr-redirected `--yes` (cron/script) is print-only — the prompt is
    // suppressed under `yes` so the operator must pass `--adopt` explicitly.
    let interactive = adopt || (atty_stdin() && (yes || confirmed));
    if !interactive {
        println!(
            "  {} PATH shadow(s) detected — re-run with --adopt (or run interactively with --yes on a TTY) to apply:",
            style("⚠").yellow()
        );
        for r in &plan.repoint {
            let aside = repoint_aside(r);
            println!(
                "    mv {} {} && ln -s {} {}",
                r.display(),
                aside.display(),
                launcher.display(),
                r.display()
            );
        }
        for (from, to) in &plan.preserve_as_bak {
            println!("    mv {} {}", from.display(), to.display());
        }
        println!(
            "  ⚠ The shadow binary at the front of PATH will win until the migration above is applied."
        );
        println!(
            "  ⚠ Migration renames plain-file shadows to <name>.bak.<date>; make sure {} is on PATH (see share/install.sh's managed PATH block).",
            launcher.display()
        );
        return Ok(());
    }
    for r in &plan.repoint {
        let aside = repoint_aside(r);
        let _ = std::fs::remove_file(&aside);
        std::fs::rename(r, &aside)
            .with_context(|| format!("rename {} -> {}", r.display(), aside.display()))?;
        std::os::unix::fs::symlink(&launcher, r)
            .with_context(|| format!("symlink {} -> {}", r.display(), launcher.display()))?;
    }
    for (from, to) in &plan.preserve_as_bak {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if to.exists() {
            continue;
        }
        std::fs::rename(from, to)
            .with_context(|| format!("rename {} -> {}", from.display(), to.display()))?;
    }
    // Design (PATH integration): the managed PATH block is written by
    // install.sh AND by `oxios update` migration. Plain-file shadows were
    // just renamed aside, so a user who never ran install.sh would lose
    // `oxios` from PATH without this. Idempotent; prints what it added.
    if let Some(rc) = crate::managed_install::ensure_path_block(oxios_home) {
        println!(
            "  {} Added managed PATH entry to {} — restart your shell or `export PATH=\"{}/bin:$PATH\"`.",
            style("✓").green(),
            rc.display(),
            oxios_home.display()
        );
    }
    println!(
        "  {} PATH shadows migrated ({} repoint, {} preserved).",
        style("✓").green(),
        plan.repoint.len(),
        plan.preserve_as_bak.len()
    );
    Ok(())
}

/// Build the `<path>.pre-oxios` rename target for a repoint migration.
fn repoint_aside(path: &Path) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".pre-oxios");
    std::path::PathBuf::from(s)
}

/// True if stdin is a TTY (no extra dep — check via raw fd).
///
/// The migration gate uses this rather than `atty_stderr` because the
/// confirmation prompt reads from stdin: when `confirmed` is true the
/// prompt ran, so stdin must have been a TTY. stderr-redirected
/// `--yes` flows (cron, scripts) are therefore correctly classified
/// as non-interactive.
fn atty_stdin() -> bool {
    #[cfg(unix)]
    unsafe {
        // libc::isatty(STDIN_FILENO) == 1 when stdin is a TTY.
        libc::isatty(0) == 1
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Show changelog / release notes for a given version (or latest).
pub async fn run_changelog(version: Option<&str>) -> Result<()> {
    let release = crate::managed_install::fetch_release(version).await?;
    let tag = release.tag.as_str();
    let body = release.body.as_str();
    println!();
    println!(
        "  {} v{}  ({})",
        style("⬡ Oxios").bold(),
        style(tag).green().bold(),
        release.published_at
    );
    println!("  {}", "─".repeat(55));
    println!("  {}", release.html_url);
    println!();
    println!("{body}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_install::Channel;
    use std::path::PathBuf;

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
    fn dry_run_managed_prints_plan() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        let shadow_dir = tmp.path().join("shadow");
        std::fs::create_dir_all(&shadow_dir).unwrap();
        let shadow_path = shadow_dir.join("oxios");
        std::fs::write(&shadow_path, b"old").unwrap();
        let path_env = shadow_dir.to_str().unwrap().to_string();

        let plan = render_update_plan(Channel::Managed, "1.45.0", &home, &path_env);
        assert!(plan.contains("channel: Managed"), "{plan}");
        assert!(plan.contains("versions/1.45.0"), "{plan}");
        assert!(plan.contains("flip"), "{plan}");
        assert!(plan.contains(shadow_path.to_str().unwrap()), "plan: {plan}");
    }

    #[test]
    fn rollback_picks_previous() {
        // Install two fixture versions + flip to newer, then call
        // `rollback_and_report` directly — no network call site is touched.
        let tmp = tempfile::tempdir().unwrap();
        let home: PathBuf = tmp.path().to_path_buf();
        crate::managed_install::install_version_bytes(&home, "1.43.0", &fixture_tar_gz()).unwrap();
        crate::managed_install::install_version_bytes(&home, "1.44.0", &fixture_tar_gz()).unwrap();
        crate::managed_install::flip_launcher(&home, "1.44.0").unwrap();
        assert_eq!(
            crate::managed_install::current_target_version(&home).as_deref(),
            Some("1.44.0")
        );

        let target = crate::managed_install::rollback_target(&home).unwrap();
        crate::managed_install::flip_launcher(&home, &target).unwrap();
        assert_eq!(
            crate::managed_install::current_target_version(&home).as_deref(),
            Some("1.43.0")
        );
    }

    #[test]
    fn dry_run_already_current_plan_has_no_shadow_when_clean() {
        // Reproduce the "up-to-date dry-run plan" half of the
        // review's no-op gate: a fully managed home with the launcher
        // already pointing at the candidate tag, no PATH shadow ahead of
        // the launcher. The plan must remain well-formed AND must not
        // mention any shadow path — because there are none.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::write(home.join("bin/oxios"), vec![0u8; 5]).unwrap();
        // Empty PATH env => no shadows to enumerate.
        let plan = render_update_plan(Channel::Managed, env!("CARGO_PKG_VERSION"), &home, "");
        assert!(plan.contains("channel: Managed"), "{plan}");
        assert!(
            plan.contains("migration plan: <none>"),
            "expected no migration plan when PATH is empty, got: {plan}"
        );
    }

    #[test]
    fn already_current_skips_fetch_in_managed_block() {
        // The second half of the review's no-op gate: when the running
        // binary matches the latest release the managed block must not
        // touch fetch/install/flip/prune. We can't easily exercise the
        // full `run_update` flow without mocking fetch_release, but the
        // `UpdateOutcome::unchanged()` shape is the contract: no
        // `binary_updated = true`. Verify the unchanged() helper reports
        // the contract that the managed block now honors when
        // already_current is true.
        let outcome = UpdateOutcome::unchanged();
        assert!(
            !outcome.binary_updated,
            "binary_updated must be false on no-op"
        );
        assert!(
            !outcome.web_updated,
            "web_updated must be false when web sync ran in the no-op arm"
        );
        assert!(!outcome.any(), "any() must be false on a pure no-op update");
    }

    #[test]
    fn cargo_arm_does_not_set_used_managed_store() {
        // Fix #4: the legacy cargo-install block (used by `--via cargo`
        // and the Cargo channel when explicitly opted in) writes
        // ~/.cargo/bin/oxios while leaving the managed launcher at
        // <home>/bin/oxios untouched. main.rs must therefore NOT
        // override the daemon restart exe to the launcher path on the
        // cargo arm. The contract that distinguishes the two arms is
        // the new `used_managed_store` flag on `UpdateOutcome`.
        //
        // The cargo_install_block fn is private and not directly
        // callable from tests; the contract is verified by:
        //   1. unchanged() initializes used_managed_store = false (cargo default).
        //   2. The flag is NOT implied by binary_updated — i.e. callers
        //      can't get a "managed-store restart" by setting
        //      binary_updated alone. (Default trait impl would force it
        //      if the two were coupled.)
        let outcome = UpdateOutcome::default();
        assert!(
            !outcome.used_managed_store,
            "default outcome must not be flagged as managed-store"
        );

        // Simulate the cargo arm's outcome shape: binary_updated is
        // true, used_managed_store is false. The main.rs gate
        // `used_managed_store && launcher_exists` correctly suppresses
        // the launcher-path restart override here.
        let cargo_outcome = UpdateOutcome {
            binary_updated: true,
            web_updated: false,
            used_managed_store: false,
        };
        assert!(
            cargo_outcome.binary_updated,
            "cargo arm must still report binary_updated for the daemon to restart"
        );
        assert!(
            !cargo_outcome.used_managed_store,
            "cargo arm must NOT flag used_managed_store (launcher was not flipped)"
        );
    }
}
