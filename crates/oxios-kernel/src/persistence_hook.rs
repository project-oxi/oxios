//! PersistenceHook — autonomous persistence after agent execution.
//!
//! Two-layer evaluation:
//! 1. Heuristic: detect markdown documents → auto-save to knowledge (no LLM call)
//! 2. LLM Reflection: extract facts/preferences → memory, detect missed knowledge saves

use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::brain::BrainConnection;
use crate::engine::EngineHandle;
use crate::event_bus::{EventBus, KernelEvent};
use crate::memory_agent::sona::TrajectoryStep;
use crate::state_store::StateStore;
use oxios_markdown::KnowledgeBase;
use oxios_markdown::types::{NoteMeta, NoteQuality, NoteSource};
use oxios_ouroboros::Directive;

/// A planned write to the knowledge vault.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeWrite {
    /// Path within the knowledge base (e.g. "notes/rust-design.md").
    pub path: String,
    /// Markdown content to write.
    pub content: String,
    /// Provenance metadata (RFC-022).
    #[serde(default = "default_knowledge_meta")]
    pub meta: NoteMeta,
}

fn default_knowledge_meta() -> NoteMeta {
    NoteMeta {
        author: "agent".to_string(),
        source: NoteSource::Hook,
        quality: NoteQuality::Raw,
        needs_review: true,
        session_id: None,
        message_index: None,
        saved_at: None,
    }
}

/// A planned write to agent memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryWrite {
    /// Memory content.
    pub content: String,
    /// Memory type: "fact" or "episode".
    #[serde(rename = "type")]
    pub memory_type: String,
    /// Importance score 0.0–1.0.
    pub importance: f32,
    /// Optional tags.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Result of evaluating an execution for persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistencePlan {
    /// Memory entries to persist.
    pub memory: Vec<MemoryWrite>,
    /// Knowledge notes to persist.
    pub knowledge: Vec<KnowledgeWrite>,
}

/// Knowledge save record for a session message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeSaveRecord {
    /// Index of the message in the session.
    pub message_index: usize,
    /// Path within the knowledge base.
    pub knowledge_path: String,
    /// ISO 8601 timestamp.
    pub saved_at: String,
    /// How the save was triggered: "hook", "user", "tool".
    pub source: String,
}

/// Autonomous persistence hook.
///
/// Evaluates agent output after execution and decides what to persist
/// to memory and/or knowledge, using heuristic rules first and an
/// optional LLM reflection pass for ambiguous cases.
pub struct PersistenceHook {
    brain: Option<Arc<BrainConnection>>,
    knowledge_base: Arc<KnowledgeBase>,
    engine_handle: Arc<EngineHandle>,
    state_store: Arc<StateStore>,
    event_bus: EventBus,
}

impl PersistenceHook {
    /// Create a new persistence hook.
    ///
    /// The reflection model is resolved live from `engine_handle` on each
    /// reflection, so it tracks hot-swaps — same source of truth as interview
    /// and execute. `brain` is the daemon connection (RFC-047); `None` when
    /// the daemon is unavailable (memory writes degrade silently).
    pub fn new(
        brain: Option<Arc<BrainConnection>>,
        knowledge_base: Arc<KnowledgeBase>,
        engine_handle: Arc<EngineHandle>,
        state_store: Arc<StateStore>,
        event_bus: EventBus,
    ) -> Self {
        Self {
            brain,
            knowledge_base,
            engine_handle,
            state_store,
            event_bus,
        }
    }

