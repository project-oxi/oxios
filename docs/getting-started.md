# Getting Started with Oxios

> **Oxios Agent OS** — the operating system where AI agents don't just talk, they work.
> This guide will take you from zero to running agents in under 5 minutes.

---

## Table of Contents

1. [Prerequisites](#1-prerequisites)
2. [Installation](#2-installation)
3. [First Run — Onboarding Wizard](#3-first-run--onboarding-wizard)
4. [Quick Start — Three Ways to Use Oxios](#4-quick-start--three-ways-to-use-oxios)
5. [Configuration](#5-configuration)
6. [CLI Reference](#6-cli-reference)
7. [Environment Variables](#7-environment-variables)
8. [Daemon Management](#8-daemon-management)
9. [Doctor Command — Diagnostics](#9-doctor-command--diagnostics)
10. [Upgrading](#10-upgrading)
11. [Uninstalling](#11-uninstalling)

---

## 1. Prerequisites

### Rust Toolchain

Oxios is built in Rust and requires **Rust 1.85 or later**.

Check your current version:

```bash
$ rustc --version
rustc 1.85.0 (4d91de4e4 2025-02-17)
```

If you don't have Rust installed:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### LLM API Key

You need at least one LLM provider API key. Oxios supports many providers through the [oxi](https://github.com/a7garden/oxi) engine:

| Provider | Environment Variable | Get a Key |
|----------|---------------------|-----------|
| **Anthropic** (recommended) | `ANTHROPIC_API_KEY` | [console.anthropic.com](https://console.anthropic.com/) |
| **OpenAI** | `OPENAI_API_KEY` | [platform.openai.com](https://platform.openai.com/) |
| Google Gemini | `GEMINI_API_KEY` | [aistudio.google.com](https://aistudio.google.com/) |
| DeepSeek | `DEEPSEEK_API_KEY` | [platform.deepseek.com](https://platform.deepseek.com/) |
| Groq | `GROQ_API_KEY` | [console.groq.com](https://console.groq.com/) |

Set your key before running Oxios:

```bash
export ANTHROPIC_API_KEY=sk-ant-api03-...
```
> **Tip (RFC-048):** Oxios no longer expects long-lived provider API keys
> in environment variables or `~/.oxios/auth.json`. The Foundation
> layer registers a non-secret profile in `~/.oxi/foundation/v1/profiles.json`
> and reads the secret from the OS Keychain. The CLI exposes
> `oxios foundation status` / `bootstrap` / `register` / `migrate` for
> that flow. The `OXIOS_<PROVIDER>_API_KEY` environment variable remains
> supported as an explicit override for non-interactive CI.
> **Tip:** You can also store credentials via `oxi login` (shared with the oxicode CLI) or through the onboarding wizard. See [Credential Resolution](#credential-resolution-order) for the full priority chain.

---

## 2. Installation

### Option A: `cargo install` (recommended)

```bash
$ cargo install oxios
  Downloaded oxios v0.1.2
  Downloaded 1 crate (245KB) in 1.2s
  Compiling oxios v0.1.2
    Finished `release` profile [optimized] target(s) in 3m 14s
   Installing ~/.cargo/bin/oxios
```

### Option B: Build from Source

Clone the repository and build a distribution binary:

```bash
$ git clone https://github.com/project-oxi/oxios
$ cd oxios
$ cargo build --profile dist
  Compiling oxios v0.1.2 (...)

$ ./target/dist/oxios --version
oxios 0.1.2
```

The binary will be at `target/dist/oxios`. Copy it to your PATH:

```bash
cp target/dist/oxios ~/.cargo/bin/
```

### Option C: Pre-built Binary

Download the latest binary from [GitHub Releases](https://github.com/project-oxi/oxios/releases):

```bash
# macOS (Apple Silicon)
curl -L https://github.com/project-oxi/oxios/releases/latest/download/oxios-aarch64-apple-darwin.tar.gz | tar xz
chmod +x oxios
mv oxios ~/.cargo/bin/

# Linux (x86_64)
curl -L https://github.com/project-oxi/oxios/releases/latest/download/oxios-x86_64-unknown-linux-gnu.tar.gz | tar xz
chmod +x oxios
mv oxios ~/.cargo/bin/
```

### Verify Installation

```bash
$ oxios --version
oxios 0.1.2
```

---

## 3. First Run — Onboarding Wizard

When you run `oxios` for the first time, it launches an **interactive setup wizard**. It detects your environment, walks you through provider selection, and writes your configuration.

```bash
$ oxios
```

### What happens:

**Step 0 — Auto-detection.** Oxios scans for existing API keys in environment variables and `~/.oxicode/auth.json` (from the oxicode CLI). If it finds one, it asks whether to use it.

```
  Detected ANTHROPIC_API_KEY in environment for 'anthropic'.
  Use this provider? › Yes
```

**Step 1 — Provider Selection.** If no key is detected, you pick a provider from the list:

```
  [1/5] Select an LLM provider:
  Provider: › anthropic [42 models] 🔑
```

Use ↑↓ arrow keys to navigate, Enter to confirm.

**Step 2 — API Key.** Enter your key (masked input):

```
  [2/5] Enter your anthropic API key:
  API key: ****
```

**Step 3 — Model Selection.** Pick a model:

```
  [3/5] Select a model for anthropic:
  Model: › claude-sonnet-4-20250514                200K ctx
```

Or choose **"✎ Enter model ID manually..."** to type a custom model ID.

**Step 4 — Workspace.** Confirm the workspace directory:

```
  [4/5] Workspace path:
  Workspace: /Users/you/.oxios/workspace
```

**Step 5 — Confirm.** Review and save:

```
  ┌─────────────────────────────────────────────┐
  │            Configuration Summary             │
  ├─────────────────────────────────────────────┤
  │  Provider:  anthropic                        │
  │  Model:     anthropic/claude-sonnet-4-20250514│
  │  Key:       sk-a...xxxx                      │
  │  Workspace: /Users/you/.oxios/workspace      │
  └─────────────────────────────────────────────┘

  [5/5] Write configuration?
  Save this configuration? › Yes

  Saving configuration... done

  ╔═══════════════════════════════════════════╗
  ║             Setup Complete!                ║
  ╚═══════════════════════════════════════════╝

    Config:  /Users/you/.oxios/config.toml
    Model:   anthropic/claude-sonnet-4-20250514

  Next steps:
    oxios start → start the daemon
    oxios daemon install → register as system service
    open http://127.0.0.1:4200 → open web dashboard
```

After onboarding, you're asked whether to start the daemon right away:

```
  Start daemon now? › Yes
```

Behind the scenes, Oxios creates:
- `~/.oxios/config.toml` — your configuration
- `~/.oxios/workspace/` — agent workspace with subdirectories for sessions, seeds, skills, and memory

### Re-running the Wizard

```bash
$ oxios onboard
```

This shows your current configuration and lets you keep, modify, or reset it.

---

## 4. Quick Start — Three Ways to Use Oxios

Once installed and configured, you have three primary interfaces:

### 4.1 Web Dashboard

```bash
$ oxios start
  ⬡ Oxios Agent OS v0.1.2
  ────────────────────────────────────────────────
  Gateway:  http://127.0.0.1:4200
```

Open **http://127.0.0.1:4200** in your browser:

```
┌─────────────────────────────────────────────────────────┐
│  ⬡ Oxios                              [Persona ▼]       │
├──────────┬──────────────────────────────────────────────┤
│ 💬 Chat  │                                              │
│ 👥 Agents│  ┌──────────────────────────────────────┐   │
│ 📅 Cron  │  │ Welcome to Oxios. How can I help you? │   │
│ 📁 Skills │ │                                              │   │
│ 🎯 Memory │  └──────────────────────────────────────┘   │
│ ⚙️ Config │                                              │
└──────────┴──────────────────────────────────────────────┘
```

The web dashboard provides:
- **Chat** — talk to your agent, it works using the Ouroboros protocol
- **Agents** — see running agents, kill misbehaving ones
- **Cron** — schedule recurring tasks
- **Skills** — manage agent capabilities (unified Programs + Skills)
- **Memory** — view and search persistent agent knowledge
- **Config** — live configuration editing

### 4.2 CLI Chat

```bash
$ oxios chat
  Entering interactive chat. Type your message and press Enter.
  Press Ctrl+C to exit.

  You: Build a TODO app in Rust

  Agent: I'll create a minimal TODO app for you. Let me start by
  scaffolding the project structure...

  You: Make it save to a file

  Agent: Adding file persistence now...
```

### 4.3 Single-Shot Execution (for scripts and pipelines)

```bash
$ oxios run "Review this code and suggest improvements"
```

For programmatic use, the `--json` flag outputs machine-readable results:

```bash
$ oxios run --json "Write a Rust function that reverses a string"
{
  "response": "Here's a function that reverses a string in Rust...",
  "session_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "primary_project_id": null,
  "project_tag": null,
  "agent_id": "f9e8d7c6-b5a4-3210-fedc-ba0987654321",
  "phase_reached": "execute",
  "evaluation_passed": true,
  "exit_code": 0,
  "duration_ms": 4200
}
```

Pipe context files into prompts:

```bash
# From a file
$ oxios run --json --context-file src/main.rs "Explain what this code does"

# From stdin
$ cat error.log | oxios run --json --context-file - "Diagnose this error"
```

Use `--exit-code` for CI integration (exit code 0 = evaluation passed, 1 = failed):

```bash
$ oxios run --exit-code --json "Run the test suite and report results"
$ echo $?
0
```

Multi-turn sessions by passing `session_id`:

```bash
$ SID=$(oxios run --json "Set up a Rust project" | jq -r '.session_id')
$ oxios run --json --session "$SID" "Now add unit tests"
```

---

## 5. Configuration

Configuration lives at **`~/.oxios/config.toml`**. Every section has sensible defaults — you only need to override what you want to change.

### Viewing Your Config

```bash
# Show full config
$ oxios config show

# Get a single value
$ oxios config get engine.default_model
anthropic/claude-sonnet-4-20250514

# Set a value
$ oxios config set gateway.port 8080
  Set gateway.port = 8080

$ oxios config set exec.allowed_commands git,gh,cargo
  Set exec.allowed_commands = git,gh,cargo
```

### Complete Configuration Reference

Below is every section with all available keys and their defaults. Copy what you need into `~/.oxios/config.toml`.

```toml
# ── Kernel ─────────────────────────────────────────────────────
# Core system settings: workspace, event bus, agent limits.

[kernel]
workspace = "~/.oxios/workspace"   # Agent working directory
event_bus_capacity = 256            # Internal event channel buffer
max_agents = 16                     # Maximum concurrent agents

# ── Engine ─────────────────────────────────────────────────────
# LLM provider and model selection. Set during onboarding.

[engine]
default_model = ""    # Must be "provider/model" format (e.g. "anthropic/claude-sonnet-4-20250514")
# api_key = ""        # Explicit key (highest priority). If empty, falls back to auth store → env vars.

# ── Daemon ─────────────────────────────────────────────────────
# Background daemon process management.

[daemon]
# pid_file = "~/.oxios/oxios.pid"    # PID file location
# log_dir = "~/.oxios/logs"          # Log output directory

# ── Gateway ────────────────────────────────────────────────────
# Web dashboard HTTP server.

[gateway]
host = "127.0.0.1"   # Bind address (use "0.0.0.0" for network access)
port = 4200           # HTTP port

# ── Exec ───────────────────────────────────────────────────────
# Host command execution — controls what agents can run on your machine.

[exec]
allowed_commands = ["git", "gh", "open", "shortcuts", "osascript"]  # Binary allowlist (empty = allow all)
default_timeout_secs = 120   # Default command timeout
max_timeout_secs = 600       # Maximum command timeout
# required_host_tools = []  # Tools that MUST exist (checked on startup)
# optional_host_tools = []  # Tools checked lazily (e.g. ["gh", "osascript"])

# ── Scheduler ──────────────────────────────────────────────────
# Task queue and concurrency control (AIOS/AgentRM-inspired).

[scheduler]
max_concurrent = 5            # Max parallel agent tasks
rate_limit_per_minute = 60    # LLM API call rate limit
zombie_timeout_secs = 300     # Kill tasks stuck longer than this

# ── Context ────────────────────────────────────────────────────
# Context window management for agents.

[context]
active_limit_tokens = 100000   # Max tokens in active context
cache_limit_entries = 50       # Max cached context entries

# ── Security ───────────────────────────────────────────────────
# Access control, RBAC, sandboxing (OWASP-inspired).

[security]
auth_enabled = false                              # Enable API key auth on gateway
cors_origins = ["http://localhost:4200"]          # Allowed CORS origins
allowed_tools = ["read", "write", "edit", "bash", "grep", "find"]  # Default agent tools
network_access = false       # Allow agents to make network requests
max_execution_time_secs = 300  # Max agent task duration
max_memory_mb = 512            # Max memory per agent task
can_fork = false               # Allow agents to fork sub-agents
# max_audit_entries = 10000
# audit_log_path = ""          # Optional file path for audit persistence
# rate_limit_per_minute = 120  # API endpoint rate limit

# ── Persona ────────────────────────────────────────────────────
# Persona system for agent behavior profiles.

# [persona]
# default_persona_id = "dev"       # Default persona on startup
# max_concurrent_personas = 5      # Max simultaneous personas

# ── Memory ─────────────────────────────────────────────────────
# Persistent vector memory for agents (cross-session recall).

[memory]
enabled = true
max_recall = 10            # Max memories returned per query
auto_summarize = true      # Auto-summarize long sessions
capture_compaction = true  # Compact memory on capture
# retention_days = 0       # Days to keep memories (0 = forever)

# ── Cron ───────────────────────────────────────────────────────
# Scheduled agent jobs (cron-style).

[cron]
enabled = true
tick_interval_secs = 60   # How often to check schedules

# Example cron job:
# [cron.jobs.morning_news]
# schedule = "0 9 * * *"         # Every day at 9:00 AM
# goal = "Summarize top tech news"
# priority = "low"
# enabled = true
# constraints = []
# acceptance_criteria = []

# ── MCP (Model Context Protocol) ──────────────────────────────
# External tool servers that agents can call.

# [mcp.servers.filesystem]
# command = "npx"
# args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
# env = {}
# enabled = true

# ── Git ────────────────────────────────────────────────────────
# In-process version control for state changes.

[git]
auto_commit = true   # Automatically commit state changes

# ── Audit ──────────────────────────────────────────────────────
# Tamper-evident audit trail for all agent actions.

[audit]
enabled = true
max_entries = 100000   # Entries before pruning

# ── Budget ─────────────────────────────────────────────────────
# Token/cost budget enforcement per agent.

[budget]
enabled = true
# default_token_budget = 0     # 0 = unlimited
# default_calls_budget = 0     # 0 = unlimited
# default_window_secs = 3600   # Budget reset window

# ── Resource Monitor ───────────────────────────────────────────
# System resource tracking for overload detection.

[resource_monitor]
interval_secs = 60       # Snapshot interval
history_max = 60         # Max history entries
cpu_threshold = 90.0     # CPU % overload threshold
memory_threshold = 90.0  # Memory % overload threshold
load_threshold = 8.0     # Load average overload threshold

# ── Channels ───────────────────────────────────────────────────
# Activate communication channels.

[channels]
enabled = ["web"]    # Options: "web", "cli", "telegram"

# [channels.telegram]
# bot_token_env = "TELEGRAM_BOT_TOKEN"   # Env var with your bot token
# allowed_users = []                      # Telegram user IDs (empty = allow all)

# ── Browser ────────────────────────────────────────────────────
# Built-in headless browser (OxiBrowser — pure Rust, in-process).

[browser]
enabled = true

# [browser.engine]
# user_agent = "OxiBrowser/1.0"
# obey_robots = true
# js_timeout_ms = 5000
```

### Credential Resolution Order

When connecting to an LLM provider, Oxios resolves credentials in this priority:

1. **`engine.api_key`** in `config.toml` (explicit, highest priority)
2. **`~/.oxicode/auth.json`** (shared with the [oxicode CLI](https://github.com/a7garden/oxi))
3. **Environment variable** (e.g. `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`)

---

## 6. CLI Reference

Oxios exposes all functionality through subcommands. Run `oxios help` for the full list.

### Daemon Control

| Command | Description |
|---------|-------------|
| `oxios` | Start daemon (default command) |
| `oxios start` | Start the daemon (alias: `oxios serve`) |
| `oxios stop` | Stop the running daemon |
| `oxios restart` | Restart the daemon |
| `oxios --foreground` | Run in foreground (for debugging) |

```bash
$ oxios start
  ⬡ Oxios Agent OS v0.1.2
  ────────────────────────────────────────────────
  Gateway:  http://127.0.0.1:4200

$ oxios stop
  Stopped oxios (PID 42421)

$ oxios --foreground --verbose
  # Runs in foreground with debug logging
```

### Execution

```bash
# Single prompt
$ oxios run "Explain this error: E0425"

# JSON output for scripts
$ oxios run --json "Summarize this file" | jq '.response'

# With context file
$ oxios run --context-file src/main.rs "Add error handling"

# From stdin
$ git diff | oxios run --context-file - "Review this diff"

# Multi-turn
$ SID=$(oxios run --json "Create a project" | jq -r '.session_id')
$ oxios run --json --session "$SID" "Add tests"

# CI mode (exit code reflects evaluation result)
$ oxios run --exit-code --json "Run tests and fix failures"
```

### Interactive Chat

```bash
$ oxios chat
```

### Status & Diagnostics

```bash
# System status
$ oxios status

  ⬡ Oxios Agent OS v0.1.2
  ────────────────────────────────────────────────
  Workspace:       /Users/you/.oxios/workspace
  Model:           anthropic/claude-sonnet-4-20250514
  Daemon:          Running (PID 42421)
  Credentials:     sk-a...xxxx [config.toml]
  MCP Servers:     2
  Active Agents:   1
      a1b2c3d4  Running  code-review

# Health check
$ oxios doctor
```

### Agent Management

```bash
# List running agents
$ oxios agent list

# Kill an agent
$ oxios agent kill a1b2c3d4-e5f6-7890-abcd-ef1234567890
  ✓ Agent a1b2c3d4 terminated.
```

### Skill Management

Skills are the unified model for agent capabilities — SKILL.md files with YAML frontmatter carrying all metadata (requirements, install specs, invocation policy). See [RFC-009](../rfc-009-skill-unification.md) for the full design.

```bash
# Install a skill
$ oxios skill install ./my-skill
  Installed 'my-skill v1.0.0'

# List all skills
$ oxios skills

# Get skill details
$ oxios skill code-review

# View skill details
$ oxios skill code-review

# Uninstall
$ oxios skill uninstall my-skill
  Removed 'my-skill'
```

### Configuration

```bash
# Show full config
$ oxios config show

# Get a value
$ oxios config get gateway.port
4200

# Set a value
$ oxios config set gateway.host 0.0.0.0
  Set gateway.host = 0.0.0.0
```

### Daemon Service Management

```bash
# Install as system service (launchd on macOS, systemd on Linux)
$ oxios daemon install

# Uninstall system service
$ oxios daemon uninstall
```

### Logging

```bash
# Tail the last 50 lines of the daemon log
$ oxios log

# Last 200 lines
$ oxios log --lines 200
```

### Budget

```bash
# Overview
$ oxios budget

  Agent Budget Overview
  ────────────────────────────────────────────────
  Run `oxios agent list` to find agent IDs,
  then `oxios budget <agent-id>` for details.

# Specific agent
$ oxios budget f9e8d7c6-b5a4-3210-fedc-ba0987654321

  Agent: f9e8d7c6-b5a4-3210-fedc-ba0987654321
  ────────────────────────────────────
  Tokens remaining:   50000
  Calls remaining:    45
  Window remaining:   2847 seconds
  Status:             ✓ OK
```

### Audit Trail

```bash
# Verify audit chain integrity and show recent entries
$ oxios audit

  ✓ Audit trail verified — chain intact.

  Recent Audit Entries (showing last 5):
        SEQ   TIMESTAMP            ACTOR            ACTION
  ──────────────────────────────────────────────────────────────────────
          1   2026-05-17 09:12:34  agent-123        Exec
          2   2026-05-17 09:12:35  agent-123        Read
          3   2026-05-17 09:12:40  agent-123        Write

  Total entries: 3
```

### Git Operations

```bash
# View commit log
$ oxios git log
$ oxios git log 10   # last 10 commits

# Create a tag
$ oxios git tag v1.0-release --message "First stable release"
  Tagged 'v1.0-release'.
```

### Backup & Restore

```bash
# Backup to default location
$ oxios backup

# Backup to specific path
$ oxios backup --output ./my-backup.tar

# Restore from backup
$ oxios restore ./my-backup.tar
```

### Model Browser

```bash
# List models for configured provider
$ oxios models

  Available Models for anthropic
  ────────────────────────────────────────────────────────────────────
  claude-sonnet-4-20250514                200K ctx
  claude-opus-4-20250514                  200K ctx  ✦reasoning
  ...

# List for a specific provider
$ oxios models --provider openai
```

### Shell Completion

```bash
# Generate completion for your shell
$ oxios completion bash > ~/.local/share/bash-completion/completions/oxios
$ oxios completion zsh > ~/.zfunc/_oxios
$ oxios completion fish > ~/.config/fish/completions/oxios.fish
```

### Reset

```bash
# Reset all Oxios data (with confirmation)
$ oxios reset

  ⚠  This will delete all Oxios configuration and data:
     /Users/you/.oxios

  Are you sure? › No
  Reset cancelled.

# Skip confirmation
$ oxios reset --yes
```

### Setup Wizard

```bash
# Re-run the onboarding wizard
$ oxios onboard
```

---

## 7. Environment Variables

### LLM Provider Keys

| Variable | Description |
|----------|-------------|
| `ANTHROPIC_API_KEY` | Anthropic (Claude) API key |
| `OPENAI_API_KEY` | OpenAI API key |
| `GEMINI_API_KEY` | Google Gemini API key |
| `DEEPSEEK_API_KEY` | DeepSeek API key |
| `GROQ_API_KEY` | Groq API key |

Oxios supports all providers available through the oxi engine. Check available providers with `oxios models --provider <name>`.

### Oxios-Specific

| Variable | Description |
|----------|-------------|
| `OXIOS_API_KEY` | Gateway authentication key (when `security.auth_enabled = true`) |
| `RUST_LOG` | Log level filter: `error`, `warn`, `info`, `debug`, `trace` |

```bash
# Enable debug logging
RUST_LOG=debug oxios --foreground

# Targeted debug (kernel only)
RUST_LOG=oxios_kernel=debug oxios --foreground
```

### MCP Server Registration via Environment

You can register MCP servers without editing `config.toml` using environment variables:

```bash
# Register an MCP server command
export OXIOS_MCP_MY_SERVER=/usr/local/bin/my-mcp-server

# Pass arguments
export OXIOS_MCP_MY_SERVER_ARGS="--port 3000 --verbose"

# Pass environment variables (comma-separated KEY=VALUE pairs)
export OXIOS_MCP_MY_SERVER_ENV="API_KEY=abc123,DEBUG=true"
```

The naming convention is:
- `OXIOS_MCP_<NAME>` — command to run
- `OXIOS_MCP_<NAME>_ARGS` — arguments (space-separated)
- `OXIOS_MCP_<NAME>_ENV` — environment variables (comma-separated `KEY=VALUE` pairs)

These are merged with servers defined in `[mcp.servers]` in `config.toml`. Config-defined servers take precedence.

### Testing (Internal)

| Variable | Description |
|----------|-------------|
| `OXIOS_E2E` | Enable end-to-end tests (set to `1`) |
| `OXIOS_MODEL` | Override model for E2E tests (e.g. `anthropic/claude-sonnet-4-20250514`) |

---

## 8. Daemon Management

### Starting the Daemon

```bash
# Start in background (default)
$ oxios start
  ⬡ Oxios Agent OS v0.1.2
  ────────────────────────────────────────────────
  Gateway:  http://127.0.0.1:4200

# Start in foreground (for debugging)
$ oxios --foreground

# Foreground with verbose logging
$ oxios --foreground --verbose
```

### Stopping and Restarting

```bash
$ oxios stop
  Stopped oxios (PID 42421)

$ oxios restart
  Stopped oxios (PID 42421)
  Starting oxios...
```

### Checking Status

```bash
$ oxios status

  ⬡ Oxios Agent OS v0.1.2
  ────────────────────────────────────────────────
  Workspace:       /Users/you/.oxios/workspace
  Model:           anthropic/claude-sonnet-4-20250514
  Daemon:          Running (PID 42421)
  Credentials:     sk-a...xxxx [config.toml]
  MCP Servers:     0
  Active Agents:   0
```

### Viewing Logs

```bash
# Last 50 lines (default)
$ oxios log

# Last 200 lines
$ oxios log --lines 200

# Live tail (standard Unix)
$ tail -f ~/.oxios/logs/oxios.log

# Filter for errors
$ oxios log --lines 500 | grep ERROR
```

Log files are stored at `~/.oxios/logs/` with daily rotation.

### Installing as a System Service

Oxios can register itself as a system service that starts automatically on boot.

**macOS (launchd):**

```bash
$ oxios daemon install
  Installed oxios as a launchd service.

$ oxios daemon uninstall
  Uninstalled oxios launchd service.
```

**Linux (systemd):**

```bash
$ oxios daemon install
  Installed oxios as a systemd service.

# Manage via systemctl
$ systemctl status oxios
$ systemctl restart oxios
$ journalctl -u oxios -f    # live logs
```

### Process Details

| File | Purpose |
|------|---------|
| `~/.oxios/oxios.pid` | PID of the running daemon |
| `~/.oxios/logs/oxios.log` | Daily-rotated log file |

### File Structure

```
~/.oxios/
├── config.toml                 # Main configuration
├── oxios.pid                   # Daemon PID file
├── logs/
│   └── oxios.log               # Daily-rotated logs
└── workspace/
    ├── memory/
    │   └── knowledge/          # Persistent agent knowledge base
    ├── sessions/               # Session data (ephemeral)
    ├── seeds/                  # Ouroboros seed specifications
    ├── skills/                 # Unified skill definitions
    ├── backups/                # State backups
    └── ...
```

---

## 9. Doctor Command — Diagnostics

The `oxios doctor` command runs a comprehensive health check:

```bash
$ oxios doctor

  ⬡ Oxios Doctor — System Diagnostics
  ────────────────────────────────────────────────
  ✓ Config file present (/Users/you/.oxios/config.toml)
  ✓ Credentials found (sk-a...xxxx, via config.toml)
  ✓ Workspace directory (/Users/you/.oxios/workspace)
  ✓ Daemon is running
  ⚠ No MCP servers configured
  ✓ Default model: anthropic/claude-sonnet-4-20250514
  ✓ oxicode CLI available (shared auth store)
  ✓ Port 4200 listening (daemon active)
  ────────────────────────────────────────────────
  7 checks passed, no issues found. All good!
```

### What it checks:

| # | Check | What it validates |
|---|-------|-------------------|
| 1 | Config file | `~/.oxios/config.toml` exists and is valid |
| 2 | Credentials | API key is available for the configured provider |
| 3 | Workspace | Workspace directory exists |
| 4 | Daemon | Daemon process is running |
| 5 | MCP servers | Whether MCP servers are connected |
| 6 | Model | A default model is configured |
| 7 | oxicode CLI | Whether the shared oxicode CLI is installed |
| 8 | Port | Gateway port is available (or listening if daemon is up) |

### When things go wrong:

```bash
$ oxios doctor

  ⬡ Oxios Doctor — System Diagnostics
  ────────────────────────────────────────────────
  ✓ Config file present (/Users/you/.oxios/config.toml)
  ✗ No credentials for provider 'anthropic'
  ✓ Workspace directory (/Users/you/.oxios/workspace)
  ⚠ Daemon is not running (Stopped)
  ⚠ No MCP servers configured
  ✓ Default model: anthropic/claude-sonnet-4-20250514
  ⚠ oxicode CLI not detected
  ✓ Port 4200 available
  ────────────────────────────────────────────────
  8 checks, 3 issue(s):

    1. No API key for 'anthropic'. Run `oxios onboard` to configure.
    2. Daemon not running. Start with `oxios start`.
    3. Install oxicode CLI for shared credential management: `cargo install oxicode-cli`
```

### Common fixes:

| Issue | Fix |
|-------|-----|
| No credentials | `oxios onboard` or `export ANTHROPIC_API_KEY=...` |
| Daemon not running | `oxios start` |
| Port in use | `oxios config set gateway.port 8080` |
| Config errors | `oxios config show` to inspect, then fix `~/.oxios/config.toml` |
| Corrupt state | `rm ~/.oxios/config.toml && oxios` (auto-regenerates) |

---

## 10. Upgrading

### Via cargo install

```bash
$ cargo install oxios --force
  Updating oxios from 0.1.0 to 0.1.2
  Compiling oxios v0.1.2
  Finished `release` profile [optimized] target(s) in 3m 14s
   Installing ~/.cargo/bin/oxios
```

### From source

```bash
$ cd oxios
$ git pull
$ cargo build --profile dist
$ cp target/dist/oxios ~/.cargo/bin/
```

### After upgrading

1. Restart the daemon:

```bash
$ oxios restart
```

2. Check health:

```bash
$ oxios doctor
$ oxios status
```

3. Review any config changes — new versions may add sections. Running `oxios config show` will display all current keys. Missing keys use defaults automatically.

---

## 11. Uninstalling

### Full Clean Removal

```bash
# 1. Stop the daemon
$ oxios stop

# 2. Unregister system service (if installed)
$ oxios daemon uninstall

# 3. Remove all Oxios data and configuration
$ oxios reset --yes

  ✓ /Users/you/.oxios removed.
  Run `oxios onboard` to set up again.

# 4. Uninstall the binary
$ cargo uninstall oxios
```

### What gets removed:

| Path | Contents |
|------|----------|
| `~/.oxios/` | Config, workspace, logs, PID file, all agent data |
| `~/.cargo/bin/oxios` | The binary itself |

> **Note:** `~/.oxicode/auth.json` (shared oxicode CLI credentials) is **not** deleted by `oxios reset`. Remove it manually if desired: `rm ~/.oxicode/auth.json`.

---

## Next Steps

- **Browse skills:** `oxios skills` — discover available agent capabilities
- **Schedule tasks:** Add cron jobs to `~/.oxios/config.toml`
- **Connect Telegram:** Open the Web UI → Settings → Telegram, paste your
  @BotFather token, and press **Connect** — the bot starts immediately and
  `channels.enabled` is persisted for you (no daemon restart). The
  headless alternative still works: set `TELEGRAM_BOT_TOKEN` and add
  `"telegram"` to `channels.enabled`.
- **Enable MCP:** Add servers under `[mcp.servers]` to give agents external tools
- **Read the architecture:** `docs/ARCHITECTURE.md` for internals

---

*Built by [a7garden](https://github.com/a7garden). Licensed under [MIT](https://github.com/project-oxi/oxios/blob/main/LICENSE).*
