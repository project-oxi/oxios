//! KnowledgeCuration — periodic LLM curation of agent-generated knowledge notes (RFC-022).
//!
//! Phase 1: Scan — find notes with `needs_review: true` and `quality: raw`.
//! Phase 2: Curate — LLM strips conversational artifacts and improves structure.
//! Phase 3: Write Back — overwrite with curated content, commit to git first.
//! Phase 4: Report — save dream report.

use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::engine::EngineHandle;
use crate::git_layer::GitLayer;
use oxios_markdown::KnowledgeBase;
use oxios_markdown::types::{NoteMeta, NoteQuality, NoteSource};

/// Configuration for knowledge curation (RFC-048 §5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeCurationConfig {
    /// Enable/disable knowledge dream.
    #[serde(default)]
    pub enabled: bool,
    /// Minimum raw notes before triggering a dream.
    #[serde(default = "default_min_raw_notes")]
    pub min_raw_notes: usize,
    /// Maximum notes to curate per dream run.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    /// Model ID for curation LLM calls.
    #[serde(default = "default_model")]
    pub model: String,
}

fn default_min_raw_notes() -> usize {
    3
}
fn default_batch_size() -> usize {
    10
}
fn default_model() -> String {
    "auto".to_string()
}

impl Default for KnowledgeCurationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_raw_notes: default_min_raw_notes(),
            batch_size: default_batch_size(),
            model: default_model(),
        }
    }
}

/// Report from a knowledge dream run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeCurationReport {
    /// Unique dream ID.
    pub curation_id: String,
    /// When the dream started.
    pub started_at: chrono::DateTime<Utc>,
    /// When the dream completed.
    pub completed_at: chrono::DateTime<Utc>,
    /// Total notes scanned.
    pub notes_scanned: usize,
    /// Notes successfully curated.
    pub notes_curated: usize,
    /// Notes skipped (errors or no changes needed).
    pub notes_skipped: usize,
    /// Error messages.
    pub errors: Vec<String>,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

/// A raw note awaiting curation.
struct RawNote {
    path: String,
    meta: NoteMeta,
    body: String,
}

/// A curated note ready for write-back.
struct CuratedNote {
    path: String,
    original_meta: NoteMeta,
    curated_body: String,
}

/// Knowledge dream process.
pub struct KnowledgeCuration {
    knowledge_base: Arc<KnowledgeBase>,
    git_layer: Arc<GitLayer>,
    engine_handle: Arc<EngineHandle>,
    model_id: String,
    config: KnowledgeCurationConfig,
}

impl KnowledgeCuration {
    /// Create a new knowledge dream process.
    ///
    /// When `config.model` is an explicit model (not `"auto"`), it is validated
    /// against the live engine now — a typo fails at boot with a clear error
    /// rather than silently at every curation run. Callers should skip
    /// spawning on `Err` (the dream is a non-critical background feature).
    pub fn new(
        knowledge_base: Arc<KnowledgeBase>,
        git_layer: Arc<GitLayer>,
        engine_handle: Arc<EngineHandle>,
        config: KnowledgeCurationConfig,
    ) -> Result<Self> {
        let model_id = if config.model == "auto" {
            // Will use the default model from engine at curation time.
            String::new()
        } else {
            // Fail-fast: resolve the explicit curation model AND its provider so
            // a malformed id or unconfigured provider surfaces here, not in the
            // background loop.
            let engine = engine_handle.get();
            let model = engine.resolve_model(&config.model).with_context(|| {
                format!(
                    "knowledge_curation.model '{}' is not a known model",
                    config.model
                )
            })?;
            engine.create_provider(&model.provider).with_context(|| {
                format!(
                    "knowledge_curation.model '{}' provider '{}' is not configured",
                    config.model, model.provider
                )
            })?;
            config.model.clone()
        };
        Ok(Self {
            knowledge_base,
            git_layer,
            engine_handle,
            model_id,
            config,
        })
    }

