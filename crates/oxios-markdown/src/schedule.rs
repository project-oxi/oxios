//! Schedule management for tasks.
//!
//! Ported from files.md (`server/schedule/mod.rs`) by Artem Zakirullin.
//! Manages scheduled tasks stored in the knowledge config.

use chrono::{Datelike, FixedOffset, TimeZone, Utc};

use crate::fs::VirtualFs;
use crate::types::{DIR_USER_ROOT, KnowledgeConfig, Schedule};

/// Schedule-specific errors.
#[derive(Debug, thiserror::Error)]
pub enum ScheduleError {
    /// Config read failure.
    #[error("config read: {0}")]
    Read(String),
    /// Config write failure.
    #[error("config write: {0}")]
    Write(String),
}

/// Manages scheduled tasks for a knowledge base.
pub struct ScheduleManager<'a> {
    fs: &'a VirtualFs,
    config_filename: &'a str,
}

impl<'a> ScheduleManager<'a> {
    /// Create a new schedule manager.
    pub fn new(fs: &'a VirtualFs, config_filename: &'a str) -> Self {
        Self {
            fs,
            config_filename,
        }
    }

    /// Get all schedules.
    pub fn schedules(&self) -> Result<Vec<Schedule>, ScheduleError> {
        let cfg = self.read_config()?;
        Ok(cfg.schedules)
    }

    /// Add or update a schedule for a filename.
    pub fn add(&self, filename: &str, scheduled_at: i64, cron: &str) -> Result<(), ScheduleError> {
        let mut cfg = self.read_config()?;
        if let Some(s) = cfg.schedules.iter_mut().find(|s| s.filename == filename) {
            s.scheduled_at = scheduled_at;
            s.cron = cron.to_string();
        } else {
            cfg.schedules.push(Schedule {
                filename: filename.to_string(),
                scheduled_at,
                cron: cron.to_string(),
                cmd: String::new(),
            });
        }
        self.write_config(&cfg)
    }

    /// Delete a schedule by filename.
    pub fn delete(&self, filename: &str) -> Result<(), ScheduleError> {
        let mut cfg = self.read_config()?;
        cfg.schedules.retain(|s| s.filename != filename);
        self.write_config(&cfg)
    }

    // -----------------------------------------------------------------------
    // Config-level helpers
    // -----------------------------------------------------------------------

    /// Create the default config file if it doesn't already exist.
    pub fn create_default_if_not_exists(&self) -> Result<(), ScheduleError> {
        if self
            .fs
            .exists(DIR_USER_ROOT, self.config_filename)
            .map_err(|e| ScheduleError::Read(e.to_string()))?
        {
            return Ok(());
        }
        self.write_config(&KnowledgeConfig::default())
    }

    /// Check whether a checklist string should be split.
    ///
    /// Currently always returns `true` (matches the Go original).
    /// The `checklist` parameter is kept for future per-list overrides.
    pub fn should_split_checklist(&self, _checklist: &str) -> bool {
        // Upstream Go parity; intentionally always true (kept public for
        // API stability — removing it would need a major bump).
        true
    }

    // -----------------------------------------------------------------------
    // Move-to commands
    // -----------------------------------------------------------------------

    /// Add a move-to command. Silently succeeds if the command already exists.
    pub fn add_move_to_cmd(&self, cmd: &str) -> Result<(), ScheduleError> {
        let mut cfg = self.read_config()?;
        if cfg.move_to_commands.iter().any(|c| c == cmd) {
            return Ok(());
        }
        cfg.move_to_commands.push(cmd.to_string());
        self.write_config(&cfg)
    }

    /// Get all move-to commands.
    pub fn move_to_cmds(&self) -> Result<Vec<String>, ScheduleError> {
        let cfg = self.read_config()?;
        Ok(cfg.move_to_commands)
    }

    /// Delete a move-to command.
    pub fn del_move_to_cmd(&self, cmd: &str) -> Result<(), ScheduleError> {
        let mut cfg = self.read_config()?;
        cfg.move_to_commands.retain(|c| c != cmd);
        self.write_config(&cfg)
    }

    // -----------------------------------------------------------------------
    // Quick commands
    // -----------------------------------------------------------------------

    /// Add a quick command. Silently succeeds if the command already exists.
    pub fn add_quick_cmd(&self, cmd: &str) -> Result<(), ScheduleError> {
        let mut cfg = self.read_config()?;
        if cfg.quick_commands.iter().any(|c| c == cmd) {
            return Ok(());
        }
        cfg.quick_commands.push(cmd.to_string());
        self.write_config(&cfg)
    }

