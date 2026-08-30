//! WorkPlanner — three-source task synthesis (RFC-031 §7).
//!
//! Selects the next task for the TokenMaxer from three prioritized sources,
//! each filtered to non-destructive, bounded work (maxing runs unattended — no
//! `rm`, no deploys, no outbound network beyond read):
//!
//! - **Source A — autonomous skills**: skills with frontmatter `autonomous:
//!   true`. This is the "자주 실행되던 스킬" axis.
//! - **Source B — registered projects**: a bounded, read-mostly review
//!   task synthesized from each project's paths.
//! - **Source C — recurring patterns**: stubbed (lowest priority; avoids
//!   inventing work).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use super::session::TaskSource;

/// A unit of work the TokenMaxer can dispatch on one provider.
#[derive(Debug, Clone)]
pub struct PlannedTask {
    /// Which source produced this task.
    pub source: TaskSource,
    /// Human label (skill/project name) — for the status panel + report.
    pub source_name: String,
    /// The synthesized goal prompt handed to the agent.
    pub goal: String,
    /// Workspace roots to sandbox the agent to (project tasks only).
    pub root_paths: Vec<PathBuf>,
}

/// Selects the next task for a maxing run.
pub struct WorkPlanner {
    skills: Arc<crate::skill::SkillManager>,
    projects: Option<Arc<crate::project::ProjectManager>>,
}

impl WorkPlanner {
    pub fn new(
        skills: Arc<crate::skill::SkillManager>,
        projects: Option<Arc<crate::project::ProjectManager>>,
    ) -> Self {
        Self { skills, projects }
    }

    /// Pick the next task not already done this session (`done_goals`).
    ///
    /// Returns `None` when no eligible work remains — the maxer then terminates
    /// the window early ("stopped: nothing to do") rather than fabricating work.
    pub async fn next_task(&self, done_goals: &HashSet<String>) -> Option<PlannedTask> {
        // Source A — autonomous-eligible skills.
        for entry in self.skills.list_skills().await {
            let autonomous = entry
                .metadata
                .as_ref()
                .map(|m| m.autonomous)
                .unwrap_or(false);
            if !autonomous {
                continue;
            }
            let goal = format!(
                "[Skill: {}] {}. Perform this skill's routine work autonomously and \
                 summarize the outcome. Do not make destructive changes or run deploys.",
                entry.skill.name, entry.skill.description
            );
            if done_goals.contains(&goal) {
                continue;
            }
            return Some(PlannedTask {
                source: TaskSource::Skill,
                source_name: entry.skill.name.clone(),
                goal,
                root_paths: Vec::new(),
            });
        }

        if let Some(pm) = &self.projects {
            for proj in pm.list_projects() {
                // A project's filesystem scope is its own root_paths. Skip
                // projects with no roots — there is nothing on disk to review.
                if proj.root_paths.is_empty() {
                    continue;
                }
                let goal = format!(
                    "[Project: {}] Review the recent state of this project: summarize open \
                     work, recent changes, and any TODOs/FIXMEs. Read-only — do not modify \
                     files or run deploys.",
                    proj.name
                );
                if done_goals.contains(&goal) {
                    continue;
                }
                return Some(PlannedTask {
                    source: TaskSource::Project,
                    source_name: proj.name.clone(),
                    goal,
                    root_paths: proj.root_paths.clone(),
                });
            }
        }

        // Source C — recurring patterns: intentionally stubbed (RFC-031 §7).
        None
    }
}
