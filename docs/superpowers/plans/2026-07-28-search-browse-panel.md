# Search & Browse Panel Implementation Plan
> **Status**: Shipped — 2026-07/08 (SearchView in portal)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a dedicated Search & Browse Panel to the chat page — agent-driven (auto-open on web_search/browse tool calls) and user-driven (manual search/browse via new API endpoints).

**Architecture:** Extend the existing PortalPanel (right-side stack panel) with a new `search` view. Two new backend endpoints (`POST /api/search`, `POST /api/browse`) enable user-driven searches without the agent loop. A separate `SearchPanel` store manages manual results, browse cache, and expand state.

**Tech Stack:** TypeScript/React (web), Rust/axum (API), oxi-sdk 0.58 (BrowserEngine), oxibrowser 0.16 (search dispatch)

## Global Constraints

- `oxibrowser = "0.16"` must be added to binary crate `Cargo.toml` deps (already transitive via oxi-sdk)
- All new PortalView variants follow the union type pattern in `stores/portal.ts`
- Backend API handlers use `State<Arc<AppState>>` pattern from existing routes
- Browse engine accessed via `state.kernel.browser` (Option\<BrowserApi\>)
- `BrowserEngine`, `BrowserTab`, `PageContent` types from `oxi_sdk` (re-exported via `oxios-kernel`)
- `oxibrowser::search::dispatch()` used directly for search endpoint
- Frontend components use Tailwind CSS + existing UI primitives (Button, ScrollArea, MarkdownMessage)

---

### Task 1: Backend API — Cargo.toml + search routes

**Files:**
- Modify: `Cargo.toml` — add oxibrowser dep
- Create: `src/api/routes/search.rs` — POST /api/search + POST /api/browse
- Modify: `src/api/routes/mod.rs` — declare module + register routes

**Interfaces:**
- Produces: `POST /api/search` and `POST /api/browse` endpoints

- [ ] **Step 1: Add oxibrowser dependency**

Add to `Cargo.toml` under `[dependencies]` (after the `include_dir` line):
```toml
oxibrowser = "0.16"
```

- [ ] **Step 2: Create `src/api/routes/search.rs`**

File contains two handlers. Imports and key types:

```rust
use std::sync::Arc;
use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use crate::api::error::AppError;
use crate::api::server::AppState;

#[derive(Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default = "default_engines")]
    pub engines: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}
fn default_engines() -> String { "ddg,wiki".to_string() }
fn default_limit() -> usize { 10 }

#[derive(Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResultItem>,
    pub elapsed_ms: u64,
}
#[derive(Serialize)]
pub struct SearchResultItem {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub engine: String,
}

#[derive(Deserialize)]
pub struct BrowseRequest {
    pub url: String,
    #[serde(default = "default_format")]
    pub format: String,
}
fn default_format() -> String { "markdown".to_string() }

#[derive(Serialize)]
pub struct BrowseResponse {
    pub url: String,
    pub title: String,
    pub markdown: String,
    pub status: u16,
    pub elapsed_ms: u64,
}

/// POST /api/search — direct web search (no agent loop).
pub(crate) async fn handle_search(
    state: State<Arc<AppState>>,
    Json(body): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, AppError> {
    let start = std::time::Instant::now();
    // oxibrowser::search::dispatch() signature:
    //   dispatch(query, source, engine_spec, repo, token, max_results, timeout_secs)
    let output = oxibrowser::search::dispatch(
        &body.query,
        "web",
        &body.engines,
        None,  // repo
        None,  // token
        body.limit,
        10,    // timeout_secs
    )
    .await
    .map_err(|e| AppError::Internal(format!("search failed: {e}")))?;

    let elapsed = start.elapsed().as_millis() as u64;
    let results: Vec<SearchResultItem> = output.results.into_iter().map(|r| SearchResultItem {
        title: r.title,
        url: r.url,
        snippet: r.snippet,
        engine: output.engine.clone(),
    }).collect();

    Ok(Json(SearchResponse { results, elapsed_ms: elapsed }))
}

/// POST /api/browse — read a web page as markdown (no agent loop).
pub(crate) async fn handle_browse(
    state: State<Arc<AppState>>,
    Json(body): Json<BrowseRequest>,
) -> Result<Json<BrowseResponse>, AppError> {
    use std::sync::Arc as _;

    let start = std::time::Instant::now();

    // Get the browser engine (requires native-browser feature)
    let browser = state
        .kernel
        .browser
        .as_ref()
        .ok_or_else(|| AppError::ServiceUnavailable("browser engine not available"))?;

    let engine = browser
        .engine()
        .await
        .map_err(|e| AppError::Internal(format!("browser init failed: {e}")))?;

    let tab = engine
        .new_tab()
        .await
        .map_err(|e| AppError::Internal(format!("browser tab create failed: {e}")))?;

    let page = tab
        .goto(&body.url)
        .await
        .map_err(|e| AppError::Internal(format!("browse failed: {e}")))?;

    let elapsed = start.elapsed().as_millis() as u64;

    Ok(Json(BrowseResponse {
        url: page.url,
        title: page.title,
        markdown: page.markdown,
        status: page.status,
        elapsed_ms: elapsed,
    }))
}
```