    /// Get all quick commands.
    pub fn quick_cmds(&self) -> Result<Vec<String>, ScheduleError> {
        let cfg = self.read_config()?;
        Ok(cfg.quick_commands)
    }

    /// Delete a quick command.
    pub fn del_quick_cmd(&self, cmd: &str) -> Result<(), ScheduleError> {
        let mut cfg = self.read_config()?;
        cfg.quick_commands.retain(|c| c != cmd);
        self.write_config(&cfg)
    }

    fn read_config(&self) -> Result<KnowledgeConfig, ScheduleError> {
        if !self
            .fs
            .exists(DIR_USER_ROOT, self.config_filename)
            .map_err(|e| ScheduleError::Read(e.to_string()))?
        {
            return Ok(KnowledgeConfig::default());
        }
        let content = self
            .fs
            .read(DIR_USER_ROOT, self.config_filename)
            .map_err(|e| ScheduleError::Read(e.to_string()))?;
        serde_json::from_str(&content).map_err(|e| ScheduleError::Read(e.to_string()))
    }

    fn write_config(&self, cfg: &KnowledgeConfig) -> Result<(), ScheduleError> {
        let json =
            serde_json::to_string_pretty(cfg).map_err(|e| ScheduleError::Write(e.to_string()))?;
        self.fs
            .write(DIR_USER_ROOT, self.config_filename, &json)
            .map_err(|e| ScheduleError::Write(e.to_string()))
    }
}

/// Format a schedule date for display.
pub fn format_schedule_date(scheduled_at: i64, timezone: FixedOffset) -> String {
    let now = Utc::now().timestamp();
    let today_start = beginning_of_day(now);
    let task_start = beginning_of_day(scheduled_at);
    let diff_days = (task_start - today_start) / 86400;

    let tz_dt = Utc
        .timestamp_opt(scheduled_at, 0)
        .single()
        .expect("valid Unix timestamp")
        .with_timezone(&timezone);

    match diff_days {
        0 => "Today".to_string(),
        1 => "Tomorrow".to_string(),
        2..=6 => format!("{} {:02}", tz_dt.format("%A"), tz_dt.day()),
        7..=13 => format!("Next {}", tz_dt.format("%A %d")),
        _ => format!(
            "{} {}, {}",
            tz_dt.format("%d %B"),
            tz_dt.weekday(),
            tz_dt.year()
        ),
    }
}

/// Calculate the beginning of a day (midnight) as a Unix timestamp.
pub fn beginning_of_day(timestamp: i64) -> i64 {
    let dt = Utc
        .timestamp_opt(timestamp, 0)
        .single()
        .expect("valid Unix timestamp");
    let date = dt.date_naive();
    date.and_hms_milli_opt(0, 0, 0, 0)
        .expect("midnight is always a valid time")
        .and_utc()
        .timestamp()
}

