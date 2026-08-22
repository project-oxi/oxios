#![allow(missing_docs)]
//! Configuration loading from TOML files.
//!
//! Configuration is stored at `~/.oxios/config.toml` and controls
//! kernel, gateway, and execution settings.

use cron::Schedule;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::approval::ApprovalConfig;
use crate::email::{SmtpProvider, SmtpTls};
use crate::types::Priority;

/// Cron scheduler configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CronConfig {
    /// Enable the cron scheduler (auto-starts at boot; a no-op tick when no
    /// jobs are registered). Defaults to `true` so registered cron jobs fire
    /// out-of-the-box without requiring explicit opt-in.
    #[serde(default = "default_cron_enabled")]
    pub enabled: bool,
    /// Tick interval in seconds.
    #[serde(default = "default_tick_interval")]
    pub tick_interval_secs: u64,
    /// Inline job definitions from config.toml.
    #[serde(default)]
    pub jobs: std::collections::HashMap<String, InlineCronJob>,
}

impl Default for CronConfig {
    fn default() -> Self {
        Self {
            enabled: default_cron_enabled(),
            tick_interval_secs: default_tick_interval(),
            jobs: std::collections::HashMap::new(),
        }
    }
}

fn default_tick_interval() -> u64 {
    60
}

/// Cron is enabled by default so registered jobs auto-fire at boot.
fn default_cron_enabled() -> bool {
    true
}

/// Inline cron job definition in config.toml.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InlineCronJob {
    /// Cron expression (e.g. "0 */6 * * *").
    pub schedule: String,
    /// Goal description for the agent.
    pub goal: String,
    /// Constraints on agent behavior.
    #[serde(default)]
    pub constraints: Vec<String>,
    /// Criteria that must be met for the job to be considered successful.
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    /// Toolchain preset name.
    #[serde(default = "default_toolchain_inline")]
    pub toolchain: String,
    /// Job priority.
    #[serde(default)]
    pub priority: Priority,
    /// Whether the job is active.
    #[serde(default = "default_true_inline")]
    pub enabled: bool,
}

fn default_toolchain_inline() -> String {
    "default".into()
}

fn default_true_inline() -> bool {
    true
}

/// Brain daemon connection configuration (RFC-047).
///
/// oxios connects to the standalone oxibrain daemon over a Unix-domain socket.
/// When the daemon is unavailable the kernel degrades (memory ops return empty)
/// and agent turns complete normally.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BrainSection {
    /// Connect to the daemon at boot. `false` skips the connection attempt
    /// entirely (fully degraded).
    pub enabled: bool,
    /// Unix-domain socket path. Empty → `~/.oxi/brain/oxibrain.sock`.
    pub socket_path: String,
    /// Space to operate in.
    pub space: String,
    /// First-party supervision (2026-08-19 spec): install the daemon from
    /// GitHub Releases when absent, keep it running via launchd with a
    /// detached-spawn fallback. `false` restores pure degradation.
    pub auto_manage: bool,
    /// Explicit daemon binary path. Non-empty skips the download entirely.
    pub binary_path: String,
}

impl Default for BrainSection {
    fn default() -> Self {
        Self {
            enabled: true,
            socket_path: String::new(),
            space: "personal".to_string(),
            auto_manage: true,
            binary_path: String::new(),
        }
    }
}
/// Foundation settings (RFC-048).
///
/// The Foundation layer owns the non-secret profile registry, the shared
/// package lock, and the Keychain-backed credential locator surface. The
/// executor stays embedded — Foundation is *not* a provider proxy and
/// never spawns an external worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FoundationConfig {
    /// Run Foundation bootstrap automatically on daemon start. When
    /// `false`, the CLI exposes `foundation bootstrap` / `foundation
    /// status` for explicit invocation.
    pub auto_bootstrap: bool,
    /// Override the registry path. Empty → `~/.oxi/foundation/v1/profiles.json`.
    pub registry_path: String,
    /// Override the package lock path. Empty → `~/.oxi/foundation/v1/packages.lock`.
    pub packages_lock_path: String,
    /// Override the Brain daemon socket. Empty → `~/.oxi/brain/oxibrain.sock`.
    pub brain_socket: String,
}

impl Default for FoundationConfig {
    fn default() -> Self {
        Self {
            auto_bootstrap: true,
            registry_path: String::new(),
            packages_lock_path: String::new(),
            brain_socket: String::new(),
        }
    }
}

/// Memory system configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Enable the memory system.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Maximum memories returned by recall.
    #[serde(default = "default_max_recall")]
    pub max_recall: usize,
    /// Auto-summarize sessions on completion.
    #[serde(default = "default_true")]
    pub auto_summarize: bool,
    /// Capture compaction summaries as conversation memory.
    #[serde(default = "default_true")]
    pub capture_compaction: bool,
    /// Memory retention in days (0 = unlimited).
    #[serde(default)]
    pub retention_days: u32,
    /// Enable embedding cache.
    #[serde(default = "default_true")]
    pub cache_enabled: bool,
    /// Embedding cache TTL in seconds.
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl_secs: u64,
    /// Maximum embedding cache entries.
    #[serde(default = "default_cache_max_entries")]
    pub cache_max_entries: usize,
    /// Consolidation configuration (RFC-008).
    #[serde(default)]
    pub consolidation: ConsolidationConfig,
    /// SQLite memory storage configuration (RFC-012).
    #[serde(default)]
    pub sqlite: SqliteMemoryConfig,
    /// Embedding provider configuration (RFC-012).
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    /// Learning configuration (RFC-012 Phase 4: SONA).
    #[serde(default)]
    pub learning: LearningConfig,
    /// Knowledge dream configuration (RFC-022).
    #[serde(default)]
    pub knowledge_curation: crate::knowledge_curation::KnowledgeCurationConfig,
    /// AutoMemoryBridge configuration (RFC-012 Phase 7: SQLite ↔ MEMORY.md sync).
    #[serde(default)]
    pub bridge: MemoryBridgeConfig,
}

fn default_true() -> bool {
    true
}

fn default_max_recall() -> usize {
    10
}

fn default_cache_ttl() -> u64 {
    3600 // 1 hour
}

fn default_cache_max_entries() -> usize {
    10000
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_recall: 10,
            auto_summarize: true,
            capture_compaction: true,
            retention_days: 0,
            cache_enabled: true,
            cache_ttl_secs: 3600,
            cache_max_entries: 10000,
            consolidation: ConsolidationConfig::default(),
            sqlite: SqliteMemoryConfig::default(),
            embedding: EmbeddingConfig::default(),
            learning: LearningConfig::default(),
            knowledge_curation: crate::knowledge_curation::KnowledgeCurationConfig::default(),
            bridge: MemoryBridgeConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// SqliteMemoryConfig (RFC-012: SQLite Memory Storage)
// ---------------------------------------------------------------------------

/// SQLite-backed memory storage configuration (RFC-012).
///
/// When enabled, memories are stored in a single `memory.db` file with
/// FTS5 BM25 + sqlite-vec KNN search. Falls back to the existing JSON
/// + TF-IDF approach when disabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteMemoryConfig {
    /// Enable SQLite-backed memory storage.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Path to the SQLite database file.
    /// Empty string means default: `~/.oxios/workspace/memory.db`
    #[serde(default)]
    pub path: String,
    /// Embedding vector dimension.
    /// Controls the `vec0` virtual table dimension.
    /// Common values: 128 (fast), 256 (balanced), 768 (full Gemma).
    #[serde(default = "default_embedding_dim")]
    pub embedding_dim: usize,
    /// Enable WAL mode for concurrent reads.
    #[serde(default = "default_true")]
    pub wal_mode: bool,
}

fn default_embedding_dim() -> usize {
    256
}

impl Default for SqliteMemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            path: String::new(),
            embedding_dim: 256,
            wal_mode: true,
        }
    }
}

// ---------------------------------------------------------------------------
// EmbeddingConfig (RFC-012: Embedding Provider)
// ---------------------------------------------------------------------------

/// Embedding provider configuration (RFC-012).
///
/// Controls which embedding model is used for semantic search.
/// When `provider = "api"`, uses an OpenAI-compatible remote embedding
/// endpoint. When `provider = "gguf"` and the `embedding-gguf` feature is
/// enabled on aarch64, uses EmbeddingGemma-300m locally. Otherwise
/// falls back to TF-IDF (sparse vectors; no sqlite-vec KNN).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Embedding provider: "tfidf" (default), "gguf", or "api".
    #[serde(default = "default_embedding_provider")]
    pub provider: String,
    /// Matryoshka dimension: 128, 256, 512, or 768 (gguf).
    /// For "api", defaults to the model's known dimensionality
    /// (text-embedding-3-small=1536, text-embedding-3-large=3072).
    #[serde(default = "default_embedding_dim")]
    pub dimension: usize,
    /// Model TTL in seconds. Unloaded after this duration of inactivity.
    /// Only used when provider = "gguf".
    #[serde(default = "default_model_ttl")]
    pub model_ttl_secs: u64,
    /// API endpoint URL (provider = "api"). E.g.
    /// `https://api.openai.com/v1/embeddings`.
    #[serde(default)]
    pub api_endpoint: String,
    /// API bearer key (provider = "api"). Empty → inherit from active
    /// LLM provider's api_key at boot.
    #[serde(default)]
    pub api_key: String,
    /// Embedding model name (provider = "api"). E.g.
    /// `text-embedding-3-small`.
    #[serde(default)]
    pub api_model: String,
}

fn default_embedding_provider() -> String {
    // Default to TF-IDF; users opt into "api" or "gguf" via config.
    // GGUF/MLX feature gating happens at runtime in `kernel.rs`.
    "tfidf".to_string()
}

fn default_model_ttl() -> u64 {
    300 // 5 minutes
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: default_embedding_provider(),
            dimension: default_embedding_dim(),
            model_ttl_secs: default_model_ttl(),
            api_endpoint: String::new(),
            api_key: String::new(),
            api_model: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// LearningConfig (RFC-012 Phase 4: SONA)
// ---------------------------------------------------------------------------

/// Learning engine configuration (RFC-012 Phase 4).
///
/// Controls SONA self-learning persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningConfig {
    /// Enable the learning subsystem (SONA).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// SONA operating mode: "realtime", "balanced", "research", "edge".
    #[serde(default = "default_sona_mode")]
    pub sona_mode: String,
    /// Interval between automatic distillation runs (hours).
    #[serde(default = "default_distill_interval")]
    pub distill_interval_hours: u64,
    /// Minimum quality score for auto-promoting patterns to long-term.
    #[serde(default = "default_auto_promote_quality")]
    pub auto_promote_quality: f32,
    /// Minimum usage count before auto-promotion is considered.
    #[serde(default = "default_auto_promote_min_usage")]
    pub auto_promote_min_usage: u32,
}

fn default_sona_mode() -> String {
    "balanced".to_string()
}

fn default_distill_interval() -> u64 {
    6
}

fn default_auto_promote_quality() -> f32 {
    0.8
}

fn default_auto_promote_min_usage() -> u32 {
    3
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sona_mode: default_sona_mode(),
            distill_interval_hours: default_distill_interval(),
            auto_promote_quality: default_auto_promote_quality(),
            auto_promote_min_usage: default_auto_promote_min_usage(),
        }
    }
}

// ---------------------------------------------------------------------------
// MemoryBridgeConfig (RFC-012 Phase 7: SQLite ↔ MEMORY.md)
// ---------------------------------------------------------------------------

/// AutoMemoryBridge configuration (RFC-012 Phase 7).
///
/// Controls bidirectional sync between SQLite memory store
/// and external MEMORY.md files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBridgeConfig {
    /// Enable bidirectional sync with MEMORY.md.
    #[serde(default)]
    pub sync_enabled: bool,
    /// Sync interval in seconds.
    #[serde(default = "default_bridge_interval")]
    pub interval_secs: u64,
}

fn default_bridge_interval() -> u64 {
    3600
}

impl Default for MemoryBridgeConfig {
    fn default() -> Self {
        Self {
            sync_enabled: false,
            interval_secs: default_bridge_interval(),
        }
    }
}