Note: `PageContent.url`, `PageContent.title`, `PageContent.markdown`, `PageContent.status` — these field names come from `oxi_sdk::PageContent`.

- [ ] **Step 3: Register module + routes in `src/api/routes/mod.rs`**

Add after the `mod marketplace;` line:
```rust
mod search;
```

Add routes after the existing tool registry route (around line 594):
```rust
// Search & Browse (Search Panel)
.route("/api/search", post(handle_search))
.route("/api/browse", post(handle_browse))
```

And import the handlers at the top of the route registration section (add to the existing `use` block near the function):
```rust
use search::{handle_browse, handle_search};
```

- [ ] **Step 4: Verify compilation**

```bash
cargo build -p oxios 2>&1 | tail -5
```
Expected: `Finished` with no errors. If `BrowserEngine` trait doesn't resolve, check import path — it's `oxi_sdk::BrowserEngine` from oxi-sdk 0.58.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/api/routes/search.rs src/api/routes/mod.rs
git commit -m "feat(api): add POST /api/search and POST /api/browse endpoints"
```

---

### Task 2: BrowseRender component

**Files:**
- Create: `web/src/components/chat/tool-renders/Browse.tsx`
- Modify: `web/src/components/chat/tool-renders/index.ts`

**Interfaces:**
- Consumes: `ToolRenderComponent` from `tool-renders/registry.tsx`
- Produces: `BrowseRender` — renders browse tool results as formatted markdown

- [ ] **Step 1: Create `web/src/components/chat/tool-renders/Browse.tsx`**

```tsx
// Browse render — browse tool result with markdown body (replaces WebFetchRender).

import { Globe } from 'lucide-react'
import { MarkdownMessage } from '@/components/chat/markdown-message'
import { Button } from '@/components/ui/button'
import { usePortalStore } from '@/stores/portal'
import type { ToolRenderComponent } from './registry'

interface BrowseResult {
  url?: string
  title?: string
  markdown?: string
  text?: string
  content?: string
  html?: string
  status?: number
}

function tryJson(s: string): BrowseResult | null {
  try { return JSON.parse(s) as BrowseResult }
  catch { return null }
}

function domain(url: string): string {
  try { return new URL(url).hostname }
  catch { return url }
}

