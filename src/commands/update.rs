//! `oxios update` — update binary via cargo, web UI from GitHub Releases.
//!
//! Binary update: `cargo install oxios` (optionally with `--version`)
//! Web UI:       `web-dist.zip` from GitHub Releases → `~/.oxios/web/dist/`

use anyhow::{Context, Result};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use std::time::Duration;

/// Outcome of `oxios update` — what was actually changed on disk.
///
/// The caller (main.rs) uses this to decide whether a daemon restart is needed.
#[derive(Debug, Default, Clone, Copy)]
pub struct UpdateOutcome {
    /// The `oxios` binary was reinstalled (requires restart to take effect).
    pub binary_updated: bool,
    /// The web UI under `~/.oxios/web/dist/` was replaced.
    pub web_updated: bool,
}

impl UpdateOutcome {
    /// Nothing changed (already latest, dry run, or user cancelled).
    pub const fn unchanged() -> Self {
        Self {
            binary_updated: false,
            web_updated: false,
        }
    }

    /// Whether anything at all was updated.
    pub fn any(&self) -> bool {
        self.binary_updated || self.web_updated
    }
}

/// Update oxios binary (via cargo) and/or web UI (from GitHub Releases).
pub async fn run_update(
    web_only: bool,
    binary_only: bool,
    version: Option<&str>,
    dry_run: bool,
    yes: bool,
) -> Result<UpdateOutcome> {
    let current = env!("CARGO_PKG_VERSION");
    let mut outcome = UpdateOutcome::unchanged();

    // ── Determine what to update ────────────────────────────────────────────
    let update_binary = !web_only;
    let update_web = !binary_only;

    println!();
    println!(
        "  {} {}",
        style("⬡ Oxios Updater").bold(),
        style(format!("v{current}")).dim()
    );
    println!("  {}", "─".repeat(52));
    println!("  Current version:  {current}");
    println!(
        "  Update binary:    {}",
        if update_binary {
            "yes (cargo install)"
        } else {
            "no"
        }
    );
    println!(
        "  Update web UI:   {}",
        if update_web { "yes" } else { "no" }
    );
    if let Some(v) = version {
        println!("  Target version:  {v}");
    } else {
        println!("  Target version:  latest");
    }
    println!();

    // ── Fetch release info from GitHub (for version check + web UI) ─────────
    let owner = "a7garden";
    let repo = "oxios";
    let tag = version.map(|v| format!("v{v}"));

    let api_url = match &tag {
        Some(t) => format!("https://api.github.com/repos/{owner}/{repo}/releases/tags/{t}"),
        None => format!("https://api.github.com/repos/{owner}/{repo}/releases/latest"),
    };

    println!("  Fetching release info from GitHub...");
    let client = reqwest::Client::builder()
        .user_agent("oxios/0.3")
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .context("failed to create HTTP client")?;

    let resp = client
        .get(&api_url)
        .send()
        .await
        .context("failed to fetch release info (check network/GITHUB_TOKEN)")?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "GitHub API error {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }

    let release: serde_json::Value = resp.json().await.context("failed to parse release JSON")?;

    let tag_name = release["tag_name"]
        .as_str()
        .unwrap_or("unknown")
        .trim_start_matches('v');
    let html_url = release["html_url"].as_str().unwrap_or("");
    let body = release["body"].as_str().unwrap_or("No release notes.");

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
    // version alone.
    if !dry_run && !yes && update_binary && !update_web && tag_name == current {
        println!(
            "  {} Already on latest version ({}).",
            style("✓").green(),
            current
        );
        println!("  Use `--version X.Y.Z` to force a specific version.");
        return Ok(UpdateOutcome::unchanged());
    }

    // ── Dry run ──────────────────────────────────────────────────────────────
    if dry_run {
        println!("  {} Dry run — no changes made.\n", style("⚠").yellow());
        if update_web {
            println!("  Would sync web UI to release {tag_name}");
        }
        if update_binary {
            let mut cmd = "cargo install oxios".to_string();
            if let Some(v) = version {
                cmd.push_str(&format!(" --version {v}"));
            }
            println!("  Would run: {cmd}");
        }
        return Ok(UpdateOutcome::unchanged());
    }

    // ── Confirmation ─────────────────────────────────────────────────────────
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
        // Empty (Enter) or y/yes → proceed; anything else cancels.
        let confirmed = answer.is_empty()
            || answer.eq_ignore_ascii_case("y")
            || answer.eq_ignore_ascii_case("yes");
        if !confirmed {
            println!("  Update cancelled.");
            return Ok(UpdateOutcome::unchanged());
        }
    }

    // ── Update binary via cargo ─────────────────────────────────────────────
    if update_binary && tag_name != current {
        let mut args = vec!["install", "oxios", "--locked"];
        if let Some(v) = version {
            args.push("--version");
            args.push(v);
        }

        // Spinner: cargo streams `Compiling X…` / `Finished` lines to stderr
        // (with carriage returns), so we parse them and update the spinner
        // message rather than dumping the raw output.
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
    }

    // ── Download and install web UI ────────────────────────────────────────
    // Sync via the shared `web_dist` core: compare version.json to the
    // target, download into a versioned staging dir, validate, and persist
    // the marker. The running daemon picks the new generation up on restart
    // (the CLI runs in its own process — no in-memory pointer to swap).
    // Embedded builds skip: the SPA ships in the binary — nothing on disk
    // can replace it.
    if update_web {
        if crate::embedded_web::is_embedded() {
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

/// Show changelog / release notes for a given version (or latest).
pub async fn run_changelog(version: Option<&str>) -> Result<()> {
    let owner = "a7garden";
    let repo = "oxios";
    let api_url = match version {
        Some(v) => format!("https://api.github.com/repos/{owner}/{repo}/releases/tags/v{v}"),
        None => format!("https://api.github.com/repos/{owner}/{repo}/releases/latest"),
    };

    let client = reqwest::Client::builder()
        .user_agent("oxios/0.3")
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .context("failed to create HTTP client")?;

    let resp = client
        .get(&api_url)
        .send()
        .await
        .context("failed to fetch release info")?;

    if !resp.status().is_success() {
        anyhow::bail!("Release not found: {}", resp.status());
    }

    let release: serde_json::Value = resp.json().await.context("failed to parse release JSON")?;
    let tag = release["tag_name"]
        .as_str()
        .unwrap_or("?")
        .trim_start_matches('v');
    let body = release["body"].as_str().unwrap_or("(no release notes)");
    let date = release["published_at"].as_str().unwrap_or("?");

    println!();
    println!(
        "  {} v{}  ({})",
        style("⬡ Oxios").bold(),
        style(tag).green().bold(),
        date
    );
    println!("  {}", "─".repeat(55));
    println!();
    println!("{body}");
    Ok(())
}
