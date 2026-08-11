//! Oxios kernel: supervisor, event bus, state store.
//!
//! The kernel is the core of the Oxios Agent OS. Everything passes
//! through here: agent lifecycle, inter-agent communication, and
//! persistent state management.

#![allow(missing_docs)]
// `.unwrap()` in tests is idiomatic (workspace convention); suppressed only
// under `cfg(test)` so production code remains linted by the `--lib` clippy pass.
#![cfg_attr(test, allow(clippy::unwrap_used))]

// ─── Lifecycle ──────────────────────────────────────────────────────
// Agent 생성, 실행, 종료. OS의 init + process management.
pub mod agent_group;
pub mod agent_lifecycle;
pub mod agent_runtime;
pub mod daemon;
pub mod readiness;
pub mod streaming_sink;
pub mod subagent_runner;
pub mod supervisor;

// ─── Agent History Log ──────────────────────────────────────────────
// 에이전트 실행 기록 — SQLite + JSON dual storage.
pub mod agent_log_db;

// ─── Orchestration ──────────────────────────────────────────────────
// 작업 조율, 스케줄링, 예산 관리.
pub mod budget;
pub mod cron;
pub mod orchestrator;
// ─── Resilience (RFC-029) ──────────────────────────────────────────────
// Failure classification + (P2) recovery coordination.
pub mod resilience;

// ─── Security ───────────────────────────────────────────────────────
pub mod approval;
// 접근 제어, 인증, 권한, 감사.
pub mod access_manager;
pub mod auth;
pub mod capability;
pub mod credential;

// ─── Audit Persistence ───────────────────────────────────────────────
//
// `audit_persistence` wires `oxicode_sdk::observability::AuditPersistence`
// to the kernel's filesystem-based `StateStore`. The trail itself
// lives in `oxicode_sdk::observability::AuditTrail` and is re-exported
// below — RFC-014 Phase F.
mod audit_persistence;

// ─── Autonomous Persistence ─────────────────────────────────────────
// RFC-016: Post-execution hook for auto-saving knowledge and memory.
pub mod knowledge_dream;
pub mod persistence_hook;

// ─── Communication ──────────────────────────────────────────────────
// 이벤트, 메시징, 외부 프로토콜, 멀티 에이전트 조정.
pub mod a2a;
pub mod coordination;
pub mod email;
pub mod event_bus;
pub mod mcp;

// ─── Intelligence ───────────────────────────────────────────────────
// 메모리, 임베딩, 페르소나, 온보딩.
pub mod embedding;
pub mod memory;
pub mod onboarding;
pub mod persona;

// ─── Tools & Skills ───────────────────────────────────────────────
pub mod hook_runner;
pub mod host_tools;
pub mod skill;
pub mod token_maxing;
pub mod tools;
#[cfg(feature = "wasm-sandbox")]
pub mod wasm_sandbox;
pub mod workers;

// ─── State & Config ─────────────────────────────────────────────────
// 영속 상태, 설정, 백업, 리소스 모니터링.
pub mod backup;
pub mod compression;
pub mod config;
pub mod git_layer;
pub mod mount;
pub mod project;
pub mod resource_monitor;
pub mod session_context;
pub mod state_store;
pub mod task;

// ─── Infrastructure ─────────────────────────────────────────────────
// 엔진, 에러, 타입, 메트릭, 텔레메트리, 옵저버빌리티.
pub mod asset_store;
pub mod engine;
pub mod error;
pub mod image_gen;
pub mod metrics;
pub mod observability;
pub mod types;

// ─── API Surface ────────────────────────────────────────────────────
// 외부에 노출하는 typed facade.
pub mod kernel_handle;

// ─────────────────────────────────────────────────────────────────────
// Re-exports (같은 섹션 순서)
// ─────────────────────────────────────────────────────────────────────