// ---------------------------------------------------------------------------
// ConsolidationConfig (RFC-008: Memory Consolidation)
// ---------------------------------------------------------------------------

/// Memory consolidation configuration (RFC-008).
/// All values have sensible defaults — users never need to configure these.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationConfig {
    /// Preset: "conservative" | "balanced" | "aggressive" | "custom".
    /// When not "custom", all other fields are overridden by the preset values.
    /// Call `apply_preset()` once during kernel init to resolve.
    #[serde(default = "default_preset")]
    pub preset: String,

    // ── Dream Process ─────────────────────────────────
    #[serde(default = "default_true")]
    pub dream_enabled: bool,
    #[serde(default = "default_dream_interval")]
    pub dream_interval_hours: u64,
    #[serde(default = "default_dream_min_sessions")]
    pub dream_min_sessions: u32,

    // ── Tier Budgets ──────────────────────────────────
    #[serde(default = "default_hot_max")]
    pub hot_max_entries: usize,
    #[serde(default = "default_warm_max")]
    pub warm_max_entries: usize,
    #[serde(default = "default_cold_max")]
    pub cold_max_entries: usize,
    #[serde(default = "default_hot_token_budget")]
    pub hot_token_budget: usize,

    // ── Decay ─────────────────────────────────────────
    #[serde(default = "default_true")]
    pub decay_enabled: bool,
    #[serde(default = "default_one")]
    pub decay_multiplier: f32,
    #[serde(default = "default_decay_threshold")]
    pub decay_threshold: f32,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,

    // ── Auto-Protection ───────────────────────────────
    #[serde(default = "default_true")]
    pub auto_protection: bool,
    #[serde(default = "default_protection_low_access")]
    pub protection_low_access: u32,
    #[serde(default = "default_protection_medium_access")]
    pub protection_medium_access: u32,
    #[serde(default = "default_protection_high_access")]
    pub protection_high_access: u32,
    #[serde(default = "default_protection_medium_sessions")]
    pub protection_medium_sessions: u32,
    #[serde(default = "default_protection_high_sessions")]
    pub protection_high_sessions: u32,

    // ── Auto-Classification ───────────────────────────
    #[serde(default = "default_true")]
    pub auto_classification: bool,
    #[serde(default = "default_type_promotion_threshold")]
    pub type_promotion_repetitions: u32,

    // ── Compaction ────────────────────────────────────
    #[serde(default = "default_compaction_threshold")]
    pub compaction_line_threshold: usize,
    #[serde(default = "default_true")]
    pub llm_compaction: bool,

    // ── Dream LLM ──────────────────────────────────────
    /// Optional model for Dream LLM operations (None = rule-based fallback).
    #[serde(default)]
    pub dream_model: Option<String>,

    // ── Protection Demotion ────────────────────────────
    #[serde(default = "default_true")]
    pub protection_demotion_enabled: bool,
    #[serde(default = "default_demotion_stale_days")]
    pub protection_demotion_stale_days: u32,
    #[serde(default = "default_demotion_max_step")]
    pub protection_demotion_max_step: u32,

    // ── Proactive Recall ──────────────────────────────
    #[serde(default = "default_true")]
    pub proactive_recall: bool,
    #[serde(default = "default_proactive_limit")]
    pub proactive_recall_limit: usize,
    #[serde(default = "default_proactive_threshold")]
    pub proactive_recall_threshold: f32,
}

fn default_dream_interval() -> u64 {
    24
}
fn default_dream_min_sessions() -> u32 {
    5
}
fn default_hot_max() -> usize {
    50
}
fn default_warm_max() -> usize {
    500
}
fn default_cold_max() -> usize {
    10_000
}
fn default_hot_token_budget() -> usize {
    3_000
}
fn default_one() -> f32 {
    1.0
}
fn default_decay_threshold() -> f32 {
    0.05
}
fn default_retention_days() -> u32 {
    90
}
fn default_protection_low_access() -> u32 {
    2
}
fn default_protection_medium_access() -> u32 {
    3
}
fn default_protection_high_access() -> u32 {
    5
}
fn default_protection_medium_sessions() -> u32 {
    2
}
fn default_protection_high_sessions() -> u32 {
    3
}
fn default_type_promotion_threshold() -> u32 {
    3
}
fn default_compaction_threshold() -> usize {
    200
}
fn default_proactive_limit() -> usize {
    5
}
fn default_proactive_threshold() -> f32 {
    0.6
}
fn default_demotion_stale_days() -> u32 {
    30
}
fn default_demotion_max_step() -> u32 {
    1
}

fn default_preset() -> String {
    "balanced".into()
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            preset: default_preset(),
            dream_enabled: true,
            dream_interval_hours: 24,
            dream_min_sessions: 5,
            hot_max_entries: 50,
            warm_max_entries: 500,
            cold_max_entries: 10_000,
            hot_token_budget: 3_000,
            decay_enabled: true,
            decay_multiplier: 1.0,
            decay_threshold: 0.05,
            retention_days: 90,
            auto_protection: true,
            protection_low_access: 2,
            protection_medium_access: 3,
            protection_high_access: 5,
            protection_medium_sessions: 2,
            protection_high_sessions: 3,
            auto_classification: true,
            type_promotion_repetitions: 3,
            compaction_line_threshold: 200,
            llm_compaction: true,
            dream_model: None,
            protection_demotion_enabled: true,
            protection_demotion_stale_days: 30,
            protection_demotion_max_step: 1,
            proactive_recall: true,
            proactive_recall_limit: 5,
            proactive_recall_threshold: 0.6,
        }
    }
}

impl ConsolidationConfig {
    /// Apply the preset to all fields.
    /// Call once during kernel initialization.
    /// When `preset` is "custom", individual fields are left untouched.
    pub fn apply_preset(&mut self) {
        let resolved = match self.preset.as_str() {
            "conservative" => Self::conservative(),
            "aggressive" => Self::aggressive(),
            "custom" => return,
            _ => Self::default(), // "balanced" 및 알 수 없는 값
        };
        *self = resolved;
    }

    /// Conservative preset: slow decay, long retention, larger capacities.
    fn conservative() -> Self {
        Self {
            preset: "conservative".into(),
            dream_enabled: true,
            dream_interval_hours: 48,
            dream_min_sessions: 10,
            hot_max_entries: 100,
            warm_max_entries: 1000,
            cold_max_entries: 50_000,
            hot_token_budget: 5_000,
            decay_enabled: true,
            decay_multiplier: 0.8,
            decay_threshold: 0.05,
            retention_days: 365,
            auto_protection: true,
            protection_low_access: 3,
            protection_medium_access: 5,
            protection_high_access: 10,
            protection_medium_sessions: 3,
            protection_high_sessions: 5,
            auto_classification: true,
            type_promotion_repetitions: 5,
            compaction_line_threshold: 300,
            llm_compaction: true,
            dream_model: None,
            protection_demotion_enabled: true,
            protection_demotion_stale_days: 90,
            protection_demotion_max_step: 1,
            proactive_recall: true,
            proactive_recall_limit: 8,
            proactive_recall_threshold: 0.5,
        }
    }

    /// Aggressive preset: fast decay, short retention, smaller capacities.
    fn aggressive() -> Self {
        Self {
            preset: "aggressive".into(),
            dream_enabled: true,
            dream_interval_hours: 4,
            dream_min_sessions: 2,
            hot_max_entries: 20,
            warm_max_entries: 100,
            cold_max_entries: 1_000,
            hot_token_budget: 2_000,
            decay_enabled: true,
            decay_multiplier: 1.0,
            decay_threshold: 0.1,
            retention_days: 30,
            auto_protection: true,
            protection_low_access: 1,
            protection_medium_access: 2,
            protection_high_access: 3,
            protection_medium_sessions: 1,
            protection_high_sessions: 2,
            auto_classification: true,
            type_promotion_repetitions: 2,
            compaction_line_threshold: 150,
            llm_compaction: true,
            dream_model: None,
            protection_demotion_enabled: true,
            protection_demotion_stale_days: 14,
            protection_demotion_max_step: 2,
            proactive_recall: true,
            proactive_recall_limit: 3,
            proactive_recall_threshold: 0.7,
        }
    }
}

/// Channel activation configuration.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ChannelsConfig {
    /// List of channel names to activate on startup.
    /// Channels are message-only interfaces (CLI, Telegram).
    #[serde(default)]
    pub enabled: Vec<String>,

    /// Telegram-specific configuration.
    #[serde(default)]
    pub telegram: TelegramChannelConfig,
}

/// Surface activation configuration.
///
/// Surfaces are kernel-connected control interfaces (Web dashboard, future desktop apps).
/// They have direct kernel access for management, monitoring, and configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SurfacesConfig {
    /// List of surface names to activate on startup.
    /// Default: ["web"] if the web feature is compiled in.
    #[serde(default = "default_surfaces_enabled")]
    pub enabled: Vec<String>,
}

fn default_surfaces_enabled() -> Vec<String> {
    vec!["web".to_string()]
}

impl Default for SurfacesConfig {
    fn default() -> Self {
        Self {
            enabled: default_surfaces_enabled(),
        }
    }
}

/// Telegram channel configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TelegramChannelConfig {
    /// Environment variable name holding the bot token.
    #[serde(default = "default_telegram_token_env")]
    pub bot_token_env: String,
    /// List of allowed Telegram user IDs (empty = allow all).
    #[serde(default)]
    pub allowed_users: Vec<i64>,
    /// Telegram session management settings.
    #[serde(default)]
    pub session: TelegramSessionConfig,
    /// Telegram Bot API base URL. Default: the official API. Point at a
    /// self-hosted Bot API server (or a local test double) to override.
    #[serde(default = "default_telegram_api_base")]
    pub api_base: String,
}

fn default_telegram_token_env() -> String {
    "TELEGRAM_BOT_TOKEN".to_string()
}

fn default_telegram_api_base() -> String {
    "https://api.telegram.org".to_string()
}

impl Default for TelegramChannelConfig {
    fn default() -> Self {
        Self {
            bot_token_env: default_telegram_token_env(),
            allowed_users: Vec::new(),
            session: TelegramSessionConfig::default(),
            api_base: default_telegram_api_base(),
        }
    }
}

/// Role-to-model routing configuration (RFC-032).
/// Maps role names to model IDs in "provider/model" format.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoleRoutingConfig {
    /// Role name → model ID mapping (e.g. "coder" → "anthropic/claude-sonnet-4-20250514").
    #[serde(default)]
    pub roles: std::collections::HashMap<String, String>,
}

/// LLM engine configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(clippy::derivable_impls)]
pub struct EngineConfig {
    /// Default model in "provider/model" format.
    /// Empty string means no model configured — onboarding required.
    #[serde(default)]
    pub default_model: String,
    /// Explicit API key override (highest priority).
    /// If empty/None, falls back to oxi auth store, then env vars.
    /// Masked when serialized to API responses.
    #[serde(default, skip_serializing)]
    pub api_key: Option<String>,
    /// Per-provider options for fine-grained control (thinking mode, etc.).
    /// Passed through to `AgentLoopConfig::provider_options`.
    #[serde(default)]
    pub provider_options: Option<oxicode_sdk::ProviderOptions>,
    /// Enable complexity-based model routing.
    /// When enabled, the engine can route simple tasks to cheaper models
    /// and complex tasks to more capable ones.
    #[serde(default)]
    pub routing_enabled: bool,
    /// Prefer cost-efficient models when routing.
    #[serde(default)]
    pub prefer_cost_efficient: bool,
    /// Fallback models to try when the primary model fails.
    #[serde(default)]
    pub fallback_models: Vec<String>,
    /// Models excluded from automatic routing.
    #[serde(default)]
    pub excluded_models: Vec<String>,
    /// Role-based model routing (RFC-032).
    /// Maps role names (e.g. "coder", "writer") to model IDs.
    /// When present, messages with a matching role will use the mapped model.
    #[serde(default)]
    pub role_routing: RoleRoutingConfig,
    /// Default model for one-shot (QuickAsk) requests in "provider/model"
    /// format. When None, one-shot falls back to `default_model`. Lets the
    /// user point throwaway questions at a cheaper/faster model.
    #[serde(default)]
    pub quick_ask_model: Option<String>,
    /// SDK lifecycle hooks (Claude Code compatible schema).
    #[serde(default)]
    pub hooks: Vec<oxicode_sdk::ports::hooks::HookSpec>,
    /// Multi-model router configuration (SDK 0.66.0 router feature).
    #[serde(default)]
    pub router: Option<RouterConfig>,
}

