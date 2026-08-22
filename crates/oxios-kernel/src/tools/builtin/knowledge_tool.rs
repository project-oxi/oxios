//! Knowledge tool — agent-facing tool for markdown note management.
//!
//! Provides a single `knowledge` tool with action-based dispatch, following
//! the same pattern as `memory_tools.rs`. Actions: read, write, delete,
//! move, tree, search, backlinks.

use async_trait::async_trait;
use std::sync::Arc;

use chrono::Datelike;
use oxicode_sdk::{AgentTool as OxicodeAgentTool, AgentToolResult, ToolContext};
use serde_json::{Value, json};

use crate::KernelHandle;
use oxios_markdown::KnowledgeBase;
use oxios_markdown::types::{NoteMeta, NoteQuality, NoteSource};

/// Tool for reading, writing, and managing markdown knowledge notes.
///
/// Uses action-based dispatch: `read`, `write`, `delete`, `move`, `tree`,
/// `search`, `backlinks`.
///
/// Delegates directly to [`KnowledgeBase`] for all operations.
pub struct KnowledgeTool {
    kb: Arc<KnowledgeBase>,
}

impl KnowledgeTool {
    /// Create from a [`KernelHandle`], extracting KnowledgeBase directly.
    pub fn from_kernel(kernel: &KernelHandle) -> Self {
        Self {
            kb: kernel.knowledge.clone(),
        }
    }

    /// Create with explicit KnowledgeBase (for testing).
    pub fn new(kb: Arc<KnowledgeBase>) -> Self {
        Self { kb }
    }
}

impl std::fmt::Debug for KnowledgeTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KnowledgeTool").finish()
    }
}

#[async_trait]

impl OxicodeAgentTool for KnowledgeTool {
    fn name(&self) -> &str {
        "knowledge"
    }

    fn label(&self) -> &str {
        "Knowledge"
    }

