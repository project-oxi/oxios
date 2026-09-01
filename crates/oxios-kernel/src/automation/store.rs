// Automation store — SQLite-backed CRUD for automations.
//
// Fresh schema (`automations` / `automation_runs`) in the store the
// assembler opens as `{workspace}/automations.db`. Major-version break:
// there is NO migration from tasks.db and no compatibility columns; the
// retired `tasks`/`task_runs` schema is simply not created here.
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::model::*;

/// Definition column order shared by every `SELECT` (see
/// [`map_automation_row`]).
const AUTOMATION_COLUMNS: &str = "id, name, description, instruction, \"trigger\", \
       cron_pattern, timezone, heartbeat_interval_secs, max_executions, execution_count, \
       persona_id, project_id, brain_space, verify_enabled, verify_requirement, \
       verify_max_iterations, status, next_run_at, last_run_at, last_error, \
       created_at, updated_at";

/// Run column order shared by every run `SELECT` (see [`map_run_row`]).
const RUN_COLUMNS: &str = "id, automation_id, session_id, \"trigger\", status, context_snapshot, \
       summary, result_content, error, cost_usd, tokens_used, started_at, completed_at";

/// SQLite-backed automation store.
pub struct AutomationStore {
    conn: Arc<Mutex<Connection>>,
}

impl AutomationStore {
    /// Create an AutomationStore from a raw connection. Schema is
    /// initialized on the connection *before* it is wrapped in the async
    /// mutex, so this constructor is safe to call from inside a Tokio
    /// runtime — no `blocking_lock` is involved.
    pub fn new(conn: Connection) -> Result<Self> {
        init_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create an AutomationStore from a database file path. The assembler
    /// passes `{workspace}/automations.db`.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open automation database: {path}"))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        Self::new(conn)
    }

