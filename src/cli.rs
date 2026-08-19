//! Command-line argument definitions (clap derive).
//!
//! Extracted from `main.rs` so the entry point stays focused on dispatch and
//! lifecycle, not argument parsing schema.

use clap::{Parser, Subcommand};
use clap_complete::Shell;

/// Oxios Agent OS
#[derive(Debug, Parser)]
#[command(
    name = "oxios",
    version,
    about = "Oxios Agent OS — Agent Operating System",
    after_help = "Examples:\n  oxios                         First run: interactive setup\n  oxios start                   Start the daemon\n  oxios web                     Open web dashboard in browser\n  oxios run \"review this code\"  Execute a single prompt\n  oxios chat                    Start interactive chat\n  oxios status                  Show system status\n  oxios doctor                  Diagnose issues\n\nGetting started:\n  After cargo install oxios, just run:\n    oxios\n  The setup wizard will guide you through configuration."
)]
pub(crate) struct Cli {
    /// Run in foreground (do not daemonize).
    #[arg(long, global = true)]
    pub(crate) foreground: bool,

    /// Enable verbose logging.
    #[arg(short, long, global = true)]
    pub(crate) verbose: bool,

    /// Path to config file.
    #[arg(short, long, default_value = "~/.oxios/config.toml", global = true)]
    pub(crate) config: String,

    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Start the daemon (default when no command is given).
    #[command(visible_alias("serve"))]
    Start {
        /// Enable the E2EE remote companion surface (RFC-044).
        ///
        /// Equivalent to setting `[remote] enabled = true` and ensuring
        /// `"remote"` is in `[surfaces].enabled`. Honors `OXIOS_REMOTE=1`.
        #[arg(long, env = "OXIOS_REMOTE")]
        remote: bool,
        /// Advertised pairing host (`--pairing-address host` or `host:port`).
        /// Wildcards (`0.0.0.0`, `::`, `*`) are rejected — they cannot be
        /// advertised. Default: `tailscale ip -4` → OS hostname → none.
        #[arg(long)]
        pairing_address: Option<String>,
    },

    /// Stop the running daemon.
    Stop,

    /// Restart the daemon.
    Restart,

    /// Run the interactive setup wizard.
    #[command(visible_alias("setup"))]
    Onboard,

    /// Reset all configuration and data (with confirmation).
    Reset {
        /// Skip confirmation prompt.
        #[arg(long)]
        yes: bool,
    },

    /// Show system status (daemon, credentials, agents).
    Status,

    /// Run a single prompt through the Ouroboros flow.
    #[command(arg_required_else_help = true)]
    Run {
        /// The prompt to execute.
        prompt: String,

        /// Output result as JSON (machine-readable).
        #[arg(long)]
        json: bool,

        /// Session ID for multi-turn conversation.
        /// Omit to start a new session.
        #[arg(long)]
        session: Option<String>,

        /// File to prepend as context to the prompt.
        /// Use `-` to read from stdin.
        #[arg(long)]
        context_file: Option<String>,

        /// Set exit code: 0 = evaluation passed, 1 = failed.
        #[arg(long)]
        exit_code: bool,

        /// Chat mode: skip Ouroboros pipeline (interview/crystallize/review)
        /// and execute directly via the agent runtime.
        #[arg(long)]
        chat: bool,
    },

    /// Start an interactive CLI chat session.
    Chat,

    /// Check system health and diagnose issues.
    Doctor,

    /// List available models for the configured (or specified) provider.
    Models {
        /// Provider to list models for (default: current provider).
        #[arg(short, long)]
        provider: Option<String>,
    },

    /// Backup Oxios state.
    Backup {
        /// Output file path (default: auto-generated timestamped file).
        #[arg(short, long)]
        output: Option<String>,
    },

    /// Restore Oxios state from a backup.
    Restore {
        /// Backup file to restore from.
        input: String,
    },