// ─── Lifecycle ──────────────────────────────────────────────────────
pub use agent_group::{OxiosAgentGroup, OxiosAgentGroupStatus, OxiosGroupAgent};
pub use agent_lifecycle::AgentLifecycleManager;
pub use agent_runtime::AgentRuntime;
pub use daemon::{DaemonManager, DaemonStatus};
pub use persistence_hook::PersistenceHook;
pub use readiness::{ReadinessGate, SubsystemState};
pub use supervisor::{BasicSupervisor, Supervisor};

// ─── Orchestration ──────────────────────────────────────────────────
pub use budget::{
    BudgetExceeded, BudgetInfo, BudgetKind, BudgetLimit, BudgetManager, FullBudgetInfo,
};
// Circuit breaker — delegates to oxicode-sdk
pub use cron::{CronJob, CronJobResult, CronJobUpdate, CronScheduler, JobSource};
pub use orchestrator::{AgentRole, OrchestrationResult, Orchestrator, SubTask};
// CircuitBreaker removed — use oxicode_sdk::ProviderCircuitBreaker directly (#11).
pub use types::Priority;

// ─── Security ───────────────────────────────────────────────────────
pub use access_manager::{
    AccessManager, Action, AgentPermissions, ApprovalStatus, PendingApproval, RbacAuditEntry,
    RbacManager, RbacPolicy, Role, Subject,
};
// AuditTrail types are re-exported from oxicode-sdk (Phase F: removed
// 1134-line duplicate). `AgentId as AuditAgentId` preserves the
// historical kernel-level type alias.
pub use auth::{AuthManager, KeyMeta};
pub use capability::template::CapabilityTemplate;
pub use capability::{CSpace, Capability, CapabilityId, Issuer, ResourceRef, Rights};
pub use credential::CredentialStore;
pub use oxicode_sdk::observability::audit_trail::AgentId as AuditAgentId;
pub use oxicode_sdk::observability::{
    AuditAction, AuditError, AuditPersistence, AuditTrail, HashDigest, TrailEntry,
};

// ─── Communication ──────────────────────────────────────────────────
pub use a2a::{
    A2ACircuitBreaker, A2AMessage, A2AProtocol, A2ARequest, A2AResponse, AgentCard,
    AgentCardRegistry, CircuitState, DelegationHandler, TaskPriority, TaskSpec,
};
pub use email::{SendReceipt, SmtpClient};
pub use event_bus::{EventBus, KernelEvent};
pub use mcp::{
    McpBridge, McpCapabilities, McpServer, McpTool, McpToolCallResult as CallToolResult,
};

// ─── Intelligence ───────────────────────────────────────────────────
pub use embedding::{EmbeddingProvider, EmbeddingVector, TfIdfEmbeddingProvider};

// ─── GGUF Embedding (RFC-012) ──────────────────────────────────────
#[cfg(feature = "embedding-gguf")]
pub use embedding::gguf::{EmbeddingDimension, GgufEmbeddingProvider, GgufModelLoader};

pub use memory::auto_memory_bridge::{
    AutoMemoryBridge, ExportResult, GuidancePattern, ImportResult, InsightCategory, MemoryInsight,
    SyncDirection, SyncResult,
};
pub use memory::{
    DreamCheckpoint, DreamConfig, DreamProcess, DreamReport, HnswIndex, HnswMemoryIndex,
    MemoryManager, ProactiveRecall, RecallTiming, SemanticHit,
};
pub use memory::{MemoryEntry, MemoryTier, MemoryType, ProtectionLevel, TextVector, content_hash};
pub use oxios_memory::memory::flash_attention::{
    BenchmarkResult as AttentionBenchmarkResult, FlashAttention, FlashAttentionConfig,
    MemoryEstimate,
};
pub use oxios_memory::memory::{
    HyperbolicConfig, HyperbolicEmbedding, batch_euclidean_to_poincare, euclidean_to_poincare,
    hyperbolic_distance, mobius_add, mobius_scalar_mul,
};
pub use oxios_memory::{
    AutoClassifier, AutoProtector, CompactionTree, CurationCandidate, CurationReport, DecayEngine,
    EmbeddingCache, HistoricalPeriod, MemoryBudget, MemoryGraph, MemoryMapEntry, MemoryNeighbor,
    RootEntry, RootIndex, SonaEngine, TopicEntry,
};

