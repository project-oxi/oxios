//! API routes for calendar event management.
//!
//! Provides endpoints for creating, listing, updating, deleting,
//! searching events, and querying free/busy slots.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use oxios_calendar::{EventDraft, EventPatch};

use super::deserialize_some;
use crate::api::error::AppError;
use crate::api::server::AppState;

// ---------------------------------------------------------------------------
// Request / query types
// ---------------------------------------------------------------------------

/// Query parameters for date-range endpoints (`from` / `to` as ISO 8601).
#[derive(Debug, Deserialize)]
pub struct DateRangeParams {
    /// Range start (ISO 8601).
    pub from: String,
    /// Range end (ISO 8601).
    pub to: String,
}

/// Request body for creating a new event.
#[derive(Debug, Deserialize)]
pub struct CreateEventRequest {
    /// Event title / summary.
    pub title: String,
    /// Start time (ISO 8601).
    pub start: String,
    /// End time (ISO 8601).
    pub end: String,
    /// Whether this is an all-day event.
    #[serde(default)]
    pub all_day: Option<bool>,
    /// Optional description.
    pub description: Option<String>,
    /// Optional location.
    pub location: Option<String>,
    /// Optional repeat rule.
    pub repeat: Option<oxios_calendar::Repeat>,
    /// Reminder offsets in minutes before the event.
    pub reminder_minutes: Option<Vec<u32>>,
    /// Optional path of a linked knowledge note.
    pub note_path: Option<String>,
}

/// Request body for updating an existing event.
///
/// All fields are optional. `None` means "don't change". Inner `Option` fields
/// (like `description`) use `Some(None)` to clear the value.
#[derive(Debug, Deserialize)]
pub struct UpdateEventRequest {
    /// New title.
    pub title: Option<String>,
    /// New start time (ISO 8601).
    pub start: Option<String>,
    /// New end time (ISO 8601).
    pub end: Option<String>,
    /// Toggle all-day.
    pub all_day: Option<bool>,
    /// Set or clear description. `Some(None)` clears it.
    pub description: Option<Option<String>>,
    /// Set or clear location. `Some(None)` clears it.
    pub location: Option<Option<String>>,
    /// Set or clear repeat rule. `Some(None)` clears it.
    pub repeat: Option<Option<oxios_calendar::Repeat>>,
    /// Replace reminder minutes.
    pub reminder_minutes: Option<Vec<u32>>,
    /// Set or clear linked knowledge note. `Some(None)` (JSON `null`) clears it.
    #[serde(default, deserialize_with = "deserialize_some")]
    pub note_path: Option<Option<String>>,
}

/// Query parameters for the search endpoint.
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    /// Search query string.
    pub q: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse an ISO 8601 datetime string into `DateTime<Utc>`.
fn parse_dt(s: &str, field: &str) -> Result<DateTime<Utc>, AppError> {
    s.parse::<DateTime<Utc>>()
        .map_err(|e| AppError::BadRequest(format!("Invalid {field}: {e}")))
}

/// Extract the calendar API, returning 503 if unavailable.
macro_rules! calendar_api {
    ($state:expr) => {
        $state
            .kernel
            .calendar
            .as_ref()
            .ok_or_else(|| AppError::ServiceUnavailable("Calendar subsystem not available".into()))
    };
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/calendar/events?from=...&to=... — List events in a date range.
pub(crate) async fn handle_calendar_events(
    state: State<Arc<AppState>>,
    Query(params): Query<DateRangeParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let api = calendar_api!(state)?;
    let from = parse_dt(&params.from, "from")?;
    let to = parse_dt(&params.to, "to")?;

    let events = api
        .list(from, to)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(serde_json::json!({ "events": events })))
}

/// GET /api/calendar/events/{uid} — Get a single event.
pub(crate) async fn handle_calendar_event_get(
    state: State<Arc<AppState>>,
    Path(uid): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let api = calendar_api!(state)?;

    let event = api
        .get(&uid)
        .await
        .map_err(|e| AppError::NotFound(e.to_string()))?;

    Ok(Json(
        serde_json::to_value(event).expect("serializable value"),
    ))
}

/// POST /api/calendar/events — Create a new event.
pub(crate) async fn handle_calendar_event_create(
    state: State<Arc<AppState>>,
    Json(body): Json<CreateEventRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let api = calendar_api!(state)?;

    let draft = EventDraft {
        title: body.title,
        start: parse_dt(&body.start, "start")?,
        end: parse_dt(&body.end, "end")?,
        all_day: body.all_day.unwrap_or(false),
        description: body.description,
        location: body.location,
        repeat: body.repeat,
        reminder_minutes: body.reminder_minutes.unwrap_or_default(),
        source: oxios_calendar::EventSource::User,
        note_path: body.note_path,
    };

    let result = api
        .create(draft)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(
        serde_json::to_value(result).expect("serializable value"),
    ))
}

/// PUT /api/calendar/events/{uid} — Update an existing event.
pub(crate) async fn handle_calendar_event_update(
    state: State<Arc<AppState>>,
    Path(uid): Path<String>,
    Json(body): Json<UpdateEventRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let api = calendar_api!(state)?;

    let mut patch = EventPatch::default();
    if let Some(title) = body.title {
        patch.title = Some(title);
    }
    if let Some(start) = body.start {
        patch.start = Some(parse_dt(&start, "start")?);
    }
    if let Some(end) = body.end {
        patch.end = Some(parse_dt(&end, "end")?);
    }
    if let Some(all_day) = body.all_day {
        patch.all_day = Some(all_day);
    }
    if let Some(desc) = body.description {
        patch.description = Some(desc);
    }
    if let Some(loc) = body.location {
        patch.location = Some(loc);
    }
    if let Some(rep) = body.repeat {
        patch.repeat = Some(rep);
    }
    if let Some(reminders) = body.reminder_minutes {
        patch.reminder_minutes = Some(reminders);
    }
    if let Some(np) = body.note_path {
        patch.note_path = Some(np);
    }

    let result = api
        .update(&uid, patch)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(
        serde_json::to_value(result).expect("serializable value"),
    ))
}

/// DELETE /api/calendar/events/{uid} — Delete an event.
pub(crate) async fn handle_calendar_event_delete(
    state: State<Arc<AppState>>,
    Path(uid): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let api = calendar_api!(state)?;

    api.delete(&uid)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(serde_json::json!({ "deleted": uid })))
}

/// GET /api/calendar/search?q=... — Search events.
pub(crate) async fn handle_calendar_search(
    state: State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let api = calendar_api!(state)?;

    let events = api
        .search(&params.q)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(serde_json::json!({ "events": events })))
}

/// GET /api/calendar/freebusy?from=...&to=... — Free/busy slots.
pub(crate) async fn handle_calendar_freebusy(
    state: State<Arc<AppState>>,
    Query(params): Query<DateRangeParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let api = calendar_api!(state)?;
    let from = parse_dt(&params.from, "from")?;
    let to = parse_dt(&params.to, "to")?;

    let slots = api
        .freebusy(from, to)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(serde_json::json!({ "slots": slots })))
}

/// Query parameters for the by-note endpoint.
#[derive(Debug, Deserialize)]
pub(crate) struct NotePathParams {
    /// Knowledge note path to look up.
    pub path: String,
}

/// GET /api/calendar/by-note?path=... — Events linked to a knowledge note.
pub(crate) async fn handle_calendar_by_note(
    state: State<Arc<AppState>>,
    Query(params): Query<NotePathParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let api = calendar_api!(state)?;

    let events = api
        .list_by_note_path(&params.path)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(serde_json::json!({ "events": events })))
}
