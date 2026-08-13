#![allow(missing_docs)]
//! SONA — Self-Optimizing Neural Architecture (simplified).
//!
//! Tracks execution trajectories, distills successful patterns,
//! and adapts future behavior based on learned experience.
//!
//! Performance target: adaptation in < 0.05ms (in-memory lookup).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::embedding::{EmbeddingProvider, EmbeddingVector};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Operating mode for the SONA engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SonaMode {
    /// Real-time adaptation (< 0.05ms target).
    RealTime,
    /// Balanced between speed and depth.
    #[default]
    Balanced,
    /// Research mode — deep analysis, no time constraints.
    Research,
    /// Edge device — minimal memory footprint.
    Edge,
}

/// Verdict for a trajectory outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Fully successful.
    Success,
    /// Partially successful (some steps failed).
    PartialFailure,
    /// Completely failed.
    Failure,
}

/// A single step within a trajectory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryStep {
    /// Step description / input.
    pub input: String,
    /// Step result / output.
    pub output: String,
    /// Duration of this step in milliseconds.
    pub duration_ms: u64,
    /// Confidence score (0.0–1.0).
    pub confidence: f32,
}

/// A trajectory — a sequence of steps with a final verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trajectory {
    /// Unique ID.
    pub id: String,
    /// Ordered steps.
    pub steps: Vec<TrajectoryStep>,
    /// Final verdict.
    pub verdict: Verdict,
    /// Domain or task type.
    pub domain: String,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Embedding of the trajectory's combined input text.
    #[serde(skip)]
    pub embedding: Option<EmbeddingVector>,
}

impl Trajectory {
    /// Create a new trajectory with the given steps and verdict.
    pub fn new(steps: Vec<TrajectoryStep>, verdict: Verdict, domain: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            steps,
            verdict,
            domain: domain.to_string(),
            created_at: Utc::now(),
            embedding: None,
        }
    }

    /// Total duration across all steps.
    pub fn total_duration_ms(&self) -> u64 {
        self.steps.iter().map(|s| s.duration_ms).sum()
    }

    /// Average confidence across all steps.
    pub fn avg_confidence(&self) -> f32 {
        if self.steps.is_empty() {
            return 0.0;
        }
        self.steps.iter().map(|s| s.confidence).sum::<f32>() / self.steps.len() as f32
    }

    /// Concatenated input text for embedding.
    pub fn input_text(&self) -> String {
        self.steps
            .iter()
            .map(|s| s.input.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// A distilled pattern extracted from successful trajectories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnedPattern {
    /// Unique ID.
    pub id: String,
    /// Source trajectory IDs.
    pub source_trajectories: Vec<String>,
    /// Distilled strategy description.
    pub strategy: String,
    /// Domain category.
    pub domain: String,
    /// Confidence in this pattern (based on number of supporting trajectories).
    pub confidence: f32,
    /// Number of trajectories this was distilled from.
    pub support_count: usize,
    /// Embedding for similarity matching.
    #[serde(skip)]
    pub embedding: Option<EmbeddingVector>,
}

// ---------------------------------------------------------------------------
// SonaEngine
// ---------------------------------------------------------------------------

/// The SONA engine for trajectory-based self-learning.
///
/// Records execution trajectories, distills patterns from successful ones,
/// and adapts future behavior by matching against learned patterns.
pub struct SonaEngine {
    /// Operating mode.
    mode: SonaMode,
    /// Recorded trajectories.
    trajectories: RwLock<Vec<Trajectory>>,
    /// Distilled patterns from successful trajectories.
    learned_patterns: RwLock<Vec<LearnedPattern>>,
    /// Embedding provider.
    embedding: std::sync::Arc<dyn EmbeddingProvider>,
    /// Maximum trajectories to keep (mode-dependent).
    max_trajectories: usize,
}

impl std::fmt::Debug for SonaEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SonaEngine")
            .field("mode", &self.mode)
            .field("trajectory_count", &self.trajectories.read().len())
            .field("pattern_count", &self.learned_patterns.read().len())
            .finish()
    }
}

