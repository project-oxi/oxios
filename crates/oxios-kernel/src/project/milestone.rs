//! Project milestones — the grouping layer above issues.
//!
//! `oxicode-sdk` owns `IssueMeta` and drops unknown frontmatter keys when it
//! writes, so a `milestone:` field cannot be added to the issue file without
//! forking the type. Membership therefore rides a **reserved label** on the
//! issue (`milestone:<slug>`), which round-trips safely through every SDK
//! write, shows up in the existing label filters, and stays readable in the
//! raw file. The milestone's own record — title, description, due date,
//! status — lives in `milestones.yaml` beside the issues.
//!
//! Progress (`closed / total`) is always derived by listing issues carrying
//! the label. It is never stored, so it cannot go stale.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// Label prefix marking an issue's milestone membership.
pub const MILESTONE_LABEL_PREFIX: &str = "milestone:";

/// Open/closed state of a milestone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MilestoneStatus {
    /// Still being worked on.
    #[default]
    Open,
    /// Shipped or abandoned.
    Closed,
}

impl std::fmt::Display for MilestoneStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open => write!(f, "open"),
            Self::Closed => write!(f, "closed"),
        }
    }
}

/// A milestone definition. Membership lives on the issues, not here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Milestone {
    /// Stable identifier used in the `milestone:<slug>` issue label.
    pub slug: String,
    /// Display title.
    pub title: String,
    /// Optional longer description.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Optional target date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<NaiveDate>,
    /// Open/closed state.
    #[serde(default)]
    pub status: MilestoneStatus,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last mutation timestamp.
    pub updated_at: DateTime<Utc>,
}

impl Milestone {
    /// Create a milestone with `now` timestamps.
    pub fn new(slug: impl Into<String>, title: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            slug: slug.into(),
            title: title.into(),
            description: String::new(),
            due: None,
            status: MilestoneStatus::default(),
            created_at: now,
            updated_at: now,
        }
    }

    /// The issue label that marks membership in this milestone.
    pub fn label(&self) -> String {
        format!("{MILESTONE_LABEL_PREFIX}{}", self.slug)
    }
}

/// Fields a caller may change. `None` keeps the current value.
#[derive(Debug, Clone, Default)]
pub struct MilestonePatch {
    /// Replace the title.
    pub title: Option<String>,
    /// Replace the description.
    pub description: Option<String>,
    /// Replace the due date. `Some(None)` clears it.
    pub due: Option<Option<NaiveDate>>,
    /// Replace the status.
    pub status: Option<MilestoneStatus>,
}

/// On-disk shape of `milestones.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct MilestoneFile {
    #[serde(default)]
    milestones: Vec<Milestone>,
}

/// Derived progress for one milestone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct MilestoneProgress {
    /// Issues carrying this milestone's label.
    pub total: usize,
    /// How many of them are closed.
    pub closed: usize,
}

impl MilestoneProgress {
    /// Completion as a 0–100 percentage. An empty milestone reads as 0, not
    /// 100 — "nothing planned yet" is not "done".
    pub fn percent(&self) -> u8 {
        if self.total == 0 {
            return 0;
        }
        ((self.closed * 100) / self.total) as u8
    }
}

/// Slugify a title into a milestone slug: lowercase `[a-z0-9-]`.
///
/// Mirrors the SDK's issue-filename slugifier so a title behaves the same in
/// both places.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

/// Extract the milestone slug from a label set, if any.
///
/// An issue carries at most one `milestone:` label; if several are present
/// (hand-edited file) the first wins, deterministically.
pub fn milestone_of(labels: &[String]) -> Option<String> {
    labels
        .iter()
        .find_map(|l| l.strip_prefix(MILESTONE_LABEL_PREFIX))
        .map(str::to_string)
}

/// Return `labels` with its milestone membership replaced.
///
/// `None` removes membership. Non-milestone labels keep their order.
pub fn with_milestone(labels: &[String], slug: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = labels
        .iter()
        .filter(|l| !l.starts_with(MILESTONE_LABEL_PREFIX))
        .cloned()
        .collect();
    if let Some(slug) = slug {
        out.push(format!("{MILESTONE_LABEL_PREFIX}{slug}"));
    }
    out
}

/// Read `path`, returning an empty list when the file does not exist yet.
pub fn load(path: &Path) -> Result<Vec<Milestone>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).context("failed to read milestones.yaml"),
    };
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let file: MilestoneFile =
        serde_yaml::from_str(&raw).context("failed to parse milestones.yaml")?;
    Ok(file.milestones)
}

