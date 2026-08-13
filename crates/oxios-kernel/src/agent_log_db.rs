//! Agent history log — SQLite-backed query engine for past agent records.
//!
//! # Architecture
//!
//! Two-tier storage:
//! - **Filesystem JSON** (`state/agents/<uuid>.json`): source of truth.
//!   Human-readable, backup-friendly, rebuildable.
//! - **SQLite** (`state/agent_log.db`): query index with indexes, FTS5.
//!   Fast filtering, sorting, search, aggregation.
//!
//! SQLite DB is rebuildable from filesystem JSON at any time
//! via [`AgentLogDb::reindex_all`].
//!
//! # Availability
//!
//! When no SQLite agent log DB is attached, all query operations
//! fall back to filesystem-only scan mode. Degraded but functional.

use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::config::AgentLogConfig;
use crate::state_store::StateStore;
use crate::types::{AgentInfo, AgentStatus, ToolCallRecord};

// ===========================================================================
// Filter / Query Types
// ===========================================================================

/// Field to search against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchField {
    All,
    Name,
    Error,
    ToolName,
    ToolOutput,
}

impl SearchField {
    #[allow(dead_code)]
    fn as_str(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Name => "name",
            Self::Error => "error",
            Self::ToolName => "tool_name",
            Self::ToolOutput => "tool_output",
        }
    }

    /// Lenient best-effort parser with a default fallback (unknown → All).
    /// Not a `FromStr` impl because it is infallible by design.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "name" => Self::Name,
            "error" => Self::Error,
            "tool_name" => Self::ToolName,
            "tool_output" => Self::ToolOutput,
            _ => Self::All,
        }
    }
}

/// Sort field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    CreatedAt,
    Cost,
    Duration,
    Tokens,
    Name,
}

impl SortBy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CreatedAt => "created_at",
            Self::Cost => "cost_usd",
            Self::Duration => "duration_secs",
            Self::Tokens => "tokens_total",
            Self::Name => "name",
        }
    }

    /// Lenient best-effort parser with a default fallback (unknown → CreatedAt).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "cost" => Self::Cost,
            "duration" => Self::Duration,
            "tokens" => Self::Tokens,
            "name" => Self::Name,
            _ => Self::CreatedAt,
        }
    }
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }

    /// Lenient best-effort parser with a default fallback (unknown → Desc).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "asc" => Self::Asc,
            _ => Self::Desc,
        }
    }
}

/// Full filter specification for querying agent history.
#[derive(Debug, Clone)]
pub struct AgentListFilter {
    pub q: Option<String>,
    pub search_field: SearchField,
    pub status: Option<AgentStatusFilter>,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub model_id: Option<String>,
    pub tool: Option<String>,
    pub has_error: Option<bool>,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
    pub cost_min: Option<f64>,
    pub cost_max: Option<f64>,
    pub tokens_min: Option<u64>,
    pub tokens_max: Option<u64>,
    pub duration_min: Option<u64>,
    pub duration_max: Option<u64>,
    pub sort_by: SortBy,
    pub sort_dir: SortDir,
    pub page: u32,
    pub per_page: u32,
}

impl Default for AgentListFilter {
    fn default() -> Self {
        Self {
            q: None,
            search_field: SearchField::All,
            status: None,
            session_id: None,
            project_id: None,
            model_id: None,
            tool: None,
            has_error: None,
            date_from: None,
            date_to: None,
            cost_min: None,
            cost_max: None,
            tokens_min: None,
            tokens_max: None,
            duration_min: None,
            duration_max: None,
            sort_by: SortBy::CreatedAt,
            sort_dir: SortDir::Desc,
            page: 1,
            per_page: 50,
        }
    }
}

/// Single status value that can be used to filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatusFilter {
    Running,
    Completed,
    Failed,
    Stopped,
    Starting,
    Idle,
}

impl AgentStatusFilter {
    pub fn as_sql(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Idle => "idle",
        }
    }

    /// Lenient best-effort parser; unknown values map to `None`.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "stopped" => Some(Self::Stopped),
            "starting" => Some(Self::Starting),
            "idle" => Some(Self::Idle),
            _ => None,
        }
    }
}

// ===========================================================================
// Filtered stats
// ===========================================================================

/// Aggregated stats computed from a query result set.
#[derive(Debug, Clone, Default)]
pub struct FilteredStats {
    pub total_cost_usd: f64,
    pub total_tokens: u64,
    pub avg_duration_secs: f64,
    pub count_running: u64,
    pub count_completed: u64,
    pub count_failed: u64,
}

/// Global aggregate stats (unfiltered).
#[derive(Debug, Clone, Default)]
pub struct AgentStats {
    pub total_agents: u64,
    pub running: u64,
    pub completed: u64,
    pub failed: u64,
    pub total_cost_usd: f64,
    pub total_tokens: u64,
    pub total_duration_secs: u64,
    pub avg_duration_secs: f64,
    pub avg_cost_usd: f64,
    pub total_sessions: u64,
    pub oldest_agent_at: Option<DateTime<Utc>>,
    pub newest_agent_at: Option<DateTime<Utc>>,
}

/// Query result with pagination and filtered stats.
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub items: Vec<AgentInfo>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
    pub total_pages: u32,
    pub stats: FilteredStats,
}

// ===========================================================================
// Rebuild report
// ===========================================================================

#[derive(Debug, Clone, Default)]
pub struct RebuildReport {
    pub reindexed: u64,
    pub orphaned: u64,
    pub errors: u64,
}
// ===========================================================================
// Cost aggregation (dollar-based spend views over agent_log_db)
// ===========================================================================

/// Aggregate spend summary for a time window.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CostSummary {
    /// Total USD spent in the window.
    pub total_cost_usd: f64,
    /// Total tokens (input + output) consumed in the window.
    pub total_tokens: u64,
    /// Number of agent executions in the window.
    pub agent_count: u64,
}

/// Per-model spend breakdown row.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelCostRow {
    /// Model ID (e.g. `anthropic/claude-sonnet-4`).
    pub model_id: String,
    /// USD spent on this model.
    pub cost_usd: f64,
    /// Tokens consumed on this model.
    pub tokens: u64,
    /// Agent executions using this model.
    pub agent_count: u64,
}

/// Per-project spend breakdown row.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectCostRow {
    /// Project ID (empty string = no project).
    pub project_id: String,
    /// USD spent in this project.
    pub cost_usd: f64,
    /// Tokens consumed in this project.
    pub tokens: u64,
    /// Agent executions in this project.
    pub agent_count: u64,
}

