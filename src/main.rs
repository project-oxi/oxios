//! Oxios Agent OS — main binary.
//!
//! Default invocation (`oxios`) starts the daemon in the background.
//! Use `oxios --foreground` to run in the foreground (for debugging).
//! First run without credentials triggers an interactive setup wizard.

// `.unwrap()` in tests is idiomatic (workspace convention); suppressed only
// under `cfg(test)` so production code remains linted by the `--lib` clippy pass.
#![cfg_attr(test, allow(clippy::unwrap_used))]

mod commands;
mod default_skills;
mod embedded_web;
mod kernel;
mod surface;
mod web_dist;

// RFC-026: HTTP API server (merged from surface/oxios-web)
#[cfg(feature = "web")]
mod api;

// RFC-044: E2EE companion surface (opt-in).
#[cfg(feature = "remote")]
mod remote;

// RFC-026: in-process channels (merged from channels/oxios-cli, channels/oxios-telegram)
// Individual sub-modules are feature-gated internally.
mod channels;
mod supervisor;

use crate::supervisor::{RestartConfig, ShutdownOutcome, TaskSupervisor, WebSurfaceRestarter};
use anyhow::{Context, Result};
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use console::style;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio_util::sync::CancellationToken;

use kernel::Kernel;
use oxios_kernel::onboarding::WORKSPACE_SUBDIRS;
use oxios_kernel::{DaemonManager, OxiosConfig, credential::CredentialStore};

#[cfg(feature = "cli")]
use crate::channels::cli::CliPlugin;
#[cfg(feature = "telegram")]
use crate::channels::telegram::TelegramPlugin;

use oxios_gateway::plugin::{ChannelContext, ChannelPlugin};

// ─── CLI ───────────────────────────────────────────────────────────────────
// Argument schema lives in `cli.rs`; main.rs handles dispatch & lifecycle.

mod cli;
use cli::{
    AgentAction, CalendarAction, Cli, Command, ConfigAction, DaemonAction, EmailAction, GitAction,
    MarketplaceAction, PkgAction, ProjectAction,
};

// ─── Constants & helpers ───────────────────────────────────────────────────

const DEFAULT_CONFIG: &str = include_str!("../share/default-config.toml");

fn ensure_workspace(oxios_home: &Path) -> Result<()> {
    if !oxios_home.exists() {
        tracing::info!(path = %oxios_home.display(), "Creating Oxios home directory");
        std::fs::create_dir_all(oxios_home)?;
    }
    for subdir in WORKSPACE_SUBDIRS {
        let dir = oxios_home.join(subdir);
        if !dir.exists() {
            std::fs::create_dir_all(&dir)?;
        }
    }
    let config_path = oxios_home.join("config.toml");
    if !config_path.exists() {
        tracing::info!(path = %config_path.display(), "Writing default config");
        std::fs::write(&config_path, DEFAULT_CONFIG)?;
    }
    Ok(())
}

fn oxios_home_from_config(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(format!("{home}/.oxios"))
        })
}

/// Read the last `n` lines from a file without external commands.
fn tail_file(path: &Path, lines: usize) -> Result<String> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let all_lines: Vec<&str> = content.lines().collect();
    let start = all_lines.len().saturating_sub(lines);
    Ok(all_lines[start..].join("\n"))
}

// ─── Subcommands ───────────────────────────────────────────────────────────

async fn cmd_pkg(kernel: &Kernel, action: &PkgAction) -> Result<()> {
    let handle = kernel.handle();
    match action {
        PkgAction::Install { source, branch } => {
            // Delegate to marketplace install
            let api = handle.marketplace_api.clone();
            match api.install(source, branch.as_deref()).await {
                Ok(result) => {
                    println!(
                        "  {} {} v{}",
                        style("Installed").green().bold(),
                        style(&result.slug).cyan(),
                        style(&result.version).cyan()
                    );
                }
                Err(e) => {
                    eprintln!(
                        "  {} Failed to install '{}': {}",
                        style("✗").red().bold(),
                        source,
                        e
                    );
                }
            }
        }
        PkgAction::Uninstall { name } => {
            handle.extensions.delete_skill(name).await?;
            println!("  {} '{}'", style("Uninstalled").green(), name);
        }
        PkgAction::List => {
            let skills = handle.extensions.list_skills_entries().await;
            if skills.is_empty() {
                println!("  No skills installed.");
            } else {
                println!("{:30} {:10} {:40}", "NAME", "STATUS", "DESCRIPTION");
                println!("{}", "─".repeat(82));
                for s in &skills {
                    println!(
                        "{:30} {:10} {:40}",
                        s.skill.name,
                        format!("{:?}", s.eligibility),
                        s.skill.description.chars().take(40).collect::<String>()
                    );
                }
            }
        }
        PkgAction::Search => {
            let skills = handle.extensions.list_skills_entries().await;
            if skills.is_empty() {
                println!("  No skills installed.");
            } else {
                println!("  {} Installed skills:", style("✔").green().bold());
                println!();
                for s in &skills {
                    println!("  {}", style(&s.skill.name).bold());
                    println!("    {}", s.skill.description);
                    println!();
                }
            }
            println!(
                "  {} To find new skills, use {}",
                style("Tip:").cyan(),
                style("oxios marketplace search --query <term>").bold()
            );
        }
    }
    Ok(())
}

async fn cmd_config(action: &ConfigAction, config_path: &Path) -> Result<()> {
    match action {
        ConfigAction::Show => {
            let config = load_config_or_default(config_path)?;
            let toml_str = toml::to_string_pretty(&config).context("failed to serialize config")?;
            println!("{toml_str}");
        }
        ConfigAction::Get { key } => {
            let config = load_config_or_default(config_path)?;
            let value = config_get(&config, key)?;
            println!("{value}");
        }
        ConfigAction::Set { key, value } => {
            // Warn if the key isn't a known config field (avoids silently
            // writing ignored keys from typos).
            let defaults = OxiosConfig::default();
            if config_get(&defaults, key).is_err() {
                eprintln!(
                    "  {} Unknown config key '{}'. It will be written but may be ignored.",
                    style("⚠").yellow().bold(),
                    key
                );
                eprintln!("  Run `oxios config list` to see available keys.");
            }
            config_set(config_path, key, value)?;
            println!("  {} {} = {}", style("Set").green(), key, value);
        }
        ConfigAction::List { prefix } => {
            let config = load_config_or_default(config_path)?;
            config_list(&config, prefix.as_deref())?;
        }
        ConfigAction::Reset { key } => {
            let defaults = OxiosConfig::default();
            let default_value = config_get(&defaults, key)?;
            config_set(config_path, key, &default_value)?;
            println!(
                "  {} {} → default ({})",
                style("Reset").green(),
                key,
                default_value
            );
        }
    }
    Ok(())
}

fn load_config_or_default(config_path: &Path) -> Result<OxiosConfig> {
    if config_path.exists() {
        oxios_kernel::config::load_config(config_path)
    } else {
        Ok(OxiosConfig::default())
    }
}

// ─── Config Get: serde_json 기반 전체 필드 dot-notation 조회 ─────────────

fn config_get(config: &OxiosConfig, key: &str) -> Result<String> {
    let json = serde_json::to_value(config).context("Failed to serialize config to JSON")?;

    let value = json
        .pointer(&format!("/{}", key.replace('.', "/")))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Unknown config key: '{key}'\n\
                 Run `oxios config list` to see available keys."
            )
        })?;

    match value {
        serde_json::Value::String(s) => Ok(s.clone()),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::Bool(b) => Ok(b.to_string()),
        serde_json::Value::Null => Ok("null".to_string()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            Ok(serde_json::to_string_pretty(value)?)
        }
    }
}

// ─── Config Set: toml_edit 기반 (코멘트/포맷팅 보존) ─────────────────────

fn config_set(config_path: &Path, key: &str, raw_value: &str) -> Result<()> {
    // config 파일이 없으면 기본 설정에서 생성
    if !config_path.exists() {
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(config_path, DEFAULT_CONFIG)?;
    }

    let toml_str = std::fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
    let mut doc = toml_str
        .parse::<toml_edit::DocumentMut>()
        .context("Failed to parse config file")?;

    // 기존 필드 타입 존중
    let existing_type = get_existing_type(&doc, key);
    let parsed = parse_toml_value(raw_value, existing_type);

    // dot-notation으로 테이블 탐색 + leaf 값 설정
    set_toml_dot(&mut doc, key, parsed)?;

    std::fs::write(config_path, doc.to_string())?;
    tracing::info!(key, value = raw_value, "Config changed");
    Ok(())
}

fn set_toml_dot(doc: &mut toml_edit::DocumentMut, key: &str, value: toml_edit::Item) -> Result<()> {
    let parts: Vec<&str> = key.split('.').collect();
    let mut table = doc.as_table_mut();

    // Navigate to the parent table, then set the leaf value
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            table[*part] = value;
            return Ok(());
        } else {
            table = table
                .entry(part)
                .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("'{}' is not a table", parts[..=i].join(".")))?;
        }
    }
    Ok(())
}

/// 기존 TOML 문서에서 해당 키의 값 타입을 조사.
enum ExistingType {
    Bool,
    Integer,
    Float,
    String,
    Unknown,
}

fn get_existing_type(doc: &toml_edit::DocumentMut, key: &str) -> ExistingType {
    let parts: Vec<&str> = key.split('.').collect();
    let mut table = doc.as_table();
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            return match table.get(part) {
                Some(toml_edit::Item::Value(v)) => {
                    if v.is_bool() {
                        ExistingType::Bool
                    } else if v.is_integer() {
                        ExistingType::Integer
                    } else if v.is_float() {
                        ExistingType::Float
                    } else {
                        ExistingType::String
                    }
                }
                _ => ExistingType::Unknown,
            };
        }
        table = match table.get(part).and_then(|t| t.as_table()) {
            Some(t) => t,
            None => return ExistingType::Unknown,
        };
    }
    ExistingType::Unknown
}

/// 기존 필드 타입을 존중하여 값을 파싱.
fn parse_toml_value(raw: &str, existing: ExistingType) -> toml_edit::Item {
    match existing {
        ExistingType::Bool => match raw.parse::<bool>() {
            Ok(v) => return toml_edit::value(v),
            Err(_) => {
                // boolean 필드에 boolean이 아닌 값 → 문자열로 폴백
                return toml_edit::value(raw);
            }
        },
        ExistingType::Integer => {
            if let Ok(n) = raw.parse::<i64>() {
                return toml_edit::value(n);
            }
        }
        ExistingType::Float => {
            if let Ok(n) = raw.parse::<f64>() {
                return toml_edit::value(n);
            }
        }
        ExistingType::String | ExistingType::Unknown => {}
    }
    // Unknown 또는 파싱 실패: 자동 추론
    if raw == "true" {
        return toml_edit::value(true);
    }
    if raw == "false" {
        return toml_edit::value(false);
    }
    if let Ok(n) = raw.parse::<i64>() {
        return toml_edit::value(n);
    }
    if let Ok(n) = raw.parse::<f64>() {
        return toml_edit::value(n);
    }
    toml_edit::value(raw)
}

// ─── Config List: 모든 leaf 키 나열 ───────────────────────────────────────

fn config_list(config: &OxiosConfig, prefix: Option<&str>) -> Result<()> {
    let json = serde_json::to_value(config)?;

    let root = if let Some(p) = prefix {
        json.pointer(&format!("/{}", p.replace('.', "/")))
            .ok_or_else(|| anyhow::anyhow!("Unknown prefix: '{p}'"))?
    } else {
        &json
    };

    let mut keys = Vec::new();
    collect_leaf_keys(root, prefix.unwrap_or(""), &mut keys);

    if keys.is_empty() {
        println!("  No configuration keys.");
    } else {
        for (key, value) in &keys {
            println!("  {:<50} {}", key, style(value).dim());
        }
        println!();
        println!("  {} config keys", style(keys.len()).cyan());
    }
    Ok(())
}