    /// Create an in-memory AutomationStore (for tests).
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::new(conn)
    }

    /// Create an automation. Trigger-specific configuration is validated
    /// before the row is written; a cron/heartbeat automation gets its
    /// first `next_run_at` immediately.
    pub async fn create_automation(&self, params: CreateAutomationParams) -> Result<Automation> {
        validate_trigger_config(
            params.trigger,
            params.cron_pattern.as_deref(),
            params.heartbeat_interval_secs,
        )?;
        if params.max_executions == Some(0) {
            anyhow::bail!("max_executions must be at least 1 when set");
        }
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let next_run = match params.trigger {
            AutomationTrigger::Manual => None,
            AutomationTrigger::Cron => Some(cron_next(
                params
                    .cron_pattern
                    .as_deref()
                    .expect("validated: cron requires a pattern"),
                &now,
            )?),
            AutomationTrigger::Heartbeat => Some(
                (now + chrono::Duration::seconds(
                    params
                        .heartbeat_interval_secs
                        .expect("validated: heartbeat requires an interval")
                        as i64,
                ))
                .to_rfc3339(),
            ),
        };
        let now_rfc = now.to_rfc3339();
        {
            let conn = self.conn.lock().await;
            conn.execute(
                r#"INSERT INTO automations
                     (id, name, description, instruction, "trigger", cron_pattern, timezone,
                      heartbeat_interval_secs, max_executions, execution_count,
                      persona_id, project_id, brain_space,
                      verify_enabled, verify_requirement, verify_max_iterations,
                      status, next_run_at, created_at, updated_at)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11, ?12,
                           ?13, ?14, ?15, 'active', ?16, ?17, ?18)"#,
                params![
                    id,
                    params.name,
                    params.description,
                    params.instruction,
                    params.trigger.to_string(),
                    params.cron_pattern,
                    params.timezone,
                    params.heartbeat_interval_secs.map(|v| v as i64),
                    params.max_executions.map(|v| v as i64),
                    params.persona_id,
                    params.project_id,
                    params.brain_space,
                    params.verify.enabled as i64,
                    params.verify.requirement,
                    params.verify.max_iterations as i64,
                    next_run,
                    now_rfc,
                    now_rfc,
                ],
            )
            .context("insert automation")?;
        }
        // Lock released — safe to call another `&self` method.
        self.get_automation(&id).await
    }

    /// Fetch one automation by id. Errors when missing.
    pub async fn get_automation(&self, id: &str) -> Result<Automation> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(&format!(
            "SELECT {AUTOMATION_COLUMNS} FROM automations WHERE id = ?1"
        ))?;
        stmt.query_row(params![id], map_automation_row)
            .optional()?
            .with_context(|| format!("automation {id} not found"))
    }

    /// List automations, newest first; optional status filter.
    pub async fn list_automations(
        &self,
        list_params: ListAutomationsParams,
    ) -> Result<Vec<Automation>> {
        let conn = self.conn.lock().await;
        // Validate any status filter up front (unknown values are caller bugs).
        if let Some(status) = &list_params.status {
            AutomationStatus::from_str(status)
                .map_err(|e| anyhow::anyhow!("invalid status filter '{status}': {e}"))?;
        }
        let limit = list_params.limit.unwrap_or(100).min(500);
        let offset = list_params.offset.unwrap_or(0);
        let mut sql = format!("SELECT {AUTOMATION_COLUMNS} FROM automations WHERE 1=1");
        if list_params.status.is_some() {
            sql.push_str(" AND status = ?1");
        }
        sql.push_str(" ORDER BY created_at DESC, id LIMIT ?2 OFFSET ?3");
        let mut stmt = conn.prepare(&sql)?;
        let automations = stmt
            .query_map(
                params![list_params.status.as_deref(), limit, offset],
                map_automation_row,
            )?
            .filter_map(|r| r.ok())
            .collect();
        Ok(automations)
    }

    /// Delete an automation. Its runs cascade.
    pub async fn delete_automation(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute("DELETE FROM automations WHERE id = ?1", params![id])
            .context("delete automation")?;
        Ok(())
    }

    /// Set the definition status. Re-activating (`Active`) a cron/heartbeat
    /// automation recomputes `next_run_at` from now, so a stale or cleared
    /// schedule resumes correctly.
    pub async fn update_status(&self, id: &str, status: &AutomationStatus) -> Result<()> {
        let conn = self.conn.lock().await;
        let now = Utc::now();
        let now_rfc = now.to_rfc3339();
        if *status == AutomationStatus::Active {
            // Read-then-compute under WAL → BEGIN IMMEDIATE so no other
            // process interleaves between the read and the derived write.
            let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate)?;
            let automation = {
                let mut stmt = tx.prepare(&format!(
                    "SELECT {AUTOMATION_COLUMNS} FROM automations WHERE id = ?1"
                ))?;
                stmt.query_row(params![id], map_automation_row)
                    .optional()?
                    .with_context(|| format!("automation {id} not found"))?
            };
            if automation.is_exhausted() {
                // Re-arming a spent definition would produce a due-but-
                // forever-skipped row: reactivation requires raising or
                // resetting `max_executions` via `set_trigger`.
                anyhow::bail!(
                    "automation {id} is exhausted; raise or reset max_executions \
                     via set_trigger to reactivate"
                );
            }
            let next_run = next_run_after(&automation, &now);
            tx.execute(
                "UPDATE automations SET status = ?1, next_run_at = ?2, updated_at = ?3 \
                 WHERE id = ?4",
                params![status.to_string(), next_run, now_rfc, id],
            )?;
            tx.commit()?;
        } else {
            conn.execute(
                "UPDATE automations SET status = ?1, updated_at = ?2 WHERE id = ?3",
                params![status.to_string(), now_rfc, id],
            )?;
        }
        Ok(())
    }

    /// Persist the verify-gate configuration. Patch rule: absent (`None`)
    /// fields keep their current value; an explicit `Some(None)` requirement
    /// clears it. Unknown ids error.
    pub async fn set_verify(&self, id: &str, params: SetVerifyParams) -> Result<()> {
        let conn = self.conn.lock().await;
        let now = Utc::now().to_rfc3339();
        let (enabled, requirement, max_iter): (i64, Option<String>, Option<i64>) = conn
            .query_row(
                "SELECT verify_enabled, verify_requirement, verify_max_iterations \
                 FROM automations WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("automation {id} not found"))?;
        let next_requirement = match params.requirement {
            Some(v) => v, // Some(None) clears, Some(Some) replaces
            None => requirement,
        };
        conn.execute(
            "UPDATE automations SET verify_enabled = ?1, verify_requirement = ?2, \
             verify_max_iterations = ?3, updated_at = ?4 WHERE id = ?5",
            params![
                params.enabled.map(|b| b as i64).unwrap_or(enabled),
                next_requirement,
                params.max_iterations.map(|v| v as i64).or(max_iter),
                now,
                id
            ],
        )?;
        Ok(())
    }

    /// Set/replace the trigger configuration. NEVER changes `status`:
    /// - Active definition → `next_run_at` is recomputed for the new config
    ///   (manual → NULL = runnable on demand).
    /// - Paused/failed/exhausted definition → `next_run_at = NULL` (dormant);
    ///   reactivation goes through `update_status(Active)`, whose
    ///   exhausted-guard still blocks spent definitions.
    ///
    /// `timezone`/`max_executions` follow the double-option patch rule:
    /// absent = preserve the stored value, explicit null = clear.
    pub async fn set_trigger(&self, id: &str, params: SetTriggerParams) -> Result<()> {
        validate_trigger_config(
            params.trigger,
            params.cron_pattern.as_deref(),
            params.heartbeat_interval_secs,
        )?;
        let conn = self.conn.lock().await;
        // Read the row first: patch fields need the current values, and the
        // next_run computation depends on the current status.
        let (current_tz, current_max, status): (Option<String>, Option<i64>, String) = conn
            .query_row(
                "SELECT timezone, max_executions, status FROM automations WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("automation {id} not found"))?;
        let timezone = params.timezone.unwrap_or(current_tz);
        let max_executions = match params.max_executions {
            Some(v) => v,
            None => current_max.map(|v| v as u32),
        };
        if max_executions == Some(0) {
            anyhow::bail!("max_executions must be at least 1 when set");
        }

        let now = Utc::now();
        let active = status == AutomationStatus::Active.to_string();
        // Only an Active definition carries a scheduled next run; every
        // other state is dormant until update_status(Active) re-arms it.
        let next_run = if active {
            match params.trigger {
                AutomationTrigger::Manual => None,
                AutomationTrigger::Cron => Some(cron_next(
                    params
                        .cron_pattern
                        .as_deref()
                        .expect("validated: cron requires a pattern"),
                    &now,
                )?),
                AutomationTrigger::Heartbeat => Some(
                    (now + chrono::Duration::seconds(
                        params
                            .heartbeat_interval_secs
                            .expect("validated: heartbeat requires an interval")
                            as i64,
                    ))
                    .to_rfc3339(),
                ),
            }
        } else {
            None
        };
        conn.execute(
            r#"UPDATE automations SET
                 "trigger" = ?1, cron_pattern = ?2, timezone = ?3,
                 heartbeat_interval_secs = ?4, max_executions = ?5,
                 next_run_at = ?6, updated_at = ?7
               WHERE id = ?8"#,
            params![
                params.trigger.to_string(),
                params.cron_pattern,
                timezone,
                params.heartbeat_interval_secs.map(|v| v as i64),
                max_executions.map(|v| v as i64),
                next_run,
                now.to_rfc3339(),
                id,
            ],
        )?;
        Ok(())
    }

    /// Partially update editable definition fields. Patch rule: absent
    /// (`None`) leaves a field unchanged; an explicit `Some(None)` clears it
    /// (bindings/description become NULL). `updated_at` is stamped;
    /// schedule state is untouched.
    pub async fn update_automation(&self, id: &str, params: UpdateAutomationParams) -> Result<()> {
        // Lock first: the boxed ToSql values are !Send, so they must not
        // live across an await point.
        let conn = self.conn.lock().await;
        // (column, value) pairs built only from present fields. A present
        // Option<Option<String>> binds Option<String> directly — None inside
        // serializes as SQL NULL (clear).
        let mut sets: Vec<(&str, Box<dyn rusqlite::ToSql>)> = Vec::new();
        if let Some(v) = params.name {
            sets.push(("name", Box::new(v)));
        }
        if let Some(v) = params.description {
            sets.push(("description", Box::new(v)));
        }
        if let Some(v) = params.instruction {
            sets.push(("instruction", Box::new(v)));
        }
        if let Some(v) = params.persona_id {
            sets.push(("persona_id", Box::new(v)));
        }
        if let Some(v) = params.project_id {
            sets.push(("project_id", Box::new(v)));
        }
        if let Some(v) = params.brain_space {
            sets.push(("brain_space", Box::new(v)));
        }
        if sets.is_empty() {
            anyhow::bail!("update_automation: no fields provided");
        }

        let now = Utc::now().to_rfc3339();
        sets.push(("updated_at", Box::new(now)));

        let mut sql = String::from("UPDATE automations SET ");
        sql.push_str(
            &sets
                .iter()
                .enumerate()
                .map(|(i, (col, _))| format!("{col} = ?{}", i + 1))
                .collect::<Vec<_>>()
                .join(", "),
        );
        sql.push_str(&format!(" WHERE id = ?{}", sets.len() + 1));
        let refs: Vec<&dyn rusqlite::ToSql> = sets.iter().map(|(_, v)| v.as_ref()).collect();
        let mut bind = Vec::with_capacity(refs.len() + 1);
        bind.extend(refs);
        bind.push(&id);
        let changed = conn.execute(&sql, bind.as_slice())?;
        if changed == 0 {
            anyhow::bail!("automation {id} not found");
        }
        Ok(())
    }

    /// Automations the scheduler tick should fire now: `Active`, cron or
    /// heartbeat trigger, `next_run_at` due. Paused/exhausted/failed
    /// definitions are excluded by construction.
    pub async fn list_due_automations(&self) -> Result<Vec<Automation>> {
        let conn = self.conn.lock().await;
        let now = Utc::now().to_rfc3339();
        let mut stmt = conn.prepare(&format!(
            "SELECT {AUTOMATION_COLUMNS} FROM automations \
             WHERE \"trigger\" != 'manual' AND status = 'active' \
               AND next_run_at IS NOT NULL AND next_run_at <= ?1 \
             ORDER BY next_run_at"
        ))?;
        let due = stmt
            .query_map(params![now], map_automation_row)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(due)
    }

    /// Override `next_run_at` (used by the tick to defer an automation).
    pub async fn set_next_run(&self, id: &str, next_run: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE automations SET next_run_at = ?1, updated_at = ?2 WHERE id = ?3",
            params![next_run, Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    /// Open a run: ONE `BEGIN IMMEDIATE` transaction that reads the
    /// definition, captures the [`AutomationContextSnapshot`] from that row,
    /// inserts the `automation_runs` row with the serialized snapshot JSON,
    /// and advances the definition's execution state (`execution_count`,
    /// `last_run_at`, `next_run_at`; flipping to `Exhausted` when the
    /// ceiling is reached). An in-flight run therefore never observes later
    /// definition edits, and no other writer can interleave between the
    /// read and the derived writes.
    pub async fn begin_run(
        &self,
        id: &str,
        trigger: AutomationRunTrigger,
        session_id: Option<&str>,
    ) -> Result<AutomationRun> {
        let conn = self.conn.lock().await;
        let now = Utc::now();
        let now_rfc = now.to_rfc3339();
        // BEGIN IMMEDIATE (not DEFERRED): the write lock must be held
        // before the definition read so no other process can interleave
        // between the SELECT and the derived INSERT/UPDATE (WAL trap).
        let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate)?;

        // 1. Read the definition inside the transaction.
        let automation = {
            let mut stmt = tx.prepare(&format!(
                "SELECT {AUTOMATION_COLUMNS} FROM automations WHERE id = ?1"
            ))?;
            stmt.query_row(params![id], map_automation_row)
                .optional()?
                .with_context(|| format!("automation {id} not found"))?
        };

        // 2. Snapshot captured from that row — the ONLY definition data the
        //    run will ever see.
        let snapshot = AutomationContextSnapshot {
            persona_id: automation.persona_id.clone(),
            project_id: automation.project_id.clone(),
            brain_space: automation.brain_space.clone(),
            trigger,
            verify: automation.verify.clone(),
            instruction: automation.instruction.clone(),
        };
        let snapshot_json = serde_json::to_string(&snapshot).context("serialize snapshot")?;

        // 3. Insert the run row with the snapshot JSON.
        let run_id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            r#"INSERT INTO automation_runs
                 (id, automation_id, session_id, "trigger", status, context_snapshot, started_at)
               VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6)"#,
            params![
                run_id,
                id,
                session_id,
                trigger.to_string(),
                snapshot_json,
                now_rfc
            ],
        )?;

        // 4. Advance the definition's execution state in the SAME
        //    transaction, flipping to `Exhausted` at the ceiling.
        let new_count = automation.execution_count + 1;
        // The slot advances MONOTONICALLY: a due run moves past its fired
        // slot, but an early/manual run never postpones the regular
        // schedule (max of stored vs. freshly computed).
        let next_run = next_run_after(&automation, &now).map(|computed| {
            let computed_dt = chrono::DateTime::parse_from_rfc3339(&computed)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or(now);
            let stored_dt = automation
                .next_run_at
                .as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc));
            match stored_dt {
                Some(stored) if stored > computed_dt => stored.to_rfc3339(),
                _ => computed,
            }
        });
        let status = if automation
            .max_executions
            .is_some_and(|max| new_count >= max)
        {
            AutomationStatus::Exhausted
        } else {
            AutomationStatus::Active
        };
        tx.execute(
            r#"UPDATE automations SET execution_count = ?1, last_run_at = ?2,
                 next_run_at = ?3, status = ?4, updated_at = ?5
               WHERE id = ?6"#,
            params![
                new_count as i64,
                now_rfc,
                next_run,
                status.to_string(),
                now_rfc,
                id
            ],
        )?;
        tx.commit()?;

        Ok(AutomationRun {
            id: run_id,
            automation_id: id.to_string(),
            session_id: session_id.map(str::to_string),
            trigger,
            status: AutomationRunStatus::Running,
            context_snapshot: snapshot,
            summary: None,
            result_content: None,
            error: None,
            cost_usd: None,
            tokens_used: None,
            started_at: now_rfc,
            completed_at: None,
        })
    }

    /// Finalize a run: the run row gets its terminal status + payload +
    /// `completed_at`; the definition records the outcome. Success keeps the
    /// definition `Active` (its `next_run_at` was already advanced at
    /// `begin_run`) and clears `last_error`.
    ///
    /// Failure policy is trigger-aware: a failed SCHEDULED run (cron or
    /// heartbeat) — or any failure of a manual-triggered definition — marks
    /// the definition `Failed`, stores `last_error`, and clears
    /// `next_run_at` (halted until reactivated). A failed MANUAL run of a
    /// scheduled (cron/heartbeat) automation only records `last_error`; the
    /// recurring schedule (status + `next_run_at`) stays intact so one bad
    /// on-demand attempt can never kill it. An `Exhausted` definition stays
    /// `Exhausted` either way.
    ///
    /// The run's trigger is read from the run row inside the same
    /// transaction — the caller cannot misreport it.
    pub async fn finish_run(
        &self,
        automation_id: &str,
        run_id: &str,
        success: bool,
        summary: Option<String>,
        result_content: Option<String>,
        error: Option<String>,
    ) -> Result<()> {
        let conn = self.conn.lock().await;
        let now_rfc = Utc::now().to_rfc3339();
        let run_status = if success {
            AutomationRunStatus::Succeeded
        } else {
            AutomationRunStatus::Failed
        };
        let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate)?;

        // 1. Run row: terminal status + payload + completed_at.
        tx.execute(
            r#"UPDATE automation_runs SET status = ?1, summary = ?2, result_content = ?3,
                 error = ?4, completed_at = ?5 WHERE id = ?6"#,
            params![
                run_status.to_string(),
                summary,
                result_content,
                error.as_ref(),
                now_rfc,
                run_id
            ],
        )?;

        // 2. Definition outcome (read under the same write lock). Both the
        //    definition's trigger and the run's own trigger come from their
        //    rows: manual and scheduled failures are policy-distinct.
        let (current, def_trigger, run_trigger): (String, String, String) = tx
            .query_row(
                "SELECT a.status, a.\"trigger\", r.\"trigger\" FROM automations a \
                 JOIN automation_runs r ON r.id = ?2 WHERE a.id = ?1",
                params![automation_id, run_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .with_context(|| format!("automation {automation_id} not found"))?;
        let current = AutomationStatus::from_str(&current)
            .map_err(|e| anyhow::anyhow!("corrupt automation status '{current}': {e}"))?;
        let def_trigger = AutomationTrigger::from_str(&def_trigger)
            .map_err(|e| anyhow::anyhow!("corrupt automation trigger '{def_trigger}': {e}"))?;
        let run_trigger = AutomationRunTrigger::from_str(&run_trigger)
            .map_err(|e| anyhow::anyhow!("corrupt run trigger '{run_trigger}': {e}"))?;
        let manual_run_on_scheduled =
            run_trigger == AutomationRunTrigger::Manual && def_trigger != AutomationTrigger::Manual;
        if current == AutomationStatus::Exhausted {
            // Ceiling is terminal either way; just record/clear the error.
            let last_error = if success { None } else { error };
            tx.execute(
                "UPDATE automations SET last_error = ?1, updated_at = ?2 WHERE id = ?3",
                params![last_error, now_rfc, automation_id],
            )?;
        } else if success {
            tx.execute(
                "UPDATE automations SET status = ?1, last_error = NULL, updated_at = ?2 \
                 WHERE id = ?3",
                params![AutomationStatus::Active.to_string(), now_rfc, automation_id],
            )?;
        } else if manual_run_on_scheduled {
            // A manual attempt at a scheduled automation failed: record the
            // error but leave the recurring schedule (status + next_run_at)
            // untouched.
            tx.execute(
                "UPDATE automations SET last_error = ?1, updated_at = ?2 WHERE id = ?3",
                params![error, now_rfc, automation_id],
            )?;
        } else {
            tx.execute(
                r#"UPDATE automations SET status = ?1, last_error = ?2,
                     next_run_at = NULL, updated_at = ?3 WHERE id = ?4"#,
                params![
                    AutomationStatus::Failed.to_string(),
                    error,
                    now_rfc,
                    automation_id
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Latest run for an automation (the UI's "last result" display).
    pub async fn latest_run(&self, automation_id: &str) -> Result<Option<AutomationRun>> {
        let conn = self.conn.lock().await;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT {RUN_COLUMNS} FROM automation_runs \
                     WHERE automation_id = ?1 ORDER BY started_at DESC, id DESC LIMIT 1"
                ),
                params![automation_id],
                map_run_row,
            )
            .optional()?)
    }

    /// Run history for an automation (newest first).
    pub async fn list_runs(&self, automation_id: &str) -> Result<Vec<AutomationRun>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(&format!(
            "SELECT {RUN_COLUMNS} FROM automation_runs \
             WHERE automation_id = ?1 ORDER BY started_at DESC, id DESC LIMIT 50"
        ))?;
        let runs = stmt
            .query_map(params![automation_id], map_run_row)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(runs)
    }

    /// Boot-time recovery: run rows stranded at `running` by a prior
    /// process crash are marked `failed`. Definitions no longer enter a
    /// running state (the definition stays `Active` while a run is in
    /// flight; `next_run_at` was advanced inside `begin_run`'s
    /// transaction), so there is no definition-side state to reset.
    pub async fn recover_stranded(&self) -> Result<()> {
        let conn = self.conn.lock().await;
        let now = Utc::now().to_rfc3339();
        let runs = conn.execute(
            "UPDATE automation_runs SET status = 'failed', \
             error = 'Interrupted by process restart', completed_at = ?1 \
             WHERE status = 'running'",
            params![now],
        )?;
        if runs > 0 {
            tracing::info!(runs, "Recovered stranded automation runs after restart");
        }
        Ok(())
    }
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS automations (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT,
            instruction TEXT NOT NULL,
            "trigger" TEXT NOT NULL DEFAULT 'manual',
            cron_pattern TEXT,
            timezone TEXT,
            heartbeat_interval_secs INTEGER,
            max_executions INTEGER,
            execution_count INTEGER DEFAULT 0,
            persona_id TEXT,
            project_id TEXT,
            brain_space TEXT,
            verify_enabled INTEGER DEFAULT 0,
            verify_requirement TEXT,
            verify_max_iterations INTEGER DEFAULT 3,
            status TEXT NOT NULL DEFAULT 'active',
            next_run_at TEXT,
            last_run_at TEXT,
            last_error TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS automation_runs (
            id TEXT PRIMARY KEY,
            automation_id TEXT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
            session_id TEXT,
            "trigger" TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'running',
            context_snapshot TEXT NOT NULL,
            summary TEXT,
            result_content TEXT,
            error TEXT,
            cost_usd REAL,
            tokens_used INTEGER,
            started_at TEXT NOT NULL,
            completed_at TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_automations_status ON automations(status);
        CREATE INDEX IF NOT EXISTS idx_automations_next_run ON automations(next_run_at);
        CREATE INDEX IF NOT EXISTS idx_runs_automation ON automation_runs(automation_id);
        "#,
    )?;
    Ok(())
}

// ── Row mappers ──

fn map_automation_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Automation> {
    let trigger_str: String = row.get(4)?;
    let trigger = AutomationTrigger::from_str(&trigger_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, e.into())
    })?;
    let status_str: String = row.get(16)?;
    let status = AutomationStatus::from_str(&status_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(16, rusqlite::types::Type::Text, e.into())
    })?;
    Ok(Automation {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        instruction: row.get(3)?,
        trigger,
        cron_pattern: row.get(5)?,
        timezone: row.get(6)?,
        heartbeat_interval_secs: row.get::<_, Option<i64>>(7)?.map(|v| v as u64),
        max_executions: row.get::<_, Option<i64>>(8)?.map(|v| v as u32),
        execution_count: row.get::<_, i64>(9)? as u32,
        persona_id: row.get(10)?,
        project_id: row.get(11)?,
        brain_space: row.get(12)?,
        verify: AutomationVerifyConfig {
            enabled: row.get::<_, i64>(13)? != 0,
            requirement: row.get(14)?,
            max_iterations: row.get::<_, i64>(15)? as u32,
        },
        status,
        next_run_at: row.get(17)?,
        last_run_at: row.get(18)?,
        last_error: row.get(19)?,
        created_at: row.get(20)?,
        updated_at: row.get(21)?,
    })
}