/// Calculate tomorrow's midnight timestamp.
pub fn tomorrow_timestamp() -> i64 {
    let tomorrow = Utc::now().date_naive() + chrono::Duration::days(1);
    tomorrow
        .and_hms_milli_opt(0, 0, 0, 0)
        .expect("midnight is always a valid time")
        .and_utc()
        .timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_fs() -> (VirtualFs, TempDir) {
        let dir = TempDir::new().unwrap();
        let fs = VirtualFs::new(dir.path().to_path_buf()).unwrap();
        (fs, dir)
    }

    #[test]
    fn test_add_and_list() {
        let (fs, _t) = test_fs();
        let mgr = ScheduleManager::new(&fs, "config.json");
        mgr.add("Task.md", 1000000, "").unwrap();
        mgr.add("Other.md", 2000000, "9:00").unwrap();
        let schedules = mgr.schedules().unwrap();
        assert_eq!(schedules.len(), 2);
    }

    #[test]
    fn test_update_existing() {
        let (fs, _t) = test_fs();
        let mgr = ScheduleManager::new(&fs, "config.json");
        mgr.add("Task.md", 1000000, "").unwrap();
        mgr.add("Task.md", 2000000, "10:00").unwrap();
        let schedules = mgr.schedules().unwrap();
        assert_eq!(schedules.len(), 1);
        assert_eq!(schedules[0].scheduled_at, 2000000);
    }

    #[test]
    fn test_delete() {
        let (fs, _t) = test_fs();
        let mgr = ScheduleManager::new(&fs, "config.json");
        mgr.add("Task.md", 1000000, "").unwrap();
        mgr.delete("Task.md").unwrap();
        assert!(mgr.schedules().unwrap().is_empty());
    }

    #[test]
    fn test_format_date() {
        let tz = FixedOffset::east_opt(0).expect("valid UTC offset");
        let ts = Utc::now().timestamp() + 86400;
        let formatted = format_schedule_date(ts, tz);
        assert_eq!(formatted, "Tomorrow");
    }

    #[test]
    fn test_tomorrow() {
        assert!(tomorrow_timestamp() > Utc::now().timestamp());
    }

    // -----------------------------------------------------------------------
    // create_default_if_not_exists
    // -----------------------------------------------------------------------

    #[test]
    fn test_create_default_if_not_exists_creates() {
        let (fs, _t) = test_fs();
        let mgr = ScheduleManager::new(&fs, "config.json");
        assert!(!fs.exists(DIR_USER_ROOT, "config.json").unwrap());
        mgr.create_default_if_not_exists().unwrap();
        assert!(fs.exists(DIR_USER_ROOT, "config.json").unwrap());
        let cfg: KnowledgeConfig =
            serde_json::from_str(&fs.read(DIR_USER_ROOT, "config.json").unwrap()).unwrap();
        assert_eq!(cfg.language, "en");
        assert!(cfg.schedules.is_empty());
        assert!(cfg.move_to_commands.is_empty());
        assert!(cfg.quick_commands.is_empty());
    }

    #[test]
    fn test_create_default_if_not_exists_idempotent() {
        let (fs, _t) = test_fs();
        let mgr = ScheduleManager::new(&fs, "config.json");
        mgr.create_default_if_not_exists().unwrap();
        // Add a schedule so we can verify the file is *not* overwritten.
        mgr.add("Task.md", 1000, "").unwrap();
        mgr.create_default_if_not_exists().unwrap();
        assert_eq!(mgr.schedules().unwrap().len(), 1);
    }

    // -----------------------------------------------------------------------
    // should_split_checklist
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_split_checklist() {
        let (fs, _t) = test_fs();
        let mgr = ScheduleManager::new(&fs, "config.json");
        assert!(mgr.should_split_checklist("- item1\n- item2"));
        assert!(mgr.should_split_checklist("anything"));
    }

    // -----------------------------------------------------------------------
    // move-to commands
    // -----------------------------------------------------------------------

    #[test]
    fn test_add_move_to_cmd() {
        let (fs, _t) = test_fs();
        let mgr = ScheduleManager::new(&fs, "config.json");
        assert!(mgr.move_to_cmds().unwrap().is_empty());
        mgr.add_move_to_cmd("Archive").unwrap();
        mgr.add_move_to_cmd("Later").unwrap();
        assert_eq!(mgr.move_to_cmds().unwrap(), vec!["Archive", "Later"]);
    }

    #[test]
    fn test_add_move_to_cmd_duplicate() {
        let (fs, _t) = test_fs();
        let mgr = ScheduleManager::new(&fs, "config.json");
        mgr.add_move_to_cmd("Archive").unwrap();
        mgr.add_move_to_cmd("Archive").unwrap();
        assert_eq!(mgr.move_to_cmds().unwrap(), vec!["Archive"]);
    }

    #[test]
    fn test_del_move_to_cmd() {
        let (fs, _t) = test_fs();
        let mgr = ScheduleManager::new(&fs, "config.json");
        mgr.add_move_to_cmd("Archive").unwrap();
        mgr.add_move_to_cmd("Later").unwrap();
        mgr.del_move_to_cmd("Archive").unwrap();
        assert_eq!(mgr.move_to_cmds().unwrap(), vec!["Later"]);
    }

    // -----------------------------------------------------------------------
    // quick commands
    // -----------------------------------------------------------------------

    #[test]
    fn test_add_quick_cmd() {
        let (fs, _t) = test_fs();
        let mgr = ScheduleManager::new(&fs, "config.json");
        assert!(mgr.quick_cmds().unwrap().is_empty());
        mgr.add_quick_cmd("/done").unwrap();
        mgr.add_quick_cmd("/shop").unwrap();
        assert_eq!(mgr.quick_cmds().unwrap(), vec!["/done", "/shop"]);
    }

    #[test]
    fn test_add_quick_cmd_duplicate() {
        let (fs, _t) = test_fs();
        let mgr = ScheduleManager::new(&fs, "config.json");
        mgr.add_quick_cmd("/done").unwrap();
        mgr.add_quick_cmd("/done").unwrap();
        assert_eq!(mgr.quick_cmds().unwrap(), vec!["/done"]);
    }

    #[test]
    fn test_del_quick_cmd() {
        let (fs, _t) = test_fs();
        let mgr = ScheduleManager::new(&fs, "config.json");
        mgr.add_quick_cmd("/done").unwrap();
        mgr.add_quick_cmd("/shop").unwrap();
        mgr.del_quick_cmd("/done").unwrap();
        assert_eq!(mgr.quick_cmds().unwrap(), vec!["/shop"]);
    }
}
