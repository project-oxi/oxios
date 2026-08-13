//! Embedding abstraction for semantic similarity.
//!
//! Re-homed from `oxios-memory` (RFC-047): the traits, TF-IDF provider,
//! `TextVector`, and the f32 cosine helpers used by them now live in the
//! kernel. The API and GGUF providers are submodules.
//!
//! Supports two embedding modes:
//! - **Sparse (TF-IDF):** Zero-dependency, works for any language.
//! - **Dense (f32):** Produced by GGUF models (EmbeddingGemma) or API-based models.
//!
//! Dense vectors are used by the HNSW index for fast ANN search.
//! Sparse vectors serve as a fallback when no embedding model is available.

use std::collections::HashMap;

use anyhow::Result;

/// An embedding vector for semantic similarity comparison.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum EmbeddingVector {
    /// Dense vector from API-based embeddings (f64 for precision).
    Dense(Vec<f64>),
    /// Dense f32 vector (for HNSW index compatibility).
    DenseF32(Vec<f32>),
    /// Sparse TF-IDF vector (term → weight).
    Sparse(HashMap<String, f64>),
}

impl EmbeddingVector {
    /// Compute cosine similarity between two vectors.
    pub fn cosine_similarity(&self, other: &Self) -> f64 {
        match (self, other) {
            (EmbeddingVector::Dense(a), EmbeddingVector::Dense(b)) => {
                if a.len() != b.len() || a.is_empty() {
                    return 0.0;
                }
                let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
                let na: f64 = a.iter().map(|v| v * v).sum::<f64>().sqrt();
                let nb: f64 = b.iter().map(|v| v * v).sum::<f64>().sqrt();
                if na == 0.0 || nb == 0.0 {
                    return 0.0;
                }
                dot / (na * nb)
            }
            (EmbeddingVector::DenseF32(a), EmbeddingVector::DenseF32(b)) => {
                cosine_similarity_f32(a, b) as f64
            }
            (EmbeddingVector::Dense(a), EmbeddingVector::DenseF32(b))
            | (EmbeddingVector::DenseF32(b), EmbeddingVector::Dense(a)) => {
                // Cross-dense: convert f32 to f64 for comparison
                let b_f64: Vec<f64> = b.iter().map(|&v| v as f64).collect();
                let (aa, bb) = if matches!(self, EmbeddingVector::Dense(_)) {
                    (a, &b_f64)
                } else {
                    (&b_f64, a)
                };
                if aa.is_empty() || bb.is_empty() || aa.len() != bb.len() {
                    return 0.0;
                }
                let dot: f64 = aa.iter().zip(bb).map(|(x, y)| x * y).sum();
                let na: f64 = aa.iter().map(|v| v * v).sum::<f64>().sqrt();
                let nb: f64 = bb.iter().map(|v| v * v).sum::<f64>().sqrt();
                if na == 0.0 || nb == 0.0 {
                    return 0.0;
                }
                dot / (na * nb)
            }
            (EmbeddingVector::Sparse(a), EmbeddingVector::Sparse(b)) => {
                if a.is_empty() || b.is_empty() {
                    return 0.0;
                }
                let mut dot = 0.0;
                for (term, w) in a {
                    if let Some(w2) = b.get(term) {
                        dot += w * w2;
                    }
                }
                let na: f64 = a.values().map(|v| v * v).sum::<f64>().sqrt();
                let nb: f64 = b.values().map(|v| v * v).sum::<f64>().sqrt();
                if na == 0.0 || nb == 0.0 {
                    return 0.0;
                }
                dot / (na * nb)
            }
            _ => 0.0, // Cross-type comparison not supported
        }
    }

    /// Returns true if this vector is empty/zero.
    pub fn is_empty(&self) -> bool {
        match self {
            EmbeddingVector::Dense(v) => v.is_empty(),
            EmbeddingVector::DenseF32(v) => v.is_empty(),
            EmbeddingVector::Sparse(m) => m.is_empty(),
        }
    }

    /// Convert to f32 dense vector (for HNSW index).
    ///
    /// - `DenseF32` → clone
    /// - `Dense` → cast f64 to f32
    /// - `Sparse` → returns None (not convertible)
    pub fn to_f32_dense(&self) -> Option<Vec<f32>> {
        match self {
            EmbeddingVector::DenseF32(v) => Some(v.clone()),
            EmbeddingVector::Dense(v) => Some(v.iter().map(|&x| x as f32).collect()),
            EmbeddingVector::Sparse(_) => None,
        }
    }

    /// Get the dimensionality of the vector.
    pub fn dimensions(&self) -> usize {
        match self {
            EmbeddingVector::Dense(v) => v.len(),
            EmbeddingVector::DenseF32(v) => v.len(),
            EmbeddingVector::Sparse(m) => m.len(),
        }
    }
}

/// Provider for generating text embeddings.
#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Generate an embedding vector for the given text.
    async fn embed(&self, text: &str) -> Result<EmbeddingVector>;
    /// Name of this provider.
    fn name(&self) -> &str;
}

/// TF-IDF based embedding provider (zero dependencies).
pub struct TfIdfEmbeddingProvider;

#[async_trait::async_trait]
impl EmbeddingProvider for TfIdfEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<EmbeddingVector> {
        let tv = TextVector::from_text(text);
        Ok(EmbeddingVector::Sparse(tv.tf_map().clone()))
    }
    fn name(&self) -> &str {
        "tfidf"
    }
}

