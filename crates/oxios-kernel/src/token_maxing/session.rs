//! TokenMaxingSession + report (RFC-031 §8).
//!
//! One session per activation window (or manual run). Persisted per run via
//! `StateStore::save_json("token-maxing", &id, &session)` and surfaced by the
//! report API. The session records what ran, on which provider/model, how much
//! quota was burned, and why the run ended.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::quota_tracker::QuotaTrackerSnapshot;

/// A token-maxing activation window `[start, end)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaxingWindow {
    /// When the burn window opens (inclusive).
    pub start: DateTime<Utc>,
    /// When the burn window closes (exclusive).
    pub end: DateTime<Utc>,
}

/// How a session was activated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MaxingStart {
    /// A scheduled time window (cron-derived or explicit start/end).
    Scheduled(MaxingWindow),
    /// A manual toggle — runs until stopped or work is exhausted.
    Manual,
}

/// Where a planned task was sourced from (RFC-031 §7).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskSource {
    /// An autonomous-eligible skill (frontmatter `autonomous: true`).
    Skill,
    /// A registered project (read-mostly review task).
    Project,
    /// A recurring pattern derived from session history (Source C).
    Recurring,
}

/// One drained window on one provider (report fidelity, RFC-031 §11.5).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderWindowRecord {
    pub started: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

/// Per-provider rollup within a session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderSessionRecord {
    pub provider: String,
    pub models_used: Vec<String>,
    pub tasks_run: u64,
    pub tokens_consumed: u64,
    pub windows_drained: Vec<ProviderWindowRecord>,
}

/// One executed task and its outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub source: TaskSource,
    pub source_name: String,
    pub goal: String,
    pub provider: String,
    pub model: String,
    pub success: bool,
    pub tokens: u64,
    pub duration_secs: f64,
    /// Truncated agent output (the per-task summary).
    pub summary: String,
}

/// Why the session ended. The report distinguishes these so the user can tell
/// "stopped: nothing to do" from "stopped: window ended" (RFC-031 §8).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The configured time window elapsed.
    WindowEnded,
    /// The planner returned `None` — no more eligible work.
    NoWork,
    /// The user requested a stop.
    Cancelled,
}

/// Session-wide totals.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionTotals {
    pub tasks: u64,
    pub tokens: u64,
    pub providers_fully_drained: u64,
    pub resets_observed: u64,
}

/// A complete token-maxing run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMaxingSession {
    pub id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub window: Option<MaxingWindow>,
    pub manual: bool,
    pub providers: Vec<ProviderSessionRecord>,
    pub tasks: Vec<TaskRecord>,
    pub totals: SessionTotals,
    pub stop_reason: Option<StopReason>,
}

impl TokenMaxingSession {
    /// Begin a new session.
    pub fn start(window: Option<MaxingWindow>, manual: bool) -> Self {
        let now = Utc::now();
        let id = format!("tm-{}", now.timestamp_millis());
        Self {
            id,
            started_at: now,
            ended_at: None,
            window,
            manual,
            providers: Vec::new(),
            tasks: Vec::new(),
            totals: SessionTotals::default(),
            stop_reason: None,
        }
    }

    /// Whether the window has not yet elapsed (always true for manual runs).
    pub fn within_window(&self) -> bool {
        match &self.window {
            Some(w) => Utc::now() < w.end,
            None => true,
        }
    }

    /// Record one task's outcome and roll it up into per-provider + total stats.
    #[allow(clippy::too_many_arguments)]
    pub fn record_task(
        &mut self,
        source: TaskSource,
        source_name: String,
        goal: String,
        provider: String,
        model: String,
        success: bool,
        tokens: u64,
        duration_secs: f64,
        summary: String,
    ) {
        // Per-provider rollup. Index/any-check first to avoid a double
        // borrow of self.providers (the find + push pattern borrows twice).
        if !self.providers.iter().any(|r| r.provider == provider) {
            self.providers.push(ProviderSessionRecord {
                provider: provider.clone(),
                ..Default::default()
            });
        }
        // Audit F-4: the just-pushed/ensured record must exist. If the
        // search invariant is broken, log loudly and skip the per-task
        // rollup rather than panicking (which under `panic=abort` would
        // kill the daemon). Changing the public signature to Result is
        // tracked separately.
        let rec = match self.providers.iter_mut().find(|r| r.provider == provider) {
            Some(r) => r,
            None => {
                tracing::error!(
                    provider = %provider,
                    "token_maxing: provider record missing after ensure — skipping per-task rollup"
                );
                return;
            }
        };
        rec.tasks_run += 1;
        rec.tokens_consumed += tokens;
        if !rec.models_used.contains(&model) {
            rec.models_used.push(model.clone());
        }

        self.tasks.push(TaskRecord {
            source,
            source_name,
            goal,
            provider,
            model,
            success,
            tokens,
            duration_secs,
            summary,
        });
        self.totals.tasks += 1;
        self.totals.tokens += tokens;
    }

    /// Mark the session as ended for `reason`.
    pub fn finalize(&mut self, reason: StopReason) {
        self.ended_at = Some(Utc::now());
        self.stop_reason = Some(reason);
    }
}

/// Live status for `GET /api/token-maxing/status` (RFC-031 §9).
#[derive(Debug, Clone, Serialize)]
pub struct MaxerStatus {
    pub running: bool,
    pub current_session_id: Option<String>,
    pub current_provider: Option<String>,
    pub current_task: Option<String>,
    pub manual: bool,
    pub window: Option<MaxingWindow>,
    pub tokens_this_session: u64,
    pub tasks_this_session: u64,
    pub providers: Vec<QuotaTrackerSnapshot>,
}
