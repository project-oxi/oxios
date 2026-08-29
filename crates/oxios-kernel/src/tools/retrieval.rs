//! Semantic search engine for OS capabilities.
//!
//! [`ToolRetriever`] maintains an in-memory index of all available tools
//! (built-in OS tools, installed programs, OS services, and MCP bridges)
//! and retrieves the most relevant ones for a given query using the
//! embedding module's cosine similarity.
//!
//! # Usage
//!
//! ```no_run
//! use std::sync::Arc;
//! use oxios_kernel::embedding::TfIdfEmbeddingProvider;
//! use oxios_kernel::tools::retrieval::{ToolRetriever, ToolEntry};
//!
//! # async fn example() {
//! let embedder = Arc::new(TfIdfEmbeddingProvider);
//! let mut retriever = ToolRetriever::new(embedder);
//!
//! let tool = ToolEntry {
//!     name: "exec".into(),
//!     category: "os-tool".into(),
//!     description: "Execute a shell command in a workspace".into(),
//!     skill_path: None,
//!     command: None,
//! };
//! retriever.index_tool(tool).await;
//! # }

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::embedding::{EmbeddingProvider, EmbeddingVector};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A searchable entry in the tool index.
///
/// Each entry describes a single capability that the agent OS exposes,
/// such as a built-in execution tool, an installed program, an OS service,
/// or an MCP bridge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEntry {
    /// Unique capability name (e.g. `"exec"`, `"git-helper"`, `"mcp:github"`).
    pub name: String,
    /// Category of the capability.
    ///
    /// One of: `"os-tool"`, `"program"`, `"os-service"`, `"mcp"`.
    pub category: String,
    /// Human-readable description used both for indexing and for the
    /// capability index presented to agents.
    pub description: String,
    /// Path to the SKILL.md instruction file, if this is a program.
    pub skill_path: Option<String>,
    /// Invocation command, if this is a program that can be called directly.
    pub command: Option<String>,
}

impl ToolEntry {
    /// Produce the text that will be embedded for semantic search.
    ///
    /// Combines name, category, and description into a single string so that
    /// the embedding captures all relevant semantics.
    fn embedding_text(&self) -> String {
        let mut parts = format!("[{}] {}: {}", self.category, self.name, self.description);
        if let Some(ref cmd) = self.command {
            parts.push_str(&format!(" command: {cmd}"));
        }
        parts
    }
}

/// A tool entry together with its pre-computed embedding vector.
#[derive(Debug, Clone)]
struct IndexedTool {
    entry: ToolEntry,
    vector: EmbeddingVector,
}

/// A tool ranked by relevance to a query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredTool {
    /// The tool entry that matched.
    pub entry: ToolEntry,
    /// Cosine similarity score in `[0.0, 1.0]`.  Higher is more relevant.
    pub score: f64,
}

// ---------------------------------------------------------------------------
// ToolRetriever
// ---------------------------------------------------------------------------

/// Semantic search engine for OS capabilities.
///
/// Maintains an in-memory vector index of all registered tools and supports
/// top-K retrieval via cosine similarity against a query embedding.
pub struct ToolRetriever {
    /// The indexed tools with their pre-computed embeddings.
    index: Vec<IndexedTool>,
    /// The embedding provider used to vectorize tool descriptions.
    embedder: Arc<dyn EmbeddingProvider>,
}

impl ToolRetriever {
    /// Create a new, empty retriever backed by the given embedder.
    pub fn new(embedder: Arc<dyn EmbeddingProvider>) -> Self {
        Self {
            index: Vec::new(),
            embedder,
        }
    }

    /// Return a reference to the underlying embedder.
    ///
    /// Useful when the caller needs to compute a query embedding before
    /// calling [`retrieve`](Self::retrieve).
    pub fn embedder(&self) -> &Arc<dyn EmbeddingProvider> {
        &self.embedder
    }

    /// Add a tool to the index.
    ///
    /// The tool's description is embedded immediately using the configured
    /// provider.  If the embedding fails the tool is silently skipped
    /// (logged at warn level in future telemetry).
    pub async fn index_tool(&mut self, entry: ToolEntry) {
        let text = entry.embedding_text();
        match self.embedder.embed(&text).await {
            Ok(vector) => {
                self.index.push(IndexedTool { entry, vector });
            }
            Err(e) => {
                tracing::warn!(name = %entry.name, error = %e, "failed to embed tool, skipping");
            }
        }
    }