    /// Show or modify configuration (default: show).
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },

    /// Manage installable programs.
    Pkg {
        #[command(subcommand)]
        action: PkgAction,
    },

    /// Manage running agents.
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },

    /// Verify audit trail integrity.
    Audit,

    /// Git operations on state store.
    Git {
        #[command(subcommand)]
        action: GitAction,
    },
    /// Show agent budget information.
    Budget {
        /// Agent UUID (default: show overview of all agents).
        agent_id: Option<String>,
    },

    /// Manage system service (launchd/systemd).
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },

    /// Tail daemon log.
    Log {
        /// Number of lines to show.
        #[arg(short, long, default_value = "50")]
        lines: usize,
    },

    /// Open the web dashboard in your browser.
    Web {
        /// Port override (default: from config).
        #[arg(short, long)]
        port: Option<u16>,
    },

    /// Update oxios binary and/or web UI from GitHub Releases.
    Update {
        /// Update web UI only (binary unchanged).
        #[arg(long)]
        web_only: bool,

        /// Update binary only (web UI unchanged).
        #[arg(long)]
        binary_only: bool,

        /// Target version (default: latest).
        #[arg(long)]
        version: Option<String>,

        /// Dry run — show what would be updated without applying.
        #[arg(long)]
        dry_run: bool,

        /// Skip confirmation prompt.
        #[arg(short = 'y')]
        yes: bool,

        /// Do not restart the daemon after updating.
        #[arg(long)]
        no_restart: bool,
    },

    /// Show changelog or release notes for a version.
    Changelog {
        /// Version to show (default: latest).
        version: Option<String>,
    },

    /// Search, browse, and install skills from ClawHub marketplace.
    Marketplace {
        #[command(subcommand)]
        action: MarketplaceAction,
    },

    /// Manage registered projects (RFC-011).
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },

    /// Generate shell completion script.
    Completion { shell: Shell },

    /// Manage calendar events.
    Calendar {
        #[command(subcommand)]
        action: CalendarAction,
    },

    /// Email commands (setup, test, history, templates).
    Email {
        #[command(subcommand)]
        action: EmailAction,
    },

    /// Interact with the oxibrain daemon (RFC-047) — status, ingest, ask.
    /// Talks to the daemon directly over its Unix socket; the kernel is not
    /// required. `export` is unsupported in this release (use `oxibrain export`).
    Brain {
        #[command(subcommand)]
        command: BrainCmd,
    },
    /// Interact with the Oxi Foundation (RFC-048) — bootstrap, status,
    /// non-secret profile registration.
    Foundation {
        #[command(subcommand)]
        command: crate::cli::FoundationCmd,
    },
}
/// `oxios brain` subcommands.
#[derive(Debug, clap::Subcommand)]
pub(crate) enum BrainCmd {
    /// Show daemon status: online/offline, space, episode count.
    Status,
    /// Ingest a file (or `-` for stdin) as a brain episode.
    Ingest {
        /// File path, or `-` to read stdin.
        path: std::path::PathBuf,
    },
    /// Assemble recall context for a query (3000-token budget) and print it.
    Ask {
        /// Query to recall context for.
        query: String,
    },
    /// (Unsupported) — use the oxibrain CLI: `oxibrain export`.
    Export {
        #[arg(long)]
        format: String,
        #[arg(long)]
        output: Option<String>,
    },
    /// Consolidate Brain episodes (derived/sourced/uncertain). RFC-048
    /// splits Brain consolidation from KnowledgeBase curation. Brain
    /// consolidation never writes KnowledgeBase files.
    Consolidate {
        /// Optional max episodes to consolidate.
        #[arg(long)]
        max: Option<usize>,
    },
    /// Curate raw KnowledgeBase notes (LLM-only note refinement). RFC-048
    /// names this operation `curate`; `dream` remains as a deprecated
    /// alias below.
    Curate {
        /// Optional maximum notes to curate in this run.
        #[arg(long)]
        max: Option<usize>,
    },
    /// Deprecated alias for `curate`. Kept for one minor.
    #[command(hide = true)]
    Dream,
    /// Install the oxibrain binary from GitHub Releases (first-party
    /// supervision, RFC-047). sha256-verified; reports no-release-asset
    /// until the oxibrain repo has a tagged binary release.
    Install,
    /// Start an already-installed oxibrain binary (no install attempt).
    Start,
    /// Stop a daemon this supervisor started (launchd bootout + kill).
    Stop,
    /// Stop + remove the launchd plist (binary and data stay).
    Uninstall,
}

#[derive(Debug, Clone, Subcommand)]
pub(crate) enum ConfigAction {
    /// Show the full configuration
    Show,
    /// Get a configuration value
    Get { key: String },
    /// Set a configuration value (preserves comments and formatting)
    Set { key: String, value: String },
    /// List all configuration keys
    List {
        /// Filter prefix (e.g. "memory" → show only memory.*)
        prefix: Option<String>,
    },
    /// Reset a configuration value to its default
    Reset { key: String },
}