fn map_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AutomationRun> {
    let trigger_str: String = row.get(3)?;
    let trigger = AutomationRunTrigger::from_str(&trigger_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, e.into())
    })?;
    let status_str: String = row.get(4)?;
    let status = AutomationRunStatus::from_str(&status_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, e.into())
    })?;
    let snapshot_json: String = row.get(5)?;
    let context_snapshot: AutomationContextSnapshot = serde_json::from_str(&snapshot_json)
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, e.into())
        })?;
    Ok(AutomationRun {
        id: row.get(0)?,
        automation_id: row.get(1)?,
        session_id: row.get(2)?,
        trigger,
        status,
        context_snapshot,
        summary: row.get(6)?,
        result_content: row.get(7)?,
        error: row.get(8)?,
        cost_usd: row.get(9)?,
        tokens_used: row.get::<_, Option<i64>>(10)?.map(|v| v as u64),
        started_at: row.get(11)?,
        completed_at: row.get(12)?,
    })
}

/// Next scheduled slot for a definition after `now`, honoring its trigger.
/// Manual automations never self-schedule (`None`).
fn next_run_after(automation: &Automation, now: &DateTime<Utc>) -> Option<String> {
    match automation.trigger {
        AutomationTrigger::Manual => None,
        AutomationTrigger::Cron => automation
            .cron_pattern
            .as_deref()
            .and_then(|p| cron_next(p, now).ok()),
        AutomationTrigger::Heartbeat => automation
            .heartbeat_interval_secs
            .map(|s| (*now + chrono::Duration::seconds(s as i64)).to_rfc3339()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Manual automation params (no scheduling fields).
    fn manual_params(name: &str) -> CreateAutomationParams {
        CreateAutomationParams {
            name: name.to_string(),
            instruction: format!("do {name}"),
            description: None,
            trigger: AutomationTrigger::Manual,
            cron_pattern: None,
            timezone: None,
            heartbeat_interval_secs: None,
            max_executions: None,
            persona_id: None,
            project_id: None,
            brain_space: None,
            verify: AutomationVerifyConfig::default(),
        }
    }

    /// Cron automation params with a daily 09:00 pattern.
    fn cron_params(name: &str) -> CreateAutomationParams {
        CreateAutomationParams {
            trigger: AutomationTrigger::Cron,
            cron_pattern: Some("0 9 * * *".into()),
            ..manual_params(name)
        }
    }

    /// Heartbeat automation params with an hourly interval.
    fn heartbeat_params(name: &str, secs: u64) -> CreateAutomationParams {
        CreateAutomationParams {
            trigger: AutomationTrigger::Heartbeat,
            heartbeat_interval_secs: Some(secs),
            ..manual_params(name)
        }
    }

    // ── Construction / persistence basics ──

    // Regression carried over from the task store: store construction must
    // be safe inside a Tokio runtime (no `blocking_lock` during schema init).
    #[tokio::test]
    async fn in_memory_store_construction_does_not_panic_on_runtime() {
        let store = AutomationStore::in_memory().expect("in-memory store builds");
        let a = store
            .create_automation(manual_params("regression"))
            .await
            .expect("create works");
        assert_eq!(a.name, "regression");
        assert_eq!(a.status, AutomationStatus::Active);
        assert_eq!(a.trigger, AutomationTrigger::Manual);
        assert!(
            a.next_run_at.is_none(),
            "manual automations never self-schedule"
        );
    }

    #[tokio::test]
    async fn open_reopen_roundtrip_on_automations_db_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        // The assembler opens exactly this file name — the store must be
        // happy with a fresh {workspace}/automations.db.
        let path = dir.path().join("automations.db");
        let path_str = path.to_str().expect("utf8 path");
        let store = AutomationStore::open(path_str).expect("open builds");
        let created = store
            .create_automation(cron_params("from-disk"))
            .await
            .expect("create");
        drop(store);
        let reopened = AutomationStore::open(path_str).expect("reopen builds");
        let fetched = reopened.get_automation(&created.id).await.expect("get");
        assert_eq!(fetched.name, "from-disk");
    }

    #[tokio::test]
    async fn create_list_delete_roundtrip() {
        let store = AutomationStore::in_memory().unwrap();
        let a1 = store
            .create_automation(manual_params("alpha"))
            .await
            .unwrap();
        let _a2 = store.create_automation(cron_params("beta")).await.unwrap();

        let listed = store
            .list_automations(ListAutomationsParams::default())
            .await
            .unwrap();
        assert_eq!(listed.len(), 2);

        let filtered = store
            .list_automations(ListAutomationsParams {
                status: Some("cron-active".into()),
                ..Default::default()
            })
            .await;
        assert!(filtered.is_err(), "unknown status filter must be rejected");

        let only_manual = store
            .list_automations(ListAutomationsParams {
                status: Some("manual".into()),
                ..Default::default()
            })
            .await
            .unwrap_or_default();
        let by_status = store
            .list_automations(ListAutomationsParams {
                status: Some("active".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(!by_status.is_empty());

        store.delete_automation(&a1.id).await.unwrap();
        let after = store
            .list_automations(ListAutomationsParams::default())
            .await
            .unwrap();
        assert_eq!(after.len(), 1);
        assert!(store.get_automation(&a1.id).await.is_err());
        let _ = only_manual;
    }

    #[tokio::test]
    async fn create_validates_trigger_config() {
        let store = AutomationStore::in_memory().unwrap();
        // Cron trigger without a pattern.
        let bad_cron = CreateAutomationParams {
            trigger: AutomationTrigger::Cron,
            cron_pattern: None,
            ..manual_params("bad-cron")
        };
        assert!(store.create_automation(bad_cron).await.is_err());
        // Heartbeat without an interval.
        let bad_heartbeat = CreateAutomationParams {
            trigger: AutomationTrigger::Heartbeat,
            heartbeat_interval_secs: None,
            ..manual_params("bad-heartbeat")
        };
        assert!(store.create_automation(bad_heartbeat).await.is_err());
        // Manual with a cron pattern.
        let bad_manual = CreateAutomationParams {
            trigger: AutomationTrigger::Manual,
            cron_pattern: Some("0 9 * * *".into()),
            ..manual_params("bad-manual")
        };
        assert!(store.create_automation(bad_manual).await.is_err());
        // max_executions = Some(0) can never run — rejected.
        let bad_max = CreateAutomationParams {
            max_executions: Some(0),
            ..cron_params("bad-max")
        };
        assert!(store.create_automation(bad_max).await.is_err());
    }

    // ── Required: absent project ID ──

    #[tokio::test]
    async fn absent_project_id_persists_and_reads_back_as_none() {
        let store = AutomationStore::in_memory().unwrap();
        let a = store
            .create_automation(cron_params("projectless"))
            .await
            .unwrap();
        assert!(a.project_id.is_none());
        assert!(a.persona_id.is_none());
        assert!(a.brain_space.is_none());
        let fetched = store.get_automation(&a.id).await.unwrap();
        assert!(fetched.project_id.is_none());
        // Scoped bindings that ARE set survive the round trip untouched.
        let scoped = CreateAutomationParams {
            persona_id: Some("persona-1".into()),
            project_id: Some("proj-1".into()),
            brain_space: Some("space-1".into()),
            ..manual_params("scoped")
        };
        let b = store.create_automation(scoped).await.unwrap();
        assert_eq!(b.persona_id.as_deref(), Some("persona-1"));
        assert_eq!(b.project_id.as_deref(), Some("proj-1"));
        assert_eq!(b.brain_space.as_deref(), Some("space-1"));
    }

    // ── Required: schedule next-run calculation ──

    #[tokio::test]
    async fn cron_trigger_computes_future_next_run_at() {
        let store = AutomationStore::in_memory().unwrap();
        let a = store
            .create_automation(cron_params("scheduled"))
            .await
            .unwrap();
        assert_eq!(a.status, AutomationStatus::Active);
        assert_eq!(a.cron_pattern.as_deref(), Some("0 9 * * *"));
        let next = a.next_run_at.expect("cron automation gets a next_run_at");
        let parsed = chrono::DateTime::parse_from_rfc3339(&next).expect("RFC 3339");
        assert!(
            parsed.with_timezone(&Utc) > Utc::now(),
            "next run must be in the future: {next}"
        );
    }

    #[tokio::test]
    async fn heartbeat_trigger_computes_next_run_at_from_interval() {
        let store = AutomationStore::in_memory().unwrap();
        let a = store
            .create_automation(heartbeat_params("pulse", 3600))
            .await
            .unwrap();
        let next = a
            .next_run_at
            .expect("heartbeat automation gets a next_run_at");
        let parsed = chrono::DateTime::parse_from_rfc3339(&next)
            .expect("RFC 3339")
            .with_timezone(&Utc);
        let delta = parsed - Utc::now();
        assert!(
            delta > chrono::Duration::seconds(3500) && delta <= chrono::Duration::seconds(3700),
            "next run must be ~now+3600s, got {delta}"
        );
    }

    #[tokio::test]
    async fn set_trigger_replaces_schedule_and_clears_on_manual() {
        let store = AutomationStore::in_memory().unwrap();
        let a = store
            .create_automation(manual_params("switchable"))
            .await
            .unwrap();
        assert!(a.next_run_at.is_none());

        store
            .set_trigger(
                &a.id,
                SetTriggerParams {
                    trigger: AutomationTrigger::Heartbeat,
                    cron_pattern: None,
                    timezone: None,
                    heartbeat_interval_secs: Some(600),
                    max_executions: None,
                },
            )
            .await
            .unwrap();
        let armed = store.get_automation(&a.id).await.unwrap();
        assert_eq!(armed.trigger, AutomationTrigger::Heartbeat);
        assert!(armed.next_run_at.is_some());
        assert_eq!(armed.status, AutomationStatus::Active);

        // Invalid replacement is rejected wholesale.
        let invalid = store
            .set_trigger(
                &a.id,
                SetTriggerParams {
                    trigger: AutomationTrigger::Cron,
                    cron_pattern: None,
                    timezone: None,
                    heartbeat_interval_secs: None,
                    max_executions: None,
                },
            )
            .await;
        assert!(invalid.is_err());

        // Back to manual: scheduling cleared, still runnable on demand.
        store
            .set_trigger(
                &a.id,
                SetTriggerParams {
                    trigger: AutomationTrigger::Manual,
                    cron_pattern: None,
                    timezone: None,
                    heartbeat_interval_secs: None,
                    max_executions: None,
                },
            )
            .await
            .unwrap();
        let cleared = store.get_automation(&a.id).await.unwrap();
        assert!(cleared.next_run_at.is_none());
        assert_eq!(cleared.cron_pattern, None);
        assert_eq!(cleared.heartbeat_interval_secs, None);
    }

    // ── Required: paused/exhausted exclusion from scheduling ──

    #[tokio::test]
    async fn paused_and_exhausted_automations_are_excluded_from_due_list() {
        let store = AutomationStore::in_memory().unwrap();
        let active = store
            .create_automation(cron_params("active-one"))
            .await
            .unwrap();
        let paused = store
            .create_automation(cron_params("paused-one"))
            .await
            .unwrap();
        let mut limited = cron_params("exhausted-one");
        limited.max_executions = Some(1);
        let exhausted = store.create_automation(limited).await.unwrap();

        // A fresh cron automation is due only at its first slot; force all
        // three into the due state deterministically.
        let past = (Utc::now() - chrono::Duration::minutes(1)).to_rfc3339();
        for id in [&active.id, &paused.id, &exhausted.id] {
            store.set_next_run(id, Some(&past)).await.unwrap();
        }

        store
            .update_status(&paused.id, &AutomationStatus::Paused)
            .await
            .unwrap();
        // Exhaust this one: begin_run flips the definition to Exhausted.
        store
            .begin_run(&exhausted.id, AutomationRunTrigger::Cron, None)
            .await
            .unwrap();

        let due = store.list_due_automations().await.unwrap();
        let due_ids: Vec<&str> = due.iter().map(|a| a.id.as_str()).collect();
        assert!(
            due_ids.contains(&active.id.as_str()),
            "active automation is due"
        );
        assert!(
            !due_ids.contains(&paused.id.as_str()),
            "paused must be excluded"
        );
        assert!(
            !due_ids.contains(&exhausted.id.as_str()),
            "exhausted must be excluded"
        );

        // Resuming the paused automation recomputes a FRESH (future) slot:
        // it re-enters scheduling but is not instantly due again.
        store
            .update_status(&paused.id, &AutomationStatus::Active)
            .await
            .unwrap();
        let resumed = store.get_automation(&paused.id).await.unwrap();
        assert!(resumed.next_run_at.is_some());
        let resumed_next =
            chrono::DateTime::parse_from_rfc3339(resumed.next_run_at.as_deref().unwrap()).unwrap();
        assert!(
            resumed_next.with_timezone(&Utc) > Utc::now(),
            "fresh slot is future"
        );
        assert_eq!(
            store
                .list_due_automations()
                .await
                .unwrap()
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>(),
            vec![active.id.as_str()],
            "only the still-due active automation remains"
        );
    }

    // ── Required: begin_run transaction + immutable snapshot ──

    #[tokio::test]
    async fn begin_run_inserts_run_and_advances_definition_in_one_transaction() {
        let store = AutomationStore::in_memory().unwrap();
        let params = CreateAutomationParams {
            persona_id: Some("persona-7".into()),
            project_id: Some("proj-7".into()),
            brain_space: Some("space-7".into()),
            verify: AutomationVerifyConfig {
                enabled: true,
                requirement: Some("must include BANANA".into()),
                max_iterations: 2,
            },
            ..cron_params("snapshot-source")
        };
        let a = store.create_automation(params).await.unwrap();
        let before_count = a.execution_count;
        // Make the automation due so begin_run's monotonic slot advance is
        // observable (a fresh cron automation waits for its first slot).
        let past = (Utc::now() - chrono::Duration::minutes(1)).to_rfc3339();
        store.set_next_run(&a.id, Some(&past)).await.unwrap();

        let run = store
            .begin_run(&a.id, AutomationRunTrigger::Cron, Some("sess-1"))
            .await
            .unwrap();
        assert_eq!(run.automation_id, a.id);
        assert_eq!(run.session_id.as_deref(), Some("sess-1"));
        assert_eq!(run.trigger, AutomationRunTrigger::Cron);
        assert_eq!(run.status, AutomationRunStatus::Running);
        assert!(run.completed_at.is_none());

        // Snapshot captured from the definition at open time.
        let snap = &run.context_snapshot;
        assert_eq!(snap.instruction, "do snapshot-source");
        assert_eq!(snap.persona_id.as_deref(), Some("persona-7"));
        assert_eq!(snap.project_id.as_deref(), Some("proj-7"));
        assert_eq!(snap.brain_space.as_deref(), Some("space-7"));
        assert_eq!(snap.trigger, AutomationRunTrigger::Cron);
        assert!(snap.verify.enabled);
        assert_eq!(
            snap.verify.requirement.as_deref(),
            Some("must include BANANA")
        );

        // The run is readable from the store (snapshot JSON round-trips
        // through the context_snapshot column).
        let stored = store.latest_run(&a.id).await.unwrap().expect("run row");
        assert_eq!(stored.id, run.id);
        assert_eq!(stored.context_snapshot, run.context_snapshot);

        // Definition execution state advanced by the same transaction.
        let after = store.get_automation(&a.id).await.unwrap();
        assert_eq!(after.execution_count, before_count + 1);
        assert!(after.last_run_at.is_some());
        // next_run_at advanced past the just-started slot: later ticks won't
        // re-fire the automation while this run is in flight.
        let old_next = chrono::DateTime::parse_from_rfc3339(&past).unwrap();
        let new_next =
            chrono::DateTime::parse_from_rfc3339(after.next_run_at.as_deref().unwrap()).unwrap();
        assert!(new_next > old_next, "next_run_at must advance at begin_run");
    }

    // ── Required: immutable snapshot after an Automation edit ──

    #[tokio::test]
    async fn snapshot_is_immutable_after_definition_edit() {
        let store = AutomationStore::in_memory().unwrap();
        let params = CreateAutomationParams {
            instruction: "instruction v1".into(),
            project_id: Some("alpha".into()),
            persona_id: Some("persona-a".into()),
            ..cron_params("frozen")
        };
        let a = store.create_automation(params).await.unwrap();
        let run = store
            .begin_run(&a.id, AutomationRunTrigger::Cron, None)
            .await
            .unwrap();

        // Edit the definition mid-run: instruction, instruction, scoped ids.
        store
            .update_automation(
                &a.id,
                UpdateAutomationParams {
                    instruction: Some("instruction v2".into()),
                    project_id: Some(Some("beta".into())),
                    persona_id: Some(Some("persona-b".into())),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // The in-flight run still reports the opening snapshot.
        let stored = store.latest_run(&a.id).await.unwrap().expect("run row");
        assert_eq!(stored.context_snapshot.instruction, "instruction v1");
        assert_eq!(stored.context_snapshot.project_id.as_deref(), Some("alpha"));
        assert_eq!(
            stored.context_snapshot.persona_id.as_deref(),
            Some("persona-a")
        );
        assert_eq!(run.context_snapshot.instruction, "instruction v1");

        // ...while the definition itself reflects the edit.
        let edited = store.get_automation(&a.id).await.unwrap();
        assert_eq!(edited.instruction, "instruction v2");
        assert_eq!(edited.project_id.as_deref(), Some("beta"));
    }

    #[tokio::test]
    async fn finish_run_records_outcome_and_definition_state() {
        let store = AutomationStore::in_memory().unwrap();
        let ok = store
            .create_automation(cron_params("finishing"))
            .await
            .unwrap();
        let run = store
            .begin_run(&ok.id, AutomationRunTrigger::Cron, None)
            .await
            .unwrap();
        store
            .finish_run(
                &ok.id,
                &run.id,
                true,
                Some("done".into()),
                Some("done".into()),
                None,
            )
            .await
            .unwrap();
        let finished = store.latest_run(&ok.id).await.unwrap().unwrap();
        assert_eq!(finished.status, AutomationRunStatus::Succeeded);
        assert_eq!(finished.summary.as_deref(), Some("done"));
        assert!(finished.completed_at.is_some());
        let def = store.get_automation(&ok.id).await.unwrap();
        assert_eq!(
            def.status,
            AutomationStatus::Active,
            "success stays scheduled"
        );
        assert!(
            def.next_run_at.is_some(),
            "next slot was advanced at begin_run"
        );

        // Failure: run failed, definition halted with last_error set.
        let bad = store
            .create_automation(cron_params("failing"))
            .await
            .unwrap();
        let bad_run = store
            .begin_run(&bad.id, AutomationRunTrigger::Cron, None)
            .await
            .unwrap();
        store
            .finish_run(
                &bad.id,
                &bad_run.id,
                false,
                Some("blew up".into()),
                None,
                Some("provider exploded".into()),
            )
            .await
            .unwrap();
        let failed_run = store.latest_run(&bad.id).await.unwrap().unwrap();
        assert_eq!(failed_run.status, AutomationRunStatus::Failed);
        assert_eq!(failed_run.error.as_deref(), Some("provider exploded"));
        let bad_def = store.get_automation(&bad.id).await.unwrap();
        assert_eq!(bad_def.status, AutomationStatus::Failed);
        assert_eq!(bad_def.last_error.as_deref(), Some("provider exploded"));
        assert!(
            bad_def.next_run_at.is_none(),
            "failed automations halt scheduling"
        );

        // Reactivating recomputes the schedule.
        store
            .update_status(&bad.id, &AutomationStatus::Active)
            .await
            .unwrap();
        let revived = store.get_automation(&bad.id).await.unwrap();
        assert!(revived.next_run_at.is_some());
    }

    #[tokio::test]
    async fn manual_failure_keeps_scheduled_automation_running() {
        let store = AutomationStore::in_memory().unwrap();
        let a = store
            .create_automation(cron_params("manual-victim"))
            .await
            .unwrap();
        // Snapshot the create-time slot. begin_run's monotonic advance
        // leaves a future slot alone, so any CHANGE after the failed manual
        // run would be attributable to finish_run.
        let scheduled_slot = a.next_run_at.clone();

        // A MANUAL run fails (e.g. provider blip while a user poked the
        // button). The error is recorded, but the recurring schedule must
        // survive untouched.
        let run = store
            .begin_run(&a.id, AutomationRunTrigger::Manual, None)
            .await
            .unwrap();
        store
            .finish_run(
                &a.id,
                &run.id,
                false,
                Some("blew up".into()),
                None,
                Some("provider exploded".into()),
            )
            .await
            .unwrap();

        let failed_run = store.latest_run(&a.id).await.unwrap().unwrap();
        assert_eq!(failed_run.status, AutomationRunStatus::Failed);
        assert_eq!(failed_run.error.as_deref(), Some("provider exploded"));

        let def = store.get_automation(&a.id).await.unwrap();
        assert_eq!(
            def.status,
            AutomationStatus::Active,
            "manual failure must not halt a scheduled automation"
        );
        assert_eq!(
            def.last_error.as_deref(),
            Some("provider exploded"),
            "the failure is still recorded for visibility"
        );
        assert_eq!(
            def.next_run_at, scheduled_slot,
            "next_run_at must be untouched by the failed manual run"
        );

        // The definition is still schedulable: forcing the slot due puts it
        // straight back on the tick's list.
        let past = (Utc::now() - chrono::Duration::minutes(1)).to_rfc3339();
        store.set_next_run(&a.id, Some(&past)).await.unwrap();
        let due = store.list_due_automations().await.unwrap();
        assert!(due.iter().any(|x| x.id == a.id), "schedule still fires");
    }

    #[tokio::test]
    async fn reactivating_exhausted_automation_is_rejected() {
        let store = AutomationStore::in_memory().unwrap();
        let mut params = cron_params("spent");
        params.max_executions = Some(1);
        let a = store.create_automation(params).await.unwrap();
        store
            .begin_run(&a.id, AutomationRunTrigger::Cron, None)
            .await
            .unwrap();
        assert_eq!(
            store.get_automation(&a.id).await.unwrap().status,
            AutomationStatus::Exhausted
        );

        // update_status(Active) must NOT re-arm a spent definition into a
        // due-but-forever-skipped row: raising/resetting max_executions via
        // set_trigger is the reactivation path.
        assert!(
            store
                .update_status(&a.id, &AutomationStatus::Active)
                .await
                .is_err()
        );
        let still = store.get_automation(&a.id).await.unwrap();
        assert_eq!(still.status, AutomationStatus::Exhausted);
        assert!(
            !store
                .list_due_automations()
                .await
                .unwrap()
                .iter()
                .any(|x| x.id == a.id)
        );
    }

    // ── Required: crash recovery ──

    #[tokio::test]
    async fn recover_stranded_marks_running_runs_failed() {
        let store = AutomationStore::in_memory().unwrap();
        let a = store
            .create_automation(cron_params("crashed"))
            .await
            .unwrap();
        let run = store
            .begin_run(&a.id, AutomationRunTrigger::Cron, None)
            .await
            .unwrap();
        // Simulate a crash: the run row is still `running`.
        assert_eq!(
            store.latest_run(&a.id).await.unwrap().unwrap().status,
            AutomationRunStatus::Running
        );

        store.recover_stranded().await.unwrap();

        let recovered = store.latest_run(&a.id).await.unwrap().unwrap();
        assert_eq!(recovered.id, run.id);
        assert_eq!(recovered.status, AutomationRunStatus::Failed);
        assert!(recovered.completed_at.is_some());
        assert!(
            recovered
                .error
                .as_deref()
                .is_some_and(|e| e.contains("restart")),
            "recovery should note the restart: {:?}",
            recovered.error
        );
        // The definition stays schedulable so the next slot still fires.
        let def = store.get_automation(&a.id).await.unwrap();
        assert_eq!(def.status, AutomationStatus::Active);
        // Recovery is idempotent.
        store.recover_stranded().await.unwrap();
        assert_eq!(
            store.latest_run(&a.id).await.unwrap().unwrap().status,
            AutomationRunStatus::Failed
        );
    }

    #[tokio::test]
    async fn max_executions_flip_definition_to_exhausted_at_begin() {
        let store = AutomationStore::in_memory().unwrap();
        let mut params = cron_params("one-shot");
        params.max_executions = Some(1);
        let a = store.create_automation(params).await.unwrap();
        store
            .begin_run(&a.id, AutomationRunTrigger::Cron, None)
            .await
            .unwrap();
        let def = store.get_automation(&a.id).await.unwrap();
        assert_eq!(def.status, AutomationStatus::Exhausted);
        assert!(def.is_exhausted());
        // A further scheduler pick is excluded.
        assert!(
            !store
                .list_due_automations()
                .await
                .unwrap()
                .iter()
                .any(|x| x.id == a.id)
        );
    }

    #[tokio::test]
    async fn set_verify_updates_verify_config() {
        let store = AutomationStore::in_memory().unwrap();
        let a = store
            .create_automation(manual_params("verified"))
            .await
            .unwrap();
        assert!(!a.verify.enabled);
        store
            .set_verify(
                &a.id,
                SetVerifyParams {
                    enabled: Some(true),
                    requirement: Some(Some("must include BANANA".into())),
                    max_iterations: Some(5),
                },
            )
            .await
            .unwrap();
        let updated = store.get_automation(&a.id).await.unwrap();
        assert!(updated.verify.enabled);
        assert_eq!(
            updated.verify.requirement.as_deref(),
            Some("must include BANANA")
        );
        assert_eq!(updated.verify.max_iterations, 5);
        // None fields keep their current value.
        store
            .set_verify(&a.id, SetVerifyParams::default())
            .await
            .unwrap();
        let kept = store.get_automation(&a.id).await.unwrap();
        assert!(kept.verify.enabled);
        assert_eq!(kept.verify.max_iterations, 5);
    }

    #[tokio::test]
    async fn update_automation_partial_fields() {
        let store = AutomationStore::in_memory().unwrap();
        let a = store
            .create_automation(CreateAutomationParams {
                description: Some("original".into()),
                ..manual_params("editable")
            })
            .await
            .unwrap();
        store
            .update_automation(
                &a.id,
                UpdateAutomationParams {
                    name: Some("renamed".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let updated = store.get_automation(&a.id).await.unwrap();
        assert_eq!(updated.name, "renamed");
        assert_eq!(
            updated.description.as_deref(),
            Some("original"),
            "untouched field kept"
        );
        assert!(
            store
                .update_automation(&a.id, UpdateAutomationParams::default())
                .await
                .is_err()
        );
    }

    // ── Fix round 2: double-option patch semantics ──

    #[tokio::test]
    async fn update_automation_explicit_null_clears_bindings_and_description() {
        let store = AutomationStore::in_memory().unwrap();
        let a = store
            .create_automation(CreateAutomationParams {
                description: Some("temp note".into()),
                persona_id: Some("persona-1".into()),
                project_id: Some("proj-1".into()),
                brain_space: Some("space-1".into()),
                ..manual_params("unbindable")
            })
            .await
            .unwrap();
        store
            .update_automation(
                &a.id,
                UpdateAutomationParams {
                    description: Some(None),
                    persona_id: Some(None),
                    project_id: Some(None),
                    brain_space: Some(None),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let updated = store.get_automation(&a.id).await.unwrap();
        assert_eq!(
            updated.description, None,
            "explicit null clears description"
        );
        assert_eq!(updated.persona_id, None, "explicit null unbinds persona");
        assert_eq!(updated.project_id, None, "explicit null unbinds project");
        assert_eq!(updated.brain_space, None, "explicit null unbinds brain");
    }

    #[tokio::test]
    async fn update_automation_absent_keys_leave_values_intact() {
        let store = AutomationStore::in_memory().unwrap();
        let a = store
            .create_automation(CreateAutomationParams {
                description: Some("kept".into()),
                project_id: Some("proj-1".into()),
                ..manual_params("stable")
            })
            .await
            .unwrap();
        store
            .update_automation(
                &a.id,
                UpdateAutomationParams {
                    name: Some("renamed".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let updated = store.get_automation(&a.id).await.unwrap();
        assert_eq!(updated.description.as_deref(), Some("kept"));
        assert_eq!(updated.project_id.as_deref(), Some("proj-1"));
    }

    #[tokio::test]
    async fn update_automation_clear_only_counts_as_a_change() {
        let store = AutomationStore::in_memory().unwrap();
        let a = store
            .create_automation(CreateAutomationParams {
                description: Some("gone".into()),
                ..manual_params("clear-only")
            })
            .await
            .unwrap();
        // Clearing the only provided field IS a change — must not bail.
        store
            .update_automation(
                &a.id,
                UpdateAutomationParams {
                    description: Some(None),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(store.get_automation(&a.id).await.unwrap().description, None);
    }

    #[tokio::test]
    async fn set_verify_explicit_null_clears_requirement() {
        let store = AutomationStore::in_memory().unwrap();
        let a = store
            .create_automation(manual_params("verifiable"))
            .await
            .unwrap();
        store
            .set_verify(
                &a.id,
                SetVerifyParams {
                    enabled: Some(true),
                    requirement: Some(Some("must contain BANANA".into())),
                    max_iterations: Some(4),
                },
            )
            .await
            .unwrap();
        store
            .set_verify(
                &a.id,
                SetVerifyParams {
                    requirement: Some(None),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let updated = store.get_automation(&a.id).await.unwrap();
        assert_eq!(updated.verify.requirement, None, "explicit null clears");
        assert!(updated.verify.enabled, "absent key leaves enabled intact");
        assert_eq!(updated.verify.max_iterations, 4);
    }

    #[tokio::test]
    async fn set_trigger_on_paused_preserves_status_and_dormant_next_run() {
        let store = AutomationStore::in_memory().unwrap();
        let a = store
            .create_automation(cron_params("pausable"))
            .await
            .unwrap();
        store
            .update_status(&a.id, &AutomationStatus::Paused)
            .await
            .unwrap();
        store
            .set_trigger(
                &a.id,
                SetTriggerParams {
                    trigger: AutomationTrigger::Heartbeat,
                    cron_pattern: None,
                    timezone: None,
                    heartbeat_interval_secs: Some(600),
                    max_executions: None,
                },
            )
            .await
            .unwrap();
        let edited = store.get_automation(&a.id).await.unwrap();
        assert_eq!(
            edited.status,
            AutomationStatus::Paused,
            "editing the trigger config must NOT un-pause"
        );
        assert_eq!(
            edited.next_run_at, None,
            "dormant while paused: no next_run_at"
        );
        // Reactivating arms the schedule again.
        store
            .update_status(&a.id, &AutomationStatus::Active)
            .await
            .unwrap();
        let resumed = store.get_automation(&a.id).await.unwrap();
        assert_eq!(resumed.status, AutomationStatus::Active);
        assert!(resumed.next_run_at.is_some(), "reactivation re-arms");
    }

    #[tokio::test]
    async fn set_trigger_on_active_recomputes_next_run() {
        let store = AutomationStore::in_memory().unwrap();
        let a = store.create_automation(cron_params("live")).await.unwrap();
        let before = a.next_run_at.clone();
        store
            .set_trigger(
                &a.id,
                SetTriggerParams {
                    trigger: AutomationTrigger::Cron,
                    cron_pattern: Some("*/5 * * * *".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let edited = store.get_automation(&a.id).await.unwrap();
        assert_eq!(edited.status, AutomationStatus::Active);
        let next = edited.next_run_at.expect("active cron keeps a next run");
        assert_ne!(Some(&next), before.as_ref(), "next_run_at was recomputed");
        let parsed = chrono::DateTime::parse_from_rfc3339(&next).unwrap();
        assert!(parsed.with_timezone(&Utc) > Utc::now());
        assert_eq!(edited.cron_pattern.as_deref(), Some("*/5 * * * *"));
    }

    #[tokio::test]
    async fn set_trigger_preserves_omitted_timezone_and_max_executions() {
        let store = AutomationStore::in_memory().unwrap();
        let mut params = cron_params("capped");
        params.timezone = Some("Asia/Seoul".into());
        params.max_executions = Some(5);
        let a = store.create_automation(params).await.unwrap();
        // Omitted timezone/max_executions: both preserved.
        store
            .set_trigger(
                &a.id,
                SetTriggerParams {
                    trigger: AutomationTrigger::Cron,
                    cron_pattern: Some("0 12 * * *".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let kept = store.get_automation(&a.id).await.unwrap();
        assert_eq!(kept.timezone.as_deref(), Some("Asia/Seoul"));
        assert_eq!(kept.max_executions, Some(5));
        // Explicit null clears them.
        store
            .set_trigger(
                &a.id,
                SetTriggerParams {
                    trigger: AutomationTrigger::Cron,
                    cron_pattern: Some("0 12 * * *".into()),
                    timezone: Some(None),
                    max_executions: Some(None),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let cleared = store.get_automation(&a.id).await.unwrap();
        assert_eq!(cleared.timezone, None);
        assert_eq!(cleared.max_executions, None);
    }

    #[tokio::test]
    async fn editing_an_exhausted_automation_cannot_rearm_it() {
        let store = AutomationStore::in_memory().unwrap();
        let mut params = cron_params("spent");
        params.max_executions = Some(1);
        let a = store.create_automation(params).await.unwrap();
        // Flip to exhausted the way the runner does.
        store
            .begin_run(&a.id, AutomationRunTrigger::Cron, None)
            .await
            .unwrap();
        assert_eq!(
            store.get_automation(&a.id).await.unwrap().status,
            AutomationStatus::Exhausted
        );
        // A trigger edit preserves the exhausted state and stays dormant.
        store
            .set_trigger(
                &a.id,
                SetTriggerParams {
                    trigger: AutomationTrigger::Heartbeat,
                    cron_pattern: None,
                    timezone: None,
                    heartbeat_interval_secs: Some(600),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let edited = store.get_automation(&a.id).await.unwrap();
        assert_eq!(edited.status, AutomationStatus::Exhausted);
        assert_eq!(edited.next_run_at, None);
        // Reactivation is blocked while exhausted...
        let rearm = store.update_status(&a.id, &AutomationStatus::Active).await;
        assert!(rearm.is_err(), "exhausted cannot reactivate");
        // ...until the cap is raised via set_trigger, which still does not
        // flip status; the explicit Active is what re-arms.
        store
            .set_trigger(
                &a.id,
                SetTriggerParams {
                    trigger: AutomationTrigger::Cron,
                    cron_pattern: Some("0 9 * * *".into()),
                    max_executions: Some(Some(10)),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            store.get_automation(&a.id).await.unwrap().status,
            AutomationStatus::Exhausted
        );
        store
            .update_status(&a.id, &AutomationStatus::Active)
            .await
            .unwrap();
        let armed = store.get_automation(&a.id).await.unwrap();
        assert_eq!(armed.status, AutomationStatus::Active);
        assert!(armed.next_run_at.is_some());
        assert_eq!(armed.max_executions, Some(10));
    }

    #[tokio::test]
    async fn list_runs_newest_first() {
        let store = AutomationStore::in_memory().unwrap();
        let a = store
            .create_automation(cron_params("historian"))
            .await
            .unwrap();
        let r1 = store
            .begin_run(&a.id, AutomationRunTrigger::Cron, None)
            .await
            .unwrap();
        store
            .finish_run(&a.id, &r1.id, true, Some("first".into()), None, None)
            .await
            .unwrap();
        let r2 = store
            .begin_run(&a.id, AutomationRunTrigger::Cron, None)
            .await
            .unwrap();
        store
            .finish_run(
                &a.id,
                &r2.id,
                false,
                Some("second".into()),
                None,
                Some("boom".into()),
            )
            .await
            .unwrap();
        let runs = store.list_runs(&a.id).await.unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].id, r2.id, "newest first");
        assert_eq!(runs[1].id, r1.id);
    }
}