fn collect_leaf_keys(value: &serde_json::Value, prefix: &str, out: &mut Vec<(String, String)>) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let new_prefix = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                collect_leaf_keys(v, &new_prefix, out);
            }
        }
        _ => {
            let display = match value {
                serde_json::Value::String(s) => format!("\"{s}\""),
                serde_json::Value::Null => "null".into(),
                other => other.to_string(),
            };
            out.push((prefix.to_string(), display));
        }
    }
}

async fn cmd_status(kernel: &Kernel) -> Result<()> {
    let config = kernel.config();
    let daemon = DaemonManager::new(&config.daemon.pid_file, &config.daemon.log_dir)
        .with_probe_port(config.gateway.port);

    println!();
    println!(
        "  {} {}",
        style("⬡ Oxios Agent OS").bold(),
        style(format!("v{}", env!("CARGO_PKG_VERSION"))).dim()
    );
    println!("  {}", "─".repeat(48));
    println!("  {:<16}  {}", "Workspace:", config.kernel.workspace);
    println!(
        "  {:<16}  {}",
        "Model:",
        style(&config.engine.default_model).cyan()
    );

    let daemon_status = daemon.status();
    let is_running = matches!(
        daemon_status,
        oxios_kernel::DaemonStatus::Running { .. } | oxios_kernel::DaemonStatus::Orphaned { .. }
    );
    if is_running {
        println!(
            "  {:<16}  {}",
            "Daemon:",
            style(daemon_status.to_string()).green()
        );
    } else {
        println!(
            "  {:<16}  {}",
            "Daemon:",
            style(daemon_status.to_string()).yellow()
        );
    }
    // Web UI integrity: reported independently of process liveness. A daemon
    // can be alive (Daemon: Running) while serving a dangling marker — a
    // raced update deleted the active dir — which is exactly the "status
    // says Running but the UI 404s" confusion this line disambiguates.
    let web_ui = match crate::web_dist::diagnose_active() {
        crate::web_dist::WebUiHealth::Ok { path, version } => {
            let v = version
                .filter(|v| !v.is_empty())
                .map(|v| format!(" (v{v})"))
                .unwrap_or_default();
            style(format!("✓ {}{}", path.display(), v)).green()
        }
        crate::web_dist::WebUiHealth::Broken { marker } => style(format!(
            "✗ broken — marker {} does not resolve to a consistent dist",
            marker.display()
        ))
        .red(),
        crate::web_dist::WebUiHealth::Embedded { version } => {
            let v = version
                .filter(|v| !v.is_empty())
                .map(|v| format!(" (v{v})"))
                .unwrap_or_default();
            style(format!("✓ embedded{}", v)).green()
        }
        crate::web_dist::WebUiHealth::NotInstalled => style(String::from("not installed")).dim(),
    };
    println!("  {:<16}  {}", "Web UI:", web_ui);
    println!();

    // Credential source
    let provider = CredentialStore::provider_from_model(&config.engine.default_model);
    match provider {
        Some(provider) => match CredentialStore::resolve(provider, config.api_key().as_deref()) {
            Some((key, source)) => {
                let source_str = match source {
                    oxios_kernel::credential::CredentialSource::Config => "config.toml",
                    oxios_kernel::credential::CredentialSource::OxicodeAuthStore => {
                        "~/.oxicode/auth.json"
                    }
                    oxios_kernel::credential::CredentialSource::EnvVar => "env var",
                };
                let preview = if key.len() > 8 {
                    format!("{}...{}", &key[..4], &key[key.len() - 4..])
                } else {
                    key.clone()
                };
                println!(
                    "  {:<16}  {} [{}]",
                    "Credentials:",
                    style(preview).green(),
                    style(source_str).dim()
                );
            }
            None => {
                println!(
                    "  {:<16}  {}",
                    "Credentials:",
                    style("✗ none (run `oxios onboard` to setup)").red()
                );
            }
        },
        None => {
            println!(
                "  {:<16}  {}",
                "Credentials:",
                style("✗ no model configured").red()
            );
        }
    }

    // Active agents
    let mcp_count = kernel.handle().mcp.server_count();
    println!("  {:<16}  {}", "MCP Servers:", mcp_count);

    let agents = kernel
        .handle()
        .agents
        .list()
        .await
        .map_err(|e| anyhow::anyhow!("failed to list agents: {e}"))?;
    println!("  {:<16}  {}", "Active Agents:", agents.len());
    if !agents.is_empty() {
        println!();
        for agent in &agents {
            let status_str = format!("{:?}", agent.status);
            let styled_status = if matches!(agent.status, oxios_kernel::types::AgentStatus::Running)
            {
                style(&status_str).green()
            } else {
                style(&status_str).yellow()
            };
            println!(
                "    {}  {}  {}",
                style(&agent.id.to_string()).dim(),
                styled_status,
                agent.name
            );
        }
    }

    println!();
    Ok(())
}

// ─── Reset command ───────────────────────────────────────────────────────────

/// Collect all paths and items that `oxios reset` would delete.
struct ResetTargets {
    /// `~/Library/LaunchAgents/com.a7garden.oxios.plist` — macOS launchd
    launchd_plist: Option<PathBuf>,
    /// Items that actually exist on disk (for display)
    existing: Vec<ResetItem>,
}

struct ResetItem {
    label: String,
    path: PathBuf,
    /// Size in bytes (0 if unknown or not a file/dir)
    size: u64,
}

fn collect_reset_targets(oxios_home: &Path) -> ResetTargets {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();

    let launchd_plist = if cfg!(target_os = "macos") {
        Some(
            dirs::home_dir()
                .map(|h| h.join("Library/LaunchAgents/com.a7garden.oxios.plist"))
                .unwrap_or_else(|| {
                    PathBuf::from(&home).join("Library/LaunchAgents/com.a7garden.oxios.plist")
                }),
        )
    } else {
        None
    };

    let mut existing = Vec::new();

    // 1. ~/.oxios/
    if oxios_home.exists() {
        let size = dir_size(oxios_home);
        existing.push(ResetItem {
            label: "Oxios home (config, workspace, logs, memory, sessions, skills)".to_string(),
            path: oxios_home.to_path_buf(),
            size,
        });
    }

    // 2. launchd plist
    if let Some(ref plist) = launchd_plist
        && plist.exists()
    {
        let size = plist.metadata().map(|m| m.len()).unwrap_or(0);
        existing.push(ResetItem {
            label: "macOS launchd service registration".to_string(),
            path: plist.clone(),
            size,
        });
    }

    ResetTargets {
        launchd_plist,
        existing,
    }
}

/// Recursively calculate directory size using std::fs.
fn dir_size(path: &Path) -> u64 {
    fn acc(p: &Path, total: &mut u64) {
        if let Ok(entries) = std::fs::read_dir(p) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        *total += meta.len();
                    } else if meta.is_dir() {
                        acc(&entry.path(), total);
                    }
                }
            }
        }
    }
    let mut total = 0u64;
    acc(path, &mut total);
    total
}

/// Format bytes into human-readable string.
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn cmd_reset(oxios_home: &Path, skip_confirm: bool, pid_file: &Path) -> Result<()> {
    let targets = collect_reset_targets(oxios_home);

    if targets.existing.is_empty() {
        println!();
        println!("  {} No Oxios data to reset.", style("✓").green().bold());
        println!();
        return Ok(());
    }

    // ── Phase 1: Show targets ──
    println!();
    println!(
        "  {} The following will be permanently deleted:",
        style("⚠ WARNING:").yellow().bold()
    );
    println!();

    let total_size: u64 = targets.existing.iter().map(|i| i.size).sum();

    for (i, item) in targets.existing.iter().enumerate() {
        let size_str = if item.size > 0 {
            format!(" ({})", format_bytes(item.size))
        } else {
            String::new()
        };
        println!(
            "    {}. {}{}",
            i + 1,
            style(&item.path.display()).cyan(),
            style(&size_str).dim()
        );
        println!("       {}", style(&item.label).dim());
    }

    println!();
    println!(
        "  {} item(s), {}",
        targets.existing.len(),
        style(format_bytes(total_size)).yellow().bold()
    );
    println!();
    println!(
        "  {}",
        style("This cannot be undone. All agents, memory, skills, sessions, and settings will be deleted.").red()
    );

    // ── Phase 2: Safety confirmation ──
    if !skip_confirm {
        println!();
        let answer = inquire::Text::new("  Type RESET to confirm:").prompt()?;

        if answer.trim() != "RESET" {
            println!();
            println!("  {} Reset cancelled.", style("✗").yellow().bold());
            println!();
            return Ok(());
        }
    }

    println!();

    // ── Phase 3: Stop daemon ──
    if pid_file.exists() {
        let pid_str = std::fs::read_to_string(pid_file).unwrap_or_default();
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            print!("  {} Stopping daemon...", style("●").cyan());
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            println!(" {}", style("done").green());
        }
    }

    // ── Phase 4: Remove launchd service ──
    if let Some(ref plist) = targets.launchd_plist
        && plist.exists()
    {
        // Unload first
        let _ = std::process::Command::new("launchctl")
            .args(["unload", &plist.to_string_lossy()])
            .output();
        match std::fs::remove_file(plist) {
            Ok(()) => println!("  {} launchd service removed", style("✓").green()),
            Err(e) => println!("  {} launchd removal failed: {}", style("⚠").yellow(), e),
        }
    }

    // ── Phase 5: Delete ~/.oxios/ ──
    if oxios_home.exists() {
        print!(
            "  {} Deleting {}...",
            style("●").cyan(),
            oxios_home.display()
        );
        match std::fs::remove_dir_all(oxios_home) {
            Ok(()) => println!(" {}", style("done").green()),
            Err(e) => {
                println!();
                println!(
                    "  {} {} failed to delete: {}",
                    style("✗").red().bold(),
                    oxios_home.display(),
                    e
                );
            }
        }
    }

    // ── Done ──
    println!();
    println!(
        "  {} All Oxios data has been reset.",
        style("✓").green().bold()
    );
    println!(
        "  {} Run {} to set up again.",
        style("→").cyan(),
        style("oxios").cyan().bold()
    );
    println!();
    Ok(())
}

// ─── Doctor command ──────────────────────────────────────────────────────────

