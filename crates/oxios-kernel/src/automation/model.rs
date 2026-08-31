// Automation model — the web IA data contract for the automation domain.
//
// Major-version break from the RFC-043 task model: Task-planning fields
// (identifier, priority, sort_order, parent_task_id, assignee/creator,
// dependencies, comments) are deleted without serde aliases. New scoped
// bindings: `persona_id`, `project_id`, `brain_space` — `None` means
// projectless/default-bound, never a string sentinel.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

// ── Enums ──

/// Lifecycle state of an automation definition (spec §8.7).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutomationStatus {
    /// Runnable: manual automations on demand, cron/heartbeat on schedule.
    #[default]
    Active,
    /// Explicitly paused by the user; excluded from scheduling.
    Paused,
    /// `max_executions` reached; excluded from scheduling.
    Exhausted,
    /// Last run failed; excluded from scheduling until reactivated.
    ///
    /// Policy is trigger-aware: only a failed SCHEDULED run (cron/heartbeat)
    /// — or a failure of a manual-triggered automation — sets this state and
    /// clears `next_run_at`. A failed MANUAL run of a scheduled automation
    /// records `last_error` but leaves the status and schedule intact, so an
    /// on-demand attempt can never kill a recurring schedule.
    Failed,
}

impl std::fmt::Display for AutomationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Paused => write!(f, "paused"),
            Self::Exhausted => write!(f, "exhausted"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

impl std::str::FromStr for AutomationStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "paused" => Ok(Self::Paused),
            "exhausted" => Ok(Self::Exhausted),
            "failed" => Ok(Self::Failed),
            other => Err(format!("unknown automation status: {other}")),
        }
    }
}

/// What can start an automation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutomationTrigger {
    /// Started only by an explicit request.
    #[default]
    Manual,
    /// Started by a cron pattern (`cron_pattern` required).
    Cron,
    /// Started on a fixed interval (`heartbeat_interval_secs` required).
    Heartbeat,
}

impl std::fmt::Display for AutomationTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Manual => write!(f, "manual"),
            Self::Cron => write!(f, "cron"),
            Self::Heartbeat => write!(f, "heartbeat"),
        }
    }
}

impl std::str::FromStr for AutomationTrigger {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "manual" => Ok(Self::Manual),
            "cron" => Ok(Self::Cron),
            "heartbeat" => Ok(Self::Heartbeat),
            other => Err(format!("unknown automation trigger: {other}")),
        }
    }
}

/// What actually started a specific run. Same value set as
/// [`AutomationTrigger`]; a distinct type so a run's trigger survives
/// later edits to the definition.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationRunTrigger {
    Manual,
    Cron,
    Heartbeat,
}

impl std::fmt::Display for AutomationRunTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Manual => write!(f, "manual"),
            Self::Cron => write!(f, "cron"),
            Self::Heartbeat => write!(f, "heartbeat"),
        }
    }
}

impl std::str::FromStr for AutomationRunTrigger {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "manual" => Ok(Self::Manual),
            "cron" => Ok(Self::Cron),
            "heartbeat" => Ok(Self::Heartbeat),
            other => Err(format!("unknown run trigger: {other}")),
        }
    }
}

/// Terminal state of a specific run.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationRunStatus {
    /// Run row opened, execution in flight.
    Running,
    /// Execution finished successfully.
    Succeeded,
    /// Execution failed (provider error, timeout, or verify gate rejected).
    Failed,
    /// Execution was canceled before completion.
    Canceled,
}

impl std::fmt::Display for AutomationRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Succeeded => write!(f, "succeeded"),
            Self::Failed => write!(f, "failed"),
            Self::Canceled => write!(f, "canceled"),
        }
    }
}

impl std::str::FromStr for AutomationRunStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "canceled" => Ok(Self::Canceled),
            other => Err(format!("unknown run status: {other}")),
        }
    }
}

// ── Errors ──

/// A trigger configuration is self-contradictory (e.g. cron trigger without
/// a cron pattern).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {trigger} trigger configuration: {reason}")]
pub struct InvalidTriggerError {
    /// Trigger that failed validation.
    pub trigger: AutomationTrigger,
    /// What exactly is wrong.
    pub reason: String,
}