    /// Retrieve the top-K tools most relevant to the given query embedding.
    ///
    /// The `query_embedding` is compared against every indexed tool using
    /// [`EmbeddingVector::cosine_similarity`].  Results are sorted by score
    /// descending.
    ///
    /// If `top_k` exceeds the number of indexed tools, all tools are returned.
    pub fn retrieve(&self, query_embedding: &EmbeddingVector, top_k: usize) -> Vec<ScoredTool> {
        let mut scored: Vec<ScoredTool> = self
            .index
            .iter()
            .map(|indexed| {
                let score = query_embedding.cosine_similarity(&indexed.vector);
                ScoredTool {
                    entry: indexed.entry.clone(),
                    score,
                }
            })
            .collect();

        // Sort descending by score.
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        scored.truncate(top_k);
        scored
    }

    /// Number of indexed tools.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Returns `true` if no tools have been indexed.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Get all indexed entries (for capability index generation or debugging).
    pub fn entries(&self) -> Vec<&ToolEntry> {
        self.index.iter().map(|i| &i.entry).collect()
    }

    /// Remove all indexed tools.
    pub fn clear(&mut self) {
        self.index.clear();
    }
}

// ---------------------------------------------------------------------------
// Capability index formatting
// ---------------------------------------------------------------------------

/// Format retrieved tools as an XML capability index suitable for injection
/// into an agent's system prompt.
///
/// Example output:
///
/// ```xml
/// <available_capabilities>
///   <capability>
///     <name>exec</name>
///     <category>os-tool</category>
///     <description>Execute a shell command in a workspace</description>
///   </capability>
///   <capability>
///     <name>git-helper</name>
///     <category>program</category>
///     <description>Git workflow automation</description>
///     <command>git-helper</command>
///     <skill>programs/git-helper/SKILL.md</skill>
///   </capability>
/// </available_capabilities>
/// ```
pub fn format_capability_index(tools: &[ScoredTool]) -> String {
    let mut xml = String::from("<available_capabilities>\n");

    for tool in tools {
        xml.push_str("  <capability>\n");
        xml.push_str(&format!(
            "    <name>{}</name>\n",
            escape_xml(&tool.entry.name)
        ));
        xml.push_str(&format!(
            "    <category>{}</category>\n",
            escape_xml(&tool.entry.category)
        ));
        xml.push_str(&format!(
            "    <description>{}</description>\n",
            escape_xml(&tool.entry.description)
        ));
        if let Some(ref cmd) = tool.entry.command {
            xml.push_str(&format!("    <command>{}</command>\n", escape_xml(cmd)));
        }
        if let Some(ref skill) = tool.entry.skill_path {
            xml.push_str(&format!("    <skill>{}</skill>\n", escape_xml(skill)));
        }
        xml.push_str("  </capability>\n");
    }

    xml.push_str("</available_capabilities>");
    xml
}

/// Escape special XML characters in a string.
fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Kernel manifest
// ---------------------------------------------------------------------------

/// Well-known domain names that can appear in a kernel manifest.
const KNOWN_DOMAINS: &[&str] = &[
    "project",
    "agent",
    "a2a",
    "memory",
    "knowledge",
    "security",
    "budget",
    "resource",
    "program",
];

/// Build a markdown kernel manifest from the set of active domains.
///
/// The manifest lists which subsystems are currently enabled so that an
/// agent can discover the OS's capabilities at a glance.
///
/// # Example
///
/// ```text
/// ## Kernel Manifest
///
/// Active domains: project, agent, memory, program
///
/// ### project
/// Project and mount (path-alias) management.
///
/// ### agent
/// ...
/// ```
pub fn build_kernel_manifest(active_domains: &[&str]) -> String {
    let mut md = String::from("## Kernel Manifest\n\n");

    let domain_list: Vec<&str> = active_domains
        .iter()
        .filter(|d| KNOWN_DOMAINS.contains(d))
        .copied()
        .collect();

    md.push_str(&format!("Active domains: {}\n\n", domain_list.join(", ")));

    for domain in &domain_list {
        let description = domain_description(domain);
        md.push_str(&format!("### {domain}\n{description}\n\n"));
    }

    md
}