#[allow(clippy::derivable_impls)]
impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            default_model: String::new(),
            api_key: None,
            provider_options: None,
            routing_enabled: false,
            prefer_cost_efficient: false,
            fallback_models: Vec::new(),
            excluded_models: Vec::new(),
            role_routing: RoleRoutingConfig::default(),
            quick_ask_model: None,
            hooks: Vec::new(),
            router: None,
        }
    }
}
/// Router profile configuration loaded from config.toml.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RouterConfig {
    /// Enable router. When true, `default_model` becomes `"router/<default_profile>"`.
    #[serde(default)]
    pub enabled: bool,
    /// Default profile name.
    #[serde(default = "default_router_profile")]
    pub default_profile: String,
    /// Optional classifier model for LLM-based classification in ambiguous cases.
    #[serde(default)]
    pub classifier_model: Option<String>,
    /// Maximum session budget in USD.
    #[serde(default)]
    pub max_session_budget: Option<f64>,
    /// Upgrade to strong tier when context tokens exceed this.
    #[serde(default)]
    pub context_upgrade_threshold: Option<usize>,
    /// Tier configurations per profile.
    #[serde(default)]
    pub profiles: std::collections::HashMap<String, RouterProfileConfig>,
    /// Scoring weights.
    #[serde(default)]
    pub scoring: RouterScoringConfig,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_profile: default_router_profile(),
            classifier_model: None,
            max_session_budget: None,
            context_upgrade_threshold: None,
            profiles: std::collections::HashMap::new(),
            scoring: RouterScoringConfig::default(),
        }
    }
}
impl RouterConfig {
    /// Validate the router configuration.
    ///
    /// Returns `Ok(())` when:
    /// - the router is disabled (it is a no-op), or
    /// - when enabled, `default_profile` resolves to a profile AND that
    ///   profile has at least one tier configured.
    ///
    /// On failure, returns an actionable message suitable for a startup
    /// failure ("router enabled but ..."). Callers should surface this as a
    /// hard boot error rather than letting the kernel proceed to a
    /// `router/<missing>` model-resolution failure.
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }

        let Some(profile) = self.profiles.get(&self.default_profile) else {
            return Err(format!(
                "router enabled but default_profile '{}' is not defined under [engine.router.profiles]",
                self.default_profile
            ));
        };

        let has_tier = profile.tiers.fast.is_some()
            || profile.tiers.balanced.is_some()
            || profile.tiers.strong.is_some();
        if !has_tier {
            return Err(format!(
                "router enabled but default_profile '{}' has no configured tiers — add [engine.router.profiles.{}.tiers] or disable router",
                self.default_profile, self.default_profile
            ));
        }

        Ok(())
    }
}

fn default_router_profile() -> String {
    "auto".to_string()
}

/// A named routing profile mapping tiers to model configs.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RouterProfileConfig {
    /// Tier configurations.
    #[serde(default)]
    pub tiers: RouterTiersConfig,
}

/// Tier-to-model mapping.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RouterTiersConfig {
    #[serde(default)]
    pub fast: Option<RouterTierConfig>,
    #[serde(default)]
    pub balanced: Option<RouterTierConfig>,
    #[serde(default)]
    pub strong: Option<RouterTierConfig>,
}

/// Configuration for a single routing tier.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RouterTierConfig {
    /// Model ID in "provider/model" format.
    pub model: String,
    /// Fallback models.
    #[serde(default)]
    pub fallbacks: Vec<String>,
    /// Thinking budget for reasoning models.
    #[serde(default)]
    pub thinking: Option<RouterThinkingConfig>,
}

/// Thinking budget for a tier model.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RouterThinkingConfig {
    pub budget: u32,
}

/// Scoring weights for router signal aggregation.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RouterScoringConfig {
    #[serde(default = "default_weight_structural")]
    pub structural: f64,
    #[serde(default = "default_weight_behavioral")]
    pub behavioral: f64,
    #[serde(default = "default_weight_context")]
    pub context: f64,
    #[serde(default = "default_weight_vision")]
    pub vision: f64,
    #[serde(default = "default_weight_message")]
    pub message: f64,
}

impl Default for RouterScoringConfig {
    fn default() -> Self {
        Self {
            structural: 0.25,
            behavioral: 0.20,
            context: 0.15,
            vision: 0.10,
            message: 0.30,
        }
    }
}

fn default_weight_structural() -> f64 {
    0.25
}
fn default_weight_behavioral() -> f64 {
    0.20
}
fn default_weight_context() -> f64 {
    0.15
}
fn default_weight_vision() -> f64 {
    0.10
}
fn default_weight_message() -> f64 {
    0.30
}

/// Daemon mode configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DaemonConfig {
    /// PID file path.
    #[serde(default = "default_pid_file")]
    pub pid_file: String,
    /// Log directory.
    #[serde(default = "default_daemon_log_dir")]
    pub log_dir: String,
}

fn default_pid_file() -> String {
    dirs::home_dir()
        .map(|h| format!("{}/.oxios/oxios.pid", h.display()))
        .unwrap_or_else(|| "./oxios.pid".into())
}

fn default_daemon_log_dir() -> String {
    dirs::home_dir()
        .map(|h| format!("{}/.oxios/logs", h.display()))
        .unwrap_or_else(|| "./logs".into())
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            pid_file: default_pid_file(),
            log_dir: default_daemon_log_dir(),
        }
    }
}

/// Session management configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SessionConfig {
    /// Maximum number of sessions to retain.
    /// When exceeded, oldest sessions (by `updated_at`) are pruned.
    /// Set to 0 for unlimited.
    #[serde(default = "default_max_sessions")]
    pub max_sessions: usize,

    /// Time-to-live for sessions in hours.
    /// Sessions older than this are automatically pruned.
    /// Set to 0 for unlimited (no TTL-based pruning).
    #[serde(default = "default_session_ttl_hours")]
    pub ttl_hours: u64,

    /// Enable automatic session pruning on every session save.
    #[serde(default = "default_true")]
    pub auto_prune: bool,
}

fn default_max_sessions() -> usize {
    100
}

fn default_session_ttl_hours() -> u64 {
    168 // 7 days
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_sessions: default_max_sessions(),
            ttl_hours: default_session_ttl_hours(),
            auto_prune: true,
        }
    }
}

/// RFC-025 Phase 5: Mount auto-promotion configuration.
/// Controls the background scanner that promotes frequently-used paths into
/// Mounts. See `mount::path_promotion`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MountsConfig {
    /// Enable the auto-promotion scanner.
    #[serde(default = "default_true")]
    pub auto_promote_enabled: bool,
    /// Minimum distinct touches within the window to trigger promotion.
    #[serde(default = "default_promote_threshold")]
    pub auto_promote_threshold: usize,
    /// How far back to look, in days.
    #[serde(default = "default_promote_window_days")]
    pub auto_promote_window_days: i64,
    /// Seconds between promotion scans (background cadence).
    #[serde(default = "default_promote_interval_secs")]
    pub auto_promote_interval_secs: u64,
}

fn default_promote_threshold() -> usize {
    3
}

fn default_promote_window_days() -> i64 {
    14
}

fn default_promote_interval_secs() -> u64 {
    3600 // hourly
}

impl Default for MountsConfig {
    fn default() -> Self {
        Self {
            auto_promote_enabled: true,
            auto_promote_threshold: default_promote_threshold(),
            auto_promote_window_days: default_promote_window_days(),
            auto_promote_interval_secs: default_promote_interval_secs(),
        }
    }
}

/// Telegram session management configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TelegramSessionConfig {
    /// Automatically rotate to a new session after this many hours of inactivity.
    /// Set to 0 to disable time-based rotation.
    #[serde(default = "default_telegram_session_rotation_hours")]
    pub rotation_hours: u64,

    /// Maximum number of messages per session before auto-rotating.
    /// Set to 0 for unlimited.
    #[serde(default = "default_telegram_session_max_messages")]
    pub max_messages: usize,
}

fn default_telegram_session_rotation_hours() -> u64 {
    2 // 2 hours
}

fn default_telegram_session_max_messages() -> usize {
    0 // unlimited by default
}

impl Default for TelegramSessionConfig {
    fn default() -> Self {
        Self {
            rotation_hours: default_telegram_session_rotation_hours(),
            max_messages: default_telegram_session_max_messages(),
        }
    }
}

/// Top-level Oxios configuration.
/// A single system agent model assignment.
/// Lets users pick a different model for each system task.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SystemAgentItem {
    /// Model id in "provider/model" format. Empty = inherit default.
    #[serde(default)]
    pub model: String,
    /// Whether this system task is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Token cap for this task.
    #[serde(default)]
    pub context_limit: Option<u32>,
    /// Override system prompt.
    #[serde(default)]
    pub custom_prompt: Option<String>,
}

/// System agent model assignments (ported from LobeHub).
/// Each field controls which model is used for a specific background task.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SystemAgentsConfig {
    /// Auto topic naming.
    #[serde(default)]
    pub topic: SystemAgentItem,
    /// AI image topic naming.
    #[serde(default)]
    pub generation_topic: SystemAgentItem,
    /// Message translation.
    #[serde(default)]
    pub translation: SystemAgentItem,
    /// Conversation history compression.
    #[serde(default)]
    pub history_compress: SystemAgentItem,
    /// Agent metadata generation (name, description, avatar, tags).
    #[serde(default)]
    pub agent_meta: SystemAgentItem,
    /// Follow-up suggestion chips.
    #[serde(default)]
    pub follow_up_action: SystemAgentItem,
    /// Input auto-complete (ghost text).
    #[serde(default)]
    pub input_completion: SystemAgentItem,
    /// Prompt rewriting.
    #[serde(default)]
    pub prompt_rewrite: SystemAgentItem,
    /// Memory analysis — extract identity, preferences, context, etc.
    #[serde(default)]
    pub memory_analysis: SystemAgentItem,
    /// Memory embedding model.
    #[serde(default)]
    pub memory_embedding: SystemAgentItem,
    /// Memory persona summary writer.
    #[serde(default)]
    pub memory_persona_writer: SystemAgentItem,
}

impl SystemAgentsConfig {
    /// Resolve the model for a given system task.
    pub fn model_for_task(&self, task: &str) -> Option<String> {
        let item = match task {
            "topic" => &self.topic,
            "generation_topic" => &self.generation_topic,
            "translation" => &self.translation,
            "history_compress" => &self.history_compress,
            "agent_meta" => &self.agent_meta,
            "follow_up_action" => &self.follow_up_action,
            "input_completion" => &self.input_completion,
            "prompt_rewrite" => &self.prompt_rewrite,
            "memory_analysis" => &self.memory_analysis,
            "memory_embedding" => &self.memory_embedding,
            "memory_persona_writer" => &self.memory_persona_writer,
            _ => return None,
        };
        if !item.enabled || item.model.is_empty() {
            None
        } else {
            Some(item.model.clone())
        }
    }

