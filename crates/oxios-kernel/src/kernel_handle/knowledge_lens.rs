//! KnowledgeLens — semantic search overlay for the markdown knowledge base.
//!
//! Wraps a [`KnowledgeBase`] and adds brain-backed memory recall (RFC-047).
//! Provides `recall_for_context()` for injecting relevant knowledge into
//! agent context windows.
//!
//! **RFC-003: Knowledge Base Independent Separation**
//! - Semantic search lives in the kernel (AI layer), not oxios-markdown
//! - Vault ingestion (the daemon watching the markdown vault) is the
//!   oxibrain daemon's job — `BrainConnection::register_vault_source` on
//!   boot, registered once. The lens is a read-side overlay only; the
//!   previous file-change → `remember` chain (index_to_brain) was removed
//!   in T17 (vault unification) so a single ingestion path owns it.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::brain::BrainConnection;

/// Knowledge context injected into agent prompts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnowledgeContext {
    /// Relevant knowledge notes for the query.
    pub notes: Vec<KnowledgeNote>,
    /// Memory entries from agent memory.
    pub memories: Vec<MemoryNote>,
    /// Number of HNSW index entries used.
    pub index_entries_used: usize,
}

/// A knowledge note extracted from the markdown knowledge base.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeNote {
    /// Relative path.
    pub path: String,
    /// Display name.
    pub name: String,
    /// Content snippet.
    pub content: String,
    /// Number of backlinks.
    pub backlink_count: usize,
}

/// A memory entry from the agent's memory system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryNote {
    /// Memory ID.
    pub id: String,
    /// Source tag (e.g. "memory:agent", "session:...").
    pub source: String,
    /// Content snippet.
    pub content: String,
    /// Importance score (0-1).
    pub importance: f32,
}

/// Copilot response (AI-powered chat about the knowledge base).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotResponse {
    /// AI-generated answer.
    pub content: String,
    /// Note paths referenced in the response.
    pub referenced_notes: Vec<String>,
    /// Memory IDs referenced in the response.
    pub referenced_memories: Vec<String>,
}

/// KnowledgeLens — semantic overlay over KnowledgeBase.
///
/// Read-side: combines markdown note search (via `KnowledgeBase`) with
/// brain recall (via the daemon). Vault ingestion is the oxibrain
/// daemon's job (T17 single ingestion path); the lens never writes to the
/// brain on file-change. The brain connection is optional so tests and
/// preliminary handles can build the lens before the daemon connects.
pub struct KnowledgeLens {
    /// The underlying knowledge base.
    kb: Arc<oxios_markdown::KnowledgeBase>,
    /// Brain daemon connection (RFC-047) — `None` when unattached.
    brain: Option<Arc<BrainConnection>>,
    /// Tracks which files were written by agents.
    agent_writes: Arc<RwLock<HashSet<String>>>,
}

impl std::fmt::Debug for KnowledgeLens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KnowledgeLens").finish()
    }
}