async fn cmd_doctor(kernel: &Kernel, config_path: &Path) -> Result<()> {
    let config = kernel.config();
    let mut issues = Vec::new();
    let mut checks = 0u32;

    println!();
    println!("  {}", style("⬡ Oxios Doctor — System Diagnostics").bold());
    println!("  {}", "─".repeat(48));

    // 1. Config file exists
    checks += 1;
    if config_path.exists() {
        println!(
            "  {} Config file present ({})",
            style("✓").green(),
            style(config_path.display()).dim()
        );
    } else {
        println!("  {} Config file missing", style("✗").red().bold());
        issues.push("Config file not found. Run `oxios onboard` to create it.".to_string());
    }

    // 2. Credentials
    checks += 1;
    let provider = CredentialStore::provider_from_model(&config.engine.default_model);
    match provider {
        Some(provider) => match CredentialStore::resolve(provider, config.api_key().as_deref()) {
            Some((key, source)) => {
                let source_str = match source {
                    oxios_kernel::credential::CredentialSource::Config => "config.toml",
                    oxios_kernel::credential::CredentialSource::OxicodeAuthStore => {
                        "~/.oxicode/auth.json"
                    }
                    oxios_kernel::credential::CredentialSource::EnvVar => "env var",
                };
                let preview = if key.len() > 8 {
                    format!("{}...{}", &key[..4], &key[key.len() - 4..])
                } else {
                    "(set)".to_string()
                };
                println!(
                    "  {} Credentials found ({}, via {})",
                    style("✓").green(),
                    style(preview).cyan(),
                    style(source_str).dim()
                );
            }
            None => {
                println!(
                    "  {} No credentials for provider '{}'",
                    style("✗").red().bold(),
                    style(provider).cyan()
                );
                issues.push(format!(
                    "No API key for '{provider}'. Run `oxios onboard` to configure."
                ));
            }
        },
        None => {
            println!("  {} No model configured", style("✗").red().bold());
            issues.push("No model set. Run `oxios onboard` to configure.".to_string());
        }
    }

    // 3. Workspace directory
    checks += 1;
    let workspace = oxios_kernel::config::expand_home(&config.kernel.workspace);
    if workspace.exists() {
        println!(
            "  {} Workspace directory ({})",
            style("✓").green(),
            style(workspace.display()).dim()
        );
    } else {
        println!(
            "  {} Workspace directory missing ({})",
            style("✗").red().bold(),
            workspace.display()
        );
        issues.push("Workspace directory not found. It will be created on first run.".to_string());
    }

    // 4. Daemon status
    checks += 1;
    let daemon = DaemonManager::new(&config.daemon.pid_file, &config.daemon.log_dir)
        .with_probe_port(config.gateway.port);
    let daemon_status = daemon.status();
    let is_running = matches!(
        daemon_status,
        oxios_kernel::DaemonStatus::Running { .. } | oxios_kernel::DaemonStatus::Orphaned { .. }
    );
    if is_running {
        println!("  {} Daemon is running", style("✓").green());
    } else {
        println!(
            "  {} Daemon is not running ({})",
            style("⚠").yellow().bold(),
            daemon_status
        );
        issues.push("Daemon not running. Start with `oxios start`.".to_string());
    }

    // 5. MCP servers
    checks += 1;
    let mcp_count = kernel.handle().mcp.server_count();
    if mcp_count > 0 {
        println!(
            "  {} {} MCP server(s) connected",
            style("✓").green(),
            mcp_count
        );
    } else {
        println!("  {} No MCP servers configured", style("⚠").yellow().bold());
    }

    // 6. Model is set
    checks += 1;
    if !config.engine.default_model.is_empty() {
        println!(
            "  {} Default model: {}",
            style("✓").green(),
            style(&config.engine.default_model).cyan()
        );
    } else {
        println!("  {} No default model set", style("✗").red().bold());
        issues.push("No default model configured.".to_string());
    }

    // 7. oxicode CLI installed
    checks += 1;
    let oxicode_auth_exists = {
        let home = std::env::var("HOME").unwrap_or_default();
        std::path::PathBuf::from(format!("{home}/.oxicode/auth.json")).exists()
    };
    let oxicode_bin_exists = std::path::PathBuf::from("/usr/local/bin/oxicode").exists()
        || std::path::PathBuf::from(
            std::env::var("HOME").unwrap_or_default() + "/.cargo/bin/oxicode",
        )
        .exists();
    let oxicode_installed = oxicode_auth_exists || oxicode_bin_exists;
    if oxicode_installed {
        println!(
            "  {} oxicode CLI available (shared auth store)",
            style("✓").green()
        );
    } else {
        println!("  {} oxicode CLI not detected", style("⚠").yellow().bold());
        issues.push(
            "Install oxicode CLI for shared credential management: `cargo install oxicode-cli`"
                .to_string(),
        );
    }

    // 8. Gateway port available
    checks += 1;
    let port = config.gateway.port;
    let port_in_use = TcpStream::connect(format!("127.0.0.1:{port}")).is_ok();
    if port_in_use && !is_running {
        println!(
            "  {} Port {} is already in use",
            style("✗").red().bold(),
            style(port).cyan()
        );
        issues.push(format!(
            "Port {port} is occupied. Change with `oxios config set gateway.port <port>`."
        ));
    } else if port_in_use && is_running {
        println!(
            "  {} Port {} listening (daemon active)",
            style("✓").green(),
            style(port).cyan()
        );
    } else {
        println!(
            "  {} Port {} available",
            style("✓").green(),
            style(port).cyan()
        );
    }

    // 9. API authentication (security posture)
    checks += 1;
    if !config.security.auth_enabled {
        println!(
            "  {} Web API authentication is DISABLED (auth_enabled=false)",
            style("⚠").yellow().bold()
        );
        issues.push(
            "Web API auth is disabled. Set [security] auth_enabled=true \
             (generate a key with `oxios auth generate`). Never bind to a \
             non-loopback interface with auth disabled."
                .to_string(),
        );
    } else {
        println!("  {} Web API authentication enabled", style("✓").green());
    }

    // Summary
    println!("  {}", "─".repeat(48));
    if issues.is_empty() {
        println!(
            "  {} checks passed, no issues found. {}",
            checks,
            style("All good!").green().bold()
        );
    } else {
        println!(
            "  {} checks, {} issue(s):",
            checks,
            style(issues.len()).yellow().bold()
        );
        println!();
        for (i, issue) in issues.iter().enumerate() {
            println!("    {}. {}", i + 1, issue);
        }
    }
    println!();

    Ok(())
}

// ─── Models command ──────────────────────────────────────────────────────────

fn cmd_models(provider: Option<&str>) -> Result<()> {
    // Resolve provider from arg or from config
    let provider_id = match provider {
        Some(p) => p.to_string(),
        None => {
            // Try reading from config
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            let config_path =
                oxios_kernel::config::expand_home(&format!("{home}/.oxios/config.toml"));
            if config_path.exists() {
                let config = oxios_kernel::config::load_config(&config_path)?;
                if config.engine.default_model.is_empty() {
                    anyhow::bail!(
                        "No provider configured. Run `oxios onboard` or use `--provider <name>`."
                    );
                }
                CredentialStore::provider_from_model(&config.engine.default_model)
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            } else {
                anyhow::bail!("No config found. Run `oxios onboard` or use `--provider <name>`.");
            }
        }
    };

    if provider_id.is_empty() {
        anyhow::bail!("Could not determine provider. Use `--provider <name>`.");
    }

    let models = oxicode_sdk::get_provider_models(&provider_id);
    if models.is_empty() {
        println!("  No models found for '{provider_id}'. Check the provider name.");
        return Ok(());
    }

    println!();
    println!(
        "  {} for {}",
        style("Available Models").bold(),
        style(&provider_id).cyan()
    );
    println!("  {}", "─".repeat(60));

    for entry in models.iter() {
        let ctx = if entry.context_window >= 1_000_000 {
            format!("{}M", entry.context_window / 1_000_000)
        } else {
            format!("{}K", entry.context_window / 1000)
        };
        let reasoning = if entry.reasoning {
            format!(" {}", style("✦reasoning").magenta())
        } else {
            String::new()
        };
        println!(
            "  {}  {} ctx{}",
            style(&entry.name).bold(),
            style(ctx).dim(),
            reasoning,
        );
    }

    println!();
    println!(
        "  {} models total. Use full ID: {}/<model-id>",
        models.len(),
        provider_id
    );
    println!();
    Ok(())
}

// ─── Calendar helpers ───────────────────────────────────────────────────────

fn today_range() -> (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) {
    let now = chrono::Local::now();
    let today = now.date_naive();
    let from = today
        .and_hms_opt(0, 0, 0)
        .expect("valid time of day")
        .and_utc();
    let to = today
        .and_hms_opt(23, 59, 59)
        .expect("valid time of day")
        .and_utc();
    (from, to)
}

fn tomorrow_range() -> (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) {
    let now = chrono::Local::now();
    let tomorrow = now.date_naive() + chrono::Duration::days(1);
    let from = tomorrow
        .and_hms_opt(0, 0, 0)
        .expect("valid time of day")
        .and_utc();
    let to = tomorrow
        .and_hms_opt(23, 59, 59)
        .expect("valid time of day")
        .and_utc();
    (from, to)
}

fn week_range() -> (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) {
    let now = chrono::Local::now();
    let today = now.date_naive();
    let from = today
        .and_hms_opt(0, 0, 0)
        .expect("valid time of day")
        .and_utc();
    let to = (today + chrono::Duration::days(7))
        .and_hms_opt(23, 59, 59)
        .expect("valid time of day")
        .and_utc();
    (from, to)
}

fn parse_range(
    from: Option<String>,
    to: Option<String>,
) -> Result<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> {
    // `None` → sensible default; `Some(bad)` → error (don't silently fall
    // back to today, which hides a typo from the user).
    let f = match from.as_deref() {
        None => chrono::Local::now().date_naive(),
        Some(s) => chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| {
            anyhow::anyhow!("Invalid --from date '{s}': {e}. Use ISO 8601 date, e.g. 2026-06-01")
        })?,
    };
    let t = match to.as_deref() {
        None => f + chrono::Duration::days(1),
        Some(s) => chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| {
            anyhow::anyhow!("Invalid --to date '{s}': {e}. Use ISO 8601 date, e.g. 2026-06-30")
        })?,
    };
    Ok((
        f.and_hms_opt(0, 0, 0).expect("valid time of day").and_utc(),
        t.and_hms_opt(23, 59, 59)
            .expect("valid time of day")
            .and_utc(),
    ))
}

fn parse_dt_cli(s: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.to_utc())
        .map_err(|e| {
            format!(
            "Invalid datetime '{s}': {e}. Use ISO 8601, e.g. \"2026-06-07T10:00:00+09:00\"",
        )
        })
}

fn print_events(label: &str, events: &[oxios_calendar::Event]) {
    if events.is_empty() {
        println!("{label}: No events.");
        return;
    }
    println!(
        "{} {} ({} events):",
        style("📅").bold(),
        label,
        events.len()
    );
    println!("{}", "─".repeat(50));
    for e in events {
        let time = e.start.format("%H:%M");
        let end = e.end.format("%H:%M");
        println!("  **{}–{}** {}", time, end, style(&e.title).bold());
        if let Some(ref loc) = e.location {
            println!("     📍 {loc}");
        }
    }
}

// ─── Email setup ────────────────────────────────────────────────────────────