    /// Check if a system task is enabled.
    pub fn is_enabled(&self, task: &str) -> bool {
        self.model_for_task(task).is_some()
            || match task {
                "topic" => self.topic.enabled,
                "generation_topic" => self.generation_topic.enabled,
                "translation" => self.translation.enabled,
                "history_compress" => self.history_compress.enabled,
                "agent_meta" => self.agent_meta.enabled,
                "follow_up_action" => self.follow_up_action.enabled,
                "input_completion" => self.input_completion.enabled,
                "prompt_rewrite" => self.prompt_rewrite.enabled,
                "memory_analysis" => self.memory_analysis.enabled,
                "memory_embedding" => self.memory_embedding.enabled,
                "memory_persona_writer" => self.memory_persona_writer.enabled,
                _ => false,
            }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct OxiosConfig {
    /// Kernel settings.
    #[serde(default)]
    pub kernel: KernelConfig,
    /// LLM engine settings.
    #[serde(default)]
    pub engine: EngineConfig,
    /// Daemon mode settings.
    #[serde(default)]
    pub daemon: DaemonConfig,
    /// Gateway settings.
    #[serde(default)]
    pub gateway: GatewayConfig,
    /// Orchestrator settings (Ouroboros protocol execution).
    #[serde(default)]
    pub orchestrator: OrchestratorConfig,
    /// Intent engine settings (assess/crystallize/review model + retry).
    #[serde(default)]
    pub intent: IntentConfig,
    /// System agent model assignments (LobeHub-inspired).
    #[serde(default)]
    pub system_agents: SystemAgentsConfig,
    /// Context manager settings (LLM context window management).
    #[serde(default)]
    pub context: ContextConfig,
    /// Security/access control settings.
    #[serde(default)]
    pub security: SecurityConfig,
    /// Persona system settings.
    #[serde(default)]
    pub persona: PersonaConfig,
    /// Memory system settings.
    #[serde(default)]
    pub memory: MemoryConfig,
    /// Brain daemon connection settings (RFC-047).
    #[serde(default)]
    pub brain: BrainSection,
    /// Foundation settings (RFC-048).
    #[serde(default)]
    pub foundation: FoundationConfig,
    /// Cron scheduler settings.
    #[serde(default)]
    pub cron: CronConfig,
    /// MCP server configurations.
    #[serde(default)]
    pub mcp: McpConfig,
    /// Git version control settings.
    #[serde(default)]
    pub git: GitConfig,
    /// Audit trail configuration.
    #[serde(default)]
    pub audit: AuditConfig,
    /// Budget enforcement configuration.
    #[serde(default)]
    pub budget: BudgetConfig,
    /// Exec configuration (host command execution bridge).
    #[serde(default)]
    pub exec: ExecConfig,
    /// Resource monitor configuration.
    #[serde(default)]
    pub resource_monitor: ResourceMonitorConfig,
    /// Logging configuration.
    #[serde(default)]
    pub logging: LoggingConfig,
    /// Channel activation configuration (message interfaces: CLI, Telegram).
    #[serde(default)]
    pub channels: ChannelsConfig,
    /// Surface activation configuration (control interfaces: Web dashboard).
    #[serde(default)]
    pub surfaces: Option<SurfacesConfig>,
    /// Remote companion surface (RFC-044). Disabled by default.
    #[serde(default)]
    pub remote: RemoteConfig,
    /// Headless browser configuration.
    #[serde(default)]
    pub browser: BrowserConfig,
    /// Session management configuration.
    #[serde(default)]
    pub session: SessionConfig,
    /// RFC-025: Mount system configuration (auto-promotion scanner).
    #[serde(default)]
    pub mounts: MountsConfig,
    /// ClawHub marketplace configuration.
    #[serde(default)]
    pub marketplace: MarketplaceConfig,
    /// Calendar configuration.
    #[serde(default)]
    pub calendar: CalendarConfig,
    /// Email configuration.
    #[serde(default)]
    pub email: EmailConfig,
    /// Agent history log configuration.
    #[serde(default)]
    pub agent_log: AgentLogConfig,
    /// Token Maxing mode configuration (RFC-031).
    #[serde(default)]
    pub token_maxing: crate::token_maxing::TokenMaxingConfig,
    /// Image generation configuration (OpenAI-compatible providers).
    #[serde(default)]
    pub image_gen: ImageGenConfig,
    /// oximemo integration (opt-in first-party app module; requires the `memo`
    /// cargo feature). oxios acts as a co-client of the oximemo vault.
    #[serde(default)]
    pub memo: MemoConfig,
    /// oxiline (timeline) integration config.
    #[serde(default)]
    pub timeline: TimelineConfig,
}

/// Image generation configuration.
///
/// Opt-in (`enabled = false` by default). When enabled, the
/// `image_generation` tool is registered and agents can generate images via
/// an OpenAI-compatible `/v1/images/generations` endpoint. The API key is
/// resolved via [`CredentialStore`](crate::credential::CredentialStore) — the
/// same provider key used for chat — so no separate credential is needed.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImageGenConfig {
    /// Enable the image generation tool.
    #[serde(default)]
    pub enabled: bool,
    /// Provider id. Currently only `"openai"` (OpenAI-compatible).
    #[serde(default = "default_image_gen_provider")]
    pub provider: String,
    /// Base URL for the image API. Defaults to the OpenAI endpoint.
    #[serde(default = "default_image_gen_base_url")]
    pub base_url: String,
    /// Default model when the agent doesn't specify one. Empty = error
    /// (the provider requires a model; set one in config).
    #[serde(default)]
    pub default_model: String,
    /// Default number of images per call (1-8).
    #[serde(default = "default_image_gen_num")]
    pub default_num: u8,
}

impl Default for ImageGenConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: default_image_gen_provider(),
            base_url: default_image_gen_base_url(),
            default_model: String::new(),
            default_num: default_image_gen_num(),
        }
    }
}

fn default_image_gen_provider() -> String {
    "openai".into()
}

fn default_image_gen_base_url() -> String {
    "https://api.openai.com/v1".into()
}

fn default_image_gen_num() -> u8 {
    1
}

/// Kernel configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KernelConfig {
    /// Path to the workspace directory.
    #[serde(default = "default_workspace")]
    pub workspace: String,
    /// Broadcast capacity for the event bus.
    #[serde(default = "default_event_bus_capacity")]
    pub event_bus_capacity: usize,
    /// Maximum number of concurrent agents.
    #[serde(default = "default_max_agents")]
    pub max_agents: usize,
    /// Explicit override for the markdown knowledge base root.
    ///
    /// Resolution order when a `KnowledgeBase` is constructed:
    /// 1. `kernel.knowledge_root` (this field, after `expand_home`).
    /// 2. `~/.oxi/config.toml [vault].path` (ecosystem-canonical; default
    ///    reads the file via `OXIOS_OXI_CONFIG_PATH` env override if set,
    ///    else `~/.oxi/config.toml`).
    /// 3. Fallback `~/.oxi/vault` (with `expand_home`).
    ///
    /// `None` ⇒ steps 2–3. The merged vault is shared by oxios and oximemo
    /// per ECOSYSTEM C5 v1.1.
    #[serde(default)]
    pub knowledge_root: Option<String>,
}

fn default_workspace() -> String {
    dirs_home().unwrap_or_else(|| ".".into())
}

fn dirs_home() -> Option<String> {
    dirs::home_dir().map(|h| format!("{}/.oxios/workspace", h.display()))
}

fn default_event_bus_capacity() -> usize {
    256
}

fn default_max_agents() -> usize {
    10
}

impl Default for KernelConfig {
    fn default() -> Self {
        Self {
            workspace: default_workspace(),
            event_bus_capacity: default_event_bus_capacity(),
            max_agents: 10,
            knowledge_root: None,
        }
    }
}

impl KernelConfig {
    /// Resolve the markdown knowledge-base root path.
    ///
    /// Precedence:
    /// 1. `self.knowledge_root` (after `expand_home`).
    /// 2. `~/.oxi/config.toml [vault].path` (after `expand_home`) — the
    ///    ecosystem-canonical vault location shared with oximemo per
    ///    ECOSYSTEM C5 v1.1. Tests can override the config path via the
    ///    `OXIOS_OXI_CONFIG_PATH` env var; production reads
    ///    `~/.oxi/config.toml`.
    /// 3. Fallback `~/.oxi/vault` (with `expand_home`).
    ///
    /// A read failure or missing `[vault]` table falls through to step 3;
    /// resolution is best-effort so a malformed shared config never
    /// blocks oxios from starting.
    pub fn resolved_knowledge_root(&self) -> std::path::PathBuf {
        if let Some(ref kr) = self.knowledge_root {
            return expand_home(kr);
        }
        if let Some(p) = read_oxi_vault_path() {
            return expand_home(&p);
        }
        expand_home("~/.oxi/vault")
    }
}

/// Read the `[vault].path` string from `~/.oxi/config.toml`. Returns
/// `None` if the file is missing, unreadable, malformed, has no `[vault]`
/// table, or the path field is empty/whitespace. Best-effort: errors are
/// swallowed with a `tracing` debug log so resolution failures degrade to
/// the default vault path.
///
/// The `OXIOS_OXI_CONFIG_PATH` env var is a **test-only seam** to redirect
/// the read; production reads `~/.oxi/config.toml` unconditionally.
fn read_oxi_vault_path() -> Option<String> {
    use std::path::PathBuf;
    #[cfg(test)]
    let path: PathBuf = std::env::var_os("OXIOS_OXI_CONFIG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| expand_home("~/.oxi/config.toml"));
    #[cfg(not(test))]
    let path: PathBuf = expand_home("~/.oxi/config.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::debug!(
                path = %path.display(),
                error = %e,
                "oxi config: read failed; falling back to default vault"
            );
            return None;
        }
    };
    #[derive(serde::Deserialize)]
    struct OxiConfig {
        vault: Option<OxiVault>,
    }
    #[derive(serde::Deserialize)]
    struct OxiVault {
        path: Option<String>,
    }
    match toml::from_str::<OxiConfig>(&text) {
        Ok(cfg) => cfg
            .vault
            .and_then(|v| v.path)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        Err(e) => {
            tracing::debug!(
                path = %path.display(),
                error = %e,
                "oxi config: parse failed; falling back to default vault"
            );
            None
        }
    }
}

/// Gateway configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayConfig {
    /// Host to bind the gateway to.
    #[serde(default = "default_gateway_host")]
    pub host: String,
    /// Port for the gateway server.
    #[serde(default = "default_gateway_port")]
    pub port: u16,
    /// Expose `/api-docs` (Swagger UI) and `/openapi.json`.
    ///
    /// For safety this is gated to localhost-only binds (127.0.0.0/8, ::1,
    /// "localhost"). Setting this to `true` while binding to a public address
    /// is a no-op. Default: `false`.
    ///
    /// Why: Swagger UI + the full OpenAPI schema expand the attack surface
    /// (route discovery, parameter names, security scheme details). Local
    /// dev typically wants them; production typically does not.
    #[serde(default)]
    pub expose_api_docs: bool,
    /// RFC-024 SP1: ceiling on `send_and_wait` for HTTP request-response
    /// matching. The HTTP layer returns 504 Gateway Timeout when the
    /// orchestrator does not respond within this duration.
    #[serde(default = "default_response_timeout_secs")]
    pub response_timeout_secs: u64,
    /// RFC-024 SP1: in-memory replay buffer tuning (per channel).
    #[serde(default)]
    pub reliability: GatewayReliabilityConfig,
}

/// Remote companion-surface configuration (RFC-044).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RemoteConfig {
    /// Whether the RemoteRpcSurface is active. Default false — opt-in.
    #[serde(default)]
    pub enabled: bool,
    /// Port for the E2EE WS listener. Default 6768 (orca-compatible).
    #[serde(default = "default_remote_port")]
    pub port: u16,
    /// Bind address for the E2EE WS listener.
    /// Loopback default keeps the surface local-only; widen to a LAN/Tailscale
    /// address (e.g. `100.64.1.20` or `0.0.0.0`) to accept network clients.
    /// Does NOT change the advertised endpoint — see `pairing_address`.
    #[serde(default = "default_remote_bind_address")]
    pub bind_address: String,
    /// Advertised pairing host (does NOT change the bind).
    /// None → auto (tailscale ip -4 → hostname).
    #[serde(default)]
    pub pairing_address: Option<String>,
}
fn default_remote_port() -> u16 {
    6768
}
fn default_remote_bind_address() -> String {
    "127.0.0.1".to_string()
}
impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: default_remote_port(),
            bind_address: default_remote_bind_address(),
            pairing_address: None,
        }
    }
}
/// RFC-024 SP1: in-memory replay buffer tuning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayReliabilityConfig {
    /// Per-channel replay buffer size. Older messages are evicted when
    /// the buffer is full.
    #[serde(default = "default_replay_buffer_size")]
    pub replay_buffer_size: usize,
    /// How long a message stays in the replay buffer.
    #[serde(default = "default_replay_ttl_secs")]
    pub replay_ttl_secs: u64,
}