/// Return a short human-readable description for a known domain.
fn domain_description(domain: &str) -> &'static str {
    match domain {
        "project" => "Project and mount (path-alias) management.",
        "agent" => "Agent lifecycle, runtime, and supervisor.",
        "a2a" => "Agent-to-agent communication and delegation.",
        "memory" => {
            "Agent memory via the `brain` skill — remember/ask through the \
             allowlisted `oxibrain` CLI. Facts, preferences, patterns. Not user-visible."
        }
        "knowledge" => {
            "Personal markdown vault — documents, articles, notes, journal. File-based with backlinks and full-text search."
        }
        "security" => "RBAC access control and audit trail.",
        "budget" => "Token and cost budget enforcement.",
        "resource" => "System resource monitoring and overload protection.",
        "program" => "Installable OS-level programs and tools.",
        _ => "Unknown domain.",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A trivial embedder that maps every non-empty text to a fixed dense
    /// vector.  Useful for unit-testing retrieval logic without depending
    /// on TF-IDF or external models.
    struct MockEmbedder;

    #[async_trait::async_trait]
    impl EmbeddingProvider for MockEmbedder {
        async fn embed(&self, text: &str) -> anyhow::Result<EmbeddingVector> {
            if text.is_empty() {
                return Ok(EmbeddingVector::DenseF32(vec![]));
            }
            // Produce a deterministic vector based on text length so different
            // texts get different vectors.
            let len = text.len() as f32;
            Ok(EmbeddingVector::DenseF32(vec![1.0, len / 100.0, 0.5]))
        }

        fn name(&self) -> &str {
            "mock"
        }
    }

    fn mock_entry(name: &str, category: &str, desc: &str) -> ToolEntry {
        ToolEntry {
            name: name.to_string(),
            category: category.to_string(),
            description: desc.to_string(),
            skill_path: None,
            command: None,
        }
    }

    #[tokio::test]
    async fn test_index_and_len() {
        let embedder = Arc::new(MockEmbedder);
        let mut retriever = ToolRetriever::new(embedder);

        assert!(retriever.is_empty());
        assert_eq!(retriever.len(), 0);

        retriever
            .index_tool(mock_entry("exec", "os-tool", "Run commands"))
            .await;
        retriever
            .index_tool(mock_entry("git", "program", "Git operations"))
            .await;

        assert_eq!(retriever.len(), 2);
        assert!(!retriever.is_empty());
    }

    #[tokio::test]
    async fn test_retrieve_top_k() {
        let embedder = Arc::new(MockEmbedder);
        let mut retriever = ToolRetriever::new(embedder);

        retriever
            .index_tool(mock_entry("exec", "os-tool", "Run shell commands"))
            .await;
        retriever
            .index_tool(mock_entry("git", "program", "Git version control"))
            .await;
        retriever
            .index_tool(mock_entry("mcp-github", "mcp", "GitHub API bridge"))
            .await;

        let query = EmbeddingVector::DenseF32(vec![1.0, 0.5, 0.5]);
        let results = retriever.retrieve(&query, 2);

        assert_eq!(results.len(), 2);
        // Results should be sorted by score descending.
        assert!(results[0].score >= results[1].score);
    }

    #[tokio::test]
    async fn test_retrieve_exceeds_index() {
        let embedder = Arc::new(MockEmbedder);
        let mut retriever = ToolRetriever::new(embedder);

        retriever
            .index_tool(mock_entry("exec", "os-tool", "Run commands"))
            .await;

        let query = EmbeddingVector::DenseF32(vec![1.0, 0.5, 0.5]);
        let results = retriever.retrieve(&query, 10);

        // Should return all available tools, not panic.
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_retrieve_empty_index() {
        let embedder = Arc::new(MockEmbedder);
        let retriever = ToolRetriever::new(embedder);

        let query = EmbeddingVector::DenseF32(vec![1.0, 0.5, 0.5]);
        let results = retriever.retrieve(&query, 5);

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_entries() {
        let embedder = Arc::new(MockEmbedder);
        let mut retriever = ToolRetriever::new(embedder);

        retriever
            .index_tool(mock_entry("exec", "os-tool", "Run commands"))
            .await;
        retriever
            .index_tool(mock_entry("git", "program", "Git ops"))
            .await;

        let entries = retriever.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "exec");
        assert_eq!(entries[1].name, "git");
    }

    #[tokio::test]
    async fn test_clear() {
        let embedder = Arc::new(MockEmbedder);
        let mut retriever = ToolRetriever::new(embedder);

        retriever
            .index_tool(mock_entry("exec", "os-tool", "Run commands"))
            .await;
        assert_eq!(retriever.len(), 1);

        retriever.clear();
        assert!(retriever.is_empty());
    }

    #[test]
    fn test_format_capability_index_basic() {
        let tool = ScoredTool {
            entry: ToolEntry {
                name: "exec".into(),
                category: "os-tool".into(),
                description: "Execute shell commands".into(),
                skill_path: None,
                command: None,
            },
            score: 0.95,
        };

        let xml = format_capability_index(&[tool]);
        assert!(xml.contains("<available_capabilities>"));
        assert!(xml.contains("<name>exec</name>"));
        assert!(xml.contains("<category>os-tool</category>"));
        assert!(xml.contains("<description>Execute shell commands</description>"));
        assert!(xml.contains("</available_capabilities>"));
        // No command/skill tags for os-tool.
        assert!(!xml.contains("<command>"));
        assert!(!xml.contains("<skill>"));
    }

    #[test]
    fn test_format_capability_index_program() {
        let tool = ScoredTool {
            entry: ToolEntry {
                name: "git-helper".into(),
                category: "program".into(),
                description: "Git workflow automation".into(),
                skill_path: Some("programs/git-helper/SKILL.md".into()),
                command: Some("git-helper".into()),
            },
            score: 0.88,
        };

        let xml = format_capability_index(&[tool]);
        assert!(xml.contains("<command>git-helper</command>"));
        assert!(xml.contains("<skill>programs/git-helper/SKILL.md</skill>"));
    }

    #[test]
    fn test_format_capability_index_xml_escaping() {
        let tool = ScoredTool {
            entry: ToolEntry {
                name: "test<>&".into(),
                category: "os-tool".into(),
                description: "A & B < C > D".into(),
                skill_path: None,
                command: None,
            },
            score: 1.0,
        };

        let xml = format_capability_index(&[tool]);
        assert!(xml.contains("<name>test&lt;&gt;&amp;</name>"));
        assert!(xml.contains("<description>A &amp; B &lt; C &gt; D</description>"));
    }

    #[test]
    fn test_escape_xml() {
        assert_eq!(escape_xml("hello"), "hello");
        assert_eq!(
            escape_xml("a&b<c>d\"e'f"),
            "a&amp;b&lt;c&gt;d&quot;e&apos;f"
        );
    }

    #[test]
    fn test_build_kernel_manifest() {
        let md = build_kernel_manifest(&["project", "agent", "memory", "knowledge", "program"]);
        assert!(md.contains("## Kernel Manifest"));
        assert!(md.contains("Active domains: project, agent, memory, knowledge, program"));
        assert!(md.contains("### project"));
        assert!(md.contains("### agent"));
        assert!(md.contains("### memory"));
        assert!(md.contains("### knowledge"));
        assert!(md.contains("### program"));
        assert!(!md.contains("### security"));
    }

    #[test]
    fn test_build_kernel_manifest_filters_unknown() {
        let md = build_kernel_manifest(&["project", "unknown-domain"]);
        assert!(md.contains("### project"));
        assert!(!md.contains("unknown-domain"));
    }

    #[test]
    fn test_build_kernel_manifest_empty() {
        let md = build_kernel_manifest(&[]);
        assert!(md.contains("## Kernel Manifest"));
        assert!(md.contains("Active domains:"));
    }

    #[test]
    fn test_tool_entry_embedding_text() {
        let entry = mock_entry("exec", "os-tool", "Run commands");
        let text = entry.embedding_text();
        assert!(text.contains("[os-tool] exec: Run commands"));
    }

    #[test]
    fn test_tool_entry_embedding_text_with_command() {
        let entry = ToolEntry {
            name: "git".into(),
            category: "program".into(),
            description: "Git ops".into(),
            skill_path: None,
            command: Some("git binary".into()),
        };
        let text = entry.embedding_text();
        assert!(text.contains("command: git binary"));
    }

    #[tokio::test]
    async fn test_embedder_accessor() {
        let embedder = Arc::new(MockEmbedder);
        let retriever = ToolRetriever::new(embedder);
        assert_eq!(retriever.embedder().name(), "mock");
    }

    // --- Integration-style test with TfIdfEmbeddingProvider ---

    #[tokio::test]
    async fn test_with_tfidf_embedder() {
        use crate::embedding::TfIdfEmbeddingProvider;

        let embedder = Arc::new(TfIdfEmbeddingProvider);
        let mut retriever = ToolRetriever::new(embedder);

        retriever
            .index_tool(ToolEntry {
                name: "exec".into(),
                category: "os-tool".into(),
                description: "Execute shell commands in workspace".into(),
                skill_path: None,
                command: None,
            })
            .await;
        retriever
            .index_tool(ToolEntry {
                name: "memory-search".into(),
                category: "os-tool".into(),
                description: "Search persistent vector memory".into(),
                skill_path: None,
                command: None,
            })
            .await;

        let query_embedding = retriever
            .embedder()
            .embed("run a bash command")
            .await
            .unwrap();
        let results = retriever.retrieve(&query_embedding, 2);

        assert_eq!(results.len(), 2);
        // "exec" should score higher for "run a bash command" query.
        assert_eq!(results[0].entry.name, "exec");
    }
}