/// Validate trigger-specific configuration consistency.
///
/// - Manual: no `cron_pattern`, no `heartbeat_interval_secs`.
/// - Cron: `cron_pattern` required and must parse (5- or 6-field);
///   no `heartbeat_interval_secs`.
/// - Heartbeat: `heartbeat_interval_secs` required (>= 1); no `cron_pattern`.
///
/// `timezone` is accepted for every trigger (it qualifies cron evaluation)
/// and is therefore not part of this check.
pub fn validate_trigger_config(
    trigger: AutomationTrigger,
    cron_pattern: Option<&str>,
    heartbeat_interval_secs: Option<u64>,
) -> Result<(), InvalidTriggerError> {
    let invalid = |reason: String| InvalidTriggerError { trigger, reason };
    match trigger {
        AutomationTrigger::Manual => {
            if cron_pattern.is_some() {
                return Err(invalid(
                    "manual automations must not set cron_pattern".into(),
                ));
            }
            if heartbeat_interval_secs.is_some() {
                return Err(invalid(
                    "manual automations must not set heartbeat_interval_secs".into(),
                ));
            }
            Ok(())
        }
        AutomationTrigger::Cron => {
            if heartbeat_interval_secs.is_some() {
                return Err(invalid(
                    "cron automations must not set heartbeat_interval_secs".into(),
                ));
            }
            let pattern = cron_pattern
                .ok_or_else(|| invalid("cron automations require cron_pattern".into()))?;
            if cron_pattern_is_valid(pattern) {
                Ok(())
            } else {
                Err(invalid(format!("cron_pattern '{pattern}' does not parse")))
            }
        }
        AutomationTrigger::Heartbeat => {
            if cron_pattern.is_some() {
                return Err(invalid(
                    "heartbeat automations must not set cron_pattern".into(),
                ));
            }
            match heartbeat_interval_secs {
                None => Err(invalid(
                    "heartbeat automations require heartbeat_interval_secs".into(),
                )),
                Some(0) => Err(invalid(
                    "heartbeat automations require heartbeat_interval_secs >= 1".into(),
                )),
                Some(_) => Ok(()),
            }
        }
    }
}

// ── Cron helpers ──

/// Normalize a cron pattern: 5-field (Linux cron) expressions get a seconds
/// field prepended, matching `CronScheduler::normalize_expr`.
pub(crate) fn normalize_cron_pattern(pattern: &str) -> String {
    let fields: Vec<&str> = pattern.split_whitespace().collect();
    if fields.len() == 5 {
        format!("0 {pattern}")
    } else {
        pattern.to_string()
    }
}

/// Compute the next cron fire time after `after` as an RFC 3339 string.
pub(crate) fn cron_next(pattern: &str, after: &DateTime<Utc>) -> anyhow::Result<String> {
    let normalized = normalize_cron_pattern(pattern);
    let schedule = cron::Schedule::from_str(&normalized)
        .map_err(|e| anyhow::anyhow!("Invalid cron expression '{pattern}': {e}"))?;
    let next = schedule
        .after(after)
        .next()
        .ok_or_else(|| anyhow::anyhow!("No future fire time for cron '{pattern}'"))?;
    Ok(next.to_rfc3339())
}

/// Whether a cron pattern parses under [`normalize_cron_pattern`].
pub(crate) fn cron_pattern_is_valid(pattern: &str) -> bool {
    cron::Schedule::from_str(&normalize_cron_pattern(pattern)).is_ok()
}

// ── Core structs ──

/// Verify-gate configuration evaluated after each run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationVerifyConfig {
    /// Arm the gate: successful runs are re-checked by a verifier pass.
    #[serde(default)]
    pub enabled: bool,
    /// Acceptance criterion the verifier checks; defaults to the instruction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
    /// Max verify/repair attempts before the run is failed.
    #[serde(default = "default_verify_iterations")]
    pub max_iterations: u32,
}