    /// Run the knowledge dream.
    pub async fn dream(&self) -> KnowledgeCurationReport {
        let curation_id = uuid::Uuid::new_v4().to_string();
        let started_at = Utc::now();

        let mut report = KnowledgeCurationReport {
            curation_id: curation_id.clone(),
            started_at,
            completed_at: started_at,
            notes_scanned: 0,
            notes_curated: 0,
            notes_skipped: 0,
            errors: Vec::new(),
            duration_ms: 0,
        };

        // Phase 1: Scan
        let raw_notes = match self.scan().await {
            Ok(notes) => notes,
            Err(e) => {
                report.errors.push(format!("Scan failed: {e}"));
                return self.finish_curation(report);
            }
        };

        report.notes_scanned = raw_notes.len();

        if raw_notes.len() < self.config.min_raw_notes {
            report.notes_skipped = raw_notes.len();
            return self.finish_curation(report);
        }

        // Phase 2: Curate
        let curated = match self.curate(&raw_notes).await {
            Ok(c) => c,
            Err(e) => {
                report.errors.push(format!("Curation failed: {e}"));
                return self.finish_curation(report);
            }
        };

        // Phase 3: Write back
        for note in &curated {
            // Git commit the original before overwriting
            if let Err(e) = self.git_layer.commit_file(
                &note.path,
                &format!("curation: pre-write snapshot ({})", curation_id),
            ) {
                tracing::warn!(
                    path = %note.path,
                    error = %e,
                    "Failed to git-commit before curation"
                );
            }

            let new_meta = NoteMeta {
                source: NoteSource::Dream,
                quality: NoteQuality::Curated,
                needs_review: false,
                ..note.original_meta.clone()
            };

            match self.knowledge_base.note_write_with_meta(
                &note.path,
                &note.curated_body,
                &new_meta,
            ) {
                Ok(true) => {
                    tracing::info!(path = %note.path, "Knowledge dream curated note");
                    report.notes_curated += 1;
                }
                Ok(false) => {
                    tracing::warn!(path = %note.path, "Dream skipped: user-authored note");
                    report.notes_skipped += 1;
                }
                Err(e) => {
                    tracing::warn!(path = %note.path, error = %e, "Failed to write curated note");
                    report
                        .errors
                        .push(format!("Write failed for {}: {e}", note.path));
                    report.notes_skipped += 1;
                }
            }
        }

        self.finish_curation(report)
    }

    /// Finalize timestamps, save report, return.
    fn finish_curation(&self, mut report: KnowledgeCurationReport) -> KnowledgeCurationReport {
        report.completed_at = Utc::now();
        report.duration_ms = (report.completed_at - report.started_at)
            .num_milliseconds()
            .max(0) as u64;

        let report_path = self
            .knowledge_base
            .root()
            .join(".oxios")
            .join("curation_reports")
            .join(format!("{}.json", report.curation_id));
        if let Some(parent) = report_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(data) = serde_json::to_string_pretty(&report) {
            let _ = std::fs::write(&report_path, data);
        }
        report
    }

    /// Spawn the dream as a background task.
    pub fn spawn(self: &Arc<Self>) {
        let kd = Arc::clone(self);
        tokio::spawn(async move {
            let report = kd.dream().await;
            if report.notes_curated > 0 || !report.errors.is_empty() {
                tracing::info!(
                    curation_id = %report.curation_id,
                    curated = report.notes_curated,
                    errors = report.errors.len(),
                    "Knowledge dream completed"
                );
            }
        });
    }

    /// Phase 1: Scan for raw notes needing review.
    async fn scan(&self) -> Result<Vec<RawNote>> {
        let review_list = self.knowledge_base.notes_needing_review()?;

        let mut notes = Vec::new();
        for (path, meta) in review_list.into_iter().take(self.config.batch_size) {
            let content = self.knowledge_base.note_read(&path)?;
            let body = match content {
                Some(c) => {
                    let (_, body) = oxios_markdown::knowledge::parse_note_meta(&c);
                    body
                }
                None => continue,
            };
            notes.push(RawNote { path, meta, body });
        }

        Ok(notes)
    }

    /// Phase 2: LLM curation of raw notes.
    async fn curate(&self, notes: &[RawNote]) -> Result<Vec<CuratedNote>> {
        let mut curated = Vec::new();

        for note in notes {
            match self.curate_single(&note.body).await {
                Ok(curated_body) => {
                    if curated_body.trim() != note.body.trim() {
                        curated.push(CuratedNote {
                            path: note.path.clone(),
                            original_meta: note.meta.clone(),
                            curated_body,
                        });
                    }
                    // If LLM returned essentially the same content, skip
                }
                Err(e) => {
                    tracing::warn!(
                        path = %note.path,
                        error = %e,
                        "Failed to curate note, skipping"
                    );
                }
            }
        }

        Ok(curated)
    }

    /// Curate a single note via LLM.
    async fn curate_single(&self, body: &str) -> Result<String> {
        let engine = self.engine_handle.get();
        let model_id = if self.model_id.is_empty() {
            engine.default_model_id().to_string()
        } else {
            self.model_id.clone()
        };

        let agent_config = oxicode_sdk::AgentConfig {
            description: Some("Knowledge curation".into()),
            model_id,
            system_prompt: Some(
                "You are a knowledge editor. You refine raw agent-generated notes into \
                 clean, well-structured knowledge documents.\n\n\
                 Rules:\n\
                 - Remove conversational artifacts: greetings, sign-offs, hedging, questions to the user.\n\
                 - Keep all substantive content: facts, analysis, code, data, explanations.\n\
                 - Improve structure if needed: add headers, organize sections.\n\
                 - Preserve the original meaning. Do not add new information.\n\
                 - Output only the cleaned markdown body. No frontmatter. No explanation."
                    .to_string(),
            ),
            max_tokens: Some(4096),
            temperature: Some(0.3),
            ..Default::default()
        };

        let agent = engine.oxi().agent(agent_config).build()?;
        let (response, _) = agent.run(body.to_string()).await?;
        Ok(response.content)
    }
}
