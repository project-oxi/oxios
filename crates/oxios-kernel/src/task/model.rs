// Task model — SQLite-backed task lifecycle management (RFC-043)
// Ported from LobeHub's builtin-tool-task system.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Enums ──

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Backlog,
    Scheduled,
    Running,
    Paused,
    Completed,
    Failed,
    Canceled,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Backlog => write!(f, "backlog"),
            Self::Scheduled => write!(f, "scheduled"),
            Self::Running => write!(f, "running"),
            Self::Paused => write!(f, "paused"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Canceled => write!(f, "canceled"),
        }
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "backlog" => Ok(Self::Backlog),
            "scheduled" => Ok(Self::Scheduled),
            "running" => Ok(Self::Running),
            "paused" => Ok(Self::Paused),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "canceled" => Ok(Self::Canceled),
            other => Err(format!("unknown task status: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskAutomationMode {
    Schedule,
    Heartbeat,
}

impl std::fmt::Display for TaskAutomationMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Schedule => write!(f, "schedule"),
            Self::Heartbeat => write!(f, "heartbeat"),
        }
    }
}

impl std::str::FromStr for TaskAutomationMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "schedule" => Ok(Self::Schedule),
            "heartbeat" => Ok(Self::Heartbeat),
            other => Err(format!("unknown automation mode: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskRunTrigger {
    Manual,
    Schedule,
    Heartbeat,
}

impl std::fmt::Display for TaskRunTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Manual => write!(f, "manual"),
            Self::Schedule => write!(f, "schedule"),
            Self::Heartbeat => write!(f, "heartbeat"),
        }
    }
}

impl std::str::FromStr for TaskRunTrigger {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "manual" => Ok(Self::Manual),
            "schedule" => Ok(Self::Schedule),
            "heartbeat" => Ok(Self::Heartbeat),
            other => Err(format!("unknown run trigger: {other}")),
        }
    }
}

// ── Core structs ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub identifier: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub instruction: String,
    #[serde(default)]
    pub status: TaskStatus,
    #[serde(default)]
    pub priority: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee_agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by_agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automation_mode: Option<TaskAutomationMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heartbeat_interval_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_executions: Option<u32>,
    #[serde(default)]
    pub execution_count: u32,
    pub verify_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_requirement: Option<String>,
    #[serde(default = "default_verify_iterations")]
    pub verify_max_iterations: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_verifier_agent_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default)]
    pub consecutive_failures: u32,
    /// Dependencies: list of task identifiers this task depends on.
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Arbitrary context metadata (lifecycle audit, origin info, etc.)
    #[serde(default)]
    pub context: HashMap<String, serde_json::Value>,
}

fn default_verify_iterations() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskComment {
    pub id: String,
    pub task_id: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_session_id: Option<String>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRun {
    pub id: String,
    pub task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub trigger: TaskRunTrigger,
    #[serde(default = "default_run_status")]
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_content: Option<String>,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<u64>,
}

fn default_run_status() -> String {
    "running".to_string()
}

// ── Create params ──

#[derive(Debug, Clone, Deserialize)]
pub struct CreateTaskParams {
    pub name: String,
    pub instruction: String,
    #[serde(default)]
    pub identifier: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub priority: Option<u8>,
    #[serde(default)]
    pub parent_task_id: Option<String>,
    #[serde(default)]
    pub assignee_agent_id: Option<String>,
    #[serde(default)]
    pub created_by_agent_id: Option<String>,
    #[serde(default)]
    pub created_by_session_id: Option<String>,
    #[serde(default)]
    pub sort_order: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ListTasksParams {
    #[serde(default)]
    pub statuses: Option<Vec<String>>,
    #[serde(default)]
    pub assignee_agent_id: Option<String>,
    #[serde(default)]
    pub parent_task_id: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SetScheduleParams {
    pub automation_mode: Option<TaskAutomationMode>,
    pub schedule_pattern: Option<String>,
    pub schedule_timezone: Option<String>,
    pub heartbeat_interval_secs: Option<u64>,
    pub max_executions: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SetVerifyParams {
    pub enabled: Option<bool>,
    pub requirement: Option<String>,
    pub max_iterations: Option<u32>,
    pub verifier_agent_id: Option<String>,
}

/// Partial task update — `None` fields are left unchanged (RFC-043).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct UpdateTaskParams {
    pub name: Option<String>,
    pub description: Option<String>,
    pub instruction: Option<String>,
    pub priority: Option<u8>,
    pub sort_order: Option<f64>,
    pub parent_task_id: Option<String>,
    pub assignee_agent_id: Option<String>,
}

// ── Helpers ──

impl Task {
    /// Deterministic slug base from a name — lowercase, non-alphanumerics
    /// collapsed to `-`, trimmed, capped at 48 chars. No random suffix:
    /// callers that need uniqueness add one (see [`Self::slug_from_name`]);
    /// callers that need dedup (cron migration) rely on determinism.
    pub fn slug_base(name: &str) -> String {
        let slug: String = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        let slug = slug.trim_matches('-').to_string();
        slug.chars().take(48).collect()
    }

    /// Generate a slug-style identifier from a name (slug base + random
    /// suffix for uniqueness).
    pub fn slug_from_name(name: &str) -> String {
        // Add short random suffix for uniqueness
        let suffix = &uuid::Uuid::new_v4().to_string()[..8];
        format!("{}-{suffix}", Self::slug_base(name))
    }

    /// Check if this task should be auto-run now based on schedule/heartbeat.
    pub fn should_run_now(&self) -> bool {
        if self.automation_mode.is_none() {
            return false;
        }
        if let Some(ref next) = self.next_run_at
            && let Ok(next_dt) = chrono::DateTime::parse_from_rfc3339(next)
        {
            return next_dt.with_timezone(&Utc) <= Utc::now();
        }
        false
    }
    /// Check if the task has hit max_executions.
    pub fn is_exhausted(&self) -> bool {
        self.max_executions
            .map(|max| self.execution_count >= max)
            .unwrap_or(false)
    }
}