async fn cmd_email_setup(kernel: &Kernel) {
    use console::style;
    use inquire::{Select, Text};

    println!(
        "{}\n  Oxios Email Setup\n{}",
        "─".repeat(40),
        "─".repeat(40)
    );

    // Check if already configured
    let handle = kernel.handle();
    if handle.email.read().is_some() {
        println!(
            "{} Email is already configured.",
            style("⚠").yellow().bold()
        );
        println!("  To reconfigure, update config.toml and restart.");
        return;
    }

    // Step 1: Email address
    let my_email = Text::new("Your email address:")
        .prompt()
        .unwrap_or_default();
    if my_email.is_empty() {
        eprintln!("{} Email address is required.", style("✗").red().bold());
        return;
    }

    // Step 2: Provider
    let provider = Select::new(
        "SMTP provider:",
        vec!["resend", "gmail", "icloud", "fastmail", "custom"],
    )
    .prompt()
    .unwrap_or("resend")
    .to_string();

    // Step 3: Password
    if provider == "resend" {
        println!("\n  Get your Resend API key at: https://resend.com/api-keys");
        println!("  The API key starts with 're_' and is used as the SMTP password.");
    } else if provider == "gmail" {
        println!("\n  For Gmail: use an App Password (not your regular password).");
        println!("  Create one at: https://myaccount.google.com/apppasswords");
    } else if provider == "icloud" {
        println!("\n  For iCloud: use an App-Specific Password (not your regular password).");
        println!("  Create one at: https://appleid.apple.com");
    }
    let password_label = match provider.as_str() {
        "resend" => "Resend API key:",
        _ => "SMTP password / app password:",
    };
    let password = Text::new(password_label).prompt().unwrap_or_default();
    if password.is_empty() {
        eprintln!("{} Password is required.", style("✗").red().bold());
        return;
    }

    // Step 4: Build config and test
    let smtp_provider = match provider.as_str() {
        "resend" => oxios_kernel::email::SmtpProvider::Resend,
        "gmail" => oxios_kernel::email::SmtpProvider::Gmail,
        "icloud" => oxios_kernel::email::SmtpProvider::Icloud,
        "fastmail" => oxios_kernel::email::SmtpProvider::Fastmail,
        _ => oxios_kernel::email::SmtpProvider::Custom,
    };

    let config = oxios_kernel::config::EmailConfig {
        enabled: true,
        my_email: my_email.clone(),
        provider: smtp_provider,
        host: String::new(),
        port: 0,
        tls: None,
        user: String::new(),
        secret_ref: "email_smtp".to_string(),
        rate_limit_per_hour: 10,
    };

    println!("\n  Testing SMTP connection...");
    match oxios_kernel::SmtpClient::from_config(&config, &password) {
        Ok(smtp) => match smtp.test_connection().await {
            Ok(()) => {
                // Save credentials
                let token = oxicode_sdk::TokenBundle {
                    access_token: password,
                    refresh_token: None,
                    token_type: "Bearer".to_string(),
                    obtained_at: chrono::Utc::now(),
                    expires_in: 0,
                    scope: None,
                };
                if let Err(e) = oxicode_sdk::save_token("email_smtp", &token) {
                    eprintln!(
                        "{} Failed to save credentials: {}",
                        style("✗").red().bold(),
                        e
                    );
                    return;
                }
                // Save to config.toml
                let config_path = oxios_kernel::config::expand_home(&format!(
                    "{}/.oxios/config.toml",
                    std::env::var("HOME").unwrap_or_default()
                ));
                if config_path.exists() {
                    let _ = upsert_email_section_in_config_main(&config_path, &config);
                }

                // Live kernel update — swap into shared slot (no restart needed)
                let handle = kernel.handle();
                let workspace = handle.state.workspace_path().to_path_buf();
                let template_dir = workspace.join("email_templates");
                let _ = std::fs::create_dir_all(&template_dir);
                let new_api = oxios_kernel::EmailApi::new(
                    smtp,
                    template_dir,
                    handle.state.store().clone(),
                    Some(handle.infra.event_bus_clone()),
                    config.rate_limit_per_hour,
                );
                *handle.email.write() = Some(new_api);

                println!(
                    "{} Email configured successfully!",
                    style("✓").green().bold()
                );
                println!("  Email: {}", style(&my_email).cyan());
                println!("  Provider: {}", style(&provider).cyan());
            }
            Err(e) => {
                eprintln!("{} SMTP test failed: {}", style("✗").red().bold(), e);
            }
        },
        Err(e) => {
            eprintln!("{} Invalid SMTP config: {}", style("✗").red().bold(), e);
        }
    }
}

/// Insert or replace the [email] section in config.toml.
fn upsert_email_section_in_config_main(
    config_path: &std::path::Path,
    config: &oxios_kernel::config::EmailConfig,
) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(config_path)?;
    let provider_str = match config.provider {
        oxios_kernel::email::SmtpProvider::Resend => "resend",
        oxios_kernel::email::SmtpProvider::Gmail => "gmail",
        oxios_kernel::email::SmtpProvider::Icloud => "icloud",
        oxios_kernel::email::SmtpProvider::Fastmail => "fastmail",
        oxios_kernel::email::SmtpProvider::Custom => "custom",
    };
    let new_section = format!(
        "# Email (configured by `oxios email setup`)\n[email]\nenabled = true\nmy_email = \"{}\"\nprovider = \"{}\"\n",
        config.my_email, provider_str
    );

    if let Some(email_pos) = content.find("[email]") {
        let line_start = content[..email_pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let rest = &content[email_pos..];
        let section_end = rest[1..]
            .find("\n[")
            .map(|p| email_pos + 1 + p + 1)
            .unwrap_or(content.len());
        let mut result = String::with_capacity(content.len());
        result.push_str(&content[..line_start]);
        result.push_str(&new_section);
        result.push_str(&content[section_end..]);
        std::fs::write(config_path, result)?;
    } else {
        let mut result = content;
        if !result.ends_with('\n') {
            result.push('\n');
        }
        result.push('\n');
        result.push_str(&new_section);
        std::fs::write(config_path, result)?;
    }
    Ok(())
}

// ─── Web command ────────────────────────────────────────────────────────────

fn cmd_web(config: &OxiosConfig, port_override: Option<u16>) -> Result<()> {
    let port = port_override.unwrap_or(config.gateway.port);

    // Ensure daemon is running
    let daemon = DaemonManager::new(&config.daemon.pid_file, &config.daemon.log_dir);
    let was_running = matches!(daemon.status(), oxios_kernel::DaemonStatus::Running { .. });

    if !was_running {
        println!("  {} Daemon not running — starting...", style("⠋").cyan());
        let config_path = oxios_kernel::config::expand_home(&format!(
            "{}/.oxios/config.toml",
            std::env::var("HOME").unwrap_or_default()
        ));
        daemon.start(&config_path, port)?;

        // Give the server a moment to bind the port
        let url = format!("http://127.0.0.1:{port}");
        let mut attempts = 0;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(300));
            if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
                break;
            }
            attempts += 1;
            if attempts >= 20 {
                println!(
                    "  {} Server didn't start in time. Open manually: {}",
                    style("⚠").yellow(),
                    style(&url).cyan()
                );
                return Ok(());
            }
        }
    }

    let url = format!("http://127.0.0.1:{port}");
    println!("  {} Opening {}", style("↗").green(), style(&url).cyan());

    webbrowser::open(&url).map_err(|e| anyhow::anyhow!("failed to open browser: {e}"))?;

    Ok(())
}

// ─── Entry point ─────────────────────────────────────────────────────────────

/// Install a panic hook that records the panic synchronously *before* the
/// process aborts.
///
/// Release builds use `panic = "abort"` (root `Cargo.toml`), so any panic —
/// in a request handler, a background loop, or a spawned task — calls
/// `abort()` immediately. The default hook writes to stderr (which the
/// daemon launcher redirects to `oxios.log`), but without a stable marker
/// and without append-mode logging the message was truncated/lost on
/// restart, and the in-process `TaskSupervisor` could not observe it at
/// all. This hook writes a greppable `PANIC` line synchronously to stderr
/// (the non-blocking tracing appender is NOT flushed on abort), then chains
/// to the previous hook for the full backtrace. Paired with append-mode
/// logging, a panic-death is finally diagnosable instead of invisible.
fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string panic payload>");
        eprintln!(
            "\nPANIC in oxios daemon @ {location}: {payload}\n  \
             thread: {thread} — process will abort",
            thread = std::thread::current().name().unwrap_or("<unnamed>")
        );
        prev(info);
    }));
}