// ─── Memory core types (extracted to oxios-memory, RFC-018 b.1) ───
// Re-exported here for back-compat — existing `use oxios_kernel::chunk_fixed;`
// and friends continue to work without code changes.
pub use oxios_memory::{
    ChunkConfig, TextChunk, chunk_fixed, chunk_paragraphs, cosine_similarity_f32, l2_normalize_f32,
    l2_normalize_f64,
};

// ─── SQLite Memory (RFC-012) ────────────────────────────────────────
#[cfg(feature = "sqlite-memory")]
pub use oxios_memory::memory::sqlite::SqliteMemoryStore;
#[cfg(feature = "sqlite-memory")]
pub use oxios_memory::memory::sqlite::cache::{self as sqlite_cache};
#[cfg(feature = "sqlite-memory")]
pub use oxios_memory::memory::sqlite::migration::{self as sqlite_migration, MigrationReport};
#[cfg(feature = "sqlite-memory")]
pub use oxios_memory::memory::sqlite::search::{
    Bm25Hit, RankedMemory, VectorHit, reciprocal_rank_fusion,
};
#[cfg(feature = "sqlite-memory")]
pub use oxios_memory::memory::sqlite::{MemoryDatabase, bytes_to_f32_slice, f32_slice_to_bytes};
pub use persona::{Persona, PersonaManager, PersonaStore, default_personas};

// ─── Tools & Skills ────────────────────────────────────────────────
pub use hook_runner::CommandHookRunner;
pub use oxicode_sdk::ports::hooks::HookSpec;
pub use skill::clawhub::{
    ClawHubClient, ClawHubInstaller, ClawHubLockEntry, ClawHubLockfile, ClawHubOrigin,
    ClawHubSearchResult, ClawHubSkillDetail, ClawHubSkillMeta, ClawHubVersion, DownloadedArchive,
    InstallResult, UpdateAvailable, UpdateResult,
};
pub use skill::skills_sh::{
    SkillsShAuditEntry, SkillsShAuditResponse, SkillsShClient, SkillsShFile, SkillsShInstallResult,
    SkillsShInstaller, SkillsShOrigin, SkillsShSearchResponse, SkillsShSkill, SkillsShSkillDetail,
};
pub use skill::{
    InstallKind, Requirements, RequirementsCheck, Skill, SkillConfig, SkillEntry, SkillFormat,
    SkillInstallSpec, SkillInvocationPolicy, SkillManager, SkillMeta, SkillMetadata, SkillRef,
    SkillSnapshot, SkillSource, SkillState, SkillStatus,
};
pub use tools::ToolMeta;
pub use tools::tool_types::{ArgumentDef, ToolDef};
pub use tools::{ExecTool, KnowledgeTool};
#[cfg(feature = "wasm-sandbox")]
pub use wasm_sandbox::{ResourceKind, WasmConfig, WasmError, WasmSandbox};
// Token-maxing (RFC-031): self-tracker + QuotaTracker + maxer/planner/session.
pub use kernel_handle::TokenMaxingApi;
// Compression — LLM session summaries (LobeHub port).
pub use compression::CompressionService;
pub use kernel_handle::CompressionApi;
pub use token_maxing::{
    Availability, CooldownRecord, ProviderBudget, ProviderSnapshot, ProviderState, QuotaTracker,
    QuotaTrackerSnapshot, RecalibrationOutcome, RecalibrationRecord, ReserveError,
    SUBSCRIPTION_BILLING_MODEL, TokenMaxingConfig, TokenMaxingProviderConfig,
};
pub use token_maxing::{
    MaxerStatus, MaxingStart, MaxingWindow, PlannedTask, TokenMaxer, TokenMaxingSession,
    WorkPlanner,
};