export const BrowseRender: ToolRenderComponent = ({ args, result, isRunning }) => {
  const url = (args?.url ?? args?.uri ?? '') as string
  const parsed: BrowseResult =
    typeof result === 'string' ? (tryJson(result) ?? { content: result }) : (result as BrowseResult)

  const title = parsed.title ?? domain(parsed.url ?? url)
  const href = parsed.url ?? url
  const body = parsed.markdown ?? parsed.text ?? parsed.content ?? ''

  return (
    <div className="space-y-2 text-sm">
      {/* Header: icon + title link + status badge */}
      <div className="flex items-center gap-2">
        <Globe className="w-3.5 h-3.5 text-muted-foreground shrink-0" />
        <a
          href={href}
          target="_blank"
          rel="noreferrer"
          className="text-primary hover:underline truncate font-medium"
        >
          {title}
        </a>
        {parsed.status !== undefined && (
          <span className={`text-xs tabular-nums shrink-0 ${
            parsed.status === 200 ? 'text-emerald-600' :
            parsed.status < 400 ? 'text-amber-600' :
            'text-destructive'
          }`}>
            {parsed.status}
          </span>
        )}
      </div>

      {/* Body: markdown content */}
      {isRunning ? (
        <div className="text-xs text-muted-foreground animate-pulse">Loading page…</div>
      ) : body ? (
        <div className="max-h-80 overflow-y-auto rounded bg-muted/40 p-2">
          <MarkdownMessage messageId="" isStreaming={false}>
            {body}
          </MarkdownMessage>
        </div>
      ) : result != null ? (
        <pre className="p-2 rounded bg-muted text-xs overflow-x-auto max-h-48 whitespace-pre-wrap">
          {typeof result === 'string' ? result.slice(0, 3000) : JSON.stringify(result, null, 2)}
        </pre>
      ) : null}

      {/* Footer: open in panel */}
      {body && (
        <div className="flex justify-end">
          <Button
            variant="ghost"
            size="sm"
            className="text-xs h-6 px-2"
            onClick={() => usePortalStore.getState().pushView({ type: 'search', query: title })}
          >
            Open in Panel
          </Button>
        </div>
      )}
    </div>
  )
}
```

- [ ] **Step 2: Register browse tools in `index.ts`**

Add import at the top (after the WebSearchRender import):
```tsx
import { BrowseRender } from './Browse'
```

Add registrations after the web_search/webFetch group:
```tsx
// Browse (headless browser page reading)
registerToolRender('browse', BrowseRender)
registerToolRender('browse_extract', BrowseRender)
registerToolRender('browse_session', BrowseRender)
registerToolRender('browse_script', BrowseRender)
```

- [ ] **Step 3: Run frontend tests**

```bash
cd web && bun run test 2>&1 | tail -10
```
Expected: tests pass. The existing WebSearch tests should be unaffected.

- [ ] **Step 4: Commit**

```bash
git add web/src/components/chat/tool-renders/Browse.tsx web/src/components/chat/tool-renders/index.ts
git commit -m "feat(web): add BrowseRender component with markdown rendering"
```

---

### Task 3: PortalView search variant

**Files:**
- Modify: `web/src/stores/portal.ts`
- Modify: `web/src/components/portal/portal-panel.tsx`

**Interfaces:**
- Produces: `PortalView` union has `{ type: 'search'; query?: string; messageId?: string }`
- Produces: `PortalPanel` renders `SearchView` when top view is `'search'`

- [ ] **Step 1: Add `search` variant to PortalView union in `stores/portal.ts`**

Find the `PortalView` type union (around line 33-47). Add after the `document` variant:
```typescript
  | {
      type: 'search'
      /** Search query (auto-set on agent-driven, entered in panel on manual). */
      query?: string
      /** Chat message ID that triggered this view (agent-driven only). */
      messageId?: string
    }
```

- [ ] **Step 2: Add `search` case to portal-panel.tsx view dispatcher**

In the `ViewBody` function (around line 67-78), add a case for `'search'`:
```tsx
      case 'search':
        return <SearchView query={view.query} messageId={view.messageId} />
```

Import SearchView:
```tsx
import { SearchView } from './views/search-view'
```

- [ ] **Step 3: Commit**

```bash
git add web/src/stores/portal.ts web/src/components/portal/portal-panel.tsx
git commit -m "feat(web): add search variant to PortalView"
```

---

### Task 4: SearchPanel store

**Files:**
- Create: `web/src/stores/search-panel.ts`

**Interfaces:**
- Produces: `SearchPanelState` — manual search results, browse cache, expand state
- Produces: Actions — `search()`, `browse()`, `toggleExpand()`, `saveToKnowledge()`, `reset()`

- [ ] **Step 1: Create `web/src/stores/search-panel.ts`**

```typescript
// Search panel store — manual search results, browse cache, UI expansion state.
//
// The portal store owns stack navigation (push/pop view). This store owns
// the data that the SearchView displays: manually submitted search queries
// (via POST /api/search), cached browse results (via POST /api/browse),
// and per-card expand state.