impl Default for AutomationVerifyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            requirement: None,
            max_iterations: default_verify_iterations(),
        }
    }
}

fn default_verify_iterations() -> u32 {
    3
}

/// Immutable execution context captured when a run opens.
///
/// Contains ONLY: persona_id, project_id, brain_space, trigger, the
/// verification setting, and the instruction. The runner uses this snapshot
/// for the whole execution; it may not reload a changed Automation mid-run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationContextSnapshot {
    /// Persona bound to the run (captured at start).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_id: Option<String>,
    /// Project bound to the run (captured at start); `None` = projectless.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Brain space bound to the run (captured at start).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brain_space: Option<String>,
    /// Trigger that started this specific run.
    pub trigger: AutomationRunTrigger,
    /// Verification setting captured at start.
    #[serde(default)]
    pub verify: AutomationVerifyConfig,
    /// Goal instruction executed for this run.
    pub instruction: String,
}

/// An automation definition: a named instruction that can be run manually
/// or on a cron/heartbeat schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Automation {
    pub id: String,
    pub name: String,
    pub instruction: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub trigger: AutomationTrigger,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron_pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heartbeat_interval_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_executions: Option<u32>,
    #[serde(default)]
    pub execution_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brain_space: Option<String>,
    #[serde(default)]
    pub verify: AutomationVerifyConfig,
    #[serde(default)]
    pub status: AutomationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// One recorded execution of an automation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationRun {
    pub id: String,
    pub automation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub trigger: AutomationRunTrigger,
    pub status: AutomationRunStatus,
    pub context_snapshot: AutomationContextSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<u64>,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

// ── Patch-field deserialization ──

/// Serde helper for PATCH fields with three states:
///
/// - JSON key absent → `None` (leave the stored value unchanged)
/// - JSON key present as `null` → `Some(None)` (clear the stored value)
/// - JSON key present with a value → `Some(Some(v))` (replace)
///
/// Hand-rolled (no serde_with dependency). Pair with `#[serde(default)]` so
/// an absent key skips the deserializer entirely.
pub(crate) mod double_option {
    use serde::Deserialize;
    use serde::de::{Deserializer, Visitor};
    use std::fmt;
    use std::marker::PhantomData;

    pub(crate) fn deserialize<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        struct DoubleOptionVisitor<T>(PhantomData<T>);

        impl<'de, T> Visitor<'de> for DoubleOptionVisitor<T>
        where
            T: Deserialize<'de>,
        {
            type Value = Option<Option<T>>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a value or null")
            }

            // Explicit JSON null → clear.
            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(Some(None))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(Some(None))
            }

            // Present value → replace.
            fn visit_some<D2>(self, d: D2) -> Result<Self::Value, D2::Error>
            where
                D2: Deserializer<'de>,
            {
                T::deserialize(d).map(|v| Some(Some(v)))
            }
        }

        deserializer.deserialize_option(DoubleOptionVisitor(PhantomData))
    }
}

// ── Create/update params ──

/// Create an automation. Trigger-specific fields are validated
/// (see [`validate_trigger_config`]).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAutomationParams {
    pub name: String,
    pub instruction: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub trigger: AutomationTrigger,
    #[serde(default)]
    pub cron_pattern: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub heartbeat_interval_secs: Option<u64>,
    #[serde(default)]
    pub max_executions: Option<u32>,
    #[serde(default)]
    pub persona_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub brain_space: Option<String>,
    #[serde(default)]
    pub verify: AutomationVerifyConfig,
}

/// List filter — `None` fields are left unfiltered.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ListAutomationsParams {
    /// Exact status match (parsed as [`AutomationStatus`]).
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
}

/// Set/replace the trigger configuration. Never changes `status`: Active
/// definitions get a recomputed `next_run_at`; paused/failed/exhausted
/// definitions go dormant (`next_run_at = NULL`) until reactivated via
/// `update_status(Active)`, whose exhausted-guard still applies.
///
/// Patch fields use the double-option rule: absent JSON key = preserve the
/// stored value, explicit `null` = clear.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTriggerParams {
    pub trigger: AutomationTrigger,
    #[serde(default)]
    pub cron_pattern: Option<String>,
    #[serde(default, deserialize_with = "double_option::deserialize")]
    pub timezone: Option<Option<String>>,
    #[serde(default)]
    pub heartbeat_interval_secs: Option<u64>,
    #[serde(default, deserialize_with = "double_option::deserialize")]
    pub max_executions: Option<Option<u32>>,
}