fn main() {
    install_panic_hook();

    // Parse CLI and set OXICODE_HOME BEFORE starting the tokio runtime.
    // Rust 2024: env::set_var is UB when other threads may access the
    // environment concurrently. The multi-threaded tokio runtime (created
    // below) spawns worker threads, so we set this env var first.
    let cli = Cli::parse();
    let config_path = oxios_kernel::config::expand_home(&cli.config);
    let oxios_home = oxios_home_from_config(&config_path);
    if std::env::var("OXICODE_HOME").is_err() {
        // SAFETY: single-threaded — tokio runtime hasn't been created yet.
        unsafe {
            std::env::set_var("OXICODE_HOME", &oxios_home);
        }
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to create tokio runtime");

    if let Err(e) = runtime.block_on(run(cli)) {
        eprintln!();
        eprintln!("  {} {}", style("error:").red().bold(), e);
        eprintln!(
            "  Run {} for diagnostics.\n",
            style("`oxios doctor`").cyan()
        );
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    let config_path = oxios_kernel::config::expand_home(&cli.config);
    let oxios_home = oxios_home_from_config(&config_path);
    // OXICODE_HOME was set in main() before the tokio runtime started — Rust 2024
    // requires single-threaded env mutation.

    // Detect first run (before ensure_workspace creates the dir).
    let is_first_run = !oxios_home.join("config.toml").exists();

    ensure_workspace(&oxios_home)?;

    // ── Load config ──
    let mut config = if config_path.exists() {
        oxios_kernel::config::load_config(&config_path)?
    } else {
        OxiosConfig::default()
    };

    // ── Tracing setup ──
    let log_dir = oxios_kernel::config::expand_home(&config.daemon.log_dir);
    std::fs::create_dir_all(&log_dir)?;
    let file_appender = tracing_appender::rolling::daily(&log_dir, "oxios.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    Box::leak(Box::new(_guard));

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        if cli.verbose {
            tracing_subscriber::EnvFilter::new("debug")
        } else if let Some(ref level) = config.logging.level {
            tracing_subscriber::EnvFilter::new(level)
        } else {
            tracing_subscriber::EnvFilter::new("info")
        }
    });

    match config.logging.format.as_str() {
        "json" => {
            tracing_subscriber::fmt()
                .json()
                .with_env_filter(env_filter)
                .with_target(true)
                .with_writer(non_blocking)
                .init();
        }
        "compact" => {
            tracing_subscriber::fmt()
                .compact()
                .with_env_filter(env_filter)
                .with_target(true)
                .with_writer(non_blocking)
                .init();
        }
        _ => {
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_target(true)
                .with_thread_ids(false)
                .with_file(false)
                .with_line_number(false)
                .compact()
                .with_writer(non_blocking)
                .init();
        }
    }

    // ── Fast-path: commands that never need the kernel ──
    match &cli.command {
        Some(Command::Stop) => {
            let daemon = DaemonManager::new(&config.daemon.pid_file, &config.daemon.log_dir)
                .with_probe_port(config.gateway.port);
            return daemon.stop();
        }
        Some(Command::Daemon { action }) => {
            let daemon = DaemonManager::new(&config.daemon.pid_file, &config.daemon.log_dir);
            return match action {
                DaemonAction::Install => daemon.install_service(),
                DaemonAction::Uninstall => daemon.uninstall_service(),
            };
        }
        Some(Command::Log { lines }) => {
            let log_file = log_dir.join("oxios.log");
            if !log_file.exists() {
                println!("  No log file at {}", log_file.display());
                return Ok(());
            }
            print!("{}", tail_file(&log_file, *lines)?);
            return Ok(());
        }
        Some(Command::Config { action }) => {
            let action = action.clone().unwrap_or(ConfigAction::Show);
            return cmd_config(&action, &config_path).await;
        }
        Some(Command::Onboard) => {
            let result = oxios_kernel::onboarding::run_onboarding(&oxios_home, &mut config, false)?;
            if result.skipped {
                println!("  Onboarding skipped or cancelled.");
            }
            return Ok(());
        }
        Some(Command::Reset { yes }) => {
            let pid_file = oxios_kernel::config::expand_home(&config.daemon.pid_file);
            return cmd_reset(&oxios_home, *yes, &pid_file);
        }
        Some(Command::Models { provider }) => {
            return cmd_models(provider.as_deref());
        }
        Some(Command::Web { port }) => {
            return cmd_web(&config, *port);
        }
        Some(Command::Update {
            web_only,
            binary_only,
            version,
            dry_run,
            yes,
            no_restart,
        }) => {
            let outcome = commands::update::run_update(
                *web_only,
                *binary_only,
                version.as_deref(),
                *dry_run,
                *yes,
            )
            .await?;

            // Auto-restart the daemon so the new binary/web UI takes effect.
            // We only restart when the daemon is *already* running — starting a
            // background daemon the user never launched would be surprising.
            if !*no_restart && outcome.any() {
                println!();
                let port = config.gateway.port;
                let daemon = DaemonManager::new(&config.daemon.pid_file, &config.daemon.log_dir)
                    .with_probe_port(port);
                if matches!(daemon.status(), oxios_kernel::DaemonStatus::Running { .. }) {
                    println!(
                        "  {} Restarting daemon to activate the update...",
                        style("⟳").cyan()
                    );
                    // The update itself already succeeded (new files are on
                    // disk), so a restart failure must NOT mask that success.
                    // We warn instead of propagating, and tell the user how to
                    // recover — `restart()` does stop()+start(), so on failure
                    // the daemon may be left stopped.
                    match daemon.restart(&config_path, port) {
                        Ok(()) => println!("  {} Daemon restarted.", style("✓").green()),
                        Err(e) => {
                            println!(
                                "  {} Update succeeded, but daemon restart failed: {e}",
                                style("⚠").yellow()
                            );
                            println!(
                                "    Run `{}` manually to launch the new build.",
                                style("oxios start").bold()
                            );
                        }
                    }
                } else {
                    println!(
                        "  {} Daemon is not running. Run `{}` to start it with the new build.",
                        style("ℹ").cyan(),
                        style("oxios restart").bold()
                    );
                }
            }

            return Ok(());
        }
        Some(Command::Changelog { version }) => {
            return commands::update::run_changelog(version.as_deref()).await;
        }
        Some(Command::Completion { shell }) => {
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            generate(*shell, &mut cmd, name, &mut std::io::stdout());
            return Ok(());
        }
        _ => {}
    }

    // ── Onboarding gate ──
    // Commands that need the kernel assembled (and therefore credentials).
    let needs_kernel = matches!(
        cli.command.as_ref(),
        None | Some(Command::Start { .. })
            | Some(Command::Run { .. })
            | Some(Command::Chat)
            | Some(Command::Status)
            | Some(Command::Doctor)
            | Some(Command::Agent { .. })
            | Some(Command::Backup { .. })
            | Some(Command::Restore { .. })
            | Some(Command::Audit)
            | Some(Command::Budget { .. })
            | Some(Command::Git { .. })
            | Some(Command::Pkg { .. })
            | Some(Command::Marketplace { .. })
    );

    if needs_kernel && !oxios_kernel::onboarding::has_credentials(&config) {
        let result =
            oxios_kernel::onboarding::run_onboarding(&oxios_home, &mut config, is_first_run)?;
        if result.configured {
            if config_path.exists() {
                config = oxios_kernel::config::load_config(&config_path)?;
            }
            // fall through to kernel assembly → daemon start
        } else {
            return Ok(());
        }
    }

    // ── Kernel assembly ──
    let term = console::Term::stderr();
    let _ = term.write_str(&format!("  {} Starting Oxios...\r", style("⠋").cyan()));
    let _ = term.flush();

    let mut kernel = Kernel::builder()
        .config_path(config_path.clone())
        .build()
        .await?;

    let _ = term.clear_line();

    // ── Dispatch subcommands ──
    match cli.command.as_ref() {
        // Default / start: launch daemon
        None => {
            start_daemon(
                &mut kernel,
                &config,
                &config_path,
                false,
                None,
                cli.foreground,
            )
            .await
        }
        Some(Command::Start {
            remote,
            pairing_address,
        }) => {
            start_daemon(
                &mut kernel,
                &config,
                &config_path,
                *remote,
                pairing_address.clone(),
                cli.foreground,
            )
            .await
        }

        Some(Command::Restart) => {
            let daemon = DaemonManager::new(&config.daemon.pid_file, &config.daemon.log_dir);
            daemon.restart(&config_path, config.gateway.port)
        }

        Some(Command::Run {
            prompt,
            json,
            session,
            context_file,
            exit_code,
            chat,
        }) => {
            let opts = commands::run::RunOptions {
                json: *json,
                session_id: session.clone(),
                context_file: context_file.clone(),
                exit_code: *exit_code,
                chat: *chat,
            };
            let code = commands::run::run(&kernel, prompt, &opts).await?;
            std::process::exit(code);
        }

        Some(Command::Status) => cmd_status(&kernel).await,

        Some(Command::Doctor) => cmd_doctor(&kernel, &config_path).await,

        Some(Command::Chat) => {
            #[cfg(feature = "cli")]
            {
                let cli_channel = crate::channels::cli::CliChannel::new(256);
                let handle = cli_channel.handle();
                if let Err(e) = kernel.register_channel(Box::new(cli_channel)).await {
                    tracing::error!(error = %e, "Failed to register CLI channel");
                }
                let mut loop_ = crate::channels::cli::InteractiveLoop::new(handle);
                loop_.run().await?;
                Ok(())
            }
            #[cfg(not(feature = "cli"))]
            {
                anyhow::bail!("CLI channel not compiled in. Rebuild with --features cli");
            }
        }

        Some(Command::Backup { output }) => {
            let handle = kernel.handle();
            let output_path = match output {
                Some(p) => PathBuf::from(p),
                None => {
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    PathBuf::from(kernel.config().kernel.workspace.clone())
                        .join("backups")
                        .join(ts.to_string())
                }
            };
            oxios_kernel::backup::create_backup(handle.state.store(), &output_path).await?;
            println!(
                "  {} Backup written to {}",
                style("✓").green().bold(),
                style(output_path.display()).cyan()
            );
            Ok(())
        }

        Some(Command::Restore { input }) => {
            let handle = kernel.handle();
            let input_path = PathBuf::from(&input);
            oxios_kernel::backup::restore_backup(handle.state.store(), &input_path).await?;
            println!(
                "  {} Restored from {}",
                style("✓").green().bold(),
                style(input_path.display()).cyan()
            );
            Ok(())
        }

        Some(Command::Pkg { action }) => cmd_pkg(&kernel, action).await,

        Some(Command::Agent { action }) => {
            let handle = kernel.handle();
            match action {
                AgentAction::List => {
                    let agents = handle
                        .agents
                        .list()
                        .await
                        .map_err(|e| anyhow::anyhow!("failed to list agents: {e}"))?;
                    if agents.is_empty() {
                        println!("  No active agents.");
                    } else {
                        println!("{:36} {:10} {:20} CREATED", "ID", "STATUS", "NAME");
                        println!("{}", "─".repeat(90));
                        for agent in &agents {
                            println!(
                                "{:36} {:10} {:20} {}",
                                agent.id,
                                format!("{:?}", agent.status),
                                agent.name,
                                agent.created_at.format("%Y-%m-%d %H:%M")
                            );
                        }
                        println!("\n{} agent(s) active.", agents.len());
                    }
                    Ok(())
                }
                AgentAction::Kill { id } => {
                    let _ = uuid::Uuid::parse_str(id)
                        .map_err(|e| anyhow::anyhow!("invalid agent id '{id}': {e}"))?;
                    handle
                        .agents
                        .kill(id)
                        .await
                        .map_err(|e| anyhow::anyhow!("failed to kill agent {id}: {e}"))?;
                    println!(
                        "  {} Agent {} terminated.",
                        style("✓").green(),
                        style(id).cyan()
                    );
                    Ok(())
                }
            }
        }

        Some(Command::Audit) => {
            let handle = kernel.handle();
            match handle.security.verify_chain() {
                Ok(_) => println!(
                    "  {} Audit trail verified — chain intact.",
                    style("✓").green().bold()
                ),
                Err(e) => {
                    eprintln!(
                        "  {} Audit verification failed: {:?}",
                        style("✗").red().bold(),
                        e
                    );
                    println!("  Some entries may have been tampered with.");
                }
            }
            let entries = handle.security.query_audit(0, 20);
            println!();
            if entries.is_empty() {
                println!("  No audit entries yet.");
            } else {
                println!("  Recent Audit Entries (showing last {}):", entries.len());
                println!("{:10} {:20} {:15} ACTION", "SEQ", "TIMESTAMP", "ACTOR");
                println!("{}", "─".repeat(70));
                for entry in &entries {
                    println!(
                        "{:10} {:20} {:15} {:?}",
                        entry.seq,
                        entry.timestamp.format("%Y-%m-%d %H:%M:%S"),
                        entry.actor,
                        entry.action
                    );
                }
            }
            println!("\n  Total entries: {}", handle.security.audit_count());
            Ok(())
        }

        Some(Command::Git { action }) => {
            let handle = kernel.handle();
            match action {
                GitAction::Log { limit } => {
                    let limit = limit.unwrap_or(20);
                    let entries = handle
                        .infra
                        .git_log(limit)
                        .map_err(|e| anyhow::anyhow!("failed to get git log: {e}"))?;
                    if entries.is_empty() {
                        println!("  No commits yet.");
                    } else {
                        println!("{:8} {:20} {:40}", "HASH", "AUTHOR", "MESSAGE");
                        println!("{}", "─".repeat(75));
                        for entry in entries {
                            let short_hash = &entry.hash[..8.min(entry.hash.len())];
                            let author = entry.author.chars().take(20).collect::<String>();
                            let msg = entry.message.chars().take(40).collect::<String>();
                            println!("{short_hash:8} {author:20} {msg:40}");
                        }
                    }
                    Ok(())
                }
                GitAction::Tag { name, message } => {
                    let msg = message.as_deref().unwrap_or("");
                    handle
                        .infra
                        .git_tag(name, msg)
                        .map_err(|e| anyhow::anyhow!("failed to create tag: {e}"))?;
                    println!("  {} '{}'.", style("Tagged").green(), style(name).cyan());
                    if !msg.is_empty() {
                        println!("  Message: {msg}");
                    }
                    Ok(())
                }
            }
        }

        Some(Command::Budget { agent_id }) => {
            let handle = kernel.handle();
            match agent_id {
                Some(id) => {
                    let uuid = uuid::Uuid::parse_str(id)
                        .map_err(|e| anyhow::anyhow!("invalid agent id '{id}': {e}"))?;
                    let budget = handle.agents.check_budget(&uuid);
                    println!("\n  Agent: {id}");
                    println!("  {}", "─".repeat(40));
                    println!("  {:<22}  {}", "Tokens remaining:", budget.tokens_remaining);
                    println!("  {:<22}  {}", "Calls remaining:", budget.calls_remaining);
                    println!(
                        "  {:<22}  {} seconds",
                        "Window remaining:", budget.window_remaining_secs
                    );
                    println!(
                        "  {:<22}  {}",
                        "Status:",
                        if budget.is_exhausted {
                            style("⚠ EXHAUSTED").yellow().bold().to_string()
                        } else {
                            style("✓ OK").green().to_string()
                        }
                    );
                    println!();
                    Ok(())
                }
                None => {
                    let agents = handle
                        .agents
                        .list()
                        .await
                        .map_err(|e| anyhow::anyhow!("failed to list agents: {e}"))?;
                    println!("\n  Agent Budget Overview");
                    println!("  {}", "─".repeat(60));
                    if agents.is_empty() {
                        println!("  No active agents.");
                        println!("  Run `oxios run \"<prompt>\"` to start one.");
                    } else {
                        println!("  {:<38} {:<14} TOKENS LEFT", "AGENT ID", "STATUS");
                        println!("  {}", "─".repeat(60));
                        for agent in &agents {
                            let budget = handle.agents.check_budget(&agent.id);
                            let status = if budget.is_exhausted {
                                style("⚠ EXHAUSTED").yellow().bold().to_string()
                            } else {
                                format!("{:?}", agent.status)
                            };
                            println!(
                                "  {:<38} {:<14} {}",
                                agent.id, status, budget.tokens_remaining
                            );
                        }
                        println!(
                            "\n  {} agent(s). Use `oxios budget <agent-id>` for details.",
                            agents.len()
                        );
                    }
                    println!();
                    Ok(())
                }
            }
        }

        Some(Command::Marketplace { action }) => {
            let api = kernel.handle().marketplace_api.clone();
            match action {
                MarketplaceAction::Search { query, limit } => {
                    let results = api.search(query, Some(*limit)).await?;
                    if results.is_empty() {
                        println!("  No results for '{query}'");
                    } else {
                        for r in results {
                            println!(
                                "{} - {} ({})",
                                style(&r.slug).bold(),
                                r.display_name,
                                r.version.as_deref().unwrap_or("unknown")
                            );
                            if let Some(summary) = &r.summary {
                                println!("  {}", summary.chars().take(80).collect::<String>());
                            }
                            println!();
                        }
                    }
                }
                MarketplaceAction::Install { slug, version } => {
                    match api.install(slug, version.as_deref()).await {
                        Ok(result) => {
                            println!(
                                "  {} {} v{}",
                                style("Installed").green().bold(),
                                style(&result.slug).cyan(),
                                style(&result.version).cyan()
                            );
                        }
                        Err(e) => {
                            eprintln!(
                                "  {} Failed to install '{}': {}",
                                style("✗").red().bold(),
                                slug,
                                e
                            );
                        }
                    }
                }
                MarketplaceAction::Update { slug } => {
                    if let Some(s) = slug {
                        match api.update(s).await {
                            Ok(result) => {
                                if result.changed {
                                    println!(
                                        "  {} {}: {} → {}",
                                        style("Updated").green().bold(),
                                        result.slug,
                                        style(result.previous_version.as_deref().unwrap_or("?"))
                                            .yellow(),
                                        style(&result.version).cyan()
                                    );
                                } else {
                                    println!("  {} is already up to date", result.slug);
                                }
                            }
                            Err(e) => {
                                eprintln!(
                                    "  {} Failed to update '{}': {}",
                                    style("✗").red().bold(),
                                    s,
                                    e
                                );
                            }
                        }
                    } else {
                        let results = api.update_all().await?;
                        if results.is_empty() {
                            println!("  No ClawHub skills installed.");
                        } else {
                            for r in results {
                                if r.changed {
                                    println!(
                                        "  {} {}: {} → {}",
                                        style("Updated").green().bold(),
                                        r.slug,
                                        style(r.previous_version.as_deref().unwrap_or("?"))
                                            .yellow(),
                                        style(&r.version).cyan()
                                    );
                                } else if r.ok {
                                    println!("  {} is already up to date", r.slug);
                                } else {
                                    eprintln!(
                                        "  {} Failed to update {}: {}",
                                        style("✗").red().bold(),
                                        r.slug,
                                        r.error.as_deref().unwrap_or("unknown error")
                                    );
                                }
                            }
                        }
                    }
                }
                MarketplaceAction::Updates => match api.check_updates().await {
                    Ok(updates) => {
                        if updates.is_empty() {
                            println!("  All skills up to date");
                        } else {
                            println!("  Available updates:");
                            println!("  {}", "─".repeat(50));
                            for u in updates {
                                println!(
                                    "  {}: {} → {}",
                                    style(&u.slug).bold(),
                                    style(&u.current_version).yellow(),
                                    style(&u.latest_version).cyan()
                                );
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "  {} Failed to check updates: {}",
                            style("✗").red().bold(),
                            e
                        );
                    }
                },
            }
            Ok(())
        }

        Some(Command::Project { action }) => {
            let pm = kernel.project_manager();
            match action {
                ProjectAction::List => {
                    let projects = pm.list_projects();
                    if projects.is_empty() {
                        println!("No projects registered.");
                        println!(
                            "Use `oxios project add <name> --path /path/to/project` to register one."
                        );
                    } else {
                        println!(
                            "{}",
                            style(format!("Projects ({}):", projects.len())).bold()
                        );
                        println!("{}", "─".repeat(50));
                        for p in &projects {
                            let paths_str = if p.paths.is_empty() {
                                "(no paths)".to_string()
                            } else {
                                p.paths
                                    .iter()
                                    .map(|x| x.to_string_lossy().to_string())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            };
                            println!("  {} {} — {}", p.emoji, style(&p.name).bold(), &paths_str);
                            if !p.tags.is_empty() {
                                println!("     tags: {}", p.tags.join(", "));
                            }
                        }
                    }
                }
                ProjectAction::Show { name } => {
                    let project = if let Ok(id) = uuid::Uuid::parse_str(name) {
                        pm.get_project(id)
                    } else {
                        pm.get_project_by_name(name)
                    };
                    match project {
                        Some(p) => {
                            println!("{}", style(format!("{} {}", p.emoji, p.name)).bold());
                            println!("{}", "─".repeat(30));
                            println!("  ID:          {}", p.id);
                            if !p.description.is_empty() {
                                println!("  Description: {}", p.description);
                            }
                            println!("  Source:       {}", p.source);
                            println!(
                                "  Paths:       {}",
                                if p.paths.is_empty() {
                                    "(none)".to_string()
                                } else {
                                    p.paths
                                        .iter()
                                        .map(|x| x.to_string_lossy().to_string())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                }
                            );
                            if !p.tags.is_empty() {
                                println!("  Tags:        {}", p.tags.join(", "));
                            }
                            println!("  Created:     {}", p.created_at.to_rfc3339());
                            println!("  Last active: {}", p.last_active_at.to_rfc3339());
                        }
                        None => {
                            eprintln!("{} Project '{}' not found", style("✗").red().bold(), name)
                        }
                    }
                }
                ProjectAction::Add {
                    name,
                    paths,
                    tags,
                    emoji,
                    description,
                } => {
                    // RFC-025: paths live on Mounts. Create the Project, then
                    // promote any provided paths into Mounts and link them via
                    // `mount_ids` so the project is a usable code context in one
                    // step. When the mount system is unavailable, paths are
                    // ignored with a warning.
                    match pm.create_project(
                        name.clone(),
                        tags.clone(),
                        Some(emoji.clone()),
                        description.clone(),
                        oxios_kernel::ProjectSource::Manual,
                    ) {
                        Ok(p) => {
                            if !paths.is_empty() {
                                if let Some(mm) = kernel.mount_manager() {
                                    let mut mount_ids: Vec<oxios_kernel::MountId> = Vec::new();
                                    for raw in paths {
                                        let root = std::path::PathBuf::from(raw);
                                        match mm.covering_mount_id(&root) {
                                            Some(mid) => {
                                                if !mount_ids.contains(&mid) {
                                                    mount_ids.push(mid);
                                                }
                                            }
                                            None => match mm.create_mount(
                                                p.name.clone(),
                                                vec![root],
                                                oxios_kernel::MountSource::Manual,
                                            ) {
                                                Ok(m) => mount_ids.push(m.id),
                                                Err(e) => eprintln!(
                                                    "{} Could not create mount for '{}': {}",
                                                    style("✗").red().bold(),
                                                    raw,
                                                    e
                                                ),
                                            },
                                        }
                                    }
                                    if !mount_ids.is_empty()
                                        && let Err(e) =
                                            pm.update_project_bundle(p.id, Some(mount_ids), None)
                                    {
                                        eprintln!(
                                            "{} Project created but mount link failed: {}",
                                            style("✗").red().bold(),
                                            e
                                        );
                                    }
                                } else {
                                    eprintln!(
                                        "{} Mount system unavailable — paths ignored. \
                                         Use `oxios mount add` and link separately.",
                                        style("⚠").yellow().bold()
                                    );
                                }
                            }
                            println!(
                                "{} Project '{}' created ({})",
                                style("✓").green().bold(),
                                p.name,
                                p.id
                            );
                        }
                        Err(e) => {
                            eprintln!(
                                "{} Failed to create project: {}",
                                style("✗").red().bold(),
                                e
                            );
                        }
                    }
                }
                ProjectAction::Remove { name } => {
                    let project = if let Ok(id) = uuid::Uuid::parse_str(name) {
                        pm.get_project(id).map(|p| p.id)
                    } else {
                        pm.get_project_by_name(name).map(|p| p.id)
                    };
                    match project {
                        Some(id) => match pm.remove_project(id) {
                            Ok(()) => {
                                println!("{} Project '{}' removed", style("✓").green().bold(), name)
                            }
                            Err(e) => eprintln!(
                                "{} Failed to remove project: {}",
                                style("✗").red().bold(),
                                e
                            ),
                        },
                        None => {
                            eprintln!("{} Project '{}' not found", style("✗").red().bold(), name)
                        }
                    }
                }
            }
            Ok(())
        }

        Some(Command::Calendar { action }) => {
            let handle = kernel.handle();
            let api = handle.calendar.as_ref();
            if api.is_none() {
                eprintln!(
                    "{} Calendar is not enabled. Add `[calendar] enabled = true` to config.toml",
                    style("✗").red().bold()
                );
                return Ok(());
            }
            let api = api.expect("checked non-None in the guard above");
            match action {
                CalendarAction::Today => {
                    let (from, to) = today_range();
                    match api.list(from, to).await {
                        Ok(events) => print_events("Today", &events),
                        Err(e) => eprintln!("{} {}", style("✗").red().bold(), e),
                    }
                }
                CalendarAction::Tomorrow => {
                    let (from, to) = tomorrow_range();
                    match api.list(from, to).await {
                        Ok(events) => print_events("Tomorrow", &events),
                        Err(e) => eprintln!("{} {}", style("✗").red().bold(), e),
                    }
                }
                CalendarAction::Week => {
                    let (from, to) = week_range();
                    match api.list(from, to).await {
                        Ok(events) => print_events("This Week", &events),
                        Err(e) => eprintln!("{} {}", style("✗").red().bold(), e),
                    }
                }
                CalendarAction::List { from, to } => {
                    let (f, t) = match parse_range(from.clone(), to.clone()) {
                        Ok(r) => r,
                        Err(e) => {
                            eprintln!("{} {}", style("✗").red().bold(), e);
                            return Ok(());
                        }
                    };
                    match api.list(f, t).await {
                        Ok(events) => print_events(
                            &format!(
                                "Events {} to {}",
                                f.format("%Y-%m-%d"),
                                t.format("%Y-%m-%d")
                            ),
                            &events,
                        ),
                        Err(e) => eprintln!("{} {}", style("✗").red().bold(), e),
                    }
                }
                CalendarAction::Create {
                    title,
                    start,
                    end,
                    location,
                    description,
                    reminder,
                } => {
                    let start_dt = parse_dt_cli(start);
                    let end_dt = parse_dt_cli(end);
                    match (start_dt, end_dt) {
                        (Ok(s), Ok(e)) => {
                            let draft = oxios_calendar::EventDraft {
                                title: title.clone(),
                                start: s,
                                end: e,
                                all_day: false,
                                description: description.clone(),
                                location: location.clone(),
                                repeat: None,
                                reminder_minutes: reminder.clone().unwrap_or_default(),
                                source: oxios_calendar::EventSource::User,
                                note_path: None,
                            };
                            match api.create(draft).await {
                                Ok(r) => {
                                    println!(
                                        "{} Event created: {} ({})",
                                        style("✓").green().bold(),
                                        r.uid,
                                        r.file
                                    );
                                    if !r.conflicts.is_empty() {
                                        for c in &r.conflicts {
                                            eprintln!(
                                                "  {} Conflicts with '{}' ({}min overlap)",
                                                style("⚠").yellow().bold(),
                                                c.title,
                                                c.overlap_minutes
                                            );
                                        }
                                    }
                                }
                                Err(e) => eprintln!("{} {}", style("✗").red().bold(), e),
                            }
                        }
                        (Err(e), _) | (_, Err(e)) => eprintln!("{} {}", style("✗").red().bold(), e),
                    }
                }
                CalendarAction::Delete { uid } => match api.delete(uid).await {
                    Ok(()) => println!("{} Event deleted", style("✓").green().bold()),
                    Err(e) => eprintln!("{} {}", style("✗").red().bold(), e),
                },
                CalendarAction::Search { query } => match api.search(query).await {
                    Ok(events) => print_events(&format!("Search: {query}"), &events),
                    Err(e) => eprintln!("{} {}", style("✗").red().bold(), e),
                },
                CalendarAction::Freebusy { date } => {
                    let d = date.as_deref().unwrap_or("today");
                    let (from, to) = if d == "today" {
                        today_range()
                    } else {
                        match parse_range(Some(d.to_string()), None) {
                            Ok(r) => r,
                            Err(e) => {
                                eprintln!("{} {}", style("✗").red().bold(), e);
                                return Ok(());
                            }
                        }
                    };
                    match api.freebusy(from, to).await {
                        Ok(slots) => {
                            println!("{} Free/Busy:", style("📅").bold());
                            for slot in &slots {
                                let label = if slot.busy { "BUSY" } else { "free" };
                                let icon = if slot.busy { "🔴" } else { "🟢" };
                                println!(
                                    "  {} {} – {} [{}]",
                                    icon,
                                    slot.start.format("%H:%M"),
                                    slot.end.format("%H:%M"),
                                    label
                                );
                            }
                        }
                        Err(e) => eprintln!("{} {}", style("✗").red().bold(), e),
                    }
                }
            }
            Ok(())
        }

        Some(Command::Email { action }) => {
            let handle = kernel.handle();
            match action {
                EmailAction::Setup => {
                    cmd_email_setup(&kernel).await;
                }
                EmailAction::Test => {
                    let api = handle.email.read().clone();
                    if let Some(api) = api {
                        let default_to = api.default_to().to_string();
                        match api.test_connection().await {
                            Ok(()) => println!(
                                "{} Test email sent to {}",
                                style("✓").green().bold(),
                                default_to
                            ),
                            Err(e) => {
                                eprintln!("{} SMTP test failed: {}", style("✗").red().bold(), e)
                            }
                        }
                    } else {
                        eprintln!(
                            "{} Email is not configured. Run `oxios email setup` first.",
                            style("✗").red().bold()
                        );
                    }
                }
                EmailAction::History { limit } => {
                    let state_store = handle.state.store();
                    let sent_dir = state_store.base_path.join("email_sent");
                    if !sent_dir.exists() {
                        println!("No emails sent yet.");
                        return Ok(());
                    }
                    let mut records: Vec<serde_json::Value> = Vec::new();
                    for entry in std::fs::read_dir(&sent_dir)? {
                        let entry = entry?;
                        if entry.path().extension().is_some_and(|ext| ext == "json")
                            && let Ok(content) = std::fs::read_to_string(entry.path())
                            && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
                        {
                            records.push(val);
                        }
                    }
                    // Sort by sent_at descending
                    records.sort_by(|a, b| {
                        let sa = a.get("sent_at").and_then(|v| v.as_str()).unwrap_or("");
                        let sb = b.get("sent_at").and_then(|v| v.as_str()).unwrap_or("");
                        sb.cmp(sa)
                    });
                    records.truncate(*limit);
                    if records.is_empty() {
                        println!("No emails sent yet.");
                    } else {
                        println!(
                            "{} Email History ({} records)",
                            style("📬").bold(),
                            records.len()
                        );
                        for r in &records {
                            let subject = r.get("subject").and_then(|v| v.as_str()).unwrap_or("?");
                            let sent_at = r.get("sent_at").and_then(|v| v.as_str()).unwrap_or("?");
                            let template = r
                                .get("template_used")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let tpl_tag = if template.is_empty() {
                                String::new()
                            } else {
                                format!(" [{}]", style(template).cyan())
                            };
                            // Parse sent_at for a shorter display
                            let display_time = if sent_at.len() >= 19 {
                                &sent_at[..19]
                            } else {
                                sent_at
                            };
                            println!(
                                "  {} {}{}",
                                style(display_time).dim(),
                                style(subject).white().bold(),
                                tpl_tag,
                            );
                        }
                    }
                }
                EmailAction::Templates => {
                    let api = handle.email.read();
                    if let Some(api) = api.as_ref() {
                        match api.list_templates() {
                            Ok(templates) => {
                                if templates.is_empty() {
                                    println!("No templates saved yet.");
                                } else {
                                    println!(
                                        "{} Email Templates ({} records)",
                                        style("📄").bold(),
                                        templates.len()
                                    );
                                    for name in &templates {
                                        let preview = api.load_template(name).unwrap_or_default();
                                        let first_line = preview
                                            .lines()
                                            .next()
                                            .unwrap_or("")
                                            .chars()
                                            .take(60)
                                            .collect::<String>();
                                        println!(
                                            "  {} {}",
                                            style(name).cyan().bold(),
                                            style(first_line).dim(),
                                        );
                                    }
                                }
                            }
                            Err(e) => eprintln!("{} {}", style("✗").red().bold(), e),
                        }
                    } else {
                        eprintln!(
                            "{} Email is not configured. Run `oxios email setup` first.",
                            style("✗").red().bold()
                        );
                    }
                }
            }
            Ok(())
        }

        // Handled before kernel assembly above — unreachable here
        Some(Command::Stop)
        | Some(Command::Daemon { .. })
        | Some(Command::Log { .. })
        | Some(Command::Config { .. })
        | Some(Command::Onboard)
        | Some(Command::Reset { .. })
        | Some(Command::Models { .. })
        | Some(Command::Web { .. })
        | Some(Command::Completion { .. })
        | Some(Command::Update { .. })
        | Some(Command::Changelog { .. }) => unreachable!(),
    }
}

/// Acquire an exclusive, non-blocking advisory lock on `<pid_file>.lock`,
/// held for the daemon's lifetime (keep the returned guard in scope).
///
/// This is the single source of truth for "an oxios daemon owns this home":
/// unlike the PID file (bypassed by a direct `--foreground`) or the port
/// bind (which misses background-only orphans), a held `flock` is
/// cross-process, binary-agnostic, and auto-released by the kernel on
/// process death — so two daemons can never mutate the shared `~/.oxios`
/// state concurrently (the root cause of the web-dist race that 404'd the
/// UI). Returns `Err` with a hint when another instance already holds it.
#[cfg(unix)]
fn acquire_instance_lock(pid_file: &Path) -> Result<std::fs::File> {
    use std::os::unix::io::AsRawFd;

    let lock_path = pid_file.with_extension("lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("failed to open instance lock {}", lock_path.display()))?;

    // LOCK_EX | LOCK_NB: fail fast (EWOULDBLOCK) rather than block forever.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
            let pid_hint = std::fs::read_to_string(&lock_path)
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok())
                .map(|pid| format!(" — likely PID {pid}"))
                .unwrap_or_default();
            anyhow::bail!(
                "another oxios daemon is already running{pid_hint}.\n  \
                 Run `oxios stop` first, or kill the existing process, then retry.\n  \
                 (lock file: {})",
                lock_path.display(),
            );
        }
        anyhow::bail!("failed to lock {}: {err}", lock_path.display());
    }

    // Best-effort: record our PID in the lock file for diagnostics. Written
    // through a fresh handle; the lock itself lives on `file` (held by the
    // caller for the daemon's lifetime).
    let _ = std::fs::write(&lock_path, std::process::id().to_string());
    Ok(file)
}