impl Default for GatewayReliabilityConfig {
    fn default() -> Self {
        Self {
            replay_buffer_size: default_replay_buffer_size(),
            replay_ttl_secs: default_replay_ttl_secs(),
        }
    }
}

fn default_response_timeout_secs() -> u64 {
    120
}
fn default_replay_buffer_size() -> usize {
    512
}
fn default_replay_ttl_secs() -> u64 {
    60
}

impl GatewayConfig {
    /// Whether the gateway may expose `/api-docs` and `/openapi.json`.
    ///
    /// Returns `true` only when both:
    /// - `expose_api_docs` is explicitly enabled, AND
    /// - the bind address is a loopback address.
    pub fn should_expose_api_docs(&self) -> bool {
        if !self.expose_api_docs {
            return false;
        }
        let h = self.host.trim();
        h == "127.0.0.1" || h == "::1" || h == "localhost" || h.starts_with("127.")
    }
}

/// ClawHub marketplace configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MarketplaceConfig {
    /// Base URL for the ClawHub registry.
    /// Defaults to `https://clawhub.ai`.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Whether the marketplace is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Skills.sh (Vercel Labs ecosystem) configuration.
    #[serde(default)]
    pub skills_sh: SkillsShConfig,
}

/// Skills.sh registry configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SkillsShConfig {
    /// Base URL for the Skills.sh API.
    /// Defaults to `https://skills.sh`.
    #[serde(default)]
    pub base_url: Option<String>,
    /// API key for Skills.sh authentication.
    /// Falls back to `SKILLS_SH_TOKEN` env var if not set.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Whether Skills.sh integration is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for MarketplaceConfig {
    fn default() -> Self {
        Self {
            base_url: Some("https://clawhub.ai".to_string()),
            enabled: true,
            skills_sh: SkillsShConfig::default(),
        }
    }
}

impl Default for SkillsShConfig {
    fn default() -> Self {
        Self {
            base_url: None,
            api_key: None,
            enabled: true,
        }
    }
}

/// Calendar configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CalendarConfig {
    /// Enable the calendar system.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Default timezone for events.
    #[serde(default = "default_calendar_timezone")]
    pub timezone: String,
    /// Default reminder minutes for new events.
    #[serde(default = "default_reminder_minutes")]
    pub default_reminder_minutes: Vec<u32>,
    /// Alarm dispatch channels.
    #[serde(default)]
    pub alarm_channels: Vec<String>,
    /// Journal sync mode: "on_open", "midnight", "both".
    #[serde(default = "default_journal_sync")]
    pub journal_sync: String,
    /// Show cron jobs on the calendar.
    #[serde(default = "default_true")]
    pub system_calendar: bool,
    /// Days after which old events are archived.
    #[serde(default = "default_archive_days")]
    pub archive_after_days: u32,
}

fn default_calendar_timezone() -> String {
    "Asia/Seoul".to_string()
}

fn default_reminder_minutes() -> Vec<u32> {
    vec![15]
}

fn default_journal_sync() -> String {
    "on_open".to_string()
}

fn default_archive_days() -> u32 {
    365
}

impl Default for CalendarConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            timezone: default_calendar_timezone(),
            default_reminder_minutes: default_reminder_minutes(),
            alarm_channels: vec![],
            journal_sync: default_journal_sync(),
            system_calendar: true,
            archive_after_days: default_archive_days(),
        }
    }
}

/// oximemo integration configuration (first-party app module, opt-in).
///
/// When `enabled` (and the `memo` cargo feature is compiled in), oxios embeds
/// `oximemo-core` and agents gain a `memo` tool to read/write the user's
/// oximemo vault directly via typed Rust APIs — no CLI shell-out. oxios is a
/// *co-client* of the vault: it shares oximemo's canonical store but never
/// replaces it as the owner. Disabled by default.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MemoConfig {
    /// Enable the oximemo integration (default off).
    #[serde(default)]
    pub enabled: bool,
    /// Path to the oximemo vault directory. Empty = oximemo's default location
    /// (`~/Library/Application Support/com.oximemo.app/` on macOS), resolved by
    /// `oximemo_core::Paths`.
    #[serde(default)]
    pub vault_path: String,
}

/// oxiline integration configuration (first-party app module, opt-in).
///
/// When `enabled` (and the `timeline` cargo feature is compiled in), oxios
/// embeds `oxiline-core` and agents gain a `timeline` tool to read the user's
/// time-tracking data (current activity, today's plan, recent records) via
/// typed Rust APIs — no CLI shell-out. oxios is a *co-client* of the store: it
/// shares oxiline's canonical SQLite database but never replaces it as the
/// owner. v1 is read-only (context-in). Disabled by default.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TimelineConfig {
    /// Enable the oxiline integration (default off).
    #[serde(default)]
    pub enabled: bool,
    /// Path to the oxiline SQLite database. Empty = oxiline's default location
    /// (resolved by `oxiline_core::paths::db_path`, honoring `OXILINE_DB_PATH`).
    #[serde(default)]
    pub db_path: String,
}

/// Email configuration.
///
/// Controls SMTP email sending. When enabled, agents gain the `send_email` tool.
/// v1 sends to the user's own email only.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmailConfig {
    /// Enable the email system.
    #[serde(default)]
    pub enabled: bool,
    /// The user's email address (used as both sender and default recipient).
    #[serde(default)]
    pub my_email: String,
    /// SMTP provider preset ("gmail", "icloud", "fastmail", "custom").
    #[serde(default = "default_email_provider")]
    pub provider: SmtpProvider,
    /// SMTP host (auto-filled from provider if empty).
    #[serde(default)]
    pub host: String,
    /// SMTP port (auto-filled from provider if 0).
    #[serde(default)]
    pub port: u16,
    /// TLS mode (auto-filled from provider if None).
    #[serde(default)]
    pub tls: Option<SmtpTls>,
    /// SMTP auth username (defaults to `my_email` if empty).
    #[serde(default)]
    pub user: String,
    /// Credential store key for the SMTP password.
    /// Falls back to `OXIOS_EMAIL_PASSWORD` env var.
    #[serde(default = "default_email_secret_ref")]
    pub secret_ref: String,
    /// Maximum emails per hour (rate limit, default: 10).
    #[serde(default = "default_rate_limit_emails")]
    pub rate_limit_per_hour: usize,
}

fn default_email_provider() -> SmtpProvider {
    SmtpProvider::Gmail
}

fn default_email_secret_ref() -> String {
    "email_smtp".to_string()
}

fn default_rate_limit_emails() -> usize {
    10
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            my_email: String::new(),
            provider: default_email_provider(),
            host: String::new(),
            port: 0,
            tls: None,
            user: String::new(),
            secret_ref: default_email_secret_ref(),
            rate_limit_per_hour: default_rate_limit_emails(),
        }
    }
}

impl EmailConfig {
    /// Resolve the effective provider, falling back to Gmail.
    pub fn provider(&self) -> SmtpProvider {
        self.provider
    }
}

fn default_gateway_host() -> String {
    "127.0.0.1".into()
}

fn default_gateway_port() -> u16 {
    4200
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            host: default_gateway_host(),
            port: default_gateway_port(),
            expose_api_docs: false,
            response_timeout_secs: default_response_timeout_secs(),
            reliability: GatewayReliabilityConfig::default(),
        }
    }
}

/// Execution mode for commands.
///
/// - `Structured`: Binary allowlist + metacharacter blocking (recommended)
/// - `Shell`: Raw bash execution (dangerous, requires `allow_shell_mode=true`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ExecMode {
    /// Structured binary execution with allowlist and metacharacter blocking.
    #[default]
    Structured,
    /// Shell execution via `bash -c`. DANGEROUS — requires explicit enable.
    Shell,
}

/// Execution allowlist behavior mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum AllowlistMode {
    /// All binaries are permitted (development only).
    Permissive,
    /// Only binaries in `allowed_commands` may execute.
    #[default]
    Enforced,
}

/// Exec configuration.
///
/// Governs how the kernel dispatches commands for execution.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExecConfig {
    /// Default execution mode.
    #[serde(default)]
    pub default_mode: ExecMode,
    /// Allow shell mode. DANGEROUS — should be false in production.
    #[serde(default = "default_false")]
    pub allow_shell_mode: bool,
    /// Commands allowed to run on the host.
    /// If empty, *all* bare-name commands are permitted (development mode).
    #[serde(default)]
    pub allowed_commands: Vec<String>,
    /// Allowlist enforcement mode.
    /// `Permissive` = empty list means all allowed (dev mode).
    /// `Enforced` = only listed commands allowed (production).
    #[serde(default)]
    pub allowlist_mode: AllowlistMode,
    /// Default timeout for an exec call in seconds.
    #[serde(default = "default_exec_timeout")]
    pub default_timeout_secs: u64,
    /// Maximum allowed timeout for an exec call in seconds.
    #[serde(default = "default_exec_max_timeout")]
    pub max_timeout_secs: u64,
}

fn default_false() -> bool {
    false
}

fn default_exec_timeout() -> u64 {
    120
}

fn default_exec_max_timeout() -> u64 {
    600
}

impl ExecConfig {
    /// Check whether a binary / command name is allowed to execute.
    ///
    /// In `Permissive` mode, returns `true` when `allowed_commands` is empty
    /// (all allowed) **or** when the name is present in the allow-list.
    ///
    /// In `Enforced` mode, only names present in the allow-list are permitted.
    pub fn is_binary_allowed(&self, name: &str) -> bool {
        match self.allowlist_mode {
            AllowlistMode::Permissive => {
                self.allowed_commands.is_empty() || self.allowed_commands.iter().any(|c| c == name)
            }
            AllowlistMode::Enforced => self.allowed_commands.iter().any(|c| c == name),
        }
    }
}

impl Default for ExecConfig {
    fn default() -> Self {
        Self {
            default_mode: ExecMode::default(),
            allow_shell_mode: default_false(),
            allowed_commands: Vec::new(),
            allowlist_mode: AllowlistMode::default(),
            default_timeout_secs: default_exec_timeout(),
            max_timeout_secs: default_exec_max_timeout(),
        }
    }
}

/// Orchestrator configuration (Ouroboros protocol execution).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrchestratorConfig {
    /// Maximum evolution iterations (0 = evaluate only, no evolution).
    /// Default: 3.
    #[serde(default = "default_max_evolution_iterations")]
    pub max_evolution_iterations: u32,

    /// Minimum evaluation score for task to be considered passed (0.0–1.0).
    /// Default: 0.8.
    #[serde(default = "default_min_evaluation_score")]
    pub min_evaluation_score: f64,
}

fn default_max_evolution_iterations() -> u32 {
    3
}

fn default_min_evaluation_score() -> f64 {
    0.8
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            max_evolution_iterations: default_max_evolution_iterations(),
            min_evaluation_score: default_min_evaluation_score(),
        }
    }
}

/// Intent engine configuration (RFC-027 unified intent handling).
///
/// Controls the unified intent engine that replaces the legacy Ouroboros
/// five-phase protocol: `assess` → `crystallize` → `execute` → `review` → `retry`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentConfig {
    /// Maximum retry attempts when a Substantial task fails review.
    /// Set to 0 to disable retries entirely.
    /// Default: 2.
    #[serde(default = "default_intent_max_retries")]
    pub max_retries: u32,

    /// Minimum review score (0.0–1.0) required for a verdict to pass.
    /// Reviews below this threshold trigger a retry.
    /// Default: 0.7.
    #[serde(default = "default_intent_score_threshold")]
    pub score_threshold: f64,

    /// Maximum clarification rounds before forcing the task to proceed
    /// with the system's best-guess understanding.
    /// Default: 3.
    #[serde(default = "default_intent_max_clarify_rounds")]
    pub max_clarify_rounds: u32,

    /// Whether to retry Substantial tasks whose review verdict fails.
    /// When false, a failing review is reported back to the user directly.
    /// Default: true.
    #[serde(default = "default_intent_enable_retry")]
    pub enable_retry: bool,

    /// Optional lightweight model ID for `assess`/`crystallize`/`review` calls.
    /// When None, the engine uses the resolver's default model.
    /// Default: None.
    #[serde(default)]
    pub lightweight_model: Option<String>,
}

