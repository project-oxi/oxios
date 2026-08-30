//! Structured web-search bridge for the kernel tool registry.
//!
//! [`KernelWebSearchTool`] / [`KernelGetSearchResultsTool`] — drop-in
//! replacements for the SDK tools (same names, same schemas, same LLM-facing
//! output format) that stash their structured payload into the shared
//! [`StructuredResultBus`], which is how title/url/snippet arrays reach the WS
//! `tool_end` chunk the Web UI renders as search cards.
//! `KernelWebSearchTool` prefers a managed provider (Tavily / Brave per the
//! `[search]` config section) and falls back to the SDK scraping engines
//! (DuckDuckGo / Wikipedia / Bing) when no provider is configured or the
//! provider call fails.
//!
//! Note: provider results intentionally bypass [`oxicode_sdk::SearchCache`] —
//! the `SearchResult` type it stores is not re-exported by `oxicode-sdk`.
//! Provider responses already carry full snippets inline, so the
//! `get_search_results` round-trip remains a scraping-path concern only.

use std::sync::Arc;

use async_trait::async_trait;
use oxicode_sdk::{AgentTool, AgentToolResult, SearchCache, ToolContext, ToolError, WebSearchTool};

use serde_json::{Value, json};

use super::structured_results::StructuredResultBus;

/// Maximum results a single search may request (mirrors the SDK tool).
const MAX_RESULTS: usize = 30;
/// Default result count (mirrors the SDK tool).
const DEFAULT_MAX_RESULTS: u64 = 10;
/// Maximum snippet characters kept per provider result.
const SNIPPET_MAX_CHARS: usize = 300;

// ---------------------------------------------------------------------------
// KernelWebSearchTool
// ---------------------------------------------------------------------------

/// `web_search` with managed-provider preference and bus stashing.
pub struct KernelWebSearchTool {
    cache: Arc<SearchCache>,
    bus: Arc<StructuredResultBus>,
    /// `[search]` config snapshot taken at tool registration (per agent run).
    search: crate::config::SearchConfig,
}

impl KernelWebSearchTool {
    /// Create a new tool with cache, bus, and a `[search]` config snapshot.
    pub fn new(
        cache: Arc<SearchCache>,
        bus: Arc<StructuredResultBus>,
        search: crate::config::SearchConfig,
    ) -> Self {
        Self { cache, bus, search }
    }

    /// Query a managed provider. `Ok(items)` on a well-formed response
    /// (possibly empty); `Err` only on transport/HTTP/auth failures, which
    /// trigger the scraping fallback.
    async fn provider_search(&self, query: &str, limit: usize) -> Result<Vec<Value>, String> {
        match self.search.provider {
            crate::config::SearchProvider::Scrape => Err("no managed provider configured".into()),
            crate::config::SearchProvider::Tavily => {
                let key = self.search.tavily_api_key.trim();
                if key.is_empty() {
                    return Err("provider=tavily but tavily_api_key is empty".into());
                }
                self.tavily_search(key, query, limit).await
            }
            crate::config::SearchProvider::Brave => {
                let key = self.search.brave_api_key.trim();
                if key.is_empty() {
                    return Err("provider=brave but brave_api_key is empty".into());
                }
                self.brave_search(key, query, limit).await
            }
        }
    }