// ─── State & Config ─────────────────────────────────────────────────
pub use backup::{BackupManifest, BackupSection};
pub use config::{
    BrowserConfig, ChannelsConfig, CronConfig, DaemonConfig, EmailConfig, EmbeddingConfig,
    EngineConfig, ExecConfig, ExecMode, GitConfig, InlineCronJob, LoggingConfig, MarketplaceConfig,
    McpConfig, McpServerDef, MemoryConfig, MountsConfig, OrchestratorConfig, OxiosConfig,
    PersonaConfig, SkillsShConfig, SqliteMemoryConfig,
};
pub use git_layer::{
    CommitContext, CommitDiff, CommitInfo, DiffKind, DiffStats, FileDiff, GitLayer, LogEntry,
};
pub use mount::{
    DetectionResult as MountDetectionResult, Mount, MountId, MountMeta, MountSource,
    PromotionConfig, detect_mounts,
};
#[cfg(feature = "sqlite-memory")]
pub use mount::{MountManager, MountManagerError};
pub use project::{
    ConversationBuffer, ConversationTurn, DetectionResult, Project, ProjectId, ProjectSource,
    detect_project, extract_path, find_by_id, find_by_name,
};
#[cfg(feature = "sqlite-memory")]
pub use project::{ProjectManager, ProjectManagerError};
pub use resource_monitor::{OverloadThreshold, ResourceMonitor, ResourceSnapshot};
pub use state_store::{
    AgentResponse, PruneConfig, PruneThrottle, Session, SessionId, SessionSummary, StateStore,
};

// ─── Infrastructure ─────────────────────────────────────────────────
pub use asset_store::{Asset, AssetFilter, AssetSource, AssetStore, AssetStoreError, asset_type};
pub use engine::{EngineHandle, EngineProvider, OxiosEngine, OxiosEngineBuilder};
pub use error::{HttpStatus, KernelError, KernelResult};
pub use metrics::{get_metrics, register_builtin_metrics, registry};
pub use observability::{
    AuditEntry as SdkAuditEntry, AuditFilter, CostSnapshot, CostTracker, Span, SpanGuard, SpanKind,
    TokenUsage, Tracer as SdkTracer, audit_log, cost_tracker, tracer,
};
pub use types::{AgentId, AgentInfo, AgentStatus, ToolCallRecord};

// ─── API Surface ────────────────────────────────────────────────────
pub use host_tools::{CredentialStatus, DetectedTool, ToolSource};
pub use kernel_handle::KernelHandle;
pub use kernel_handle::MarketplaceApi;
pub use kernel_handle::{
    A2aApi, AgentApi, BrowserApi, CalendarApi, CopilotResponse, EmailApi, EngineApi,
    EngineConfigResponse, ExecApi, ExtensionApi, FallbackEvent, HostToolsApi, InfraApi,
    InputModality as EngineInputModality, KnowledgeContext, KnowledgeLens, KnowledgeNote, McpApi,
    MemoApi, MemoryNote, ModelInfo, MountApi, MountInfo, PersonaApi, ProjectApi, ProjectInfo,
    ProviderCategory, ProviderInfo, RoutingConfigSnapshot, RoutingStats, RoutingStatsSnapshot,
    RoutingUpdate, SecurityApi, SharedExecConfig, StateApi, TimelineApi, ValidateKeyResult,
};
#[cfg(feature = "browser")]
pub use kernel_handle::{ScreenshotEngine, ScreenshotViewport};
pub use session_context::SessionContext;

// ─── oxicode-sdk re-exports ─────────────────────────────────────────────
//
// Removed: dead re-exports (#11). Consumers should depend on oxicode-sdk
// directly and use `oxicode_sdk::` instead of going through oxios-kernel.