import { create } from 'zustand'

// ── Types ──

export interface SearchResultItem {
  title: string
  url: string
  snippet: string
  engine: string
}

export interface BrowseResult {
  url: string
  title: string
  markdown: string
  status: number
  elapsed_ms?: number
}

interface SearchResponse {
  results: SearchResultItem[]
  elapsed_ms: number
}

interface BrowseResponse {
  url: string
  title: string
  markdown: string
  status: number
  elapsed_ms: number
}

// ── Store ──

export interface SearchPanelState {
  // Manual search state
  manualQuery: string
  manualResults: SearchResultItem[]
  manualLoading: boolean
  manualError: string | null

  // Browse cache (URL → content)
  browseCache: Record<string, BrowseResult>
  browseLoading: Record<string, boolean>
  browseError: Record<string, string | null>

  // UI state
  expandedUrls: Set<string>

  // Actions
  search: (query: string) => Promise<void>
  browse: (url: string) => Promise<void>
  toggleExpand: (url: string) => void
  saveToKnowledge: (url: string, title: string, content: string) => Promise<void>
  reset: () => void
}

export const useSearchPanelStore = create<SearchPanelState>((set, get) => ({
  manualQuery: '',
  manualResults: [],
  manualLoading: false,
  manualError: null,

  browseCache: {},
  browseLoading: {},
  browseError: {},

  expandedUrls: new Set(),

  search: async (query: string) => {
    if (!query.trim()) return
    set({ manualQuery: query, manualLoading: true, manualError: null })
    try {
      const res = await fetch('/api/search', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ query, engines: 'ddg,wiki', limit: 10 }),
      })
      if (!res.ok) throw new Error(`Search failed: ${res.status}`)
      const data: SearchResponse = await res.json()
      set({ manualResults: data.results, manualLoading: false })
    } catch (e) {
      set({ manualError: (e as Error).message, manualLoading: false })
    }
  },

  browse: async (url: string) => {
    const cached = get().browseCache[url]
    if (cached) return // already loaded

    set((s) => ({
      browseLoading: { ...s.browseLoading, [url]: true },
      browseError: { ...s.browseError, [url]: null },
    }))

    try {
      const res = await fetch('/api/browse', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ url, format: 'markdown' }),
      })
      if (!res.ok) throw new Error(`Browse failed: ${res.status}`)
      const data: BrowseResponse = await res.json()
      set((s) => ({
        browseCache: { ...s.browseCache, [url]: data },
        browseLoading: { ...s.browseLoading, [url]: false },
      }))
    } catch (e) {
      set((s) => ({
        browseError: { ...s.browseError, [url]: (e as Error).message },
        browseLoading: { ...s.browseLoading, [url]: false },
      }))
    }
  },

  toggleExpand: (url: string) => {
    set((s) => {
      const next = new Set(s.expandedUrls)
      if (next.has(url)) next.delete(url)
      else next.add(url)
      return { expandedUrls: next }
    })
  },

  saveToKnowledge: async (url: string, title: string, content: string) => {
    try {
      const path = `web-clippings/${title.replace(/[^a-zA-Z0-9가-힣]/g, '_').slice(0, 50)}.md`
      const body = `# ${title}\n\n> Source: ${url}\n\n${content}`
      await fetch('/api/knowledge/file', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path, content: body }),
      })
    } catch (e) {
      console.error('Failed to save to knowledge:', e)
    }
  },

  reset: () => {
    set({
      manualQuery: '',
      manualResults: [],
      manualLoading: false,
      manualError: null,
      expandedUrls: new Set(),
    })
  },
}))
```

- [ ] **Step 2: Commit**

```bash
git add web/src/stores/search-panel.ts
git commit -m "feat(web): add SearchPanel store for manual search and browse cache"
```

---

### Task 5: SearchView component

**Files:**
- Create: `web/src/components/portal/views/search-view.tsx`

**Interfaces:**
- Consumes: `PortalView.search` props (`query?`, `messageId?`)
- Consumes: `useSearchPanelStore` — manual results, browse cache, actions
- Consumes: `useChatStore` — agent-driven results (when `messageId` is set)

- [ ] **Step 1: Create `web/src/components/portal/views/search-view.tsx`**

```tsx
// SearchView — Search & Browse Panel (PortalPanel view).
//
// Two data flows:
//   1. Agent-driven: `messageId` prop → reads web_search/browse tool blocks
//      from the chat store's messages[messageId].
//   2. User-driven: search input → POST /api/search → results.
//
// Each result card is expandable — click "Read page" → POST /api/browse → markdown.