/// Write `milestones` to `path` atomically (temp + rename).
///
/// Mirrors the SDK issue store's durability: a crash mid-write leaves the
/// previous file intact rather than a truncated one.
pub fn save(path: &Path, milestones: &[Milestone]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("failed to create project data directory")?;
    }
    let file = MilestoneFile {
        milestones: milestones.to_vec(),
    };
    let yaml = serde_yaml::to_string(&file).context("failed to serialize milestones")?;

    let tmp = path.with_extension(format!(
        "yaml.tmp.{}.{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(&tmp, yaml).context("failed to write milestones temp file")?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e).context("failed to install milestones.yaml")
        }
    }
}

/// Count issues per milestone slug from `(labels, is_closed)` pairs.
///
/// Takes an iterator rather than the issues themselves so the caller keeps
/// ownership of the SDK types and this module stays free of that dependency.
pub fn tally<'a>(
    issues: impl IntoIterator<Item = (&'a [String], bool)>,
) -> BTreeMap<String, MilestoneProgress> {
    let mut out: BTreeMap<String, MilestoneProgress> = BTreeMap::new();
    for (labels, is_closed) in issues {
        let Some(slug) = milestone_of(labels) else {
            continue;
        };
        let entry = out.entry(slug).or_default();
        entry.total += 1;
        if is_closed {
            entry.closed += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn slugify_matches_the_sdk_shape() {
        assert_eq!(slugify("v0.4 Release!"), "v0-4-release");
        assert_eq!(slugify("  spaces  "), "spaces");
        assert_eq!(slugify("a__b"), "a-b");
        assert_eq!(slugify(""), "");
    }

    #[test]
    fn milestone_of_reads_the_reserved_prefix() {
        assert_eq!(
            milestone_of(&labels(&["bug", "milestone:v0-4", "auth"])),
            Some("v0-4".to_string())
        );
        assert_eq!(milestone_of(&labels(&["bug"])), None);
    }

    #[test]
    fn with_milestone_replaces_and_preserves_other_labels() {
        let before = labels(&["bug", "milestone:old", "auth"]);
        let after = with_milestone(&before, Some("new"));
        assert_eq!(after, labels(&["bug", "auth", "milestone:new"]));
    }

    #[test]
    fn with_milestone_none_clears_membership() {
        let before = labels(&["bug", "milestone:old"]);
        assert_eq!(with_milestone(&before, None), labels(&["bug"]));
    }

    #[test]
    fn with_milestone_is_idempotent() {
        let once = with_milestone(&labels(&["bug"]), Some("v1"));
        let twice = with_milestone(&once, Some("v1"));
        assert_eq!(once, twice);
    }

    #[test]
    fn load_missing_file_is_empty_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("milestones.yaml");
        assert!(load(&path).unwrap().is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested").join("milestones.yaml");
        let mut m = Milestone::new("v0-4", "v0.4 release");
        m.description = "Issue tracker".into();
        m.due = Some(NaiveDate::from_ymd_opt(2026, 9, 30).unwrap());

        save(&path, std::slice::from_ref(&m)).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded, vec![m]);
    }

    #[test]
    fn save_leaves_no_temp_files_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("milestones.yaml");
        save(&path, &[Milestone::new("v1", "One")]).unwrap();
        let stray: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("tmp"))
            .collect();
        assert!(stray.is_empty(), "temp file left behind: {stray:?}");
    }

    #[test]
    fn tally_counts_only_labelled_issues() {
        let a = labels(&["milestone:v1", "bug"]);
        let b = labels(&["milestone:v1"]);
        let c = labels(&["milestone:v2"]);
        let d = labels(&["chore"]);
        let counts = tally(vec![
            (a.as_slice(), true),
            (b.as_slice(), false),
            (c.as_slice(), false),
            (d.as_slice(), true),
        ]);
        assert_eq!(
            counts["v1"],
            MilestoneProgress {
                total: 2,
                closed: 1
            }
        );
        assert_eq!(
            counts["v2"],
            MilestoneProgress {
                total: 1,
                closed: 0
            }
        );
        assert!(!counts.contains_key("chore"));
    }

    #[test]
    fn empty_milestone_reads_as_zero_percent() {
        assert_eq!(MilestoneProgress::default().percent(), 0);
        assert_eq!(
            MilestoneProgress {
                total: 2,
                closed: 1
            }
            .percent(),
            50
        );
        assert_eq!(
            MilestoneProgress {
                total: 3,
                closed: 3
            }
            .percent(),
            100
        );
    }

    #[test]
    fn label_matches_the_prefix_contract() {
        assert_eq!(Milestone::new("v0-4", "t").label(), "milestone:v0-4");
    }
}