    /// Evaluate an execution and produce a persistence plan.
    ///
    /// `already_saved_knowledge` = true if tool-calling already did a knowledge write.
    pub async fn evaluate(
        &self,
        directive: &Directive,
        trajectory: &[TrajectoryStep],
        output: &str,
        already_saved_knowledge: bool,
    ) -> Result<PersistencePlan> {
        let mut plan = PersistencePlan {
            memory: Vec::new(),
            knowledge: Vec::new(),
        };

        // Layer 1: Heuristic — detect markdown documents
        if !already_saved_knowledge && looks_like_document(output) {
            let path = auto_save_path(directive, output);
            let now = chrono::Utc::now().to_rfc3339();
            plan.knowledge.push(KnowledgeWrite {
                path,
                content: output.to_string(),
                meta: NoteMeta {
                    author: "agent".to_string(),
                    source: NoteSource::Hook,
                    quality: NoteQuality::Raw,
                    needs_review: true,
                    session_id: None,
                    message_index: None,
                    saved_at: Some(now),
                },
            });
        }

        // Layer 2: LLM Reflection, with heuristic fallback when empty.
        let knowledge_already_handled = !plan.knowledge.is_empty();
        let reflection_plan = self
            .reflect(directive, trajectory, output, knowledge_already_handled)
            .await;
        match reflection_plan {
            Ok(reflected) => {
                plan.memory.extend(reflected.memory);
                if !already_saved_knowledge {
                    plan.knowledge.extend(reflected.knowledge);
                }
                // LLM produced nothing for memory — fall back to user-input scan.
                if plan.memory.is_empty() {
                    let heuristic = Self::heuristic_fallback(directive, output);
                    if !heuristic.is_empty() {
                        tracing::info!(
                            count = heuristic.len(),
                            "PersistenceHook heuristic filled empty reflection"
                        );
                        plan.memory.extend(heuristic);
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "PersistenceHook reflection failed; trying heuristic fallback");
                let heuristic = Self::heuristic_fallback(directive, output);
                plan.memory.extend(heuristic);
            }
        }
        Ok(plan)
    }

    /// Execute a persistence plan (fire-and-forget style, but still awaits I/O).
    pub async fn execute_plan(
        &self,
        mut plan: PersistencePlan,
        session_id: &str,
        message_index: usize,
    ) {
        // Memory writes — persisted to the brain daemon (RFC-047). The
        // memory_type field is kept for the plan's serialized shape but the
        // brain owns classification; content is stored as-is with the session
        // id embedded in the source label.
        for mw in &plan.memory {
            let source = format!("persistence-hook:{session_id}");
            match &self.brain {
                Some(brain) => match brain.remember(&mw.content, &source).await {
                    Some(_id) => tracing::debug!(session = session_id, "Hook saved memory"),
                    None => tracing::debug!("brain unavailable — memory write skipped"),
                },
                None => tracing::debug!("brain unavailable — memory write skipped"),
            }
        }

        // Knowledge writes
        let now_iso = chrono::Utc::now().to_rfc3339();
        for kw in &mut plan.knowledge {
            // Backfill session context into meta (reflection path leaves these empty)
            if kw.meta.session_id.is_none() {
                kw.meta.session_id = Some(session_id.to_string());
            }
            if kw.meta.message_index.is_none() {
                kw.meta.message_index = Some(message_index);
            }
            if kw.meta.saved_at.is_none() {
                kw.meta.saved_at = Some(now_iso.clone());
            }
        }
        for kw in &plan.knowledge {
            match self
                .knowledge_base
                .note_write_with_meta(&kw.path, &kw.content, &kw.meta)
            {
                Ok(true) => {
                    tracing::info!(
                        path = %kw.path,
                        session = session_id,
                        "Hook saved knowledge note"
                    );
                    // Record the save mapping
                    let record = KnowledgeSaveRecord {
                        message_index,
                        knowledge_path: kw.path.clone(),
                        saved_at: chrono::Utc::now().to_rfc3339(),
                        source: "hook".to_string(),
                    };
                    self.record_save(session_id, &record).await;
                    // Publish event
                    let _ = self.event_bus.publish(KernelEvent::KnowledgePersisted {
                        session_id: session_id.to_string(),
                        message_index,
                        path: kw.path.clone(),
                        source: "hook".to_string(),
                    });
                }
                Ok(false) => {
                    tracing::warn!(
                        path = %kw.path,
                        "Hook skipped knowledge save: path is a user-authored note"
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, path = %kw.path, "Hook failed to save knowledge")
                }
            }
        }
    }

    /// Record a knowledge save to StateStore.
    async fn record_save(&self, session_id: &str, record: &KnowledgeSaveRecord) {
        let saves: Vec<KnowledgeSaveRecord> = self
            .state_store
            .load_json("knowledge-saves", session_id)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        // Note: we load, push, save — not append. This is fine for the
        // low-throughput knowledge-save path. If contention becomes an
        // issue, switch to append-only log + compaction.
        let mut saves = saves;
        saves.push(record.clone());
        if let Err(e) = self
            .state_store
            .save_json("knowledge-saves", session_id, &saves)
            .await
        {
            tracing::warn!(error = %e, "Failed to record knowledge save");
        }
    }

    /// LLM reflection — ask the model what to persist.
    async fn reflect(
        &self,
        directive: &Directive,
        trajectory: &[TrajectoryStep],
        output: &str,
        knowledge_already_handled: bool,
    ) -> Result<PersistencePlan> {
        let trajectory_summary: Vec<String> = trajectory
            .iter()
            .take(20)
            .map(|s| {
                let out_preview = if s.output.len() > 100 {
                    // Char-boundary safe: multibyte UTF-8 (Korean, emoji)
                    // would panic on a raw byte slice.
                    let mut end = 100;
                    while end > 0 && !s.output.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}...", &s.output[..end])
                } else {
                    s.output.clone()
                };
                format!("- {} → {}", s.input, out_preview)
            })
            .collect();
        let result_snippet = if output.len() > 500 {
            let mut end = 500;
            while end > 0 && !output.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...", &output[..end])
        } else {
            output.to_string()
        };

        let knowledge_hint = if knowledge_already_handled {
            "Knowledge has already been saved by the agent — do NOT include a knowledge array."
        } else {
            "You may also save knowledge (documents, reference material) if the output contains substantive content worth keeping."
        };

        let prompt = format!(
            "Review this agent execution. Decide what to persist.\n\n\
             Goal: {}\n\
             Request: {}\n\
             Steps:\n{}\n\
             Result: {}\n\n\
             Two stores:\n\
             - Memory: facts about the user, preference corrections, project context. Not visible to the user. Agent's own learning.\n\
             - Knowledge: documents, research, reference material the user would want later. Visible via Web UI.\n\
             {knowledge_hint}\n\
             Look especially hard at the user's REQUEST — user preferences and facts live there, not in the agent's output.\n\
             \n\
             When saving to knowledge, strip conversational wrapping: greetings, sign-offs, questions to the user, hedging. Extract only substantive content.\n\
             JSON only:\n\
             {{\"memory\":[{{\"content\":\"...\",\"type\":\"fact|episode\",\"importance\":0.0-1.0}}],\"knowledge\":[{{\"path\":\"cat/file.md\",\"content\":\"...\"}}]}}",
            directive.goal,
            directive.original_request,
            trajectory_summary.join("\n"),
            result_snippet,
        );

        // Build a lightweight agent via EngineHandle → Oxicode → AgentBuilder
        let engine = self.engine_handle.get();
        let model_id = engine.default_model_id().to_string();
        let agent_config = oxicode_sdk::AgentConfig {
            description: Some("Persistence reflection".into()),
            model_id: model_id.clone(),
            system_prompt: Some("You output JSON only. No explanation.".to_string()),
            max_tokens: Some(512),
            temperature: Some(0.3),
            ..Default::default()
        };

        tracing::info!(
            model = %model_id,
            prompt_len = prompt.len(),
            request_len = directive.original_request.len(),
            output_len = output.len(),
            "PersistenceHook reflection starting"
        );

        let agent = engine.oxi().agent(agent_config).build()?;
        let (response, _events) = agent.run(prompt).await?;

        tracing::info!(
            response_len = response.content.len(),
            "PersistenceHook reflection response received"
        );

        // Parse JSON from response
        let json_str = response.content.trim();
        // Strip markdown code fences if present
        let json_str = json_str
            .strip_prefix("```json\n")
            .or_else(|| json_str.strip_prefix("```\n"))
            .unwrap_or(json_str);
        let json_str = json_str.strip_suffix("```").unwrap_or(json_str);

        let plan: PersistencePlan = serde_json::from_str(json_str.trim())?;
        tracing::info!(
            memory_count = plan.memory.len(),
            knowledge_count = plan.knowledge.len(),
            "PersistenceHook reflection plan parsed"
        );
        Ok(plan)
    }