import { Globe, Search, Loader2, ChevronDown, ChevronRight, ExternalLink, BookmarkPlus } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { MarkdownMessage } from '@/components/chat/markdown-message'
import { Button } from '@/components/ui/button'
import { ScrollArea } from '@/components/ui/scroll-area'
import { useChatStore } from '@/stores/chat'
import { useSearchPanelStore, type SearchResultItem } from '@/stores/search-panel'
import { cn } from '@/lib/utils'

interface SearchViewProps {
  query?: string
  messageId?: string
}

function domain(url: string): string {
  try { return new URL(url).hostname.replace(/^www\./, '') }
  catch { return url }
}

function faviconUrl(url: string): string {
  try {
    const host = new URL(url).hostname
    return `https://www.google.com/s2/favicons?domain=${host}&sz=32`
  } catch { return '' }
}

export function SearchView({ query: propQuery, messageId }: SearchViewProps) {
  const { t } = useTranslation()
  const [input, setInput] = useState(propQuery ?? '')

  const { messages } = useChatStore()
  const {
    manualResults, manualLoading, manualError,
    browseCache, browseLoading, browseError,
    expandedUrls,
    search: doSearch, browse: doBrowse, toggleExpand, saveToKnowledge,
  } = useSearchPanelStore()

  // ── Agent-driven results (derived from chat store) ──
  const agentResults = useMemo<SearchResultItem[]>(() => {
    if (!messageId) return []
    const msg = messages.find((m) => m.id === messageId)
    if (!msg?.toolCalls) return []
    // Only extract from web_search results
    const searchCall = msg.toolCalls.find((tc) => tc.apiName === 'web_search')
    if (!searchCall?.result) return []

    const raw = searchCall.result
    // Try parsing JSON
    if (typeof raw === 'string') {
      try {
        const parsed = JSON.parse(raw)
        if (Array.isArray(parsed)) return parsed.slice(0, 10)
        if (parsed.results) return parsed.results.slice(0, 10)
      } catch {
        // String — try extracting URLs via regex
        const urlRe = /\[([^\]]+)\]\(([^)]+)\)/g
        const items: SearchResultItem[] = []
        let match
        while ((match = urlRe.exec(raw)) !== null) {
          items.push({ title: match[1], url: match[2], snippet: '', engine: '' })
        }
        return items.slice(0, 10)
      }
    }
    if (Array.isArray(raw)) return raw.slice(0, 10)
    return []
  }, [messageId, messages])

  // ── Results display (agent-driven OR manual) ──
  const results: SearchResultItem[] = messageId ? agentResults : manualResults
  const loading = messageId ? false : manualLoading
  const error = messageId ? null : manualError

  // ── Search handler ──
  const handleSearch = useCallback(() => {
    if (input.trim()) doSearch(input.trim())
  }, [input, doSearch])

  // ── Auto-search on mount if query prop is set ──
  useEffect(() => {
    if (propQuery && !messageId) {
      doSearch(propQuery)
    }
  }, [propQuery, messageId, doSearch])

  // ── Keyboard ──
  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === 'Enter') handleSearch()
  }, [handleSearch])

  return (
    <div className="flex flex-col h-full">
      {/* Search input */}
      <div className="p-3 border-b">
        <div className="flex items-center gap-2 rounded-lg border bg-background px-3 py-1.5">
          <Search className="w-4 h-4 text-muted-foreground shrink-0" />
          <input
            type="text"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={t('search.panel.placeholder', 'Search the web…')}
            className="flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground/60"
            disabled={!!messageId}
          />
          {loading && <Loader2 className="w-3.5 h-3.5 animate-spin text-muted-foreground" />}
        </div>
        {messageId && (
          <p className="mt-1 text-xs text-muted-foreground">
            {t('search.panel.agentResults', 'Agent search results')}
          </p>
        )}
      </div>

      {/* Results */}
      <ScrollArea className="flex-1">
        <div className="p-3 space-y-2">
          {/* Empty state */}
          {!loading && !error && results.length === 0 && (
            <div className="flex flex-col items-center justify-center py-12 text-center text-muted-foreground">
              <Globe className="w-8 h-8 mb-2 opacity-40" />
              <p className="text-sm">
                {t('search.panel.empty', 'Search the web or wait for agent results')}
              </p>
            </div>
          )}

          {/* Loading skeleton */}
          {loading && Array.from({ length: 3 }).map((_, i) => (
            <div key={i} className="animate-pulse rounded-lg border bg-muted/30 p-3 space-y-2">
              <div className="h-4 bg-muted-foreground/20 rounded w-3/4" />
              <div className="h-3 bg-muted-foreground/10 rounded w-1/2" />
              <div className="h-3 bg-muted-foreground/10 rounded w-full" />
            </div>
          ))}

          {/* Error */}
          {error && (
            <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
              {error}
              <Button variant="ghost" size="sm" className="ml-2 text-xs" onClick={() => doSearch(input)}>
                Retry
              </Button>
            </div>
          )}

          {/* Result cards */}
          {!loading && !error && results.length > 0 && results.map((item, idx) => {
            const expanded = expandedUrls.has(item.url)
            const cached = browseCache[item.url]
            const browsing = browseLoading[item.url]
            const browseErr = browseError[item.url]

            return (
              <div
                key={`${item.url}-${idx}`}
                className={cn(
                  'rounded-lg border transition-colors',
                  expanded ? 'border-border' : 'border-border/60 hover:border-border',
                )}
              >
                {/* Card header (always visible) */}
                <button
                  type="button"
                  onClick={() => toggleExpand(item.url)}
                  className="flex w-full items-start gap-2.5 p-3 text-left hover:bg-muted/30 transition-colors rounded-lg"
                >
                  {/* Chevron */}
                  <ChevronRight
                    className={cn(
                      'w-3.5 h-3.5 mt-1 shrink-0 text-muted-foreground transition-transform',
                      expanded && 'rotate-90',
                    )}
                  />
                  {/* Favicon */}
                  {faviconUrl(item.url) && (
                    <img src={faviconUrl(item.url)} alt="" className="w-4 h-4 mt-0.5 shrink-0 rounded" />
                  )}
                  {/* Content */}
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-medium truncate">{item.title || item.url}</p>
                    <p className="text-xs text-muted-foreground truncate">{domain(item.url)}</p>
                    {item.snippet && (
                      <p className="text-xs text-muted-foreground/70 mt-1 line-clamp-2">{item.snippet}</p>
                    )}
                  </div>
                  {/* Status badges */}
                  <div className="flex gap-1 shrink-0 mt-0.5">
                    {cached && (
                      <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-emerald-100 text-emerald-700 dark:bg-emerald-900/30 dark:text-emerald-400">
                        Browsed
                      </span>
                    )}
                  </div>
                </button>

                {/* Expanded body: browse content */}
                {expanded && (
                  <div className="border-t border-border/60 px-3 pb-3 pt-2">
                    {/* Trigger browse if not cached */}
                    {!cached && !browsing && !browseErr && (
                      <Button
                        variant="outline"
                        size="sm"
                        className="w-full text-xs"
                        onClick={() => doBrowse(item.url)}
                      >
                        Read page
                      </Button>
                    )}

                    {/* Browsing skeleton */}
                    {browsing && (
                      <div className="space-y-2 animate-pulse">
                        <div className="h-3 bg-muted-foreground/10 rounded w-3/4" />
                        <div className="h-3 bg-muted-foreground/10 rounded w-full" />
                        <div className="h-3 bg-muted-foreground/10 rounded w-5/6" />
                      </div>
                    )}

                    {/* Browse error */}
                    {browseErr && (
                      <div className="text-xs text-destructive">
                        {browseErr}
                        <Button variant="ghost" size="sm" className="ml-2 text-xs" onClick={() => doBrowse(item.url)}>
                          Retry
                        </Button>
                      </div>
                    )}

                    {/* Browse content */}
                    {cached && (
                      <div className="space-y-2">
                        <div className="max-h-80 overflow-y-auto rounded bg-muted/40 p-2">
                          <MarkdownMessage messageId="" isStreaming={false}>
                            {cached.markdown}
                          </MarkdownMessage>
                        </div>

                        {/* Actions footer */}
                        <div className="flex justify-end gap-1">
                          <Button
                            variant="ghost"
                            size="sm"
                            className="text-xs h-6 px-2"
                            onClick={() => window.open(item.url, '_blank')}
                          >
                            <ExternalLink className="w-3 h-3 mr-1" />
                            Open
                          </Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            className="text-xs h-6 px-2"
                            onClick={() => saveToKnowledge(item.url, cached.title, cached.markdown)}
                          >
                            <BookmarkPlus className="w-3 h-3 mr-1" />
                            Save
                          </Button>
                        </div>
                      </div>
                    )}
                  </div>
                )}
              </div>
            )
          })}

          {/* Status bar */}
          {!loading && results.length > 0 && (
            <p className="text-2xs text-muted-foreground/60 text-center pt-2">
              {results.length} {t('search.panel.results', 'results')}
            </p>
          )}
        </div>
      </ScrollArea>
    </div>
  )
}
```

- [ ] **Step 2: Verify SearchView builds**

```bash
cd web && bun x tsc --noEmit 2>&1 | grep -i search-view | head -5
```
Expected: no errors. If MarkdownMessage type issues arise (empty `messageId`), check its props interface.

- [ ] **Step 3: Commit**

```bash
git add web/src/components/portal/views/search-view.tsx
git commit -m "feat(web): add SearchView component for Search & Browse Panel"
```

---

### Task 6: Chat auto-open + WebSearch "Open in Panel"

**Files:**
- Modify: `web/src/stores/chat.ts`
- Modify: `web/src/components/chat/tool-renders/WebSearch.tsx`

**Interfaces:**
- Produces: Auto-opens PortalPanel SearchView when agent calls web_search/browse
- Produces: "Open in Panel" action on WebSearch render

- [ ] **Step 1: Add auto-open in chat store**

In `web/src/stores/chat.ts`, find the `handleChunk` function or the section that processes tool events.

Import `usePortalStore`:
```typescript
import { usePortalStore } from '@/stores/portal'
```

Inside the tool event handler (where `chunk.type === 'tool.result'` or similar), add logic:
```typescript
// Auto-open search panel on web_search/browse tool calls
if (
  chunk.tool_name === 'web_search' ||
  chunk.tool_name === 'browse'
) {
  const portalState = usePortalStore.getState()
  const stack = portalState.stack
  const top = stack[stack.length - 1]
  // Only auto-open if panel isn't already showing search
  if (!top || top.type !== 'search') {
    // Find the current assistant message ID
    const msgs = get().messages
    const assistantMsg = [...msgs].reverse().find(m => m.role === 'assistant')
    if (assistantMsg) {
      portalState.pushView({
        type: 'search',
        messageId: assistantMsg.id,
      })
    }
  }
}
```

Find where to insert this. Look for `chunk.type === 'tool.'` patterns in chat.ts (around the `handleChunk` implementation, lines ~860-980).

- [ ] **Step 2: Add "Open in Panel" to WebSearchRender**

In `web/src/components/chat/tool-renders/WebSearch.tsx`, add the Search button import and logic:

Add imports:
```tsx
import { Button } from '@/components/ui/button'
import { usePortalStore } from '@/stores/portal'
```

In the parsed results display area (after the snippet), add a button:
```tsx
<Button
  variant="ghost"
  size="sm"
  className="text-xs h-6 px-2"
  onClick={() => usePortalStore.getState().pushView({
    type: 'search',
    messageId: messageId, // Need to receive this as prop
  })}