fn default_intent_max_retries() -> u32 {
    2
}

fn default_intent_score_threshold() -> f64 {
    0.7
}

fn default_intent_max_clarify_rounds() -> u32 {
    3
}

fn default_intent_enable_retry() -> bool {
    true
}

impl Default for IntentConfig {
    fn default() -> Self {
        Self {
            max_retries: default_intent_max_retries(),
            score_threshold: default_intent_score_threshold(),
            max_clarify_rounds: default_intent_max_clarify_rounds(),
            enable_retry: default_intent_enable_retry(),
            lightweight_model: None,
        }
    }
}

/// Context manager configuration (inspired by AIOS).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContextConfig {
    /// Maximum tokens in the active (in-context) tier.
    #[serde(default = "default_active_limit")]
    pub active_limit_tokens: usize,
    /// Maximum entries in the cache tier.
    #[serde(default = "default_cache_limit")]
    pub cache_limit_entries: usize,
}

fn default_active_limit() -> usize {
    100_000
}

fn default_cache_limit() -> usize {
    50
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            active_limit_tokens: default_active_limit(),
            cache_limit_entries: default_cache_limit(),
        }
    }
}

/// Security/access control configuration (inspired by OWASP Agentic AI).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SecurityConfig {
    /// Default allowed tools for agents (least privilege).
    #[serde(default = "default_allowed_tools")]
    pub allowed_tools: Vec<String>,
    /// Whether agents can make network requests by default.
    #[serde(default)]
    pub network_access: bool,
    /// Maximum execution time in seconds for agent tasks.
    #[serde(default = "default_max_exec_time")]
    pub max_execution_time_secs: u64,
    /// Maximum memory in MB for agent tasks.
    #[serde(default = "default_max_memory")]
    pub max_memory_mb: u64,
    /// Whether agents can fork sub-agents by default.
    #[serde(default)]
    pub can_fork: bool,
    /// Tool approval mode system configuration (RFC-035).
    #[serde(default)]
    pub approval: ApprovalConfig,
    /// Maximum audit log entries to retain.
    #[serde(default = "default_max_audit")]
    pub max_audit_entries: usize,
    /// Enable API key authentication.
    #[serde(default)]
    pub auth_enabled: bool,
    /// Allowed CORS origins.
    #[serde(default = "default_cors_origins")]
    pub cors_origins: Vec<String>,
    /// Path for audit log file (optional, enables file-based persistence).
    #[serde(default)]
    pub audit_log_path: Option<String>,
    /// Rate limit for API endpoints (requests per minute).
    #[serde(default = "default_rate_limit_per_minute")]
    pub rate_limit_per_minute: u32,
}

fn default_allowed_tools() -> Vec<String> {
    vec![
        "read".to_string(),
        "write".to_string(),
        "edit".to_string(),
        "bash".to_string(),
        "grep".to_string(),
        "find".to_string(),
        "exec".to_string(),
    ]
}

fn default_max_exec_time() -> u64 {
    300
}

fn default_max_memory() -> u64 {
    512
}

fn default_max_audit() -> usize {
    10_000
}

fn default_rate_limit_per_minute() -> u32 {
    // Local-first single-user server — 600/min (10 req/s) gives ample headroom
    // for the ~20 frontend polling queries without throttling legitimate use.
    // 0 = unlimited (see RateLimiter::new).
    600
}

fn default_cors_origins() -> Vec<String> {
    // Browsers treat `localhost` and `127.0.0.1` as distinct origins, so both
    // must be allow-listed or cross-origin requests silently fail CORS checks.
    // 4200 = backend that also serves the production SPA (same origin).
    // 5173 = Vite dev server (`bun dev` in web/).
    vec![
        "http://localhost:4200".to_string(),
        "http://127.0.0.1:4200".to_string(),
        "http://localhost:5173".to_string(),
        "http://127.0.0.1:5173".to_string(),
    ]
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            allowed_tools: default_allowed_tools(),
            network_access: false,
            max_execution_time_secs: default_max_exec_time(),
            max_memory_mb: default_max_memory(),
            can_fork: false,
            approval: ApprovalConfig::default(),
            max_audit_entries: default_max_audit(),
            auth_enabled: false,
            cors_origins: default_cors_origins(),
            audit_log_path: None,
            rate_limit_per_minute: default_rate_limit_per_minute(),
        }
    }
}

/// Persona system configuration.
///
/// Only one persona is active at a time (single slot in `PersonaManager`).
/// See `docs/rfc-039-persona-completion.md` for the rationale.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct PersonaConfig {
    /// Default persona ID to activate on startup.
    #[serde(default)]
    pub default_persona_id: Option<String>,
}

/// MCP server configuration loaded from config.toml.
///
/// Each key is a server name; the value is a table with:
/// - `command`: executable to run (e.g. "npx", "python")
/// - `args`: arguments array
/// - `env`: optional map of environment variables
/// - `enabled`: whether to start this server on boot (default: true)
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct McpConfig {
    /// Map of server-name → server definition.
    #[serde(default)]
    pub servers: std::collections::HashMap<String, McpServerDef>,
}

/// A single MCP server definition in config.toml.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpServerDef {
    /// Command to execute.
    pub command: String,
    /// Arguments passed to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Whether this server is enabled (default: true).
    #[serde(default = "default_mcp_enabled")]
    pub enabled: bool,
}

fn default_mcp_enabled() -> bool {
    true
}

/// Git version control configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitConfig {
    /// Enable automatic commits for state changes.
    #[serde(default = "default_true")]
    pub auto_commit: bool,
    /// Adopt an existing foreign git repo at the vault root by writing
    /// the `.oxios-git` ownership marker. Default `false` — a foreign
    /// repo disables auto-commit + S-4 reconcile with a loud warning,
    /// so the operator can choose to opt in explicitly (avoids sweeping
    /// a user's uncommitted edits one-commit-per-file through a repo
    /// they didn't author). R16 review F2 (b).
    #[serde(default)]
    pub adopt_foreign_repo: bool,
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            auto_commit: true,
            adopt_foreign_repo: false,
        }
    }
}

/// Audit trail configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuditConfig {
    /// Maximum audit entries before pruning.
    #[serde(default = "default_audit_max_entries")]
    pub max_entries: usize,
    /// Enable audit trail.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_audit_max_entries() -> usize {
    100_000
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            max_entries: default_audit_max_entries(),
            enabled: true,
        }
    }
}

/// Budget enforcement configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BudgetConfig {
    /// Default token budget per agent (0 = unlimited).
    #[serde(default)]
    pub default_token_budget: u64,
    /// Default call budget per agent (0 = unlimited).
    #[serde(default)]
    pub default_calls_budget: u64,
    /// Default budget window in seconds.
    #[serde(default = "default_budget_window")]
    pub default_window_secs: u64,
    /// Enable budget enforcement.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Monthly spend limit in USD. When set, the cost summary includes
    /// month-to-date spend and remaining budget. Phase 1: monitoring +
    /// alerts only. Phase 2: pre-execution enforcement.
    #[serde(default)]
    pub monthly_spend_limit_usd: Option<f64>,
}

fn default_budget_window() -> u64 {
    3600
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            default_token_budget: 0,
            default_calls_budget: 0,
            default_window_secs: default_budget_window(),
            enabled: true,
            monthly_spend_limit_usd: None,
        }
    }
}

/// Resource monitor configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResourceMonitorConfig {
    /// Snapshot interval in seconds.
    #[serde(default = "default_rm_interval")]
    pub interval_secs: u64,
    /// Maximum history entries.
    #[serde(default = "default_rm_history_max")]
    pub history_max: usize,
    /// CPU threshold for overload.
    #[serde(default = "default_rm_cpu_threshold")]
    pub cpu_threshold: f32,
    /// Memory threshold for overload (percentage).
    #[serde(default = "default_rm_mem_threshold")]
    pub memory_threshold: f32,
    /// Load average threshold for overload.
    #[serde(default = "default_rm_load_threshold")]
    pub load_threshold: f32,
}

fn default_rm_interval() -> u64 {
    60
}

fn default_rm_history_max() -> usize {
    60
}

fn default_rm_cpu_threshold() -> f32 {
    90.0
}

fn default_rm_mem_threshold() -> f32 {
    90.0
}

fn default_rm_load_threshold() -> f32 {
    8.0
}

impl Default for ResourceMonitorConfig {
    fn default() -> Self {
        Self {
            interval_secs: default_rm_interval(),
            history_max: default_rm_history_max(),
            cpu_threshold: default_rm_cpu_threshold(),
            memory_threshold: default_rm_mem_threshold(),
            load_threshold: default_rm_load_threshold(),
        }
    }
}

/// Agent history log configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLogConfig {
    /// Maximum number of agent records to keep (0 = unlimited).
    #[serde(default = "default_agent_log_max_entries")]
    pub max_entries: usize,
    /// TTL for agent records in hours (0 = unlimited).
    #[serde(default = "default_agent_log_ttl_hours")]
    pub ttl_hours: u64,
    /// Max tool_calls per agent to persist (0 = unlimited).
    #[serde(default = "default_agent_log_max_tool_calls")]
    pub max_tool_calls_per_agent: usize,
    /// How many agents to prune per cycle.
    #[serde(default = "default_agent_log_prune_batch")]
    pub prune_batch_size: usize,
    /// Path to the SQLite database file (empty = default).
    #[serde(default)]
    pub db_path: String,
}

fn default_agent_log_max_entries() -> usize {
    10_000
}
fn default_agent_log_ttl_hours() -> u64 {
    720
}
fn default_agent_log_max_tool_calls() -> usize {
    500
}
fn default_agent_log_prune_batch() -> usize {
    100
}

impl Default for AgentLogConfig {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            ttl_hours: 720,
            max_tool_calls_per_agent: 500,
            prune_batch_size: 100,
            db_path: String::new(),
        }
    }
}

/// Logging configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingConfig {
    /// Log format: "pretty", "json", or "compact".
    #[serde(default = "default_log_format")]
    pub format: String,
    /// Log level override (e.g. "info", "debug"). Falls back to RUST_LOG env var.
    #[serde(default)]
    pub level: Option<String>,
}

fn default_log_format() -> String {
    "pretty".into()
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            format: default_log_format(),
            level: None,
        }
    }
}

/// Headless browser configuration.
///
/// Engine configuration. Passes through to `oxicode-sdk` browser tools.
/// with an `enabled` toggle. The engine config is passed through directly
/// to the browser — no field-by-field duplication.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BrowserConfig {
    /// Enable the browser integration.
    #[serde(default = "default_browser_enabled")]
    pub enabled: bool,

    /// Engine configuration — deserialized directly into oxios's
    /// [`BrowseConfig`] and propagated to the `oxibrowser-core` backend
    /// on first use (RFC-046). All fields have sensible defaults.
    ///
    /// ```toml
    /// [browser.engine]
    /// user_agent = "MyBot/1.0"
    /// obey_robots = false
    /// js_timeout_ms = 10000
    /// ```
    ///
    /// [`BrowseConfig`]: crate::tools::browse::BrowseConfig
    #[serde(default)]
    pub engine: crate::tools::browse::BrowseConfig,
}

fn default_browser_enabled() -> bool {
    true
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            engine: crate::tools::browse::BrowseConfig::default(),
        }
    }
}

/// Loads configuration from a TOML file.
pub fn load_config(path: &std::path::Path) -> anyhow::Result<OxiosConfig> {
    let content = std::fs::read_to_string(path)?;
    let config: OxiosConfig = toml::from_str(&content)?;
    let (errors, warnings) = config.validate();
    for w in warnings {
        tracing::warn!("config: {}", w);
    }
    if !errors.is_empty() {
        let msg = errors.join("; ");
        anyhow::bail!("Configuration validation failed: {msg}");
    }
    Ok(config)
}

impl OxiosConfig {
    /// Returns the effective API key from the engine config.
    pub fn api_key(&self) -> Option<String> {
        self.engine.api_key.clone().filter(|k| !k.is_empty())
    }