    /// Heuristic fallback: when the LLM reflection fails or returns an empty
    /// memory plan, scan the user's request (not the agent output) for
    /// preference/fact patterns. The user's stated facts live in the request,
    /// not in the agent's response.
    fn heuristic_fallback(directive: &Directive, output: &str) -> Vec<MemoryWrite> {
        let mut found = Vec::new();
        let request = &directive.original_request;
        if request.is_empty() {
            return found;
        }
        // Preference indicators (Korean + English).
        let preference_patterns = [
            "I prefer",
            "I like",
            "I always",
            "I never",
            "remember that",
            "기억해",
            "항상 ",
            "절대 ",
            "내가 ",
            "나는 ",
            "저는 ",
            "선호",
        ];
        for pat in &preference_patterns {
            if request.contains(pat) || output.contains(pat) {
                let content = if request.contains(pat) {
                    truncate(request, 200)
                } else {
                    truncate(output, 200)
                };
                found.push(MemoryWrite {
                    content,
                    memory_type: "fact".to_string(),
                    importance: 0.6,
                    tags: vec!["persistence-hook-heuristic".to_string()],
                });
                break; // one heuristic hit per execution is enough
            }
        }
        found
    }
}

/// Truncate a string for error messages without panicking on UTF-8 boundaries.
fn truncate(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Check if content looks like a structured markdown document.
fn looks_like_document(content: &str) -> bool {
    if content.len() < 300 {
        return false;
    }
    let has_headers = content.contains("## ") || content.contains("# ");
    let has_structure = content.contains("- ")
        || content.contains("* ")
        || content.contains("```")
        || content.contains("| ");
    has_headers && has_structure
}

/// Generate an auto-save path from the directive goal and content.
fn auto_save_path(directive: &Directive, content: &str) -> String {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();

    // Try to extract a meaningful name from the first ## heading
    let heading = content
        .lines()
        .find(|l| l.starts_with("## ") || l.starts_with("# "))
        .map(|l| l.trim_start_matches('#').trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| {
            directive
                .goal
                .split_whitespace()
                .take(5)
                .collect::<Vec<_>>()
                .join("-")
        });

    // Slugify
    let slug: String = heading
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.len() > 60 {
        // Slug is ASCII-only (slugified above), but guard the slice for
        // safety and consistency with the rest of the codebase.
        let mut end = 60;
        while end > 0 && !slug.is_char_boundary(end) {
            end -= 1;
        }
        slug[..end].to_string()
    } else {
        slug
    };

    format!("notes/{slug}-{date}.md")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_looks_like_document_short() {
        assert!(!looks_like_document("short text"));
    }

    #[test]
    fn test_looks_like_document_structured() {
        let content = "# Title\n\nSome intro text here that makes this longer than three hundred characters. We need more text to reach the threshold. Adding some more content here. And even more text to be absolutely sure we cross the 300 character limit. Extra padding.\n\n## Section 1\n\n- Item 1\n- Item 2\n\n## Section 2\n\nSome content.";
        assert!(looks_like_document(content));
    }

    #[test]
    fn test_looks_like_document_no_structure() {
        let content = "## Title\n\nJust plain text without any lists or code blocks. We need to make this longer than 300 characters to pass the length check. Let me add more text. And more text. And even more text to be sure.";
        assert!(!looks_like_document(content));
    }

    #[test]
    fn test_looks_like_document_has_list() {
        let content = "## Title\n\nSome intro text here that makes this longer than three hundred characters. We need more text to reach the threshold. Adding some more content here. And even more text to be absolutely sure we cross the 300 character limit. Extra padding added. More text here too for good measure.\n\n- Item one\n- Item two";
        assert!(looks_like_document(content));
    }

    #[test]
    fn test_auto_save_path() {
        let directive = Directive {
            goal: "Write a Rust design document".to_string(),
            ..Default::default()
        };
        let content = "## Rust Ownership Design\n\nContent here...";
        let path = auto_save_path(&directive, content);
        assert!(path.starts_with("notes/"));
        assert!(path.ends_with(".md"));
        assert!(path.contains("rust"));
    }
    #[test]
    fn test_auto_save_path_from_goal() {
        let directive = Directive {
            goal: "Fetch hacker news".to_string(),
            ..Default::default()
        };
        let content = "Plain text without headings but we still need a path.";
        let path = auto_save_path(&directive, content);
        assert!(path.starts_with("notes/"));
        assert!(path.contains("fetch"));
    }
}
