//! Oxios Memory — tiered agent memory extracted from `oxios-kernel`.
//!
//! ## Status (RFC-018)
//!
//! This crate holds the memory subsystem extracted from `oxios-kernel`:
//!
//! - **b.1**: `chunking`, `normalizer`, `hyperbolic` (math/text utilities)
//! - **b.2**: `embedding` (TF-IDF + GGUF dense vectors)
//! - **b.3**: `root_index`, `quota`
//! - **b.4**: `decay`, `auto_classify`, `auto_protect`
//! - **b.5**: `compaction`, `flash_attention`, `graph`, `embedding_cache`, `embedding_viz`
//! - **b.6**: `MemoryStorage` trait + `StateStore` impl
//! - **b.7**: `MemoryManager` move
//! - **b.8**: SQLite backend
//! - **b.9**: `migrate`, `dream`, `auto_memory_bridge`
//!
//! `oxios-kernel` depends on this crate (not the other way around) for
//! all memory types and modules.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use oxios_memory::MemoryEntry;
//! use oxios_memory::MemoryType;
//! use oxios_memory::chunk_fixed;
//! use oxios_memory::HyperbolicEmbedding;
//! use oxios_memory::cosine_similarity_f32;
//! ```

// `.unwrap()` in tests is idiomatic (workspace convention); suppressed only
// under `cfg(test)` so production code remains linted by the `--lib` clippy pass.
#![cfg_attr(test, allow(clippy::unwrap_used))]

// ─── Memory subsystem modules (extracted from oxios-kernel) ──
pub mod memory;

// Re-export storage traits
pub use crate::memory::{
    MarkdownSource, MemoryBackend, MemoryGit, MemoryStorage, MemoryStorageExt, NoteEntry,
};

// Re-export core types (RFC-018)
pub use crate::memory::types::{
    MemoryEntry, MemoryTier, MemoryType, ProtectionLevel, TextVector, content_hash, dedup_by_id,
    extract_keywords,
};

// Re-export extracted modules (b.1 — chunking/normalizer/hyperbolic)
pub use crate::memory::{
    ChunkConfig, HyperbolicConfig, HyperbolicEmbedding, TextChunk, chunk_fixed, chunk_paragraphs,
    cosine_similarity_f32, l2_normalize_f32, l2_normalize_f64,
};

// Re-export lifecycle modules (b.3-b.5)
pub use crate::memory::{
    AutoClassifier, AutoProtector, CacheStats, CompactionTree, CurationCandidate, CurationReport,
    DecayEngine, EmbeddingCache, FlashAttention, FlashAttentionConfig, HistoricalPeriod,
    MemoryBudget, MemoryEstimate, MemoryGraph, MemoryMapEntry, MemoryNeighbor, RootEntry,
    RootIndex, TopicEntry,
};

// Re-export HNSW
pub use crate::memory::hnsw::HnswIndex;
pub use crate::memory::hnsw_memory_index::{HnswMemoryIndex, SemanticHit};

// Re-export SONA pattern engine
pub use crate::memory::sona::{
    LearnedPattern, SonaEngine, SonaMode, Trajectory, TrajectoryStep, Verdict,
};

// Re-export MemoryManager (RFC-018 Phase 4)
pub use crate::memory::manager::MemoryManager;

// Re-export Dream consolidation process (RFC-018 Phase 5)
pub use crate::memory::dream::{DreamCheckpoint, DreamConfig, DreamProcess, DreamReport};

// Re-export Proactive recall (RFC-018 Phase 5)
pub use crate::memory::proactive::{ProactiveRecall, RecallTiming};