    /// Validate configuration values and return a list of warnings.
    /// Returns (errors, warnings). Empty errors = valid config.
    pub fn validate(&self) -> (Vec<String>, Vec<String>) {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // Kernel validation
        if self.kernel.max_agents == 0 {
            errors.push("kernel.max_agents must be > 0".into());
        }
        if self.kernel.workspace.is_empty() {
            errors.push("kernel.workspace must not be empty".into());
        }

        // Gateway validation
        if self.gateway.port == 0 {
            errors.push("gateway.port must be > 0".into());
        }
        if self.gateway.port < 1024 && self.gateway.host == "0.0.0.0" {
            warnings.push("Running on port <1024 as 0.0.0.0 may require root".into());
        }

        // Cron validation
        for (name, job) in &self.cron.jobs {
            if job.schedule.is_empty() {
                errors.push(format!("cron.jobs.{name}: schedule is empty"));
            } else {
                // Normalize 5-field to 6-field (prepend "0 " for seconds)
                let normalized = {
                    let fields: Vec<&str> = job.schedule.split_whitespace().collect();
                    match fields.len() {
                        5 => format!("0 {}", job.schedule),
                        _ => job.schedule.clone(),
                    }
                };
                if Schedule::from_str(&normalized).is_err() {
                    errors.push(format!(
                        "cron.jobs.{}: invalid cron expression '{}'",
                        name, job.schedule
                    ));
                }
            }
            if job.goal.is_empty() {
                errors.push(format!("cron.jobs.{name}: goal is empty"));
            }
        }

        // Security validation
        if self.security.max_execution_time_secs == 0 {
            warnings.push("security.max_execution_time_secs is 0 — no timeout".into());
        }

        // Audit validation
        if self.audit.max_entries == 0 {
            warnings.push("audit.max_entries is 0 — audit will never prune".into());
        }

        // Budget validation
        if self.budget.default_window_secs == 0 {
            warnings.push("budget.default_window_secs is 0 — no time window".into());
        }

        // Gateway field-level validation
        if self.gateway.response_timeout_secs == 0 {
            errors.push("gateway.response_timeout_secs must be > 0".into());
        }

        // Engine: warn when an API key is committed to config in plaintext.
        // The auth store and env-var fallback are preferred for secret hygiene.
        if self.engine.api_key.as_ref().is_some_and(|k| !k.is_empty()) {
            warnings.push(
                "engine.api_key is set in config — prefer the oxi auth store or env var to avoid storing a secret on disk"
                    .into(),
            );
        }

        // MCP server validation: reject empty commands (would spawn a no-op).
        for (name, server) in &self.mcp.servers {
            if server.command.trim().is_empty() {
                errors.push(format!("mcp.servers.{name}: command must not be empty"));
            }
        }

        // Session validation
        if self.session.max_sessions == 0 && self.session.ttl_hours == 0 && self.session.auto_prune
        {
            warnings.push("session: auto_prune is enabled but both max_sessions and ttl_hours are 0 — nothing will be pruned".into());
        }

        // Exec validation
        if self.exec.default_timeout_secs == 0 {
            errors.push("exec.default_timeout_secs must be > 0".into());
        }
        if self.exec.max_timeout_secs == 0 {
            errors.push("exec.max_timeout_secs must be > 0".into());
        }
        if self.exec.default_timeout_secs > self.exec.max_timeout_secs {
            errors.push(format!(
                "exec.default_timeout_secs ({}) must not exceed max_timeout_secs ({})",
                self.exec.default_timeout_secs, self.exec.max_timeout_secs
            ));
        }

        // Resource monitor validation
        if self.resource_monitor.cpu_threshold > 100.0 {
            errors.push("resource_monitor.cpu_threshold must be <= 100".into());
        }
        if self.resource_monitor.memory_threshold > 100.0 {
            errors.push("resource_monitor.memory_threshold must be <= 100".into());
        }

        // Channels validation (message interfaces only)
        for name in &self.channels.enabled {
            let valid = ["cli", "telegram"];
            if !valid.contains(&name.as_str()) {
                warnings.push(format!("channels.enabled: unknown channel '{name}'"));
            }
        }
        // Warn if 'web' is listed in channels — it should be in surfaces
        if self.channels.enabled.iter().any(|c| c == "web") {
            warnings.push(
                "channels.enabled: 'web' should be listed under [surfaces], not [channels]".into(),
            );
        }
        if self.channels.enabled.iter().any(|c| c == "telegram")
            && crate::credential::CredentialStore::resolve_secret(
                crate::credential::TELEGRAM_TOKEN_STORE_KEY,
                &self.channels.telegram.bot_token_env,
            )
            .is_none()
        {
            warnings.push(telegram_token_warning(
                &self.channels.telegram.bot_token_env,
            ));
        }
        // Token Maxing (RFC-031) — only fail-closed at startup if the
        // user explicitly opted in but the entry is broken. A valid
        // empty/disabled config never errors.
        for err in self.token_maxing.validate() {
            errors.push(err);
        }

        (errors, warnings)
    }
}

/// Expand `~/` in paths to the user's home directory.
///
/// Shared utility for path expansion across the binary and kernel.
///
/// Resolution order for the home directory:
/// 1. `$HOME` environment variable (preserves existing behavior).
/// 2. `dirs::home_dir()` (works in environments where HOME is unset, e.g.
///    systemd units, containers, cron jobs).
/// 3. If neither is available, the literal path is returned unchanged so the
///    caller still gets a usable `PathBuf` rather than a panic — the failure
///    will surface as a normal "path not found" downstream.
pub fn expand_home(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return std::path::PathBuf::from(format!("{home}/{rest}"));
        }
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    std::path::PathBuf::from(path)
}