impl KnowledgeLens {
    /// Create a new KnowledgeLens wrapping the given knowledge base.
    ///
    /// No file-change subscription: vault ingestion is the daemon's job
    /// (T17 single ingestion path). The lens is purely read-side.
    pub fn new(
        kb: Arc<oxios_markdown::KnowledgeBase>,
        brain: Option<Arc<BrainConnection>>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            kb,
            brain,
            agent_writes: Arc::new(RwLock::new(HashSet::new())),
        })
    }

    /// Get the root path of the knowledge base.
    pub fn root(&self) -> PathBuf {
        self.kb.root()
    }

    /// Get the underlying knowledge base (read-only access).
    pub fn knowledge_base(&self) -> &Arc<oxios_markdown::KnowledgeBase> {
        &self.kb
    }

    /// Mark a file as having been written by an agent.
    pub fn mark_agent_write(&self, path: &str) {
        self.agent_writes.write().insert(path.to_string());
    }

    /// Check if a file was written by an agent.
    pub fn is_agent_write(&self, path: &str) -> bool {
        self.agent_writes.read().contains(path)
    }

    /// Clear the agent-write marker for a file.
    pub fn clear_agent_write(&self, path: &str) {
        self.agent_writes.write().remove(path);
    }

    /// Recall relevant knowledge for a given context/query.
    ///
    /// Combines markdown note search (via KnowledgeBase) with brain recall
    /// (via the daemon). Returns notes ranked by relevance.
    pub async fn recall_for_context(&self, query: &str, limit: usize) -> Result<KnowledgeContext> {
        // Recall relevant memory from the brain (assembled context text).
        let memories: Vec<MemoryNote> = if let Some(brain) = &self.brain {
            brain
                .recall(query, 2000)
                .await
                .map(|text| {
                    vec![MemoryNote {
                        id: "recall".to_string(),
                        source: "brain".to_string(),
                        content: text.chars().take(300).collect(),
                        importance: 1.0,
                    }]
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let memories_len = memories.len();

        // Search knowledge notes
        let note_hits = self.kb.search(query, limit)?;

        let notes: Vec<KnowledgeNote> = note_hits
            .into_iter()
            .map(|h| {
                let content = self
                    .kb
                    .note_read(&h.path)
                    .ok()
                    .flatten()
                    .map(|c| c.chars().take(500).collect::<String>())
                    .unwrap_or_default();
                KnowledgeNote {
                    path: h.path,
                    name: h.name,
                    content,
                    backlink_count: h.backlink_count,
                }
            })
            .collect();

        Ok(KnowledgeContext {
            notes,
            memories,
            index_entries_used: memories_len,
        })
    }

    /// Copilot chat — AI-powered question answering about the knowledge base.
    ///
    /// This method is async (uses `provider.stream()` which is Send).
    #[allow(clippy::unused_async)]
    pub async fn copilot_chat(
        &self,
        engine_handle: Arc<crate::engine::EngineHandle>,
        question: &str,
        context_path: Option<&str>,
    ) -> Result<CopilotResponse> {
        let mut context_parts = Vec::new();
        let mut referenced_notes = Vec::new();

        // 1. Current file context
        if let Some(path) = context_path
            && let Ok(Some(content)) = self.kb.note_read(path)
        {
            let snippet: String = content.chars().take(2000).collect();
            context_parts.push(format!("## Current: {path}\n\n{snippet}"));
            referenced_notes.push(path.to_string());
        }

        // 2. Related notes
        let hits = self.kb.search(question, 5).unwrap_or_default();
        for hit in &hits {
            if referenced_notes.contains(&hit.path) {
                continue;
            }
            if let Ok(Some(content)) = self.kb.note_read(&hit.path) {
                let snippet: String = content.chars().take(500).collect();
                context_parts.push(format!("## Related: {}\n\n{}", hit.path, snippet));
                referenced_notes.push(hit.path.clone());
            }
        }

        // 3. Memory context
        let mut referenced_memories = Vec::new();
        if let Some(brain) = &self.brain
            && let Some(value) = brain.search(question, "hybrid", 3).await
            && let Some(items) = value.get("items").and_then(|i| i.as_array())
        {
            for item in items {
                let kind = item
                    .pointer("/target/kind")
                    .and_then(|k| k.as_str())
                    .unwrap_or("memory");
                let id = item
                    .pointer("/target/id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("?");
                context_parts.push(format!("## Memory [{kind}]: {id}"));
                referenced_memories.push(id.to_string());
            }
        }

        // 4. AI call
        let system_prompt = format!(
            "You are a knowledge assistant embedded in a markdown note-taking system.\n\
             Answer questions about the user's notes using ONLY the provided context.\n\n\
             ## Rules\n\
             - Only answer based on the context below. If the context doesn't contain\n\
               the answer, say \"I couldn't find relevant notes on that topic.\"\n\
             - Cite which notes you're referencing by name.\n\
             - Be concise — the user is in an editor, not a chat room.\n\n\
             ## Available Notes\n\n{}",
            context_parts.join("\n\n")
        );

        // Resolve the live default model + a cached provider through the same
        // single source of truth the rest of the kernel uses (interview,
        // execute, persistence). Honors hot-swaps and the user's configured
        // provider/key — fixes the old hardcoded anthropic engine bug.
        let resolved = engine_handle
            .resolve_default()
            .map_err(|e| anyhow::anyhow!("Model/provider: {e}"))?;

        let mut ctx = oxicode_sdk::Context::new();
        ctx.set_system_prompt(&system_prompt);
        ctx.add_message(oxicode_sdk::Message::User(oxicode_sdk::UserMessage::new(
            question,
        )));

        let stream = resolved
            .provider
            .stream(&resolved.model, &ctx, None)
            .await
            .map_err(|e| anyhow::anyhow!("Stream: {e}"))?;
        let mut text = String::new();
        use futures::StreamExt;
        let mut pinned = std::pin::pin!(stream);
        while let Some(event) = pinned.next().await {
            match event {
                oxicode_sdk::ProviderEvent::TextDelta { delta, .. } => text.push_str(&delta),
                oxicode_sdk::ProviderEvent::Done { .. } => break,
                oxicode_sdk::ProviderEvent::Error { error, .. } => {
                    return Err(anyhow::anyhow!("AI: {error:?}"));
                }
                _ => {}
            }
        }

        Ok(CopilotResponse {
            content: text,
            referenced_notes,
            referenced_memories,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_knowledge_context_default() {
        let ctx = KnowledgeContext::default();
        assert!(ctx.notes.is_empty());
        assert!(ctx.memories.is_empty());
        assert_eq!(ctx.index_entries_used, 0);
    }

    #[test]
    fn test_knowledge_note_serialization() {
        let note = KnowledgeNote {
            path: "notes/Rust.md".to_string(),
            name: "Rust".to_string(),
            content: "Rust is a systems language".to_string(),
            backlink_count: 3,
        };
        let json = serde_json::to_string(&note).unwrap();
        let restored: KnowledgeNote = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.path, "notes/Rust.md");
        assert_eq!(restored.backlink_count, 3);
    }

    #[test]
    fn test_memory_note_serialization() {
        let note = MemoryNote {
            id: "mem-123".to_string(),
            source: "session:abc".to_string(),
            content: "User prefers dark mode".to_string(),
            importance: 0.85,
        };
        let json = serde_json::to_string(&note).unwrap();
        let restored: MemoryNote = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.id, "mem-123");
        assert!((restored.importance - 0.85).abs() < 0.01);
    }

    #[test]
    fn test_copilot_response_serialization() {
        let resp = CopilotResponse {
            content: "The answer is 42".to_string(),
            referenced_notes: vec!["notes/answer.md".to_string()],
            referenced_memories: vec!["mem-1".to_string()],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let restored: CopilotResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.content, "The answer is 42");
        assert_eq!(restored.referenced_notes.len(), 1);
        assert_eq!(restored.referenced_memories.len(), 1);
    }

    #[test]
    fn test_knowledge_context_with_data() {
        let ctx = KnowledgeContext {
            notes: vec![KnowledgeNote {
                path: "test.md".to_string(),
                name: "Test".to_string(),
                content: "Hello".to_string(),
                backlink_count: 0,
            }],
            memories: vec![],
            index_entries_used: 42,
        };
        let json = serde_json::to_string(&ctx).unwrap();
        let restored: KnowledgeContext = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.notes.len(), 1);
        assert_eq!(restored.index_entries_used, 42);
    }
}