/// Non-Unix fallback: daemon install is Unix-only, so on Windows the guard
/// is a no-op (the foreground path still compiles).
#[cfg(not(unix))]
fn acquire_instance_lock(_pid_file: &Path) -> Result<()> {
    Ok(())
}

// ─── Server mode (foreground) ────────────────────────────────────────────────

/// Dispatch entry-point for `oxios start` / `oxios serve` and the bare
/// `oxios` default. Splits the daemonized vs foreground path.
///
/// `--remote` forces the foreground path **only when the `remote` feature is
/// compiled in**: the daemonized child re-reads `config.toml` and would never
/// receive the in-memory override, so the readiness JSON + pairing URL would
/// silently disappear. To run the remote surface under the long-lived daemon,
/// set `[remote]` in `config.toml` instead of passing `--remote` on the CLI.
///
/// In a default (feature-off) build, `--remote` is genuinely ignored: the
/// "feature disabled" warning fires and the daemon is started normally.
async fn start_daemon(
    kernel: &mut Kernel,
    config: &OxiosConfig,
    config_path: &Path,
    remote: bool,
    pairing_address: Option<String>,
    foreground: bool,
) -> Result<()> {
    // Only honor --remote when the feature is compiled in; otherwise it is a
    // true no-op and we keep the original daemonize-by-default behavior.
    #[cfg(feature = "remote")]
    let effective_remote = remote;
    #[cfg(not(feature = "remote"))]
    let effective_remote = false;

    #[cfg(feature = "remote")]
    if effective_remote {
        kernel.apply_remote_overrides(true, pairing_address);
        if !foreground {
            eprintln!("--remote implies --foreground: running in foreground so the readiness JSON");
            eprintln!("  + pairing URL print here. For daemon mode, set [remote] in config.toml");
            eprintln!("  instead of passing --remote on the CLI.");
            tracing::info!(
                "--remote implies --foreground; daemon child cannot receive CLI-only overrides"
            );
        }
    }
    #[cfg(not(feature = "remote"))]
    if remote {
        // --remote is genuinely a no-op in feature-off builds: we cannot
        // apply the override, so warn and fall through to the daemonized
        // path (which reads config.toml and ignores the flag).
        let _ = pairing_address;
        tracing::warn!("--remote requested but `remote` feature is disabled; ignoring");
    }
    let daemon = DaemonManager::new(&config.daemon.pid_file, &config.daemon.log_dir);
    if foreground || effective_remote {
        cmd_serve(kernel, config_path).await
    } else {
        daemon.start(config_path, config.gateway.port)
    }
}