/// Simple TF-IDF vector for text similarity.
///
/// Tokenizes text into terms, computes normalized term frequency,
/// and supports cosine similarity comparison. No external embedding
/// model needed — language-agnostic.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TextVector {
    /// Term frequencies (normalized).
    tf: HashMap<String, f64>,
}

impl TextVector {
    /// Create a text vector from input text.
    pub fn from_text(text: &str) -> Self {
        let mut tf: HashMap<String, f64> = HashMap::new();
        let terms = Self::tokenize(text);
        let total = terms.len() as f64;

        for term in terms {
            *tf.entry(term).or_insert(0.0) += 1.0;
        }

        // Normalize by total term count
        if total > 0.0 {
            for v in tf.values_mut() {
                *v /= total;
            }
        }

        Self { tf }
    }

    /// Tokenize text into terms (language-agnostic).
    /// Splits on whitespace and punctuation, lowercases.
    /// Preserves non-ASCII alphanumeric runs (CJK, Hangul, etc.) within tokens.
    pub fn tokenize(text: &str) -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric() && !('\u{AC00}'..='\u{D7A3}').contains(&c))
            .filter(|s| !s.is_empty() && s.len() > 1)
            .map(|s| s.to_string())
            .collect()
    }

    /// Returns a reference to the term-frequency map.
    pub fn tf_map(&self) -> &HashMap<String, f64> {
        &self.tf
    }

    /// Compute cosine similarity between two vectors.
    pub fn cosine_similarity(&self, other: &TextVector) -> f64 {
        let mut dot = 0.0;
        let mut norm_a = 0.0;
        let mut norm_b = 0.0;

        for (term, &a) in &self.tf {
            norm_a += a * a;
            if let Some(&b) = other.tf.get(term) {
                dot += a * b;
            }
        }
        for &b in other.tf.values() {
            norm_b += b * b;
        }

        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }

        dot / (norm_a.sqrt() * norm_b.sqrt())
    }
}

/// Return the L2 norm of a vector.
pub fn l2_norm_f32(vector: &[f32]) -> f32 {
    vector.iter().map(|v| v * v).sum::<f32>().sqrt()
}

/// Compute dot product of two f32 vectors.
pub fn dot_product_f32(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Compute cosine similarity between two f32 vectors.
pub fn cosine_similarity_f32(a: &[f32], b: &[f32]) -> f32 {
    let dot = dot_product_f32(a, b);
    let na = l2_norm_f32(a);
    let nb = l2_norm_f32(b);
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na * nb)
}

pub mod api;

#[cfg(feature = "embedding-gguf")]
pub mod gguf;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dense_f32_similarity() {
        let a = EmbeddingVector::DenseF32(vec![1.0, 0.0, 0.0]);
        let b = EmbeddingVector::DenseF32(vec![1.0, 0.0, 0.0]);
        let sim = a.cosine_similarity(&b);
        assert!((sim - 1.0).abs() < 1e-6, "identical should be 1.0");
    }

    #[test]
    fn test_cross_dense_similarity() {
        let a = EmbeddingVector::Dense(vec![1.0, 0.0, 0.0]);
        let b = EmbeddingVector::DenseF32(vec![1.0, 0.0, 0.0]);
        let sim = a.cosine_similarity(&b);
        assert!((sim - 1.0).abs() < 1e-6, "cross-dense should be 1.0");
    }

    #[test]
    fn test_to_f32_dense_from_dense() {
        let v = EmbeddingVector::Dense(vec![1.0, 2.0]);
        let f32 = v.to_f32_dense().unwrap();
        assert_eq!(f32, vec![1.0f32, 2.0]);
    }

    #[test]
    fn test_to_f32_dense_from_sparse_returns_none() {
        let v = EmbeddingVector::Sparse(HashMap::from([("a".to_string(), 1.0)]));
        assert!(v.to_f32_dense().is_none());
    }

    #[test]
    fn test_dimensions() {
        assert_eq!(EmbeddingVector::Dense(vec![1.0; 10]).dimensions(), 10);
        assert_eq!(EmbeddingVector::DenseF32(vec![1.0; 5]).dimensions(), 5);
        assert_eq!(EmbeddingVector::Sparse(HashMap::new()).dimensions(), 0);
    }

    #[test]
    fn test_tfidf_provider_sparse_vector() {
        let provider = TfIdfEmbeddingProvider;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let vec = rt.block_on(provider.embed("the quick brown fox")).unwrap();
        match vec {
            EmbeddingVector::Sparse(map) => assert!(!map.is_empty()),
            _ => panic!("tfidf must produce sparse"),
        }
    }

    #[test]
    fn test_text_vector_tokenize_handles_hangul() {
        let tv = TextVector::from_text("안녕하세요 세계");
        assert_eq!(tv.tf_map().len(), 2);
    }

    #[test]
    fn test_cosine_similarity_f32_basic() {
        let a = [1.0f32, 0.0, 0.0];
        let b = [1.0f32, 0.0, 0.0];
        assert!((cosine_similarity_f32(&a, &b) - 1.0).abs() < 1e-6);
        let c = [0.0f32, 1.0, 0.0];
        assert!(cosine_similarity_f32(&a, &c).abs() < 1e-6);
    }
}
