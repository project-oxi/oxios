//! Legacy cron job model (retired scheduler, kept for state migration).
//!
//! The `CronScheduler` was retired in favor of the RFC-043 task system:
//! recurring goals live as tasks with `automation_mode = schedule`. This
//! module keeps the `CronJob` wire shape so persisted state files
//! (`state_store: cron/jobs.json`) still deserialize during the one-time
//! boot migration. See [`crate::task::migrate`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::Priority;

/// Source of a legacy cron job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum JobSource {
    /// Defined in config.toml.
    Config,
    /// Created via API.
    #[default]
    Api,
}

/// A legacy cron job definition (wire-compatible with the retired scheduler's
/// persisted `cron/jobs.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    /// Unique job identifier.
    pub id: Uuid,
    /// Human-readable job name.
    pub name: String,
    /// Cron expression (e.g. "0 */6 * * *").
    pub schedule: String,
    /// Goal description for the agent.
    pub goal: String,
    /// Constraints on agent behavior.
    #[serde(default)]
    pub constraints: Vec<String>,
    /// Criteria that must be met for success.
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    /// Toolchain preset name.
    #[serde(default = "default_toolchain")]
    pub toolchain: String,
    /// Job priority.
    #[serde(default)]
    pub priority: Priority,
    /// Whether the job was active.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Timestamp of the last execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<DateTime<Utc>>,
    /// Timestamp of the next scheduled execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_run: Option<DateTime<Utc>>,
    /// Number of times this job ran.
    #[serde(default)]
    pub run_count: u64,
    /// Summary of the last execution result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_result: Option<String>,
    /// Whether the last execution succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success: Option<bool>,
    /// How the job was created.
    #[serde(default)]
    pub source: JobSource,
}

fn default_toolchain() -> String {
    "default".into()
}

fn default_true() -> bool {
    true
}

impl CronJob {
    /// Create a new legacy job record from raw fields.
    pub fn new(name: String, schedule: String, goal: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            schedule,
            goal,
            constraints: vec![],
            acceptance_criteria: vec![],
            toolchain: default_toolchain(),
            priority: Priority::default(),
            enabled: true,
            last_run: None,
            next_run: None,
            run_count: 0,
            last_result: None,
            last_success: None,
            source: JobSource::Api,
        }
    }
}
