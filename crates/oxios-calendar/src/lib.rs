//! Oxios calendar: .ics-based event management.
//!
//! This crate provides a complete calendar system built on the iCalendar (RFC 5545)
//! standard. Events are stored as `.ics` files on disk with an in-memory index
//! for fast lookups.
//!
//! # Architecture
//!
//! - [`CalendarEngine`] — the primary API for all calendar operations.
//! - [`ical`] — build and parse `.ics` files using the `icalendar` crate.
//! - [`rrule`] — convert simple repeat rules to RRULE strings.
//! - [`index`] — in-memory index for fast UID and range lookups.
//! - [`conflict`] — detect overlapping events.
//! - [`alarm`] — alarm dispatch (stub).
//! - [`archive`] — move old events to archive directory.
//!
//! # Example
//!
//! ```no_run
//! use oxios_calendar::{CalendarEngine, EventDraft};
//! use chrono::Utc;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let engine = CalendarEngine::new("/tmp/calendar".into()).await?;
//!
//!     let draft = EventDraft {
//!         title: "Team Standup".into(),
//!         start: Utc::now(),
//!         end: Utc::now() + chrono::Duration::minutes(30),
//!         all_day: false,
//!         description: None,
//!         location: None,
//!         repeat: None,
//!         reminder_minutes: vec![15],
//!         source: oxios_calendar::EventSource::Agent,
//!         note_path: None,
//!     };
//!
//!     let result = engine.create(draft).await?;
//!     println!("Created event: {} ({})", result.uid, result.file);
//!
//!     Ok(())
//! }
//! ```

// `.unwrap()` in tests is idiomatic (workspace convention); suppressed only
// under `cfg(test)` so production code remains linted by the `--lib` clippy pass.
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod alarm;
pub mod archive;
pub mod conflict;
pub mod cron_bridge;
pub mod engine;
pub mod ical;
pub mod index;
pub mod journal_bridge;
pub mod rrule;
pub mod types;

pub use cron_bridge::{CronJobDef, CronSyntheticEvent, expand_cron_events};
pub use engine::CalendarEngine;
pub use journal_bridge::JournalBridge;
pub use types::*;