impl SonaEngine {
    /// Create a new SONA engine with the given mode and embedding provider.
    pub fn new(mode: SonaMode, embedding: std::sync::Arc<dyn EmbeddingProvider>) -> Self {
        let max_trajectories = match mode {
            SonaMode::RealTime => 100,
            SonaMode::Balanced => 500,
            SonaMode::Research => 5000,
            SonaMode::Edge => 50,
        };

        Self {
            mode,
            trajectories: RwLock::new(Vec::new()),
            learned_patterns: RwLock::new(Vec::new()),
            embedding,
            max_trajectories,
        }
    }

    /// Return the current operating mode.
    pub fn mode(&self) -> SonaMode {
        self.mode
    }

    /// Record a new trajectory.
    ///
    /// Generates an embedding for the trajectory's input text
    /// and stores it for future distillation.
    pub async fn record(&self, mut trajectory: Trajectory) -> Result<String, anyhow::Error> {
        if trajectory.id.is_empty() {
            trajectory.id = Uuid::new_v4().to_string();
        }

        // Generate embedding
        let text = trajectory.input_text();
        if !text.is_empty() {
            let embedding = self.embedding.embed(&text).await?;
            trajectory.embedding = Some(embedding);
        }

        let id = trajectory.id.clone();

        let mut trajs = self.trajectories.write();

        // Enforce capacity limit
        if trajs.len() >= self.max_trajectories {
            // Remove oldest failure trajectories first
            let remove_count = trajs.len() - self.max_trajectories + 1;
            let mut removed = 0;
            trajs.retain(|t| {
                if removed >= remove_count {
                    return true;
                }
                if t.verdict == Verdict::Failure {
                    removed += 1;
                    false
                } else {
                    true
                }
            });
            // If still over capacity, remove oldest
            while trajs.len() >= self.max_trajectories {
                trajs.remove(0);
            }
        }

        trajs.push(trajectory);
        Ok(id)
    }

    /// Distill learned patterns from successful trajectories.
    ///
    /// Groups successful trajectories by domain and extracts common
    /// patterns. Returns the newly distilled patterns.
    pub async fn distill(&self) -> Result<Vec<LearnedPattern>, anyhow::Error> {
        // Collect data under lock, then release before any .await
        let domain_groups: HashMap<String, Vec<Trajectory>> = {
            let trajs = self.trajectories.read();
            let mut groups: HashMap<String, Vec<Trajectory>> = HashMap::new();
            for traj in trajs.iter() {
                if traj.verdict == Verdict::Success {
                    groups
                        .entry(traj.domain.clone())
                        .or_default()
                        .push(traj.clone());
                }
            }
            groups
        }; // lock dropped here

        let mut new_patterns = Vec::new();

        for (domain, group) in &domain_groups {
            if group.len() < 2 {
                continue; // Need at least 2 trajectories to distill
            }

            // Simple distillation: extract common step patterns
            // For each trajectory, get the strategy from concatenated inputs
            let mut strategy_parts: Vec<String> = Vec::new();
            for traj in group {
                let summary: String = traj
                    .steps
                    .iter()
                    .take(3) // Use first 3 steps as summary
                    .map(|s| s.input.clone())
                    .collect::<Vec<_>>()
                    .join(" → ");
                strategy_parts.push(summary);
            }

            // Combine strategies into a distilled pattern
            let combined = strategy_parts.join("; ");
            let strategy = if combined.chars().count() > 500 {
                // Truncate by chars, not bytes. `&combined[..500]` panics when
                // byte index 500 falls inside a multi-byte UTF-8 sequence
                // (Korean, emoji) — common for this Korean-first codebase.
                let truncated: String = combined.chars().take(500).collect();
                format!("{truncated}...")
            } else {
                combined
            };

            let embedding = self.embedding.embed(&strategy).await?;

            let source_ids: Vec<String> = group.iter().map(|t| t.id.clone()).collect();

            let pattern = LearnedPattern {
                id: Uuid::new_v4().to_string(),
                source_trajectories: source_ids,
                strategy,
                domain: domain.clone(),
                confidence: (group.len() as f32 * 0.2).min(1.0),
                support_count: group.len(),
                embedding: Some(embedding),
            };

            new_patterns.push(pattern);
        }

        // Store new patterns
        {
            let mut patterns = self.learned_patterns.write();
            for pattern in &new_patterns {
                // Don't duplicate — check by strategy similarity (simplified: exact match)
                let is_dup = patterns
                    .iter()
                    .any(|p| p.strategy == pattern.strategy && p.domain == pattern.domain);
                if !is_dup {
                    patterns.push(pattern.clone());
                }
            }
        }

        tracing::info!(
            new_patterns = new_patterns.len(),
            "SONA distillation complete"
        );
        Ok(new_patterns)
    }