/// Warning text for an enabled telegram channel whose bot token resolves
/// from neither the credential stores nor the configured env var.
fn telegram_token_warning(env_var: &str) -> String {
    format!(
        "channels.telegram: no bot token found — store it in the Web UI \
         (Settings → Secrets) or set the {env_var} env var; the telegram \
         channel will fail to start"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brain_section_defaults_and_overrides() {
        let sec: BrainSection = toml::from_str("").unwrap();
        assert!(sec.enabled);
        assert!(sec.auto_manage);
        assert_eq!(sec.binary_path, "");
        assert_eq!(sec.space, "personal");

        let sec: BrainSection = toml::from_str(
            "enabled = false\nauto_manage = false\nbinary_path = \"/opt/oxibrain\"\n",
        )
        .unwrap();
        assert!(!sec.enabled);
        assert!(!sec.auto_manage);
        assert_eq!(sec.binary_path, "/opt/oxibrain");
    }
    #[test]
    fn telegram_api_base_defaults_and_parses() {
        let cfg: OxiosConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.channels.telegram.api_base, "https://api.telegram.org");

        let cfg: OxiosConfig =
            toml::from_str("[channels.telegram]\napi_base = \"http://127.0.0.1:8081\"\n").unwrap();
        assert_eq!(cfg.channels.telegram.api_base, "http://127.0.0.1:8081");
    }

    #[test]
    fn telegram_api_base_default_impl_matches_serde_default() {
        assert_eq!(
            TelegramChannelConfig::default().api_base,
            default_telegram_api_base()
        );
    }
    #[test]
    fn test_default_config_validates() {
        let config = OxiosConfig::default();
        let (errors, _warnings) = config.validate();
        assert!(
            errors.is_empty(),
            "Default config should have no errors: {:?}",
            errors
        );
    }

    #[test]
    fn security_config_parses_approval_section() {
        let toml = r#"
[kernel]

[security.approval]
mode = "auto-run"
allow_list = ["exec:curl", "web_search"]

[security.approval.tool_overrides]
exec = "always"
"#;
        let cfg: OxiosConfig = toml::from_str(toml).unwrap();
        assert_eq!(
            cfg.security.approval.mode,
            crate::approval::ApprovalMode::AutoRun
        );
        assert_eq!(
            cfg.security.approval.allow_list,
            vec!["exec:curl", "web_search"]
        );
        assert_eq!(
            cfg.security.approval.tool_overrides.get("exec"),
            Some(&crate::approval::ToolPolicy::Always)
        );
    }

    #[test]
    fn security_config_defaults_approval_to_manual() {
        let cfg = OxiosConfig::default();
        assert_eq!(
            cfg.security.approval.mode,
            crate::approval::ApprovalMode::Manual
        );
        assert!(cfg.security.approval.allow_list.is_empty());
    }

    #[test]
    fn test_exec_config_default_allowed_commands() {
        let config = ExecConfig::default();
        // Default is Enforced mode — empty list means NOTHING allowed.
        assert!(config.allowed_commands.is_empty());
        assert_eq!(config.allowlist_mode, AllowlistMode::Enforced);
        assert!(!config.is_binary_allowed("anything"));
        assert!(!config.is_binary_allowed("bash"));
    }

    #[test]
    fn test_exec_config_permissive_mode() {
        let config = ExecConfig {
            allowlist_mode: AllowlistMode::Permissive,
            ..Default::default()
        };
        // Permissive + empty list = all allowed
        assert!(config.is_binary_allowed("anything"));
        assert!(config.is_binary_allowed("bash"));
    }

    #[test]
    fn test_is_binary_allowed_with_allowlist() {
        let config = ExecConfig {
            allowed_commands: vec!["git".into(), "echo".into()],
            ..Default::default()
        };
        assert!(config.is_binary_allowed("git"));
        assert!(config.is_binary_allowed("echo"));
        assert!(!config.is_binary_allowed("bash"));
        assert!(!config.is_binary_allowed("rm"));
        assert!(!config.is_binary_allowed("sudo"));
    }

    #[test]
    fn test_expand_home() {
        // With HOME set.
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp/testhome".into());
        let expanded = expand_home("~/projects/test");
        assert_eq!(
            expanded.to_str().unwrap(),
            format!("{}/projects/test", home)
        );

        // Non-tilde path should pass through unchanged.
        let abs = expand_home("/absolute/path");
        assert_eq!(abs, std::path::PathBuf::from("/absolute/path"));

        // Just ~ without slash should not expand.
        let bare = expand_home("~something");
        assert_eq!(bare, std::path::PathBuf::from("~something"));
    }

    #[test]
    fn test_invalid_cron_expression() {
        let mut config = OxiosConfig::default();
        config.cron.enabled = true;
        config.cron.jobs.insert(
            "bad-job".to_string(),
            InlineCronJob {
                schedule: "not a valid cron".to_string(),
                goal: "Test goal".to_string(),
                constraints: vec![],
                acceptance_criteria: vec![],
                toolchain: "default".to_string(),
                priority: Priority::Normal,
                enabled: true,
            },
        );

        let (errors, _warnings) = config.validate();
        assert!(
            !errors.is_empty(),
            "Expected validation error for invalid cron"
        );
        let has_cron_error = errors.iter().any(|e| e.contains("invalid cron expression"));
        assert!(
            has_cron_error,
            "Expected 'invalid cron expression' error, got: {:?}",
            errors
        );
    }

    #[test]
    fn test_config_serialization_roundtrip() {
        let config = OxiosConfig::default();

        // Serialize to TOML string.
        let toml_str = toml::to_string(&config).expect("serialization should succeed");

        // Deserialize back.
        let deserialized: OxiosConfig =
            toml::from_str(&toml_str).expect("deserialization should succeed");

        // Key fields should match.
        assert_eq!(config.kernel.max_agents, deserialized.kernel.max_agents);
        assert_eq!(config.kernel.workspace, deserialized.kernel.workspace);
        assert_eq!(config.gateway.host, deserialized.gateway.host);
        assert_eq!(config.gateway.port, deserialized.gateway.port);
        assert_eq!(
            config.exec.default_timeout_secs,
            deserialized.exec.default_timeout_secs
        );
        assert_eq!(
            config.exec.max_timeout_secs,
            deserialized.exec.max_timeout_secs
        );
    }

    #[test]
    fn telegram_token_warning_names_both_sources() {
        let msg = telegram_token_warning("TELEGRAM_BOT_TOKEN");
        assert!(msg.contains("TELEGRAM_BOT_TOKEN"), "message: {msg}");
        assert!(msg.contains("Secrets"), "message: {msg}");
    }

    #[test]
    fn telegram_enabled_with_env_token_has_no_missing_token_warning() {
        // Unique env name keeps parallel tests safe; a resolvable token must
        // suppress the missing-token warning regardless of machine stores.
        unsafe { std::env::set_var("OXIOS_TEST_TG_WARN_ENV", "tok") };
        let cfg: OxiosConfig = toml::from_str(
            "[channels]\nenabled = [\"telegram\"]\n[channels.telegram]\nbot_token_env = \"OXIOS_TEST_TG_WARN_ENV\"\n",
        )
        .unwrap();
        let (_errors, warnings) = cfg.validate();
        unsafe { std::env::remove_var("OXIOS_TEST_TG_WARN_ENV") };
        assert!(
            !warnings.iter().any(|w| w.contains("no bot token")),
            "unexpected warning: {warnings:?}"
        );
    }

    #[test]
    fn test_exec_timeout_validation() {
        let mut config = OxiosConfig::default();
        // default_timeout > max_timeout should be an error.
        config.exec.default_timeout_secs = 999;
        config.exec.max_timeout_secs = 100;
        let (errors, _warnings) = config.validate();
        let has_error = errors.iter().any(|e| e.contains("must not exceed"));
        assert!(
            has_error,
            "Expected timeout ordering error, got: {:?}",
            errors
        );
    }

    #[test]
    fn test_zero_max_agents_error() {
        let mut config = OxiosConfig::default();
        config.kernel.max_agents = 0;
        let (errors, _warnings) = config.validate();
        assert!(errors.iter().any(|e| e.contains("max_agents must be > 0")));
    }

    /// Rust Default와 share/default-config.toml 간 핵심 기본값 일치 확인.
    /// TOML 템플릿은 "프로덕션 준비" 기본값을 가지며,
    /// Rust Default는 "안전한 최소" 기본값을 가질 수 있음.
    /// 핵심 스칼라 값(포트, 호스트, max_agents 등)은 반드시 일치해야 함.
    #[test]
    fn test_default_config_matches_toml() {
        let from_rust = OxiosConfig::default();

        let toml_str = include_str!("../../../share/default-config.toml");
        let from_toml: OxiosConfig =
            toml::from_str(toml_str).expect("share/default-config.toml이 유효하지 않습니다");

        // 핵심 스칼라 필드 — Rust와 TOML이 반드시 일치해야 함
        assert_eq!(
            from_rust.kernel.max_agents, from_toml.kernel.max_agents,
            "kernel.max_agents 불일치: Rust={}, TOML={}",
            from_rust.kernel.max_agents, from_toml.kernel.max_agents
        );
        assert_eq!(
            from_rust.gateway.host, from_toml.gateway.host,
            "gateway.host 불일치: Rust={}, TOML={}",
            from_rust.gateway.host, from_toml.gateway.host
        );
        assert_eq!(
            from_rust.gateway.port, from_toml.gateway.port,
            "gateway.port 불일치: Rust={}, TOML={}",
            from_rust.gateway.port, from_toml.gateway.port
        );
        assert_eq!(
            from_rust.kernel.event_bus_capacity, from_toml.kernel.event_bus_capacity,
            "kernel.event_bus_capacity 불일치"
        );
        assert_eq!(
            from_rust.memory.consolidation.preset, from_toml.memory.consolidation.preset,
            "memory.consolidation.preset 불일치"
        );

        // TOML 템플릿이 파싱 가능한지 확인
        let (_, warnings) = from_toml.validate();
        for w in &warnings {
            eprintln!("default-config.toml 경고: {}", w);
        }
    }

    /// `gateway.expose_api_docs` is gated to loopback binds for safety.
    /// Verifies all four cases: opt-out, opt-in + public, opt-in + loopback.
    #[test]
    fn test_gateway_should_expose_api_docs() {
        // Default: opt-out — never expose.
        let cfg = GatewayConfig::default();
        assert!(!cfg.should_expose_api_docs());

        // Opt-in + public bind (0.0.0.0) — still NOT exposed.
        let cfg = GatewayConfig {
            host: "0.0.0.0".into(),
            port: 4200,
            expose_api_docs: true,
            ..Default::default()
        };
        assert!(
            !cfg.should_expose_api_docs(),
            "public bind must not expose api docs even when opt-in is true"
        );

        // Opt-in + loopback (127.0.0.1) — exposed.
        let cfg = GatewayConfig {
            host: "127.0.0.1".into(),
            port: 4200,
            expose_api_docs: true,
            ..Default::default()
        };
        assert!(cfg.should_expose_api_docs());

        // Opt-in + ::1 — exposed.
        let cfg = GatewayConfig {
            host: "::1".into(),
            port: 4200,
            expose_api_docs: true,
            ..Default::default()
        };
        assert!(cfg.should_expose_api_docs());

        // Opt-in + "localhost" — exposed.
        let cfg = GatewayConfig {
            host: "localhost".into(),
            port: 4200,
            expose_api_docs: true,
            ..Default::default()
        };
        assert!(cfg.should_expose_api_docs());
    }
    #[test]
    fn test_remote_config_default_disabled() {
        let cfg = OxiosConfig::default();
        assert!(!cfg.remote.enabled);
        assert_eq!(cfg.remote.port, 6768);
        assert_eq!(cfg.remote.bind_address, "127.0.0.1");
        assert!(cfg.remote.pairing_address.is_none());
    }

    #[test]
    fn test_remote_config_parse() {
        let toml = "[remote]\nenabled = true\nport = 7000\nbind_address = \"0.0.0.0\"\npairing_address = \"100.64.1.20\"\n";
        let cfg: OxiosConfig = toml::from_str(toml).unwrap();
        assert!(cfg.remote.enabled);
        assert_eq!(cfg.remote.port, 7000);
        assert_eq!(cfg.remote.bind_address, "0.0.0.0");
        assert_eq!(cfg.remote.pairing_address.as_deref(), Some("100.64.1.20"));
    }

    #[test]
    fn test_remote_config_default_bind_address() {
        // When no [remote] section is present, bind_address falls back to loopback.
        let cfg: OxiosConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.remote.bind_address, "127.0.0.1");
        assert!(!cfg.remote.enabled);
        assert_eq!(cfg.remote.port, 6768);
    }
    #[test]
    fn test_router_config_deserialization() {
        let toml_str = r#"
[engine.router]
enabled = true
default_profile = "auto"

[engine.router.scoring]
structural = 0.25
behavioral = 0.20
context = 0.15
vision = 0.10
message = 0.30

[engine.router.profiles.auto.tiers]
fast = { model = "anthropic/claude-haiku-4-20250514" }
balanced = { model = "anthropic/claude-sonnet-4-20250514" }
strong = { model = "anthropic/claude-opus-4-20250514" }
"#;

        let config: OxiosConfig = toml::from_str(toml_str).unwrap();
        let router = config
            .engine
            .router
            .expect("router config should be present");
        assert!(router.enabled);
        assert_eq!(router.default_profile, "auto");

        let auto = router
            .profiles
            .get("auto")
            .expect("auto profile should exist");
        let fast = auto.tiers.fast.as_ref().expect("fast tier should exist");
        assert_eq!(fast.model, "anthropic/claude-haiku-4-20250514");
        let strong = auto
            .tiers
            .strong
            .as_ref()
            .expect("strong tier should exist");
        assert_eq!(strong.model, "anthropic/claude-opus-4-20250514");
    }

    #[test]
    fn test_router_validate_disabled_is_ok() {
        // The router being off is always valid.
        let cfg = RouterConfig::default();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_router_validate_missing_default_profile() {
        // Enabled router, but `default_profile = "auto"` doesn't resolve to
        // any defined profile.
        let cfg = RouterConfig {
            enabled: true,
            default_profile: "auto".into(),
            profiles: std::collections::HashMap::new(),
            ..Default::default()
        };
        let err = cfg
            .validate()
            .expect_err("must fail when default_profile missing");
        assert!(
            err.contains("default_profile 'auto' is not defined"),
            "got: {err}"
        );
    }

    #[test]
    fn test_router_validate_empty_profile() {
        // Enabled router with a profile that has no tiers configured.
        let mut profiles = std::collections::HashMap::new();
        profiles.insert(
            "auto".into(),
            RouterProfileConfig {
                tiers: RouterTiersConfig::default(),
            },
        );
        let cfg = RouterConfig {
            enabled: true,
            default_profile: "auto".into(),
            profiles,
            ..Default::default()
        };
        let err = cfg.validate().expect_err("must fail on empty profile");
        assert!(err.contains("no configured tiers"), "got: {err}");
    }
    #[test]
    fn test_router_validate_with_one_tier_is_ok() {
        // The minimum to be valid: the default profile exists and has at
        // least one tier.
        let mut profiles = std::collections::HashMap::new();
        let tiers = RouterTiersConfig {
            balanced: Some(RouterTierConfig {
                model: "anthropic/claude-sonnet-4-20250514".into(),
                fallbacks: vec![],
                thinking: None,
            }),
            ..Default::default()
        };
        profiles.insert("auto".into(), RouterProfileConfig { tiers });
        let cfg = RouterConfig {
            enabled: true,
            default_profile: "auto".into(),
            profiles,
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn knowledge_root_resolution_order() {
        // Tier 1: explicit `kernel.knowledge_root` wins OVER a present
        // `[vault].path` in the oxi config — proof of precedence, not just
        // "explicit vs nothing". A reversed-priority implementation must
        // NOT pass.
        let dir = tempdir();
        let oxi_config = dir.join("oxi-config.toml");
        std::fs::write(&oxi_config, "[vault]\npath = \"~/should-not-win\"\n").unwrap();
        unsafe {
            std::env::set_var("OXIOS_OXI_CONFIG_PATH", &oxi_config);
        }
        let explicit = KernelConfig {
            knowledge_root: Some("~/explicit-vault".into()),
            ..KernelConfig::default_with_workspace_for_test("~/ignored")
        };
        assert_eq!(
            explicit.resolved_knowledge_root(),
            expand_home("~/explicit-vault"),
        );

        // Tier 2: explicit absent, `[vault].path` present →
        // `expand_home("~/from-ecosystem")` is returned.
        std::fs::write(&oxi_config, "[vault]\npath = \"~/from-ecosystem\"\n").unwrap();
        unsafe {
            std::env::set_var("OXIOS_OXI_CONFIG_PATH", &oxi_config);
        }
        let cfg = KernelConfig::default_with_workspace_for_test("~/ignored");
        let resolved = cfg.resolved_knowledge_root();
        assert_eq!(resolved, expand_home("~/from-ecosystem"));

        // Tier 3: explicit absent AND oxi config absent → fallback
        // `~/.oxi/vault`. Hermetic: the seam points at a NONEXISTENT path
        // inside the tempdir so no workstation's real `~/.oxi/config.toml`
        // can leak into the assertion.
        let nonexistent = dir.join("oxi-config-absent.toml");
        // (Path does not exist — read returns NotFound → fallback.)
        unsafe {
            std::env::set_var("OXIOS_OXI_CONFIG_PATH", &nonexistent);
        }
        let cfg = KernelConfig::default_with_workspace_for_test("~/ignored");
        let resolved = cfg.resolved_knowledge_root();
        unsafe {
            std::env::remove_var("OXIOS_OXI_CONFIG_PATH");
        }
        assert_eq!(resolved, expand_home("~/.oxi/vault"));
    }

    // Test-only constructors/helpers for the knowledge_root test above.
    impl KernelConfig {
        fn default_with_workspace_for_test(workspace: &str) -> Self {
            Self {
                workspace: workspace.into(),
                event_bus_capacity: default_event_bus_capacity(),
                max_agents: default_max_agents(),
                knowledge_root: None,
            }
        }
    }

    /// Unique OS temp dir per call; auto-removed on drop so the suite
    /// never accumulates cruft in `$TMPDIR`.
    fn tempdir() -> TempDir {
        let base =
            std::env::temp_dir().join(format!("oxios-kernel-config-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).expect("tempdir create");
        TempDir(base)
    }

    struct TempDir(std::path::PathBuf);

    impl std::ops::Deref for TempDir {
        type Target = std::path::Path;
        fn deref(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