/// Partial verify-gate update. Patch fields use the double-option rule:
/// absent JSON key = keep, explicit `null` = clear (`requirement` only —
/// enabled/max_iterations are never null on the row).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SetVerifyParams {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default, deserialize_with = "double_option::deserialize")]
    pub requirement: Option<Option<String>>,
    #[serde(default)]
    pub max_iterations: Option<u32>,
}

/// Partial definition update. Patch fields use the double-option rule:
/// absent JSON key = leave unchanged, explicit `null` = clear (set to NULL).
/// `name`/`instruction` are plain options — they are never cleared.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAutomationParams {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "double_option::deserialize")]
    pub description: Option<Option<String>>,
    #[serde(default)]
    pub instruction: Option<String>,
    #[serde(default, deserialize_with = "double_option::deserialize")]
    pub persona_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option::deserialize")]
    pub project_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option::deserialize")]
    pub brain_space: Option<Option<String>>,
}

impl Automation {
    /// Whether `max_executions` has been reached.
    pub fn is_exhausted(&self) -> bool {
        self.max_executions
            .is_some_and(|max| self.execution_count >= max)
    }

    /// Whether the scheduler may pick this automation up: only cron/
    /// heartbeat automations that are `Active` and have not hit
    /// `max_executions`. `Paused`/`Exhausted`/`Failed` are excluded.
    pub fn is_schedulable(&self) -> bool {
        self.trigger != AutomationTrigger::Manual
            && self.status == AutomationStatus::Active
            && !self.is_exhausted()
    }