    /// Adapt to a new query by finding the most similar learned pattern.
    ///
    /// Returns the best matching pattern if similarity exceeds threshold.
    /// Target: < 0.05ms for in-memory lookup.
    pub async fn adapt(&self, query: &str) -> Result<Option<LearnedPattern>, anyhow::Error> {
        let query_embedding = self.embedding.embed(query).await?;

        let patterns = self.learned_patterns.read();
        let mut best: Option<(&LearnedPattern, f64)> = None;

        for pattern in patterns.iter() {
            if let Some(ref emb) = pattern.embedding {
                let sim = query_embedding.cosine_similarity(emb);
                match best {
                    Some((_, best_sim)) if sim <= best_sim => {}
                    _ => best = Some((pattern, sim)),
                }
            }
        }

        Ok(best.filter(|(_, sim)| *sim > 0.3).map(|(p, sim)| {
            let mut adapted = p.clone();
            adapted.confidence = (p.confidence * sim as f32).min(1.0);
            adapted
        }))
    }

    /// Return counts of trajectories and patterns.
    pub fn counts(&self) -> (usize, usize) {
        let traj_count = self.trajectories.read().len();
        let pattern_count = self.learned_patterns.read().len();
        (traj_count, pattern_count)
    }

    /// Get all learned patterns for persistence.
    pub fn get_learned_patterns(&self) -> Vec<LearnedPattern> {
        self.learned_patterns.read().clone()
    }

    /// Load learned patterns from persistence.
    pub fn load_learned_patterns(&self, patterns: Vec<LearnedPattern>) {
        let mut existing = self.learned_patterns.write();
        *existing = patterns;
    }

