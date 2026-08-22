//! # oxios-markdown
//!
//! Markdown knowledge management library — ported from [files.md](https://github.com/zakirullin/files.md)
//! by Artem Zakirullin. Licensed under MIT — see LICENSE-THIRD-PARTY.
//!
//! This crate provides core functionality for managing a knowledge base
//! stored as plain `.md` files:
//!
//! - **VirtualFs** — sandboxed filesystem with path traversal protection
//! - **BacklinkIndex** — bidirectional link tracking between notes
//! - **Merge** — LCS-based conflict resolution
//! - **Parser** — text processing utilities (similarity, links, headings)
//!
//! # Example
//!
//! ```no_run
//! use oxios_markdown::VirtualFs;
//! use std::path::PathBuf;
//!
//! let fs = VirtualFs::new(PathBuf::from("/path/to/knowledge")).unwrap();
//! fs.write("brain", "Rust.md", "# Rust\n\nSee [Ownership](brain/Ownership.md)").unwrap();
//! let content = fs.read("brain", "Rust.md").unwrap();
//! ```

// `.unwrap()` in tests is idiomatic (workspace convention); suppressed only
// under `cfg(test)` so production code remains linted by the `--lib` clippy pass.
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod backlinks;
pub mod chat;
pub mod checklist;
pub mod fs;
#[allow(dead_code)]
pub mod fslog;
pub mod habits;
pub mod html;
pub mod i18n;
pub mod journal;
pub mod knowledge;
pub mod merge;
pub mod parser;
pub mod plugins;
pub mod frontformat;
pub mod schedule;
pub mod stats;
#[allow(dead_code)]
pub mod sync;
pub mod tgtxt;
pub mod tokens;
pub mod types;
pub mod worker;

// Re-export core types for convenience
pub use types::{
    CHAT_FILENAME, DIR_ARCHIVE, DIR_JOURNAL, DIR_MEDIA, DIR_USER_ROOT, DONE_FILENAME, FileEntry,
    FsError, HABIT_COMPLETED, HABIT_COMPLETED_AT_WEEKEND, HABIT_SKIPPED, Habits, KnowledgeConfig,
    LATER_FILENAME, MD_EXT, MODE_CHAT, MODE_FULL, MODE_JOURNAL, MODE_NOTES, MODE_TASKS,
    MOOD_EMOJIS, MOOD_HABIT, READ_FILENAME, SHOP_FILENAME, STATUS_MERGED, STATUS_NOT_MODIFIED,
    STATUS_OK, STATUS_UPDATED_ON_SERVER, SyncError, SyncFile, SyncRequest, SyncResponse,
    WATCH_FILENAME,
};

pub use backlinks::{Backlink, BacklinkIndex, LinkEdge, LinkGraph, LinkNode};
pub use chat::{
    append_to_chat_msg, delete_chat_msg, find_chat_msg_by_hash, move_from_chat, read_chat_msgs,
    rename_chat_msg, today_header as chat_today_header,
};
pub use checklist::{
    add_checklist_item, add_header_and_text, checklist_item, checklist_items,
    complete_checklist_item, incomplete_checklist_items, remove_checklist_item,
    remove_completed_checklist_items,
};
pub use fs::VirtualFs;
pub use fs::split_posix_path;
pub use fslog::FsLog;
pub use habits::{
    emoji_for_status, habit_emoji, habits, last_week_habits, weekday_emoji, write_habits,
};
pub use html::{
    escape_html, markdown_to_html, replace_with_placeholders, restore_from_placeholders,
    strip_html_tags,
};
pub use i18n::{add_emoji, emoji_for};
pub use journal::{
    add_emoji as journal_add_emoji, add_record as journal_add_record,
    today_header as journal_today_header, today_journal_filename,
};
pub use knowledge::{FileChange, KnowledgeBase, NoteHit};
pub use merge::merge;
pub use parser::{
    emoji_prefix, extract_headings, extract_markdown_links, is_multiline, lcfirst, levenshtein,
    norm_new_lines, similar, split_text_into_chunks, substr, today_chat_header, today_journal_path,
    ucfirst,
};
pub use plugins::{
    TimezoneEntry, can_handle as world_clock_can_handle,
    format_report as format_world_clock_report, handle as world_clock_handle,
    world_clock_for_names, world_clock_now,
};
pub use schedule::ScheduleManager;
pub use stats::{CompletedItem, TodayReport, done_today, format_today_report, today_report};
pub use sync::{MediaEntry, MediaSyncResponse, SyncEngine};
pub use tgtxt::{ExtractResult, extract_text_imgs_links};
pub use tokens::TokenManager;
pub use worker::{
    NightlyReport, move_due_tasks, next_exclude_today, remove_completed_checklist,
    remove_completed_inbox_entries, remove_completed_items, schedule_report,
};