    /// Whether this automation is due to fire on the current tick:
    /// schedulable and its `next_run_at` has passed.
    pub fn is_due_now(&self) -> bool {
        self.is_schedulable()
            && self
                .next_run_at
                .as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .is_some_and(|t| t.with_timezone(&Utc) <= Utc::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn automation_with(trigger: AutomationTrigger, status: AutomationStatus) -> Automation {
        Automation {
            id: "a-1".into(),
            name: "nightly digest".into(),
            instruction: "compile the digest".into(),
            description: None,
            trigger,
            cron_pattern: None,
            timezone: None,
            heartbeat_interval_secs: None,
            max_executions: None,
            execution_count: 0,
            persona_id: None,
            project_id: None,
            brain_space: None,
            verify: AutomationVerifyConfig::default(),
            status,
            next_run_at: None,
            last_run_at: None,
            last_error: None,
            created_at: "2026-08-31T00:00:00+00:00".into(),
            updated_at: "2026-08-31T00:00:00+00:00".into(),
        }
    }

    // ── Required: manual/cron/heartbeat validation ──

    #[test]
    fn trigger_validation_accepts_well_formed_configs() {
        validate_trigger_config(AutomationTrigger::Manual, None, None).unwrap();
        validate_trigger_config(AutomationTrigger::Cron, Some("0 9 * * *"), None).unwrap();
        validate_trigger_config(AutomationTrigger::Cron, Some("0 9 * * *"), None).unwrap();
        validate_trigger_config(AutomationTrigger::Heartbeat, None, Some(3600)).unwrap();
    }

    #[test]
    fn trigger_validation_rejects_contradictory_configs() {
        // Manual with scheduling fields.
        assert!(
            validate_trigger_config(AutomationTrigger::Manual, Some("0 9 * * *"), None).is_err()
        );
        assert!(validate_trigger_config(AutomationTrigger::Manual, None, Some(60)).is_err());
        // Cron without a pattern, with a heartbeat, or with a broken pattern.
        assert!(validate_trigger_config(AutomationTrigger::Cron, None, None).is_err());
        assert!(
            validate_trigger_config(AutomationTrigger::Cron, Some("0 9 * * *"), Some(60)).is_err()
        );
        assert!(
            validate_trigger_config(AutomationTrigger::Cron, Some("not a cron"), None).is_err()
        );
        // Heartbeat without an interval, with zero, or with a cron pattern.
        assert!(validate_trigger_config(AutomationTrigger::Heartbeat, None, None).is_err());
        assert!(validate_trigger_config(AutomationTrigger::Heartbeat, None, Some(0)).is_err());
        assert!(
            validate_trigger_config(AutomationTrigger::Heartbeat, Some("0 9 * * *"), Some(60))
                .is_err()
        );
    }

    #[test]
    fn validation_errors_name_the_trigger_and_reason() {
        let err = validate_trigger_config(AutomationTrigger::Cron, None, None).unwrap_err();
        assert_eq!(err.trigger, AutomationTrigger::Cron);
        assert!(err.to_string().contains("cron_pattern"));
    }

    // ── Required: absent project ID (None is valid, no sentinel) ──

    #[test]
    fn absent_project_id_round_trips_as_none_without_sentinel() {
        let a = automation_with(AutomationTrigger::Manual, AutomationStatus::Active);
        // All scoped bindings absent — the valid projectless shape.
        assert!(a.project_id.is_none() && a.persona_id.is_none() && a.brain_space.is_none());

        let json = serde_json::to_value(&a).unwrap();
        // No sentinel strings: the fields are absent from the payload, and
        // never serialized as "" or a placeholder.
        for key in ["projectId", "personaId", "brainSpace"] {
            assert!(
                json.get(key).map(|v| v == "None") != Some(true),
                "sentinel leaked for {key}"
            );
            assert_ne!(
                json.get(key),
                Some(&serde_json::json!("")),
                "{key} empty-string sentinel"
            );
        }

        // Deserializing a payload without the fields restores None.
        let plain = serde_json::json!({
            "id": "a-2",
            "name": "n",
            "instruction": "i",
            "trigger": "manual",
            "createdAt": "2026-08-31T00:00:00+00:00",
            "updatedAt": "2026-08-31T00:00:00+00:00",
        });
        let parsed: Automation = serde_json::from_value(plain).unwrap();
        assert_eq!(parsed.project_id, None);
        assert_eq!(parsed.persona_id, None);
        assert_eq!(parsed.brain_space, None);
        assert_eq!(parsed.trigger, AutomationTrigger::Manual);
        assert_eq!(parsed.status, AutomationStatus::Active);
    }

    // ── Serde value sets (contract §8.7) ──

    #[test]
    fn status_and_trigger_serde_values_are_exact() {
        assert_eq!(
            serde_json::to_value(AutomationStatus::Exhausted).unwrap(),
            "exhausted"
        );
        assert_eq!(
            serde_json::to_value(AutomationStatus::Paused).unwrap(),
            "paused"
        );
        assert_eq!(
            serde_json::to_value(AutomationRunStatus::Succeeded).unwrap(),
            "succeeded"
        );
        assert_eq!(
            serde_json::to_value(AutomationRunStatus::Canceled).unwrap(),
            "canceled"
        );
        assert_eq!(
            serde_json::to_value(AutomationTrigger::Cron).unwrap(),
            "cron"
        );
        assert_eq!(
            serde_json::to_value(AutomationRunTrigger::Heartbeat).unwrap(),
            "heartbeat"
        );
        assert_eq!(
            "failed".parse::<AutomationStatus>().unwrap(),
            AutomationStatus::Failed
        );
    }

    // ── Fix round 2: double-option patch deserialization ──

    #[test]
    fn double_option_fields_distinguish_absent_null_and_value() {
        // Absent key → None (leave unchanged).
        let absent: UpdateAutomationParams = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(absent.project_id, None);
        assert_eq!(absent.description, None);

        // Explicit null → Some(None) (clear).
        let cleared: UpdateAutomationParams = serde_json::from_value(serde_json::json!({
            "projectId": null,
            "personaId": null,
            "brainSpace": null,
            "description": null,
        }))
        .unwrap();
        assert_eq!(cleared.project_id, Some(None));
        assert_eq!(cleared.persona_id, Some(None));
        assert_eq!(cleared.brain_space, Some(None));
        assert_eq!(cleared.description, Some(None));

        // Present value → Some(Some(v)) (replace).
        let set: UpdateAutomationParams = serde_json::from_value(serde_json::json!({
            "projectId": "proj-1",
            "description": "note",
        }))
        .unwrap();
        assert_eq!(set.project_id, Some(Some("proj-1".into())));
        assert_eq!(set.description, Some(Some("note".into())));

        // Same rule on SetTriggerParams timezone/maxExecutions and
        // SetVerifyParams requirement.
        let trigger: SetTriggerParams = serde_json::from_value(serde_json::json!({
            "trigger": "cron",
            "cronPattern": "0 9 * * *",
            "timezone": null,
        }))
        .unwrap();
        assert_eq!(trigger.timezone, Some(None));
        assert_eq!(trigger.max_executions, None);

        let verify: SetVerifyParams =
            serde_json::from_value(serde_json::json!({ "requirement": null })).unwrap();
        assert_eq!(verify.requirement, Some(None));
        assert_eq!(verify.enabled, None);
    }

    // ── Snapshot contract: ONLY the six fields, immutable semantics ──

    #[test]
    fn context_snapshot_carries_exactly_the_contract_fields() {
        let snapshot = AutomationContextSnapshot {
            persona_id: Some("p".into()),
            project_id: None,
            brain_space: Some("bs".into()),
            trigger: AutomationRunTrigger::Cron,
            verify: AutomationVerifyConfig {
                enabled: true,
                requirement: Some("must include BANANA".into()),
                max_iterations: 2,
            },
            instruction: "do the thing".into(),
        };
        let json = serde_json::to_value(&snapshot).unwrap();
        let obj = json.as_object().unwrap();
        let mut keys: Vec<_> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "brainSpace",
                "instruction",
                "personaId",
                "trigger",
                "verify"
            ]
        );
        assert_eq!(obj["trigger"], "cron");
        assert_eq!(obj["verify"]["maxIterations"], 2);
        // Round trip preserves values (and the absent project id stays None).
        let back: AutomationContextSnapshot = serde_json::from_value(json).unwrap();
        assert_eq!(back, snapshot);
        assert!(back.project_id.is_none());
    }