/// Daily spend row for time-series charts.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DailyCostRow {
    /// Local date in `YYYY-MM-DD` format.
    pub date: String,
    /// USD spent that day.
    pub cost_usd: f64,
    /// Tokens consumed that day.
    pub tokens: u64,
    /// Agent executions that day.
    pub agent_count: u64,
}

// ===========================================================================
// AgentLogDb
// ===========================================================================

/// SQLite-backed agent history query engine.
///
/// Interior mutability via `parking_lot::Mutex` so all methods take `&self`,
/// compatible with `Arc<AgentLogDb>` shared across tokio tasks.
pub struct AgentLogDb {
    conn: parking_lot::Mutex<rusqlite::Connection>,
}

impl AgentLogDb {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = rusqlite::Connection::open(path)
            .with_context(|| format!("Failed to open agent log database at {}", path.display()))?;
        let db = Self {
            conn: parking_lot::Mutex::new(conn),
        };
        db.migrate()?;
        db.configure_wal()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS agents (
                id              TEXT PRIMARY KEY,
                name            TEXT NOT NULL,
                status          TEXT NOT NULL,
                created_at      TEXT NOT NULL,
                started_at      TEXT,
                completed_at    TEXT,
                session_id      TEXT,
                project_id      TEXT,
                model_id        TEXT NOT NULL DEFAULT '',
                error           TEXT,
                steps_completed INTEGER NOT NULL DEFAULT 0,
                steps_total     INTEGER,
                tokens_input    INTEGER NOT NULL DEFAULT 0,
                tokens_output   INTEGER NOT NULL DEFAULT 0,
                cost_usd        REAL NOT NULL DEFAULT 0.0,
                duration_secs   INTEGER
            );

            CREATE TABLE IF NOT EXISTS agent_tool_calls (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id    TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                seq         INTEGER NOT NULL,
                tool_name   TEXT NOT NULL,
                input       TEXT NOT NULL DEFAULT '',
                output      TEXT NOT NULL DEFAULT '',
                duration_ms INTEGER NOT NULL DEFAULT 0,
                is_error    INTEGER NOT NULL DEFAULT 0,
                timestamp   TEXT,
                tool_call_id TEXT NOT NULL DEFAULT ''
            );

            CREATE INDEX IF NOT EXISTS idx_agents_status_created ON agents(status, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_agents_session     ON agents(session_id);
            CREATE INDEX IF NOT EXISTS idx_agents_project     ON agents(project_id);
            CREATE INDEX IF NOT EXISTS idx_agents_model       ON agents(model_id);
            CREATE INDEX IF NOT EXISTS idx_agents_cost        ON agents(cost_usd);
            CREATE INDEX IF NOT EXISTS idx_agents_duration    ON agents(duration_secs);
            CREATE INDEX IF NOT EXISTS idx_agents_name        ON agents(name);
            CREATE INDEX IF NOT EXISTS idx_tool_calls_agent   ON agent_tool_calls(agent_id, seq);
            CREATE INDEX IF NOT EXISTS idx_tool_calls_name    ON agent_tool_calls(tool_name);

            CREATE TABLE IF NOT EXISTS agent_log_meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
        ",
        )
        .context("Failed to run agent log SQLite migration")?;