    async fn tavily_search(
        &self,
        key: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Value>, String> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(
                self.search.timeout_secs.max(1),
            ))
            .build()
            .map_err(|e| format!("client build failed: {e}"))?;
        let resp = client
            .post("https://api.tavily.com/search")
            .bearer_auth(key)
            .json(&json!({
                "query": query,
                "max_results": limit,
                "search_depth": "basic",
                "include_answer": false,
            }))
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("invalid JSON: {e}"))?;
        let arr = body
            .get("results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(arr
            .iter()
            .filter_map(|item| {
                let url = item.get("url")?.as_str()?.to_string();
                let title = item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let snippet = truncate_chars(
                    item.get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default(),
                    SNIPPET_MAX_CHARS,
                );
                Some(json!({"title": title, "url": url, "snippet": snippet, "source": "Tavily"}))
            })
            .collect())
    }

    async fn brave_search(
        &self,
        key: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Value>, String> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(
                self.search.timeout_secs.max(1),
            ))
            .build()
            .map_err(|e| format!("client build failed: {e}"))?;
        let mut req = client
            .get("https://api.search.brave.com/res/v1/web/search")
            .header("Accept", "application/json")
            .header("X-Subscription-Token", key)
            .query(&[("q", query.to_string()), ("count", limit.to_string())]);
        // Region hint "kr-KR" → country=KR, search_lang=kr.
        let region = self.search.region.trim();
        if !region.is_empty() {
            let mut parts = region.splitn(2, '-');
            let lang = parts.next().unwrap_or_default();
            let country = parts.next().unwrap_or(lang);
            if !lang.is_empty() {
                req = req.query(&[("search_lang", lang), ("country", country)]);
            }
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("invalid JSON: {e}"))?;
        let arr = body
            .pointer("/web/results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(arr
            .iter()
            .filter_map(|item| {
                let url = item.get("url")?.as_str()?.to_string();
                let title = item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let snippet = truncate_chars(
                    item.get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default(),
                    SNIPPET_MAX_CHARS,
                );
                Some(json!({"title": title, "url": url, "snippet": snippet, "source": "Brave"}))
            })
            .collect())
    }

    /// Format provider results for the LLM — same shape as the SDK tool's
    /// markdown so prompt behavior is identical across providers.
    fn format_results(query: &str, results: &[Value]) -> String {
        if results.is_empty() {
            return format!("No results found for: {query}");
        }
        results
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let title = r.get("title").and_then(|v| v.as_str()).unwrap_or_default();
                let url = r.get("url").and_then(|v| v.as_str()).unwrap_or_default();
                let snippet = r
                    .get("snippet")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                format!("{}. **{}**\n   {}\n   {}", i + 1, title, url, snippet)
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[async_trait]
impl AgentTool for KernelWebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn label(&self) -> &str {
        "Web Search"
    }

    fn description(&self) -> &str {
        "Search the web. Uses a managed provider when configured (Tavily/Brave), \
         otherwise multi-engine scraping (DuckDuckGo, Wikipedia, Bing). Returns \
         results with titles, URLs, and snippets."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query string"
                },
                "engines": {
                    "type": "string",
                    "description": "Scraping fallback engines, comma-separated (ddg,wiki,bing). Default: ddg,wiki",
                    "default": "ddg,wiki"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 10, max: 30)",
                    "default": 10
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        tool_call_id: &str,
        params: Value,
        signal: Option<tokio::sync::oneshot::Receiver<()>>,
        ctx: &ToolContext,
    ) -> Result<AgentToolResult, ToolError> {
        let query = params["query"]
            .as_str()
            .ok_or_else(|| "Missing required parameter: query".to_string())?
            .to_string();
        let limit = params["limit"]
            .as_u64()
            .unwrap_or(DEFAULT_MAX_RESULTS)
            .min(MAX_RESULTS as u64) as usize;

        // Managed provider first.
        match self.provider_search(&query, limit).await {
            Ok(items) => {
                let output = Self::format_results(&query, &items);
                let payload = json!({"query": query, "resultCount": items.len(), "results": items});
                self.bus.insert(tool_call_id, payload.clone());
                return Ok(AgentToolResult::success(output).with_metadata(payload));
            }
            Err(e) => {
                tracing::warn!(
                    provider = ?self.search.provider,
                    error = %e,
                    "managed search provider unavailable; falling back to scraping engines"
                );
            }
        }

        // Fallback: SDK scraping engines. The inner tool inserts into the
        // SearchCache itself; stash its metadata payload into the bus.
        let inner = WebSearchTool::new(Arc::clone(&self.cache));
        let result = inner.execute(tool_call_id, params, signal, ctx).await?;
        if result.success
            && let Some(meta) = &result.metadata
        {
            self.bus.insert(tool_call_id, meta.clone());
        }
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// KernelGetSearchResultsTool
// ---------------------------------------------------------------------------

/// `get_search_results` wrapper that stashes the structured payload into the
/// bus so retrieval round-trips render richly too.
pub struct KernelGetSearchResultsTool {
    inner: oxicode_sdk::GetSearchResultsTool,
    bus: Arc<StructuredResultBus>,
}

impl KernelGetSearchResultsTool {
    /// Create a new wrapper around the SDK tool.
    pub fn new(inner: oxicode_sdk::GetSearchResultsTool, bus: Arc<StructuredResultBus>) -> Self {
        Self { inner, bus }
    }
}

#[async_trait]
impl AgentTool for KernelGetSearchResultsTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn label(&self) -> &str {
        self.inner.label()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> Value {
        self.inner.parameters_schema()
    }

    async fn execute(
        &self,
        tool_call_id: &str,
        params: Value,
        signal: Option<tokio::sync::oneshot::Receiver<()>>,
        ctx: &ToolContext,
    ) -> Result<AgentToolResult, ToolError> {
        let result = self
            .inner
            .execute(tool_call_id, params, signal, ctx)
            .await?;
        if result.success
            && let Some(meta) = &result.metadata
        {
            self.bus.insert(tool_call_id, meta.clone());
        }
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Char-boundary-safe truncation (CJK-safe).
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_handles_multibyte() {
        let s = "한글무시"; // 4 chars, 12 bytes
        assert_eq!(truncate_chars(s, 2), "한글");
        assert_eq!(truncate_chars(s, 10), s);
        assert_eq!(truncate_chars(s, 3).chars().count(), 3);
    }

    #[test]
    fn format_results_matches_sdk_shape() {
        let items = vec![json!({
            "title": "T", "url": "https://x.example", "snippet": "s", "source": "Tavily"
        })];
        let out = KernelWebSearchTool::format_results("q", &items);
        assert!(out.starts_with("1. **T**\n   https://x.example\n   s"));
    }

    #[tokio::test]
    async fn scrape_provider_without_fallback_returns_error_then_inner_tool_used() {
        // provider=Scrape → provider_search always errs; execute() must then
        // delegate to the SDK tool (verified by name-registered inner path in
        // registration tests). Here we only assert the provider layer errs.
        let tool = KernelWebSearchTool::new(
            Arc::new(SearchCache::new()),
            Arc::new(StructuredResultBus::new()),
            crate::config::SearchConfig::default(),
        );
        assert!(tool.provider_search("q", 5).await.is_err());
    }
}