async fn cmd_serve(kernel: &Kernel, config_path: &Path) -> Result<()> {
    // ── Single-instance guard ──────────────────────────────────────────
    // Acquire an exclusive advisory lock BEFORE any background task starts.
    // See `acquire_instance_lock`. The guard stays in scope for the whole
    // supervised lifetime (released automatically when cmd_serve returns).
    let _instance_lock = {
        let pid_file = oxios_kernel::config::expand_home(&kernel.config().daemon.pid_file);
        acquire_instance_lock(&pid_file)?
    };

    // Initialize MCP servers
    if let Err(e) = kernel.init_mcp_servers().await {
        tracing::warn!(error = %e, "Some MCP servers failed to initialize");
    }

    // Initialize default skills. In an installed binary the compile-time
    // CARGO_MANIFEST_DIR/share path does not exist, so fall back to the
    // embedded tree extracted into the workspace (see `default_skills`).
    let workspace = PathBuf::from(&kernel.config().kernel.workspace);
    let share_dir = default_skills::resolve_share_dir(&workspace);
    if let Err(e) = kernel.init_default_skills(&share_dir).await {
        tracing::warn!(error = %e, "Failed to initialize default skills");
    }

    // ── Ensure web UI is available before starting the server ─────────────
    // This is a blocking check on every start (not just first run) so that:
    //   1. No "web UI not found" on first `oxios start`
    //   2. Users who `cargo install oxios` without building web get it auto-downloaded
    //   3. Server binding is delayed until web UI is ready — no 404 on startup
    let workspace = PathBuf::from(&kernel.config().kernel.workspace);
    let web_result = web_dist::ensure_web_dist(&workspace).await;

    // RFC-024 SP4: finalize the engine readiness. State store is already
    // `Ready` (set in kernel::build). An engine with a configured API key
    // is `Ready`; a missing key (or one that resolves to a fallback model
    // only) is `Degraded` — still usable but signals a partial setup to
    // the readiness middleware.
    {
        let cfg = kernel.config();
        let has_key = cfg
            .engine
            .api_key
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        let engine_state = if has_key {
            oxios_kernel::SubsystemState::Ready
        } else {
            oxios_kernel::SubsystemState::Degraded
        };
        kernel.handle().readiness.set_engine(engine_state);
    }

    // Extract path for surface activation
    let web_dist_path: Option<PathBuf> = match &web_result {
        web_dist::WebDistResult::UserDir(p) => Some(p.clone()),
        web_dist::WebDistResult::WorkspaceDir(p) => Some(p.clone()),
        web_dist::WebDistResult::Downloaded { path, .. } => Some(path.clone()),
        web_dist::WebDistResult::Embedded => None,
        web_dist::WebDistResult::DownloadFailed { .. } => None,
    };

    // Print user-facing status (only show download step, not cached/workspace hits)
    match &web_result {
        web_dist::WebDistResult::Downloaded { .. } => {
            if let Some(tag) = web_result.version_display() {
                println!();
                println!(
                    "  {} Web UI downloaded (v{})",
                    style("✓").green(),
                    style(tag).cyan()
                );
            }
        }
        web_dist::WebDistResult::DownloadFailed { reason } => {
            println!();
            println!(
                "  {} Web UI download failed: {}",
                style("⚠").yellow(),
                style(reason).dim()
            );
            println!(
                "  {} Run {} to restore later.",
                style("→").cyan(),
                style("oxios update --web-only").cyan()
            );
        }
        _ => {}
    }

    // Fail fast: if the web surface is enabled but its assets couldn't be
    // obtained, refuse to start a daemon that would serve 503 on every web
    // request and mask the real cause (download failure). CLI/Telegram-only
    // setups — where the web surface is disabled in config — are unaffected.
    {
        let web_enabled = match &kernel.config().surfaces {
            Some(s) => s.enabled.iter().any(|n| n == "web"),
            None => cfg!(feature = "web"),
        };
        if web_enabled && matches!(web_result, web_dist::WebDistResult::DownloadFailed { .. }) {
            anyhow::bail!(
                "web UI is enabled but could not be downloaded; refusing to start a \
                 broken web server. Check your network or run `oxios update --web-only` \
                 to retry, or disable the web surface in config ([surfaces] enabled = []) \
                 to start without it."
            );
        }
    }

    // ── Supervisor: runtime task liveness (RFC-030) ──────────────────────
    // The supervisor owns the single shutdown signal (CancellationToken) and
    // observes every critical task handle so an unexpected exit is never
    // silent. Before this, cmd_serve awaited only ctrl_c and treated all
    // handles as fire-and-forget — a crashed web server left the daemon
    // half-dead while `oxios status` still reported "Running".
    let root = CancellationToken::new();
    let active_web_dist = oxios_gateway::ActiveWebDist::new(web_dist_path);

    // The web-surface restarter captures everything needed to re-invoke
    // WebSurface::start after a crash (scoped restart, bounded backoff).
    let web_restarter = Arc::new(WebSurfaceRestarter::new(
        kernel,
        Arc::new(parking_lot::RwLock::new(kernel.config().clone())),
        config_path.to_path_buf(),
        active_web_dist.clone(),
    ));

    // Activate surfaces — the web surface is tracked as restartable, every
    // other surface as fail-fast. The root token is threaded in so the web
    // server wires its graceful-shutdown path to a single signal source
    // instead of an independent ctrl_c consumer.
    let surfaces =
        surface::activate_surfaces(kernel, config_path, active_web_dist.clone(), root.clone())
            .await?;
    let channel_tasks = activate_channels(kernel, config_path).await?;

    // ── RFC-044 §6.7: emit readiness contract when --remote is active ───────
    // Best-effort: failures (e.g. workspace state dir unreadable) only log a
    // warning — they MUST NOT crash the serve path. The single JSON line is
    // scraped by automation / future `oxios pair` UX; the pairing URL is
    // printed verbatim for manual QR workflows.
    #[cfg(feature = "remote")]
    {
        let remote_cfg = &kernel.config().remote;
        if remote_cfg.enabled && surfaces.iter().any(|s| s.name == "remote") {
            print_remote_readiness(kernel, config_path);
        }
    }
    // Start guardian (RFC-024 SP3 + RFC-040 A: heartbeat watchdog for hang
    // detection) and daily health check. Both handles are tracked as
    // secondary safety nets for clean exits (audit F-14: health check was
    // previously fire-and-forget); hang detection is via the heartbeat.
    let guardian_heartbeat = Arc::new(AtomicU64::new(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    ));
    let (guardian_task, health_task) =
        kernel.start_guardian(active_web_dist, guardian_heartbeat.clone());

    // Run the gateway event loop. Propagate the Result (no `.expect`): the
    // supervisor observes the handle and treats any exit as fatal. Under
    // panic=abort a panic aborts the process directly, so the non-panic Err
    // path is what the supervisor actually intercepts here.
    let gateway = kernel.gateway();
    let gateway_for_stop = gateway.clone();
    let gateway_task = tokio::spawn(async move {
        if let Err(e) = gateway.run().await {
            tracing::error!(error = %e, "Gateway run error");
        }
    });

    // Build the supervisor and register every task.
    let mut supervisor = TaskSupervisor::new(root.clone(), RestartConfig::default());
    supervisor.with_gateway_stop(move || gateway_for_stop.signal_shutdown());
    supervisor.watch_guardian(guardian_heartbeat);
    supervisor.track_critical("gateway", gateway_task);
    supervisor.track_critical("guardian", guardian_task);
    supervisor.track_critical("health", health_task);
    // Start the cron scheduler loop (restore + config load + tick). Returns
    // None when cron is disabled in config — nothing to track in that case
    // (a completed handle would otherwise look like a fatal critical-task exit).
    if let Some(cron_task) = kernel.start_cron_loop() {
        supervisor.track_critical("cron", cron_task);
    }
    for task in channel_tasks {
        supervisor.track_critical("channel", task);
    }
    for s in surfaces {
        if s.name == "web" {
            supervisor.track_web(
                s.tasks
                    .into_iter()
                    .next()
                    .expect("web surface spawns a task"),
                web_restarter.clone(),
            );
        } else {
            for task in s.tasks {
                supervisor.track_critical(s.name.clone(), task);
            }
        }
    }

    let config = kernel.config();
    println!();
    println!(
        "  {} {}",
        style("⬡ Oxios Agent OS").bold(),
        style(format!("v{}", env!("CARGO_PKG_VERSION"))).dim()
    );
    println!("  {}", "─".repeat(48));
    println!(
        "  Gateway:  {}",
        style(format!(
            "http://{}:{}",
            config.gateway.host, config.gateway.port
        ))
        .cyan()
    );
    println!();
    tracing::info!(
        "Oxios started on http://{}:{}",
        config.gateway.host,
        config.gateway.port
    );

    // ── Supervised run: blocks until graceful (ctrl_c) or fatal ──────────
    // The supervisor drains all tracked tasks before returning — for BOTH
    // outcomes (RFC-030 A6): it cancels the root token (so the web surface
    // drains in-flight requests instead of being aborted mid-request), stops
    // the gateway, and awaits every handle with a timeout. Only the exit code
    // differs.
    let outcome = supervisor.run().await;
    match &outcome {
        ShutdownOutcome::Graceful => tracing::info!("Graceful shutdown requested"),
        ShutdownOutcome::Fatal { name, reason } => {
            tracing::error!(task = %name, %reason, "Fatal task failure; shutting down");
        }
    }

    // ── Kernel-level cleanup (runs for both outcomes) ────────────────
    // Task handles were already drained by the supervisor above. Here
    // we stop the kernel's own resources (agents, MCP, audit) with a
    // bounded timeout — a hang in any phase can't block the process
    // exit indefinitely (RFC-040 B).
    let cleanup_timeout = match &outcome {
        ShutdownOutcome::Graceful => std::time::Duration::from_secs(3),
        ShutdownOutcome::Fatal { .. } => std::time::Duration::from_secs(1),
    };
    kernel.cleanup(cleanup_timeout).await;

    match outcome {
        ShutdownOutcome::Graceful => {
            tracing::info!("Oxios shut down gracefully");
            Ok(())
        }
        ShutdownOutcome::Fatal { name, reason } => {
            // Non-zero exit → the OS supervisor (systemd/launchd) restarts
            // the daemon to a known-good state (RFC-030 C7).
            Err(anyhow::anyhow!("critical task '{name}' exited: {reason}"))
        }
    }
}