        // FTS5 virtual table (separately, catch errors if FTS5 not available)
        let _ = conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS agent_tool_calls_fts USING fts5(
                tool_name, input, output,
                content='agent_tool_calls',
                content_rowid='id'
            );",
        );

        Ok(())
    }

    fn configure_wal(&self) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .context("Failed to configure WAL mode")
    }

    // ── Upsert ──────────────────────────────────────────────────────

    pub fn upsert_agent(&self, info: &AgentInfo) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn
            .transaction()
            .context("Failed to begin agent upsert transaction")?;

        let duration_secs = match (info.started_at, info.completed_at) {
            (Some(start), Some(end)) => Some((end - start).num_seconds().max(0)),
            _ => None,
        };

        tx.execute(
            "INSERT INTO agents (id, name, status, created_at, started_at, completed_at,
                 session_id, project_id, model_id, error,
                 steps_completed, steps_total, tokens_input, tokens_output, cost_usd, duration_secs)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ON CONFLICT(id) DO UPDATE SET
                 status=excluded.status,
                 completed_at=excluded.completed_at,
                 error=excluded.error,
                 steps_completed=excluded.steps_completed,
                 steps_total=excluded.steps_total,
                 tokens_input=excluded.tokens_input,
                 tokens_output=excluded.tokens_output,
                 cost_usd=excluded.cost_usd,
                 duration_secs=excluded.duration_secs",
            rusqlite::params![
                info.id.to_string(),
                info.name,
                info.status.to_string(),
                info.created_at.to_rfc3339(),
                info.started_at.map(|t| t.to_rfc3339()),
                info.completed_at.map(|t| t.to_rfc3339()),
                info.session_id,
                info.project_id.map(|p| p.to_string()),
                info.model_id,
                info.error,
                info.steps_completed as i64,
                info.steps_total.map(|s| s as i64),
                info.tokens_input as i64,
                info.tokens_output as i64,
                info.cost_usd,
                duration_secs,
            ],
        )?;

        // Replace tool calls
        tx.execute(
            "DELETE FROM agent_tool_calls WHERE agent_id = ?1",
            rusqlite::params![info.id.to_string()],
        )?;

        for (i, tc) in info.tool_calls.iter().enumerate() {
            tx.execute(
                "INSERT INTO agent_tool_calls (agent_id, seq, tool_name, input, output,
                     duration_ms, is_error, timestamp, tool_call_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    info.id.to_string(),
                    i as i64,
                    tc.tool,
                    tc.input,
                    tc.output,
                    tc.duration_ms as i64,
                    tc.is_error as i64,
                    tc.timestamp.map(|t| t.to_rfc3339()),
                    tc.tool_call_id,
                ],
            )?;
        }

        tx.commit().context("Failed to commit agent upsert")?;

        // Rebuild FTS
        let _ = conn.execute_batch(
            "INSERT INTO agent_tool_calls_fts(agent_tool_calls_fts) VALUES('rebuild');",
        );

        Ok(())
    }

    // ── Query ───────────────────────────────────────────────────────

    pub fn query(&self, filter: &AgentListFilter) -> Result<QueryResult> {
        let (where_clause, params) = self.build_where(filter);
        let offset = ((filter.page.max(1) - 1) * filter.per_page) as i64;
        let limit = filter.per_page.min(200) as i64;

        let conn = self.conn.lock();

        // Count
        let count_sql = format!("SELECT COUNT(*) FROM agents WHERE {}", where_clause);
        let total: u64 = conn
            .query_row(
                &count_sql,
                rusqlite::params_from_iter(params.iter()),
                |row| row.get::<_, i64>(0),
            )
            .context("Failed to count agents")? as u64;

        // Data
        let safe_sort_col = filter.sort_by.as_str();
        let safe_sort_dir = filter.sort_dir.as_str();
        let param_count = params.len();
        let data_sql = format!(
            "SELECT * FROM agents WHERE {} ORDER BY {} {} LIMIT ?{} OFFSET ?{}",
            where_clause,
            safe_sort_col,
            safe_sort_dir,
            param_count + 1,
            param_count + 2,
        );

        let mut stmt = conn.prepare(&data_sql)?;
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = params
            .into_iter()
            .map(|p| -> Box<dyn rusqlite::types::ToSql> { p })
            .collect();
        all_params.push(Box::new(limit));
        all_params.push(Box::new(offset));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| p.as_ref()).collect();

        let items: Vec<AgentInfo> = stmt
            .query_map(param_refs.as_slice(), Self::row_to_agent)?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to collect agent query results")?;

        // Stats for filtered set
        let stats = self.filtered_stats_inner(filter, &conn)?;

        let total_pages = if total == 0 {
            1
        } else {
            ((total as f64) / filter.per_page as f64).ceil() as u32
        };

        Ok(QueryResult {
            items,
            total,
            page: filter.page,
            per_page: filter.per_page,
            total_pages,
            stats,
        })
    }

    pub fn stats(&self) -> Result<AgentStats> {
        let conn = self.conn.lock();

        let mut s = AgentStats::default();

        let row = conn
            .query_row(
                "SELECT
                    COUNT(*) as total,
                    COALESCE(SUM(CASE WHEN status='running' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status IN ('completed','stopped') THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status='failed' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(cost_usd), 0.0),
                    COALESCE(SUM(tokens_input + tokens_output), 0),
                    COALESCE(SUM(duration_secs), 0),
                    COALESCE(AVG(duration_secs), 0.0),
                    COALESCE(AVG(cost_usd), 0.0),
                    COUNT(DISTINCT session_id),
                    MIN(created_at),
                    MAX(created_at)
                 FROM agents",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)? as u64,
                        row.get::<_, i64>(1)? as u64,
                        row.get::<_, i64>(2)? as u64,
                        row.get::<_, i64>(3)? as u64,
                        row.get::<_, f64>(4)?,
                        row.get::<_, i64>(5)? as u64,
                        row.get::<_, i64>(6)? as u64,
                        row.get::<_, f64>(7)?,
                        row.get::<_, f64>(8)?,
                        row.get::<_, i64>(9)? as u64,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, Option<String>>(11)?,
                    ))
                },
            )
            .context("Failed to query agent stats")?;

        s.total_agents = row.0;
        s.running = row.1;
        s.completed = row.2;
        s.failed = row.3;
        s.total_cost_usd = row.4;
        s.total_tokens = row.5;
        s.total_duration_secs = row.6;
        s.avg_duration_secs = if s.total_agents > 0 { row.7 } else { 0.0 };
        s.avg_cost_usd = if s.total_agents > 0 { row.8 } else { 0.0 };
        s.total_sessions = row.9;
        s.oldest_agent_at = row
            .10
            .as_deref()
            .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
            .map(|dt| dt.with_timezone(&Utc));
        s.newest_agent_at = row
            .11
            .as_deref()
            .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
            .map(|dt| dt.with_timezone(&Utc));

        Ok(s)
    }
    // ── Cost aggregation (dollar-based spend views) ─────────────────

    /// Aggregate spend for a time window (`since` = inclusive lower bound).
    ///
    /// When `since` is `None`, returns all-time totals.
    pub fn cost_summary(&self, since: Option<DateTime<Utc>>) -> Result<CostSummary> {
        let conn = self.conn.lock();
        match since {
            Some(ts) => {
                let row = conn
                    .query_row(
                        "SELECT COALESCE(SUM(cost_usd), 0.0),
                                COALESCE(SUM(tokens_input + tokens_output), 0),
                                COUNT(*)
                         FROM agents WHERE created_at >= ?1",
                        rusqlite::params![ts.to_rfc3339()],
                        |row| {
                            Ok(CostSummary {
                                total_cost_usd: row.get(0)?,
                                total_tokens: row.get::<_, i64>(1)? as u64,
                                agent_count: row.get::<_, i64>(2)? as u64,
                            })
                        },
                    )
                    .context("Failed to compute cost summary")?;
                Ok(row)
            }
            None => {
                let row = conn
                    .query_row(
                        "SELECT COALESCE(SUM(cost_usd), 0.0),
                                COALESCE(SUM(tokens_input + tokens_output), 0),
                                COUNT(*)
                         FROM agents",
                        [],
                        |row| {
                            Ok(CostSummary {
                                total_cost_usd: row.get(0)?,
                                total_tokens: row.get::<_, i64>(1)? as u64,
                                agent_count: row.get::<_, i64>(2)? as u64,
                            })
                        },
                    )
                    .context("Failed to compute cost summary")?;
                Ok(row)
            }
        }
    }

    /// Per-model spend breakdown for a time window.
    pub fn cost_by_model(&self, since: Option<DateTime<Utc>>) -> Result<Vec<ModelCostRow>> {
        let conn = self.conn.lock();
        let since_ts = since.map(|t| t.to_rfc3339());
        let mut stmt = if since_ts.is_some() {
            conn.prepare(
                "SELECT model_id,
                        COALESCE(SUM(cost_usd), 0.0),
                        COALESCE(SUM(tokens_input + tokens_output), 0),
                        COUNT(*)
                 FROM agents
                 WHERE created_at >= ?1 AND model_id != ''
                 GROUP BY model_id
                 ORDER BY SUM(cost_usd) DESC",
            )?
        } else {
            conn.prepare(
                "SELECT model_id,
                        COALESCE(SUM(cost_usd), 0.0),
                        COALESCE(SUM(tokens_input + tokens_output), 0),
                        COUNT(*)
                 FROM agents
                 WHERE model_id != ''
                 GROUP BY model_id
                 ORDER BY SUM(cost_usd) DESC",
            )?
        };

        let map = |row: &rusqlite::Row| -> rusqlite::Result<ModelCostRow> {
            Ok(ModelCostRow {
                model_id: row.get(0)?,
                cost_usd: row.get(1)?,
                tokens: row.get::<_, i64>(2)? as u64,
                agent_count: row.get::<_, i64>(3)? as u64,
            })
        };

        let rows = match &since_ts {
            Some(ts) => stmt.query_map(rusqlite::params![ts], map)?,
            None => stmt.query_map([], map)?,
        };
        rows.collect::<Result<Vec<_>, _>>()
            .context("Failed to collect cost-by-model rows")
    }

    /// Per-project spend breakdown for a time window.
    pub fn cost_by_project(&self, since: Option<DateTime<Utc>>) -> Result<Vec<ProjectCostRow>> {
        let conn = self.conn.lock();
        let since_ts = since.map(|t| t.to_rfc3339());
        let mut stmt = if since_ts.is_some() {
            conn.prepare(
                "SELECT COALESCE(project_id, ''),
                        COALESCE(SUM(cost_usd), 0.0),
                        COALESCE(SUM(tokens_input + tokens_output), 0),
                        COUNT(*)
                 FROM agents
                 WHERE created_at >= ?1 AND project_id IS NOT NULL AND project_id != ''
                 GROUP BY project_id
                 ORDER BY SUM(cost_usd) DESC",
            )?
        } else {
            conn.prepare(
                "SELECT COALESCE(project_id, ''),
                        COALESCE(SUM(cost_usd), 0.0),
                        COALESCE(SUM(tokens_input + tokens_output), 0),
                        COUNT(*)
                 FROM agents
                 WHERE project_id IS NOT NULL AND project_id != ''
                 GROUP BY project_id
                 ORDER BY SUM(cost_usd) DESC",
            )?
        };

        let map = |row: &rusqlite::Row| -> rusqlite::Result<ProjectCostRow> {
            Ok(ProjectCostRow {
                project_id: row.get(0)?,
                cost_usd: row.get(1)?,
                tokens: row.get::<_, i64>(2)? as u64,
                agent_count: row.get::<_, i64>(3)? as u64,
            })
        };

        let rows = match &since_ts {
            Some(ts) => stmt.query_map(rusqlite::params![ts], map)?,
            None => stmt.query_map([], map)?,
        };
        rows.collect::<Result<Vec<_>, _>>()
            .context("Failed to collect cost-by-project rows")
    }

    /// Daily spend time-series for the last `days` days (inclusive of today).
    pub fn cost_daily(&self, days: u32) -> Result<Vec<DailyCostRow>> {
        let conn = self.conn.lock();
        let since = Utc::now() - chrono::Duration::days(days as i64);
        let mut stmt = conn.prepare(
            "SELECT date(created_at) AS d,
                    COALESCE(SUM(cost_usd), 0.0),
                    COALESCE(SUM(tokens_input + tokens_output), 0),
                    COUNT(*)
             FROM agents
             WHERE created_at >= ?1
             GROUP BY d
             ORDER BY d ASC",
        )?;

        let rows = stmt.query_map(rusqlite::params![since.to_rfc3339()], |row| {
            Ok(DailyCostRow {
                date: row.get(0)?,
                cost_usd: row.get(1)?,
                tokens: row.get::<_, i64>(2)? as u64,
                agent_count: row.get::<_, i64>(3)? as u64,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .context("Failed to collect daily cost rows")
    }

    pub fn get(&self, id: &str) -> Result<Option<AgentInfo>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT * FROM agents WHERE id = ?1")
            .context("Failed to prepare agent get statement")?;

        let mut rows = stmt
            .query_map(rusqlite::params![id], Self::row_to_agent)
            .context("Failed to query agent by id")?;

        match rows.next() {
            Some(Ok(mut agent)) => {
                agent.tool_calls = Self::get_tool_calls_inner(&conn, id)?;
                Ok(Some(agent))
            }
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    pub fn get_tool_calls(&self, agent_id: &str) -> Result<Vec<ToolCallRecord>> {
        let conn = self.conn.lock();
        Self::get_tool_calls_inner(&conn, agent_id)
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock();
        let changes = conn
            .execute("DELETE FROM agents WHERE id = ?1", rusqlite::params![id])
            .context("Failed to delete agent")?;
        Ok(changes > 0)
    }

    // ── Pruning ─────────────────────────────────────────────────────

    pub fn prune(&self, config: &AgentLogConfig) -> Result<usize> {
        let mut pruned = 0usize;

        if config.ttl_hours > 0 || config.max_entries > 0 {
            let conn = self.conn.lock();

            // 1. TTL-based
            if config.ttl_hours > 0 {
                let cutoff = Utc::now() - chrono::Duration::hours(config.ttl_hours as i64);
                let cutoff_str = cutoff.to_rfc3339();
                let deleted = conn
                    .execute(
                        "DELETE FROM agents WHERE created_at < ?1",
                        rusqlite::params![cutoff_str],
                    )
                    .context("Failed to prune agents by TTL")?;
                pruned += deleted;
            }

            // 2. Count-based
            if config.max_entries > 0 {
                let count: i64 = conn
                    .query_row("SELECT COUNT(*) FROM agents", [], |row| row.get(0))
                    .context("Failed to count agents for pruning")?;

                if count > config.max_entries as i64 {
                    let excess = count - config.max_entries as i64;
                    let to_delete = excess.min(config.prune_batch_size as i64);
                    let deleted = conn
                        .execute(
                            "DELETE FROM agents WHERE id IN (
                            SELECT id FROM agents ORDER BY created_at ASC LIMIT ?1
                        )",
                            rusqlite::params![to_delete],
                        )
                        .context("Failed to prune agents by count")?;
                    // Report the rows actually deleted, not the estimate:
                    // FK CASCADE or a concurrent prune can make the real
                    // count differ from `to_delete`. (state-area F7.)
                    pruned += deleted;
                }
            }
        }

        if pruned > 0 {
            tracing::info!(pruned = pruned, "Agent log SQLite pruning completed");
        }

        Ok(pruned)
    }

    // ── Recovery ────────────────────────────────────────────────────

    pub async fn reindex_all(&self, state_store: &StateStore) -> Result<RebuildReport> {
        let agent_names = state_store
            .list_category("agents")
            .await
            .unwrap_or_default();
        let mut report = RebuildReport::default();

        {
            let conn = self.conn.lock();
            conn.execute_batch("DELETE FROM agent_tool_calls; DELETE FROM agents;")
                .context("Failed to clear agent tables for reindex")?;
        }

        for name in &agent_names {
            match state_store.load_json::<AgentInfo>("agents", name).await {
                Ok(Some(agent)) => {
                    if let Err(e) = self.upsert_agent(&agent) {
                        tracing::warn!(agent_id = %name, error = %e, "Failed to reindex agent");
                        report.errors += 1;
                    } else {
                        report.reindexed += 1;
                    }
                }
                _ => {
                    report.errors += 1;
                }
            }
        }

        Ok(report)
    }

    // ── Internal Helpers ────────────────────────────────────────────

    fn row_to_agent(row: &rusqlite::Row) -> rusqlite::Result<AgentInfo> {
        let id_str: String = row.get("id")?;
        let project_id_str: Option<String> = row.get("project_id")?;
        let created_at_str: String = row.get("created_at")?;
        let started_at_str: Option<String> = row.get("started_at")?;
        let completed_at_str: Option<String> = row.get("completed_at")?;
        Ok(AgentInfo {
            id: uuid::Uuid::parse_str(&id_str).unwrap_or_else(|e| {
                tracing::error!(agent_id = %id_str, error = %e, "Corrupt agent id in DB, substituting Nil");
                uuid::Uuid::nil()
            }),
            name: row.get("name")?,
            status: Self::parse_status(&row.get::<_, String>("status")?),
            created_at: DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|e| {
                    tracing::error!(
                        agent_id = %id_str,
                        created_at = %created_at_str,
                        error = %e,
                        "Corrupt created_at in DB, substituting Utc::now()"
                    );
                    Utc::now()
                }),
            project_id: project_id_str.and_then(|s| uuid::Uuid::parse_str(&s).ok()),
            started_at: started_at_str
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc)),
            completed_at: completed_at_str
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc)),
            error: row.get("error")?,
            steps_completed: row.get::<_, i64>("steps_completed")? as usize,
            steps_total: row
                .get::<_, Option<i64>>("steps_total")?
                .map(|v| v as usize),
            tool_calls: vec![],
            tokens_input: row.get::<_, i64>("tokens_input")? as u64,
            tokens_output: row.get::<_, i64>("tokens_output")? as u64,
            cost_usd: row.get("cost_usd")?,
            model_id: row.get("model_id")?,
            session_id: row.get("session_id")?,
        })
    }

    fn get_tool_calls_inner(
        conn: &rusqlite::Connection,
        agent_id: &str,
    ) -> Result<Vec<ToolCallRecord>> {
        let mut stmt = conn
            .prepare(
                "SELECT tool_name, input, output, duration_ms, is_error, timestamp, tool_call_id
                 FROM agent_tool_calls WHERE agent_id = ?1 ORDER BY seq",
            )
            .context("Failed to prepare tool calls statement")?;

        let calls = stmt
            .query_map(rusqlite::params![agent_id], |row| {
                let ts: Option<String> = row.get(5)?;
                Ok(ToolCallRecord {
                    tool: row.get(0)?,
                    input: row.get(1)?,
                    output: row.get(2)?,
                    duration_ms: row.get::<_, i64>(3)? as u64,
                    is_error: row.get::<_, i64>(4)? != 0,
                    tool_call_id: row.get(6)?,
                    timestamp: ts
                        .and_then(|s| DateTime::parse_from_rfc3339(&s as &str).ok())
                        .map(|dt| dt.with_timezone(&Utc)),
                })
            })
            .context("Failed to query tool calls")?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to collect tool calls")?;

        Ok(calls)
    }

    fn parse_status(s: &str) -> AgentStatus {
        match s {
            "starting" => AgentStatus::Starting,
            "running" => AgentStatus::Running,
            "idle" => AgentStatus::Idle,
            "stopped" => AgentStatus::Stopped,
            "failed" => AgentStatus::Failed,
            "completed" => AgentStatus::Completed,
            _ => AgentStatus::Idle,
        }
    }

    /// Build WHERE clause and params from the filter.
    fn build_where(
        &self,
        filter: &AgentListFilter,
    ) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
        let mut conditions: Vec<String> = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        // Status filter
        if let Some(status) = filter.status {
            let idx = params.len() + 1;
            conditions.push(format!("status = ?{}", idx));
            params.push(Box::new(status.as_sql().to_string()));
        }

        // Date range
        if let Some(from) = filter.date_from {
            let idx = params.len() + 1;
            conditions.push(format!("created_at >= ?{}", idx));
            params.push(Box::new(from.to_rfc3339()));
        }
        if let Some(to) = filter.date_to {
            let idx = params.len() + 1;
            conditions.push(format!("created_at <= ?{}", idx));
            params.push(Box::new(to.to_rfc3339()));
        }

        // Session / project
        if let Some(ref sid) = filter.session_id {
            let idx = params.len() + 1;
            conditions.push(format!("session_id = ?{}", idx));
            params.push(Box::new(sid.clone()));
        }
        if let Some(ref pid) = filter.project_id {
            let idx = params.len() + 1;
            conditions.push(format!("project_id = ?{}", idx));
            params.push(Box::new(pid.clone()));
        }

        // Model (substring)
        if let Some(ref model) = filter.model_id {
            let idx = params.len() + 1;
            conditions.push(format!("model_id LIKE ?{}", idx));
            params.push(Box::new(format!("%{}%", model)));
        }

        // Cost range
        if let Some(min) = filter.cost_min {
            let idx = params.len() + 1;
            conditions.push(format!("cost_usd >= ?{}", idx));
            params.push(Box::new(min));
        }
        if let Some(max) = filter.cost_max {
            let idx = params.len() + 1;
            conditions.push(format!("cost_usd <= ?{}", idx));
            params.push(Box::new(max));
        }

        // Tokens range
        if filter.tokens_min.is_some() || filter.tokens_max.is_some() {
            let total_expr = "(tokens_input + tokens_output)";
            if let Some(min) = filter.tokens_min {
                let idx = params.len() + 1;
                conditions.push(format!("{} >= ?{}", total_expr, idx));
                params.push(Box::new(min as i64));
            }
            if let Some(max) = filter.tokens_max {
                let idx = params.len() + 1;
                conditions.push(format!("{} <= ?{}", total_expr, idx));
                params.push(Box::new(max as i64));
            }
        }

        // Duration range
        if let Some(min) = filter.duration_min {
            let idx = params.len() + 1;
            conditions.push(format!("duration_secs >= ?{}", idx));
            params.push(Box::new(min as i64));
        }
        if let Some(max) = filter.duration_max {
            let idx = params.len() + 1;
            conditions.push(format!("duration_secs <= ?{}", idx));
            params.push(Box::new(max as i64));
        }

        // Error filter
        if let Some(has_err) = filter.has_error {
            if has_err {
                conditions.push("error IS NOT NULL AND error != ''".to_string());
            } else {
                conditions.push("(error IS NULL OR error = '')".to_string());
            }
        }

        // Tool filter (JOIN subquery)
        if let Some(ref tool) = filter.tool {
            let idx = params.len() + 1;
            conditions.push(format!(
                "id IN (SELECT DISTINCT agent_id FROM agent_tool_calls WHERE tool_name LIKE ?{})",
                idx
            ));
            params.push(Box::new(format!("%{}%", tool)));
        }

        // Full-text search via FTS5
        let has_fts = matches!(
            filter.search_field,
            SearchField::All | SearchField::ToolName | SearchField::ToolOutput
        ) && filter.q.is_some();

        if has_fts {
            if let Some(ref q) = filter.q {
                // Treat user input as an FTS5 *phrase* (double-quoted
                // string) so FTS5 query syntax — `*` wildcards, `:`
                // column qualifiers, `AND`/`OR`/`NEAR`, parentheses —
                // is treated as literal text, not operators. Internal
                // double-quotes are escaped by doubling (FTS5 rule).
                // Bound as a parameter so it never reaches the SQL parser
                // as code. (state-area F8.)
                let phrase = format!("\"{}\"", q.replace('"', "\"\""));
                let idx = params.len() + 1;
                conditions.push(format!(
                    "id IN (SELECT DISTINCT agent_id FROM agent_tool_calls_fts WHERE agent_tool_calls_fts MATCH ?{idx})"
                ));
                params.push(Box::new(phrase));
            }
        } else if let Some(ref q) = filter.q {
            match filter.search_field {
                SearchField::Name => {
                    let idx = params.len() + 1;
                    conditions.push(format!("name LIKE ?{}", idx));
                    params.push(Box::new(format!("%{}%", q)));
                }
                SearchField::Error => {
                    let idx = params.len() + 1;
                    conditions.push(format!("error LIKE ?{}", idx));
                    params.push(Box::new(format!("%{}%", q)));
                }
                _ => {
                    let idx1 = params.len() + 1;
                    let idx2 = params.len() + 2;
                    conditions.push(format!("(name LIKE ?{} OR error LIKE ?{})", idx1, idx2));
                    params.push(Box::new(format!("%{}%", q)));
                    params.push(Box::new(format!("%{}%", q)));
                }
            }
        }

        if conditions.is_empty() {
            ("1=1".to_string(), params)
        } else {
            (conditions.join(" AND "), params)
        }
    }

    /// Aggregate stats constrained to a filter (acquires the connection lock).
    /// Reserved for filtered dashboard endpoints; currently `list()` reuses the
    /// inner helper directly to avoid a double lock.
    #[allow(dead_code)]
    fn filtered_stats(&self, filter: &AgentListFilter) -> Result<FilteredStats> {
        let conn = self.conn.lock();
        self.filtered_stats_inner(filter, &conn)
    }

    fn filtered_stats_inner(
        &self,
        filter: &AgentListFilter,
        conn: &rusqlite::Connection,
    ) -> Result<FilteredStats> {
        let (where_clause, params) = self.build_where(filter);

        let sql = format!(
            "SELECT
                COALESCE(SUM(cost_usd), 0.0),
                COALESCE(SUM(tokens_input + tokens_output), 0),
                COALESCE(AVG(duration_secs), 0.0),
                COALESCE(SUM(CASE WHEN status='running' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status IN ('completed','stopped') THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status='failed' THEN 1 ELSE 0 END), 0)
             FROM agents WHERE {}",
            where_clause
        );

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let row = stmt
            .query_row(param_refs.as_slice(), |row| {
                Ok(FilteredStats {
                    total_cost_usd: row.get(0)?,
                    total_tokens: row.get::<_, i64>(1)? as u64,
                    avg_duration_secs: row.get(2)?,
                    count_running: row.get::<_, i64>(3)? as u64,
                    count_completed: row.get::<_, i64>(4)? as u64,
                    count_failed: row.get::<_, i64>(5)? as u64,
                })
            })
            .context("Failed to compute filtered stats")?;

        Ok(row)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AgentStatus;

    fn make_test_db() -> (AgentLogDb, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent_log.db");
        let db = AgentLogDb::open(&path).unwrap();
        (db, dir)
    }

    fn sample_agent(id: &str, status: AgentStatus, created: &str, cost: f64) -> AgentInfo {
        AgentInfo {
            id: uuid::Uuid::parse_str(id).unwrap(),
            name: format!("test-agent-{}", &id[..8]),
            status,
            created_at: DateTime::parse_from_rfc3339(created)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap(),
            project_id: None,
            started_at: None,
            completed_at: None,
            error: None,
            steps_completed: 5,
            steps_total: Some(10),
            tool_calls: vec![],
            tokens_input: 1000,
            tokens_output: 500,
            cost_usd: cost,
            model_id: "anthropic/claude-sonnet-4".into(),
            session_id: None,
        }
    }

    #[test]
    fn test_upsert_and_query() {
        let (db, _dir) = make_test_db();

        let agent = sample_agent(
            "550e8400-e29b-41d4-a716-446655440000",
            AgentStatus::Completed,
            "2026-06-01T00:00:00Z",
            0.05,
        );

        db.upsert_agent(&agent).unwrap();

        let filter = AgentListFilter::default();
        let result = db.query(&filter).unwrap();
        assert_eq!(result.total, 1);
        assert_eq!(result.items[0].name, "test-agent-550e8400");
        assert_eq!(result.items[0].status, AgentStatus::Completed);
    }

    #[test]
    fn test_filter_by_status() {
        let (db, _dir) = make_test_db();

        for i in 0..5 {
            let status = if i % 2 == 0 {
                AgentStatus::Completed
            } else {
                AgentStatus::Failed
            };
            db.upsert_agent(&sample_agent(
                &format!("550e8400-e29b-41d4-a716-44665544000{}", i),
                status,
                "2026-06-01T00:00:00Z",
                0.01,
            ))
            .unwrap();
        }

        let filter = AgentListFilter {
            status: Some(AgentStatusFilter::Failed),
            ..Default::default()
        };
        let result = db.query(&filter).unwrap();
        assert_eq!(result.total, 2);
    }

    #[test]
    fn test_filter_by_date_range() {
        let (db, _dir) = make_test_db();

        for (i, (created, cost)) in [
            ("2026-06-01T00:00:00Z", 0.01),
            ("2026-06-10T00:00:00Z", 0.02),
            ("2026-06-20T00:00:00Z", 0.03),
        ]
        .iter()
        .enumerate()
        {
            db.upsert_agent(&sample_agent(
                &format!("550e8400-e29b-41d4-a716-44665544000{}", i),
                AgentStatus::Completed,
                created,
                *cost,
            ))
            .unwrap();
        }

        let filter = AgentListFilter {
            date_from: Some(
                DateTime::parse_from_rfc3339("2026-06-05T00:00:00Z")
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap(),
            ),
            date_to: Some(
                DateTime::parse_from_rfc3339("2026-06-15T00:00:00Z")
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap(),
            ),
            ..Default::default()
        };
        let result = db.query(&filter).unwrap();
        assert_eq!(result.total, 1);
    }

    #[test]
    fn test_search_by_name() {
        let (db, _dir) = make_test_db();

        let mut agent = sample_agent(
            "550e8400-e29b-41d4-a716-446655440000",
            AgentStatus::Completed,
            "2026-06-01T00:00:00Z",
            0.01,
        );
        agent.name = "Refactor authentication module".into();
        db.upsert_agent(&agent).unwrap();

        let mut agent2 = sample_agent(
            "550e8400-e29b-41d4-a716-446655440001",
            AgentStatus::Failed,
            "2026-06-02T00:00:00Z",
            0.02,
        );
        agent2.name = "Fix build error".into();
        db.upsert_agent(&agent2).unwrap();

        let filter = AgentListFilter {
            q: Some("Refactor".into()),
            search_field: SearchField::Name,
            ..Default::default()
        };
        let result = db.query(&filter).unwrap();
        assert_eq!(result.total, 1);
        assert!(result.items[0].name.contains("Refactor"));
    }

    #[test]
    fn test_sorting() {
        let (db, _dir) = make_test_db();

        for (i, cost) in [0.10, 0.01, 0.50].iter().enumerate() {
            db.upsert_agent(&sample_agent(
                &format!("550e8400-e29b-41d4-a716-44665544000{}", i),
                AgentStatus::Completed,
                "2026-06-01T00:00:00Z",
                *cost,
            ))
            .unwrap();
        }

        let filter = AgentListFilter {
            sort_by: SortBy::Cost,
            sort_dir: SortDir::Desc,
            ..Default::default()
        };
        let result = db.query(&filter).unwrap();
        assert_eq!(result.items[0].cost_usd, 0.50);
        assert_eq!(result.items[1].cost_usd, 0.10);
        assert_eq!(result.items[2].cost_usd, 0.01);
    }

    #[test]
    fn test_pagination() {
        let (db, _dir) = make_test_db();

        for i in 0..10 {
            db.upsert_agent(&sample_agent(
                &format!("550e8400-e29b-41d4-a716-44665544000{}", i),
                AgentStatus::Completed,
                &format!("2026-06-{:02}T00:00:00Z", i + 1),
                0.01,
            ))
            .unwrap();
        }

        let mut filter = AgentListFilter {
            per_page: 3,
            page: 1,
            ..Default::default()
        };
        let result = db.query(&filter).unwrap();
        assert_eq!(result.items.len(), 3);
        assert_eq!(result.total_pages, 4);

        filter.page = 2;
        let result = db.query(&filter).unwrap();
        assert_eq!(result.items.len(), 3);
    }

    #[test]
    fn test_stats() {
        let (db, _dir) = make_test_db();

        for (i, (status, cost)) in [
            (AgentStatus::Completed, 0.05),
            (AgentStatus::Completed, 0.03),
            (AgentStatus::Failed, 0.01),
            (AgentStatus::Running, 0.02),
        ]
        .iter()
        .enumerate()
        {
            db.upsert_agent(&sample_agent(
                &format!("550e8400-e29b-41d4-a716-44665544000{}", i),
                *status,
                "2026-06-01T00:00:00Z",
                *cost,
            ))
            .unwrap();
        }

        let stats = db.stats().unwrap();
        assert_eq!(stats.total_agents, 4);
        assert_eq!(stats.completed, 2);
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.running, 1);
        assert!((stats.total_cost_usd - 0.11).abs() < 0.001);
    }

    #[test]
    fn test_prune_by_ttl() {
        let (db, _dir) = make_test_db();

        // Anchor timestamps to `now` so the TTL boundary is deterministic
        // regardless of when the test runs (hardcoded calendar dates rot).
        let old = (Utc::now() - chrono::Duration::hours(800)).to_rfc3339();
        let recent = (Utc::now() - chrono::Duration::hours(100)).to_rfc3339();

        db.upsert_agent(&sample_agent(
            "11111111-1111-4114-a716-446655440000",
            AgentStatus::Completed,
            &old,
            0.01,
        ))
        .unwrap();

        db.upsert_agent(&sample_agent(
            "22222222-2222-4114-a716-446655440000",
            AgentStatus::Completed,
            &recent,
            0.02,
        ))
        .unwrap();

        let config = AgentLogConfig {
            ttl_hours: 720,
            ..Default::default()
        };
        let pruned = db.prune(&config).unwrap();
        assert!(pruned > 0);

        let result = db.query(&AgentListFilter::default()).unwrap();
        assert_eq!(result.total, 1);
        assert_eq!(
            result.items[0].id.to_string(),
            "22222222-2222-4114-a716-446655440000"
        );
    }

    #[test]
    fn test_delete() {
        let (db, _dir) = make_test_db();

        db.upsert_agent(&sample_agent(
            "550e8400-e29b-41d4-a716-446655440000",
            AgentStatus::Completed,
            "2026-06-01T00:00:00Z",
            0.01,
        ))
        .unwrap();

        assert!(db.delete("550e8400-e29b-41d4-a716-446655440000").unwrap());
        assert!(!db.delete("nonexistent").unwrap());

        let result = db.query(&AgentListFilter::default()).unwrap();
        assert_eq!(result.total, 0);
    }

    #[test]
    fn test_get_tool_calls() {
        let (db, _dir) = make_test_db();

        let mut agent = sample_agent(
            "550e8400-e29b-41d4-a716-446655440000",
            AgentStatus::Completed,
            "2026-06-01T00:00:00Z",
            0.01,
        );
        agent.tool_calls = vec![
            ToolCallRecord {
                tool: "bash".into(),
                input: "ls -la".into(),
                output: "total 42".into(),
                duration_ms: 150,
                is_error: false,
                tool_call_id: "call_1".into(),
                timestamp: Some(Utc::now()),
            },
            ToolCallRecord {
                tool: "read".into(),
                input: "file.rs".into(),
                output: "fn main()".into(),
                duration_ms: 5,
                is_error: false,
                tool_call_id: "call_2".into(),
                timestamp: Some(Utc::now()),
            },
        ];
        db.upsert_agent(&agent).unwrap();

        let calls = db
            .get_tool_calls("550e8400-e29b-41d4-a716-446655440000")
            .unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].tool, "bash");
        assert_eq!(calls[1].tool, "read");
    }

    // ── Cost aggregation tests ──────────────────────────────────────

    /// Helper: build an agent with configurable model/project/tokens.
    fn cost_agent(
        id: &str,
        model: &str,
        project: Option<&str>,
        created: &str,
        cost: f64,
        tokens_in: u64,
        tokens_out: u64,
    ) -> AgentInfo {
        let mut a = sample_agent(id, AgentStatus::Completed, created, cost);
        a.model_id = model.into();
        a.project_id = project.map(|p| uuid::Uuid::parse_str(p).unwrap());
        a.tokens_input = tokens_in;
        a.tokens_output = tokens_out;
        a
    }

    #[test]
    fn test_cost_summary_all_time() {
        let (db, _dir) = make_test_db();
        db.upsert_agent(&cost_agent(
            "550e8400-e29b-41d4-a716-446655440001",
            "anthropic/claude-sonnet-4",
            None,
            "2025-01-15T10:00:00Z",
            0.10,
            1000,
            500,
        ))
        .unwrap();
        db.upsert_agent(&cost_agent(
            "550e8400-e29b-41d4-a716-446655440002",
            "openai/gpt-4o",
            None,
            "2025-06-20T10:00:00Z",
            0.25,
            2000,
            1000,
        ))
        .unwrap();

        let s = db.cost_summary(None).unwrap();
        assert!((s.total_cost_usd - 0.35).abs() < 1e-9);
        assert_eq!(s.total_tokens, 4500); // (1000+500) + (2000+1000)
        assert_eq!(s.agent_count, 2);
    }

    #[test]
    fn test_cost_summary_time_window() {
        let (db, _dir) = make_test_db();
        db.upsert_agent(&cost_agent(
            "550e8400-e29b-41d4-a716-446655440001",
            "anthropic/claude-sonnet-4",
            None,
            "2025-01-15T10:00:00Z",
            0.10,
            100,
            50,
        ))
        .unwrap();
        db.upsert_agent(&cost_agent(
            "550e8400-e29b-41d4-a716-446655440002",
            "openai/gpt-4o",
            None,
            "2025-06-24T10:00:00Z",
            0.25,
            200,
            100,
        ))
        .unwrap();

        // Window starting 2025-06-01 should exclude the January agent.
        let since = DateTime::parse_from_rfc3339("2025-06-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let s = db.cost_summary(Some(since)).unwrap();
        assert!((s.total_cost_usd - 0.25).abs() < 1e-9);
        assert_eq!(s.agent_count, 1);
    }

    #[test]
    fn test_cost_by_model_groups_and_sorts() {
        let (db, _dir) = make_test_db();
        db.upsert_agent(&cost_agent(
            "550e8400-e29b-41d4-a716-446655440001",
            "openai/gpt-4o",
            None,
            "2025-06-20T10:00:00Z",
            0.05,
            100,
            50,
        ))
        .unwrap();
        db.upsert_agent(&cost_agent(
            "550e8400-e29b-41d4-a716-446655440002",
            "openai/gpt-4o",
            None,
            "2025-06-21T10:00:00Z",
            0.10,
            100,
            50,
        ))
        .unwrap();
        db.upsert_agent(&cost_agent(
            "550e8400-e29b-41d4-a716-446655440003",
            "anthropic/claude-sonnet-4",
            None,
            "2025-06-22T10:00:00Z",
            0.50,
            500,
            200,
        ))
        .unwrap();

        let rows = db.cost_by_model(None).unwrap();
        assert_eq!(rows.len(), 2);
        // Sorted by cost desc: claude (0.50) first, then gpt-4o (0.15).
        assert_eq!(rows[0].model_id, "anthropic/claude-sonnet-4");
        assert!((rows[0].cost_usd - 0.50).abs() < 1e-9);
        assert_eq!(rows[0].agent_count, 1);
        assert_eq!(rows[1].model_id, "openai/gpt-4o");
        assert!((rows[1].cost_usd - 0.15).abs() < 1e-9);
        assert_eq!(rows[1].agent_count, 2);
        assert_eq!(rows[1].tokens, 300); // 2 × (100+50)
    }

    #[test]
    fn test_cost_by_project_excludes_null() {
        let (db, _dir) = make_test_db();
        let pid = "660e8400-e29b-41d4-a716-446655440099";
        // Agent WITH a project.
        db.upsert_agent(&cost_agent(
            "550e8400-e29b-41d4-a716-446655440001",
            "openai/gpt-4o",
            Some(pid),
            "2025-06-20T10:00:00Z",
            0.30,
            100,
            50,
        ))
        .unwrap();
        // Agent WITHOUT a project (project_id = None).
        db.upsert_agent(&cost_agent(
            "550e8400-e29b-41d4-a716-446655440002",
            "openai/gpt-4o",
            None,
            "2025-06-21T10:00:00Z",
            0.99,
            100,
            50,
        ))
        .unwrap();

        let rows = db.cost_by_project(None).unwrap();
        // Only the project-tagged agent should appear.
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].project_id, pid);
        assert!((rows[0].cost_usd - 0.30).abs() < 1e-9);
    }

    #[test]
    fn test_cost_daily_groups_by_date() {
        let (db, _dir) = make_test_db();
        db.upsert_agent(&cost_agent(
            "550e8400-e29b-41d4-a716-446655440001",
            "openai/gpt-4o",
            None,
            "2026-06-23T08:00:00Z",
            0.10,
            100,
            50,
        ))
        .unwrap();
        db.upsert_agent(&cost_agent(
            "550e8400-e29b-41d4-a716-446655440002",
            "openai/gpt-4o",
            None,
            "2026-06-23T20:00:00Z",
            0.20,
            200,
            100,
        ))
        .unwrap();
        db.upsert_agent(&cost_agent(
            "550e8400-e29b-41d4-a716-446655440003",
            "openai/gpt-4o",
            None,
            "2026-06-24T10:00:00Z",
            0.05,
            50,
            25,
        ))
        .unwrap();

        // 365-day window captures all three.
        let rows = db.cost_daily(365).unwrap();
        assert_eq!(rows.len(), 2); // two distinct dates

        // Sorted ascending by date.
        assert_eq!(rows[0].date, "2026-06-23");
        assert!((rows[0].cost_usd - 0.30).abs() < 1e-9); // 0.10 + 0.20
        assert_eq!(rows[0].agent_count, 2);
        assert_eq!(rows[1].date, "2026-06-24");
        assert!((rows[1].cost_usd - 0.05).abs() < 1e-9);
    }

    #[test]
    fn test_cost_aggregations_empty_db() {
        let (db, _dir) = make_test_db();

        let s = db.cost_summary(None).unwrap();
        assert!((s.total_cost_usd).abs() < 1e-9);
        assert_eq!(s.agent_count, 0);

        assert!(db.cost_by_model(None).unwrap().is_empty());
        assert!(db.cost_by_project(None).unwrap().is_empty());
        assert!(db.cost_daily(30).unwrap().is_empty());
    }
}