    fn description(&self) -> &'static str {
        "Personal markdown vault — documents, articles, notes, journal entries. File-based with backlinks, full-text search, and directory structure. Read, write, search, and organize user content as markdown files."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                    "read", "write", "delete", "move", "tree", "search", "backlinks",
                    "checklist_items", "checklist_add", "checklist_complete", "checklist_remove",
                    "chat_append", "chat_messages", "chat_delete", "chat_move",
                    "journal_add", "journal_emoji", "journal_today",
                    "habits", "habits_last_week",
                    "today_report", "done_today",
                    "config_read", "config_write",
                    "nightly_cleanup", "run_scheduled",
                    "markdown_to_html", "auto_emoji"
                ],
                    "description": "The action to perform"
                },
                "path": {
                    "type": "string",
                    "description": "Note path (e.g., 'brain/Rust.md' or 'Chat.md')"
                },
                "content": {
                    "type": "string",
                    "description": "Content for write action"
                },
                "old_path": {
                    "type": "string",
                    "description": "Old path for move action"
                },
                "new_path": {
                    "type": "string",
                    "description": "New path for move action"
                },
                "dir": {
                    "type": "string",
                    "description": "Directory for tree action (default: root)"
                },
                "query": {
                    "type": "string",
                    "description": "Search query for search action"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results for search/tree (default: 20)"
                },
                "item": {
                    "type": "string",
                    "description": "Checklist item text (for checklist_add)"
                },
                "checked": {
                    "type": "boolean",
                    "description": "Whether the checklist item is checked (for checklist_add, default: false)"
                },
                "item_hash": {
                    "type": "string",
                    "description": "Hash identifying a checklist or chat item (for checklist_complete, chat_delete, chat_move)"
                },
                "item_or_hash": {
                    "type": "string",
                    "description": "Checklist item text or hash (for checklist_remove)"
                },
                "message": {
                    "type": "string",
                    "description": "Chat message text (for chat_append)"
                },
                "msg_hash": {
                    "type": "string",
                    "description": "Hash identifying a chat message (for chat_delete, chat_move)"
                },
                "target_path": {
                    "type": "string",
                    "description": "Target note path (for chat_move)"
                },
                "record": {
                    "type": "string",
                    "description": "Journal record text (for journal_add)"
                },
                "emoji": {
                    "type": "string",
                    "description": "Emoji string (for journal_emoji)"
                },
                "year": {
                    "type": "integer",
                    "description": "Year number (for habits action)"
                },
                "config": {
                    "type": "object",
                    "description": "KnowledgeConfig JSON object (for config_write)"
                },
                "md": {
                    "type": "string",
                    "description": "Markdown text to convert (for markdown_to_html)"
                },
                "text": {
                    "type": "string",
                    "description": "Text to find emoji for (for auto_emoji)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: Value,
        _signal: Option<tokio::sync::oneshot::Receiver<()>>,
        _ctx: &ToolContext,
    ) -> Result<AgentToolResult, oxicode_sdk::ToolError> {
        let action = params["action"].as_str().unwrap_or("");
        if action.is_empty() {
            return Ok(AgentToolResult::error("action is required"));
        }

        match action {
            "read" => {
                let path = params["path"].as_str().unwrap_or("");
                if path.is_empty() {
                    return Ok(AgentToolResult::error("path is required for read"));
                }
                match self.kb.note_read(path) {
                    Ok(Some(content)) => Ok(AgentToolResult::success(&content)),
                    Ok(None) => Ok(AgentToolResult::error(format!("Note '{path}' not found"))),
                    Err(e) => Ok(AgentToolResult::error(format!("Failed to read note: {e}"))),
                }
            }
            "write" => {
                let path = params["path"].as_str().unwrap_or("");
                let content = params["content"].as_str().unwrap_or("");
                if path.is_empty() {
                    return Ok(AgentToolResult::error("path is required for write"));
                }
                if content.is_empty() {
                    return Ok(AgentToolResult::error("content is required for write"));
                }
                let meta = NoteMeta {
                    author: "agent".to_string(),
                    source: NoteSource::Tool,
                    quality: NoteQuality::Raw,
                    needs_review: true,
                    session_id: None,
                    message_index: None,
                    saved_at: Some(chrono::Utc::now().to_rfc3339()),
                };
                match self.kb.note_write_with_meta(path, content, &meta) {
                    Ok(true) => Ok(AgentToolResult::success(format!(
                        "Note '{path}' written successfully"
                    ))),
                    Ok(false) => Ok(AgentToolResult::error(format!(
                        "Note '{path}' is a user-authored file, not overwriting"
                    ))),
                    Err(e) => Ok(AgentToolResult::error(format!("Failed to write note: {e}"))),
                }
            }
            "delete" => {
                let path = params["path"].as_str().unwrap_or("");
                if path.is_empty() {
                    return Ok(AgentToolResult::error("path is required for delete"));
                }
                match self.kb.note_delete(path) {
                    Ok(()) => Ok(AgentToolResult::success(format!("Note '{path}' deleted"))),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to delete note: {e}"
                    ))),
                }
            }
            "move" => {
                let old_path = params["old_path"]
                    .as_str()
                    .or_else(|| {
                        // Also accept "path" as old_path if old_path not provided
                        if params["new_path"].as_str().is_some() {
                            params["path"].as_str()
                        } else {
                            None
                        }
                    })
                    .unwrap_or("");
                let new_path = params["new_path"].as_str().unwrap_or("");
                if old_path.is_empty() || new_path.is_empty() {
                    return Ok(AgentToolResult::error(
                        "old_path and new_path are required for move",
                    ));
                }
                match self.kb.note_move(old_path, new_path) {
                    Ok(()) => Ok(AgentToolResult::success(format!(
                        "Note moved from '{old_path}' to '{new_path}'"
                    ))),
                    Err(e) => Ok(AgentToolResult::error(format!("Failed to move note: {e}"))),
                }
            }
            "tree" => {
                let dir = params["dir"].as_str().unwrap_or("/");
                let limit = params["limit"].as_u64().unwrap_or(50) as usize;
                match self.kb.note_tree(dir) {
                    Ok(entries) => {
                        let count = entries.len();
                        let entries: Vec<_> = entries.into_iter().take(limit).collect();
                        if entries.is_empty() {
                            return Ok(AgentToolResult::success("Directory is empty"));
                        }
                        let mut output =
                            format!("Found {} entries (showing {}):\n\n", count, entries.len());
                        for entry in &entries {
                            let kind = if entry.is_dir { "📁" } else { "📄" };
                            output.push_str(&format!(
                                "{} {} ({})\n",
                                kind, entry.display_name, entry.name
                            ));
                        }
                        Ok(AgentToolResult::success(&output))
                    }
                    Err(e) => Ok(AgentToolResult::error(format!("Failed to list notes: {e}"))),
                }
            }
            "search" => {
                let query = params["query"].as_str().unwrap_or("");
                if query.is_empty() {
                    return Ok(AgentToolResult::error("query is required for search"));
                }
                let limit = params["limit"].as_u64().unwrap_or(10) as usize;
                match self.kb.search(query, limit) {
                    Ok(hits) => {
                        if hits.is_empty() {
                            return Ok(AgentToolResult::success("No matching notes found"));
                        }
                        let mut output = format!("Found {} matching notes:\n\n", hits.len());
                        for hit in &hits {
                            output.push_str(&format!(
                                "- {} (path: {}, backlinks: {}, name_sim: {}%)\n",
                                hit.name, hit.path, hit.backlink_count, hit.name_similarity,
                            ));
                        }
                        Ok(AgentToolResult::success(&output))
                    }
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to search notes: {e}"
                    ))),
                }
            }
            "backlinks" => {
                let path = params["path"].as_str().unwrap_or("");
                if path.is_empty() {
                    return Ok(AgentToolResult::error("path is required for backlinks"));
                }
                let backlinks = self.kb.backlinks_for(path);
                if backlinks.is_empty() {
                    return Ok(AgentToolResult::success(format!(
                        "No backlinks for '{path}'"
                    )));
                }
                let mut output = format!("Backlinks for '{}' ({}):\n\n", path, backlinks.len());
                for bl in &backlinks {
                    output.push_str(&format!(
                        "- {} → {} (line {})\n",
                        bl.source_path, bl.target_path, bl.line_number
                    ));
                }
                Ok(AgentToolResult::success(&output))
            }
            // ── Checklist ─────────────────────────────────────────
            "checklist_items" => {
                let path = params["path"].as_str().unwrap_or("");
                if path.is_empty() {
                    return Ok(AgentToolResult::error(
                        "path is required for checklist_items",
                    ));
                }
                match self.kb.checklist_items(path) {
                    Ok((items, checked_map)) => {
                        if items.is_empty() {
                            return Ok(AgentToolResult::success("No checklist items found"));
                        }
                        let mut output =
                            format!("Checklist items for '{}' ({}):\n\n", path, items.len());
                        for item in &items {
                            let status = checked_map
                                .get(item)
                                .map(|b| if *b { "✅" } else { "⬜" })
                                .unwrap_or("⬜");
                            output.push_str(&format!("{status} {item}\n"));
                        }
                        Ok(AgentToolResult::success(&output))
                    }
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to get checklist items: {e}"
                    ))),
                }
            }

            "checklist_add" => {
                let path = params["path"].as_str().unwrap_or("");
                let item = params["item"].as_str().unwrap_or("");
                let checked = params["checked"].as_bool().unwrap_or(false);
                if path.is_empty() {
                    return Ok(AgentToolResult::error("path is required for checklist_add"));
                }
                if item.is_empty() {
                    return Ok(AgentToolResult::error("item is required for checklist_add"));
                }
                match self.kb.checklist_add(path, item, checked) {
                    Ok(()) => Ok(AgentToolResult::success(format!(
                        "Checklist item added to '{path}'"
                    ))),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to add checklist item: {e}"
                    ))),
                }
            }

            "checklist_complete" => {
                let path = params["path"].as_str().unwrap_or("");
                let item_hash = params["item_hash"].as_str().unwrap_or("");
                if path.is_empty() {
                    return Ok(AgentToolResult::error(
                        "path is required for checklist_complete",
                    ));
                }
                if item_hash.is_empty() {
                    return Ok(AgentToolResult::error(
                        "item_hash is required for checklist_complete",
                    ));
                }
                match self.kb.checklist_complete(path, item_hash) {
                    Ok(true) => Ok(AgentToolResult::success(format!(
                        "Checklist item completed in '{path}'"
                    ))),
                    Ok(false) => Ok(AgentToolResult::error(format!(
                        "Checklist item '{item_hash}' not found in '{path}'"
                    ))),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to complete checklist item: {e}"
                    ))),
                }
            }

            "checklist_remove" => {
                let path = params["path"].as_str().unwrap_or("");
                let item_or_hash = params["item_or_hash"].as_str().unwrap_or("");
                if path.is_empty() {
                    return Ok(AgentToolResult::error(
                        "path is required for checklist_remove",
                    ));
                }
                if item_or_hash.is_empty() {
                    return Ok(AgentToolResult::error(
                        "item_or_hash is required for checklist_remove",
                    ));
                }
                match self.kb.checklist_remove(path, item_or_hash) {
                    Ok(true) => Ok(AgentToolResult::success(format!(
                        "Checklist item removed from '{path}'"
                    ))),
                    Ok(false) => Ok(AgentToolResult::error(format!(
                        "Checklist item '{item_or_hash}' not found in '{path}'"
                    ))),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to remove checklist item: {e}"
                    ))),
                }
            }

            // ── Chat ────────────────────────────────────────────────
            "chat_append" => {
                let message = params["message"].as_str().unwrap_or("");
                if message.is_empty() {
                    return Ok(AgentToolResult::error(
                        "message is required for chat_append",
                    ));
                }
                match self.kb.chat_append(message) {
                    Ok(()) => Ok(AgentToolResult::success("Message appended to chat")),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to append chat message: {e}"
                    ))),
                }
            }

            "chat_messages" => match self.kb.chat_messages() {
                Ok(messages) => {
                    if messages.is_empty() {
                        return Ok(AgentToolResult::success("No chat messages found"));
                    }
                    let mut output = format!("Chat messages ({}):\n\n", messages.len());
                    for (i, msg) in messages.iter().enumerate() {
                        output.push_str(&format!("{}. {}\n", i + 1, msg));
                    }
                    Ok(AgentToolResult::success(&output))
                }
                Err(e) => Ok(AgentToolResult::error(format!(
                    "Failed to get chat messages: {e}"
                ))),
            },

            "chat_delete" => {
                let msg_hash = params["msg_hash"].as_str().unwrap_or("");
                if msg_hash.is_empty() {
                    return Ok(AgentToolResult::error(
                        "msg_hash is required for chat_delete",
                    ));
                }
                match self.kb.chat_delete(msg_hash) {
                    Ok(true) => Ok(AgentToolResult::success(format!(
                        "Chat message '{msg_hash}' deleted"
                    ))),
                    Ok(false) => Ok(AgentToolResult::error(format!(
                        "Chat message '{msg_hash}' not found"
                    ))),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to delete chat message: {e}"
                    ))),
                }
            }

            "chat_move" => {
                let msg_hash = params["msg_hash"].as_str().unwrap_or("");
                let target_path = params["target_path"].as_str().unwrap_or("");
                if msg_hash.is_empty() {
                    return Ok(AgentToolResult::error("msg_hash is required for chat_move"));
                }
                if target_path.is_empty() {
                    return Ok(AgentToolResult::error(
                        "target_path is required for chat_move",
                    ));
                }
                match self.kb.chat_move_to(msg_hash, target_path) {
                    Ok(true) => Ok(AgentToolResult::success(format!(
                        "Chat message moved to '{target_path}'"
                    ))),
                    Ok(false) => Ok(AgentToolResult::error(format!(
                        "Chat message '{msg_hash}' not found"
                    ))),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to move chat message: {e}"
                    ))),
                }
            }

            // ── Journal ─────────────────────────────────────────────
            "journal_add" => {
                let record = params["record"].as_str().unwrap_or("");
                if record.is_empty() {
                    return Ok(AgentToolResult::error("record is required for journal_add"));
                }
                match self.kb.journal_add_record(record) {
                    Ok(()) => Ok(AgentToolResult::success("Journal record added")),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to add journal record: {e}"
                    ))),
                }
            }

            "journal_emoji" => {
                let emoji = params["emoji"].as_str().unwrap_or("");
                if emoji.is_empty() {
                    return Ok(AgentToolResult::error(
                        "emoji is required for journal_emoji",
                    ));
                }
                match self.kb.journal_add_emoji(emoji) {
                    Ok(()) => Ok(AgentToolResult::success(format!(
                        "Journal emoji set to '{emoji}'"
                    ))),
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Failed to set journal emoji: {e}"
                    ))),
                }
            }

            "journal_today" => {
                let path = self.kb.journal_today_path();
                Ok(AgentToolResult::success(&path))
            }

            // ── Habits ──────────────────────────────────────────────
            "habits" => {
                let year = params["year"]
                    .as_i64()
                    .unwrap_or_else(|| chrono::Local::now().year() as i64)
                    as i32;
                match self.kb.habits(year) {
                    Ok(habits) => {
                        let json = serde_json::to_string_pretty(&habits)
                            .unwrap_or_else(|_| "{}".to_string());
                        Ok(AgentToolResult::success(&json))
                    }
                    Err(e) => Ok(AgentToolResult::error(format!("Failed to get habits: {e}"))),
                }
            }

            "habits_last_week" => match self.kb.habits_last_week() {
                Ok(habits) => {
                    let json =
                        serde_json::to_string_pretty(&habits).unwrap_or_else(|_| "{}".to_string());
                    Ok(AgentToolResult::success(&json))
                }
                Err(e) => Ok(AgentToolResult::error(format!(
                    "Failed to get last week habits: {e}"
                ))),
            },

            // ── Stats ───────────────────────────────────────────────
            "today_report" => match self.kb.today_report() {
                Ok(report) => {
                    let json =
                        serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_string());
                    Ok(AgentToolResult::success(&json))
                }
                Err(e) => Ok(AgentToolResult::error(format!(
                    "Failed to get today report: {e}"
                ))),
            },

            "done_today" => match self.kb.done_today() {
                Ok(entries) => {
                    if entries.is_empty() {
                        return Ok(AgentToolResult::success("No completed items today"));
                    }
                    let mut output = format!("Done today ({}):\n\n", entries.len());
                    for entry in &entries {
                        let kind = if entry.is_dir { "📁" } else { "📄" };
                        output.push_str(&format!(
                            "{} {} ({})\n",
                            kind, entry.display_name, entry.name
                        ));
                    }
                    Ok(AgentToolResult::success(&output))
                }
                Err(e) => Ok(AgentToolResult::error(format!(
                    "Failed to get done today: {e}"
                ))),
            },

            // ── Config ──────────────────────────────────────────────
            "config_read" => match self.kb.config() {
                Ok(config) => {
                    let json =
                        serde_json::to_string_pretty(&config).unwrap_or_else(|_| "{}".to_string());
                    Ok(AgentToolResult::success(&json))
                }
                Err(e) => Ok(AgentToolResult::error(format!(
                    "Failed to read config: {e}"
                ))),
            },

            "config_write" => {
                let config_val = params.get("config").cloned().unwrap_or(json!({}));
                match serde_json::from_value::<oxios_markdown::types::KnowledgeConfig>(config_val) {
                    Ok(config) => match self.kb.set_config(&config) {
                        Ok(()) => Ok(AgentToolResult::success("Config updated successfully")),
                        Err(e) => Ok(AgentToolResult::error(format!(
                            "Failed to write config: {e}"
                        ))),
                    },
                    Err(e) => Ok(AgentToolResult::error(format!(
                        "Invalid config object: {e}"
                    ))),
                }
            }

            // ── Automation ──────────────────────────────────────────
            "nightly_cleanup" => match self.kb.run_nightly_cleanup() {
                Ok(report) => {
                    let json =
                        serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_string());
                    Ok(AgentToolResult::success(&json))
                }
                Err(e) => Ok(AgentToolResult::error(format!(
                    "Failed to run nightly cleanup: {e}"
                ))),
            },

            "run_scheduled" => match self.kb.run_scheduled_tasks() {
                Ok(moved) => {
                    if moved.is_empty() {
                        Ok(AgentToolResult::success("No scheduled tasks due"))
                    } else {
                        let mut output =
                            format!("Moved {} scheduled tasks to chat:\n\n", moved.len());
                        for task in &moved {
                            output.push_str(&format!("- {task}\n"));
                        }
                        Ok(AgentToolResult::success(&output))
                    }
                }
                Err(e) => Ok(AgentToolResult::error(format!(
                    "Failed to run scheduled tasks: {e}"
                ))),
            },

            // ── Utils ───────────────────────────────────────────────
            "markdown_to_html" => {
                let md = params["md"].as_str().unwrap_or("");
                if md.is_empty() {
                    return Ok(AgentToolResult::error(
                        "md is required for markdown_to_html",
                    ));
                }
                let html = self.kb.markdown_to_html(md);
                Ok(AgentToolResult::success(&html))
            }

            "auto_emoji" => {
                let text = params["text"].as_str().unwrap_or("");
                if text.is_empty() {
                    return Ok(AgentToolResult::error("text is required for auto_emoji"));
                }
                let emoji = self.kb.auto_emoji(text);
                Ok(AgentToolResult::success(&emoji))
            }

            _ => Ok(AgentToolResult::error(format!(
                "Unknown action '{action}'. Must be one of: read, write, delete, move, tree, search, backlinks, \
             checklist_items, checklist_add, checklist_complete, checklist_remove, \
             chat_append, chat_messages, chat_delete, chat_move, \
             journal_add, journal_emoji, journal_today, \
             habits, habits_last_week, today_report, done_today, \
             config_read, config_write, nightly_cleanup, run_scheduled, \
             markdown_to_html, auto_emoji"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_knowledge_tool_schema() {
        let dir = std::env::temp_dir().join(format!("test-kb-tool-{}", uuid::Uuid::new_v4()));
        let kb = Arc::new(oxios_markdown::KnowledgeBase::new(dir).unwrap());
        let tool = KnowledgeTool::new(kb);
        assert_eq!(tool.name(), "knowledge");
        let schema = tool.parameters_schema();
        assert!(schema["required"].is_array());
        let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
        assert_eq!(actions.len(), 28);
    }

    fn make_tool() -> (Arc<oxios_markdown::KnowledgeBase>, KnowledgeTool) {
        let dir = std::env::temp_dir().join(format!("test-kb-tool-{}", uuid::Uuid::new_v4()));
        let kb = Arc::new(oxios_markdown::KnowledgeBase::new(dir).unwrap());
        (Arc::clone(&kb), KnowledgeTool::new(kb))
    }

    async fn tool_write(
        tool: &KnowledgeTool,
        path: &str,
        content: &str,
    ) -> oxicode_sdk::AgentToolResult {
        tool.execute(
            "test-call",
            json!({"action": "write", "path": path, "content": content}),
            None,
            &ToolContext::default(),
        )
        .await
        .unwrap()
    }

    /// RFC-022 user-authored contract (design §5): user-authored means
    /// "frontmatter present WITHOUT an `oxios:` table". The agent tool
    /// write must refuse such notes and still succeed on its own
    /// notes (frontmatter WITH an `oxios:` table).
    #[tokio::test]
    #[ignore = "vault-unification T12 spec: note_write_with_meta merge path not yet converted to frontformat"]
    async fn write_refuses_user_authored_and_accepts_agent_notes() {
        let (kb, tool) = make_tool();

        // Seed a user-authored note: frontmatter without an `oxios:`
        // table (Obsidian-style). Seeded via std::fs::write because
        // note_write itself now synthesizes frontmatter.
        let user_note = "---\ntags: [rust, design]\nauthor: jane\n---\n\n# My note\n";
        std::fs::create_dir_all(kb.root().join("brain")).unwrap();
        std::fs::write(kb.root().join("brain").join("User.md"), user_note).unwrap();

        // Agent write to the user note ⇒ refusal message, file untouched.
        let res = tool_write(&tool, "brain/User.md", "# overwritten").await;
        assert!(!res.success, "agent write to user note must be refused");
        assert!(
            res.output.contains("user-authored"),
            "refusal must name user-authored; got: {}",
            res.output
        );
        assert_eq!(
            kb.note_read("brain/User.md").unwrap().unwrap(),
            user_note,
            "refused write must leave the user note byte-identical"
        );

        // Agent write to its own note (lands an `oxios:` table) ⇒ succeeds.
        let res = tool_write(&tool, "brain/Agent.md", "# Agent note").await;
        assert!(
            res.success,
            "agent write to own note must succeed; got: {}",
            res.output
        );
        let agent_note = kb.note_read("brain/Agent.md").unwrap().unwrap();
        assert!(
            agent_note.contains("oxios:"),
            "own note must carry the oxios: table"
        );
        assert!(agent_note.contains("# Agent note"));

        // Re-write of its own note still succeeds — an `oxios:` table
        // means the note is agent-authored, NOT user-authored.
        let res = tool_write(&tool, "brain/Agent.md", "# Agent note v2").await;
        assert!(
            res.success,
            "re-write of own note must succeed; got: {}",
            res.output
        );
        let agent_note = kb.note_read("brain/Agent.md").unwrap().unwrap();
        assert!(agent_note.contains("# Agent note v2"));
        assert!(agent_note.contains("oxios:"));
    }
}