// ─── Remote companion surface — readiness contract (RFC-044 §6.7) ────────────
//
// Best-effort: any failure (unreadable state dir, identity load error,
// QR generation failure) logs a warning and returns — the serve path
// must NOT crash on a pairing-UX hiccup. The single JSON line is the
// automation contract; the pairing URL is printed for manual QR workflows.
#[cfg(feature = "remote")]
fn print_remote_readiness(kernel: &Kernel, _config_path: &Path) {
    use crate::remote::endpoints::{build_offer_endpoints, enumerate_direct};
    use crate::remote::identity::DeviceIdentity;
    use crate::remote::pairing::PairingOffer;
    use crate::remote::serve::{ReadinessContract, resolve_advertised};

    let cfg = kernel.config();
    let port = cfg.remote.port;
    let bound_endpoint = format!("ws://127.0.0.1:{port}");

    let advertised_endpoint = resolve_advertised(cfg.remote.pairing_address.as_deref(), port);

    // Load identity + endpoints from disk — same path the surface uses. Any
    // failure here is logged but does not propagate; the readiness contract
    // simply omits the pairing URL.
    let workspace = std::path::PathBuf::from(&cfg.kernel.workspace);
    let state_dir = workspace.join("state");
    let (device_id, pairing_url) = match DeviceIdentity::load_or_create(&state_dir) {
        Ok(identity) => {
            let endpoints = enumerate_direct(port);
            let offer_endpoints = build_offer_endpoints(&endpoints);
            let primary = advertised_endpoint
                .clone()
                .unwrap_or_else(|| format!("ws://127.0.0.1:{port}"));
            let device_id = identity.device_id();
            let offer = PairingOffer {
                v: 1,
                endpoint: primary,
                device_id: device_id.clone(),
                public_key_b64: identity.public_key_b64(),
                endpoints: offer_endpoints,
                scope: "mobile".to_string(),
            };
            let url = offer.encode_url();
            // Best-effort QR — if rendering fails (e.g. payload too long for
            // the qrcode crate), we still print the URL.
            if let Ok(svg) = offer.qr_svg() {
                eprintln!();
                eprintln!("── Remote pairing QR ────────────────────────────────");
                for line in svg.lines().take(18) {
                    eprintln!("{line}");
                }
                eprintln!("────────────────────────────────────────────────────");
            }
            (device_id, Some(url))
        }
        Err(e) => {
            tracing::warn!(error = %e, "remote identity load failed; pairing URL omitted");
            ("0000000000000000".to_string(), None)
        }
    };

    let contract =
        ReadinessContract::new(bound_endpoint, advertised_endpoint, pairing_url, device_id);
    let line = contract.to_json_line();
    println!("{line}");
    // Hint to humans tailing stdout: the JSON line is for tooling.
    eprintln!(
        "(remote) readiness contract written to stdout; scrape the single JSON line above for pairing details."
    );
}

fn build_channel_plugins() -> Vec<Box<dyn ChannelPlugin>> {
    let plugins: Vec<Box<dyn ChannelPlugin>> = vec![];
    let mut plugins = plugins;
    #[cfg(feature = "cli")]
    plugins.push(Box::new(CliPlugin::new()));
    #[cfg(feature = "telegram")]
    plugins.push(Box::new(TelegramPlugin::new()));
    plugins
}

async fn activate_channels(
    kernel: &Kernel,
    config_path: &Path,
) -> Result<Vec<tokio::task::JoinHandle<()>>> {
    let plugins = build_channel_plugins();
    let plugin_map: std::collections::HashMap<&str, &dyn ChannelPlugin> =
        plugins.iter().map(|p| (p.name(), p.as_ref())).collect();

    let config = kernel.config();
    let mut all_tasks = Vec::new();

    for name in &config.channels.enabled {
        match plugin_map.get(name.as_str()) {
            Some(plugin) => {
                let ctx = ChannelContext {
                    config: Arc::new(parking_lot::RwLock::new(config.clone())),
                    config_path: config_path.to_path_buf(),
                };
                match plugin.setup(ctx).await {
                    Ok(bundle) => {
                        tracing::info!(channel = %name, "Channel activated");
                        if let Err(e) = kernel.register_channel(bundle.channel).await {
                            tracing::error!(channel = %name, error = %e, "Failed to register channel");
                        }
                        all_tasks.extend(bundle.tasks);
                    }
                    Err(e) => {
                        tracing::error!(channel = %name, error = %e, "Failed to activate channel")
                    }
                }
            }
            None => tracing::warn!(
                channel = %name,
                "Channel '{}' not available (not compiled in). Available: {}",
                name,
                plugin_map.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
        }
    }

    Ok(all_tasks)
}