    // ── Required: paused/exhausted exclusion (model level) ──

    #[test]
    fn only_active_cron_or_heartbeat_automations_are_schedulable() {
        let cron = automation_with(AutomationTrigger::Cron, AutomationStatus::Active);
        assert!(cron.is_schedulable());

        // Manual is never scheduler-driven.
        assert!(
            !automation_with(AutomationTrigger::Manual, AutomationStatus::Active).is_schedulable()
        );
        // Paused / exhausted / failed are excluded.
        assert!(
            !automation_with(AutomationTrigger::Cron, AutomationStatus::Paused).is_schedulable()
        );
        assert!(
            !automation_with(AutomationTrigger::Cron, AutomationStatus::Failed).is_schedulable()
        );

        let mut exhausted = automation_with(AutomationTrigger::Heartbeat, AutomationStatus::Active);
        exhausted.max_executions = Some(2);
        exhausted.execution_count = 2;
        assert!(exhausted.is_exhausted());
        assert!(!exhausted.is_schedulable());
    }

    #[test]
    fn is_due_now_requires_a_past_next_run() {
        let mut due = automation_with(AutomationTrigger::Cron, AutomationStatus::Active);
        due.next_run_at = Some((Utc::now() - chrono::Duration::minutes(1)).to_rfc3339());
        assert!(due.is_due_now());

        due.next_run_at = Some((Utc::now() + chrono::Duration::minutes(5)).to_rfc3339());
        assert!(!due.is_due_now());

        due.status = AutomationStatus::Paused;
        due.next_run_at = Some((Utc::now() - chrono::Duration::minutes(1)).to_rfc3339());
        assert!(!due.is_due_now(), "paused automations are never due");
    }
}