>
  <Search className="w-3 h-3 mr-1" />
  Panel
</Button>
```

Note: `ToolRenderComponent` doesn't receive `messageId` by default. Check if `ToolRenderProps` has it — if not, use the data attribute from the parent message context, or pass a generic search view. Fallback: just `pushView({ type: 'search' })` without `messageId`.

- [ ] **Step 3: Run frontend tests**

```bash
cd web && bun run test 2>&1 | tail -10
```
Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add web/src/stores/chat.ts web/src/components/chat/tool-renders/WebSearch.tsx
git commit -m "feat(web): auto-open search panel on web_search/browse tool calls"
```

---

### Task 7: i18n translation keys

**Files:**
- Modify: `web/src/i18n/locales/en.json`
- Modify: `web/src/i18n/locales/ko.json`

- [ ] **Step 1: Add keys to en.json**

```json
"search": {
  "panel": {
    "placeholder": "Search the web…",
    "agentResults": "Agent search results",
    "empty": "Search the web or wait for agent results",
    "results": "results"
  }
}
```

- [ ] **Step 2: Add keys to ko.json**

```json
"search": {
  "panel": {
    "placeholder": "웹 검색…",
    "agentResults": "에이전트 검색 결과",
    "empty": "웹을 검색하거나 에이전트 결과를 기다리세요",
    "results": "개 결과"
  }
}
```