    /// Get trajectories filtered by verdict.
    pub fn trajectories_by_verdict(&self, verdict: Verdict) -> Vec<Trajectory> {
        self.trajectories
            .read()
            .iter()
            .filter(|t| t.verdict == verdict)
            .cloned()
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::TfIdfEmbeddingProvider;

    fn make_step(input: &str, output: &str) -> TrajectoryStep {
        TrajectoryStep {
            input: input.to_string(),
            output: output.to_string(),
            duration_ms: 10,
            confidence: 0.9,
        }
    }

    fn make_trajectory(domain: &str, verdict: Verdict) -> Trajectory {
        Trajectory::new(
            vec![
                make_step("analyze input", "parsed"),
                make_step("execute plan", "completed"),
            ],
            verdict,
            domain,
        )
    }

    #[tokio::test]
    async fn test_record_trajectory() {
        let engine = SonaEngine::new(
            SonaMode::Balanced,
            std::sync::Arc::new(TfIdfEmbeddingProvider),
        );
        let traj = make_trajectory("testing", Verdict::Success);

        let id = engine.record(traj).await.unwrap();
        assert!(!id.is_empty());

        let (traj_count, _) = engine.counts();
        assert_eq!(traj_count, 1);
    }

    #[tokio::test]
    async fn test_distill_patterns() {
        let engine = SonaEngine::new(
            SonaMode::Balanced,
            std::sync::Arc::new(TfIdfEmbeddingProvider),
        );

        // Record multiple successful trajectories in the same domain
        for _ in 0..3 {
            let traj = make_trajectory("security", Verdict::Success);
            engine.record(traj).await.unwrap();
        }

        let patterns = engine.distill().await.unwrap();
        assert!(
            !patterns.is_empty(),
            "Should distill patterns from 3+ successful trajectories"
        );

        let (_, pattern_count) = engine.counts();
        assert!(pattern_count > 0);
    }

    #[tokio::test]
    async fn test_distill_needs_multiple_successes() {
        let engine = SonaEngine::new(
            SonaMode::Balanced,
            std::sync::Arc::new(TfIdfEmbeddingProvider),
        );

        engine
            .record(make_trajectory("testing", Verdict::Success))
            .await
            .unwrap();
        let patterns = engine.distill().await.unwrap();
        assert!(patterns.is_empty(), "Need 2+ trajectories to distill");
    }

    #[tokio::test]
    async fn test_distill_ignores_failures() {
        let engine = SonaEngine::new(
            SonaMode::Balanced,
            std::sync::Arc::new(TfIdfEmbeddingProvider),
        );

        engine
            .record(make_trajectory("testing", Verdict::Failure))
            .await
            .unwrap();
        engine
            .record(make_trajectory("testing", Verdict::Failure))
            .await
            .unwrap();

        let patterns = engine.distill().await.unwrap();
        assert!(patterns.is_empty(), "Failures should not produce patterns");
    }

    #[tokio::test]
    async fn test_adapt_finds_similar_pattern() {
        let engine = SonaEngine::new(
            SonaMode::Balanced,
            std::sync::Arc::new(TfIdfEmbeddingProvider),
        );

        // Record and distill
        for _ in 0..3 {
            let mut traj = make_trajectory("security", Verdict::Success);
            traj.steps[0].input =
                "scan for SQL injection vulnerabilities in the codebase".to_string();
            engine.record(traj).await.unwrap();
        }
        engine.distill().await.unwrap();

        // Adapt should find the pattern
        let result = engine
            .adapt("check for SQL injection security issues")
            .await
            .unwrap();
        assert!(result.is_some(), "Should find a matching pattern");
        let pattern = result.unwrap();
        assert_eq!(pattern.domain, "security");
        assert!(pattern.confidence > 0.0);
    }

    #[tokio::test]
    async fn test_adapt_no_match_below_threshold() {
        let engine = SonaEngine::new(
            SonaMode::Balanced,
            std::sync::Arc::new(TfIdfEmbeddingProvider),
        );

        // No patterns learned
        let result = engine
            .adapt("completely unrelated query about cooking")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_capacity_limit() {
        let engine = SonaEngine::new(SonaMode::Edge, std::sync::Arc::new(TfIdfEmbeddingProvider));
        // Edge mode: max 50 trajectories

        for i in 0..55 {
            let mut traj = make_trajectory("testing", Verdict::Success);
            traj.id = format!("traj-{}", i);
            engine.record(traj).await.unwrap();
        }

        let (count, _) = engine.counts();
        assert!(count <= 50, "Should not exceed capacity: got {}", count);
    }

    #[test]
    fn test_trajectory_total_duration() {
        let traj = Trajectory::new(
            vec![make_step("a", "b"), make_step("c", "d")],
            Verdict::Success,
            "testing",
        );
        assert_eq!(traj.total_duration_ms(), 20);
    }

    #[test]
    fn test_trajectory_avg_confidence() {
        let traj = Trajectory::new(
            vec![
                TrajectoryStep {
                    input: "a".into(),
                    output: "b".into(),
                    duration_ms: 10,
                    confidence: 0.8,
                },
                TrajectoryStep {
                    input: "c".into(),
                    output: "d".into(),
                    duration_ms: 10,
                    confidence: 0.6,
                },
            ],
            Verdict::Success,
            "testing",
        );
        assert!((traj.avg_confidence() - 0.7).abs() < 0.01);
    }

    #[test]
    fn test_sona_mode_default() {
        assert_eq!(SonaMode::default(), SonaMode::Balanced);
    }
}