/// `oxios foundation` subcommands (RFC-048).
#[derive(Debug, clap::Subcommand)]
pub(crate) enum FoundationCmd {
    /// Show Foundation status (directory, profiles, Brain handshake).
    Status,
    /// Run the idempotent bootstrap (create directory, handshake Brain).
    Bootstrap {
        /// When set, allow attempting to start a missing compatible daemon.
        #[arg(long)]
        may_start: bool,
    },
    /// Register a non-secret Foundation profile from a JSON file.
    Register {
        /// Path to a profile JSON document.
        #[arg(long)]
        from: std::path::PathBuf,
    },
    /// Migrate legacy credentials into Keychain-backed profile locators.
    Migrate {
        /// Path to the Foundation profile registry.
        #[arg(long)]
        registry: Option<std::path::PathBuf>,
        /// Print the result without touching the legacy stores.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum PkgAction {
    /// Install a skill/program from a Git URL or local path.
    Install {
        /// Git URL or local path to install from.
        source: String,
        /// Branch to checkout (default: repository default).
        #[arg(short, long)]
        branch: Option<String>,
    },
    /// Uninstall a previously installed skill/program by name.
    Uninstall {
        /// Name of the skill/program to remove.
        name: String,
    },
    /// List all installed skills/programs.
    List,
    /// List installed skills with descriptions (alias for a richer `list`).
    Search,
}

#[derive(Debug, Subcommand)]
pub(crate) enum AgentAction {
    /// List all running agents.
    List,
    /// Terminate a running agent by ID.
    Kill {
        /// Agent UUID to terminate.
        id: String,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum GitAction {
    /// Show recent commits in the state store history.
    Log {
        /// Maximum number of commits to show (default: 20).
        limit: Option<usize>,
    },
    /// Create a tagged checkpoint in the state store.
    Tag {
        /// Tag name.
        name: String,
        /// Optional descriptive message.
        message: Option<String>,
    },
}
#[derive(Debug, Subcommand)]
pub(crate) enum DaemonAction {
    /// Install as system service (launchd/systemd).
    Install,
    /// Uninstall system service.
    Uninstall,
}

/// Marketplace subcommands (ClawHub).
#[derive(Debug, Subcommand)]
pub(crate) enum MarketplaceAction {
    /// Search skills on ClawHub.
    Search {
        /// Search query.
        #[arg(short, long)]
        query: String,
        /// Maximum results.
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
    /// Install a skill from ClawHub.
    Install {
        /// Skill slug.
        slug: String,
        /// Specific version (default: latest).
        #[arg(short, long)]
        version: Option<String>,
    },
    /// Update installed ClawHub skill(s).
    Update {
        /// Skill slug (default: all).
        slug: Option<String>,
    },
    /// Check for available updates.
    Updates,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ProjectAction {
    /// List all registered projects.
    List,

    /// Show project details.
    Show {
        /// Project name or ID.
        name: String,
    },

    /// Register a new project.
    Add {
        /// Project name (unique).
        name: String,

        /// Filesystem path(s) for the project.
        #[arg(short, long = "path", num_args = 1..)]
        paths: Vec<String>,

        /// Tags for keyword matching.
        #[arg(short, long = "tag", num_args = 1..)]
        tags: Vec<String>,

        /// Display emoji.
        #[arg(short, long, default_value = "📦")]
        emoji: String,

        /// Description.
        #[arg(short, long)]
        description: Option<String>,
    },

    /// Remove a project.
    Remove {
        /// Project name or ID.
        name: String,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum CalendarAction {
    /// Show today's events.
    Today,

    /// Show tomorrow's events.
    Tomorrow,

    /// Show events for this week.
    Week,

    /// List events in a date range.
    List {
        /// Start date (ISO 8601, e.g. 2026-06-01).
        #[arg(short, long)]
        from: Option<String>,

        /// End date (ISO 8601, e.g. 2026-06-30).
        #[arg(short, long)]
        to: Option<String>,
    },

    /// Create a new event.
    Create {
        /// Event title.
        #[arg(short, long)]
        title: String,

        /// Start time (ISO 8601, e.g. "2026-06-07T10:00:00+09:00").
        #[arg(short, long)]
        start: String,

        /// End time (ISO 8601).
        #[arg(short, long)]
        end: String,

        /// Location.
        #[arg(short, long)]
        location: Option<String>,

        /// Description.
        #[arg(short, long)]
        description: Option<String>,

        /// Reminder in minutes before event.
        #[arg(short, long)]
        reminder: Option<Vec<u32>>,
    },

    /// Delete an event.
    Delete {
        /// Event UID.
        uid: String,
    },

    /// Search events.
    Search {
        /// Search query.
        query: String,
    },

    /// Show free/busy slots for a date.
    Freebusy {
        /// Date (ISO 8601, default: today).
        #[arg(short, long)]
        date: Option<String>,
    },
}

/// Email subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum EmailAction {
    /// Interactive SMTP setup wizard.
    Setup,

    /// Send a test email to verify SMTP configuration.
    Test,

    /// Show email sending history.
    History {
        /// Maximum number of records to show.
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },

    /// List saved email templates.
    Templates,
}