- [ ] **Step 3: Commit**

```bash
git add web/src/i18n/locales/en.json web/src/i18n/locales/ko.json
git commit -m "feat(web): add search panel i18n keys"
```

---

### Task 8: Manual search button in chat header

**Files:**
- Modify: `web/src/routes/chat.tsx`

**Interfaces:**
- Produces: 🔍 button in chat header → opens PortalPanel SearchView

- [ ] **Step 1: Find the chat header in `chat.tsx`**

Look for the header/controls section near the top of the ChatPage component (around line 32-50).

- [ ] **Step 2: Add 🔍 search button**

```tsx
import { Search } from 'lucide-react'
import { usePortalStore } from '@/stores/portal'

// In the header controls section, add:
<Button
  variant="ghost"
  size="icon"
  className="h-8 w-8"
  onClick={() => usePortalStore.getState().pushView({ type: 'search' })}
>
  <Search className="w-4 h-4" />
</Button>
```

- [ ] **Step 3: Verify build**

```bash
cd web && bun x tsc --noEmit 2>&1 | head -5
```

- [ ] **Step 4: Commit**

```bash
git add web/src/routes/chat.tsx
git commit -m "feat(web): add search panel open button to chat header"
```

---

### Task 9: Integration test & verify

**Files:** None (verification only)

- [ ] **Step 1: Run backend compilation check**

```bash
cargo build -p oxios 2>&1 | tail -10
```
Expected: `Finished` with no errors.

- [ ] **Step 2: Run frontend test + typecheck**

```bash
cd web && bun run test 2>&1 | tail -10
cd web && bun x tsc --noEmit 2>&1 | head -20
```
Expected: all tests pass, no type errors.

- [ ] **Step 3: Run kernel tests**

```bash
cargo test -p oxios-kernel --lib --features native-browser --no-fail-fast 2>&1 | tail -5
```
Expected: all tests pass.

- [ ] **Step 4: Verify final git status**

```bash
git status --porcelain=v1 --untracked-files=all
```
Expected: clean (all changes committed).
