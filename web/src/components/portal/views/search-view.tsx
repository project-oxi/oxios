// SearchView — Search & Browse Panel (PortalPanel view).
//
// Two data flows:
//   1. Agent-driven: `messageId` prop → reads web_search/browse tool blocks
//      from the chat store's messages[messageId].
//   2. User-driven: search input → POST /api/search → results.
//
// Each result card is expandable — click "Read page" → POST /api/browse
// → markdown content via MarkdownMessage.

import { BookmarkPlus, ChevronRight, ExternalLink, Globe, Loader2, Search } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { MarkdownMessage } from '@/components/chat/markdown-message'
import { ScrollArea } from '@/components/ui/scroll-area'
import { cn } from '@/lib/utils'
import { useChatStore } from '@/stores/chat'
import { type SearchResultItem, useSearchPanelStore } from '@/stores/search-panel'
import { KnowledgeBrowser } from './knowledge-browser'

interface SearchViewProps {
  query?: string
  messageId?: string
}

function domain(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '')
  } catch {
    return url
  }
}

function faviconUrl(url: string): string {
  try {
    const host = new URL(url).hostname
    return `https://www.google.com/s2/favicons?domain=${host}&sz=32`
  } catch {
    return ''
  }
}

export function SearchView({ query: propQuery, messageId }: SearchViewProps) {
  const { t } = useTranslation()
  const [input, setInput] = useState(propQuery ?? '')

  const activeTab = useSearchPanelStore((s) => s.activeTab)
  const setActiveTab = useSearchPanelStore((s) => s.setActiveTab)

  const { messages } = useChatStore()
  const {
    manualResults,
    manualLoading,
    manualError,
    browseCache,
    browseLoading,
    browseError,
    expandedUrls,
    search: doSearch,
    browse: doBrowse,
    toggleExpand,
    saveToKnowledge,
  } = useSearchPanelStore()

  // ── Agent-driven results (derived from chat store) ──
  const agentResults = useMemo<SearchResultItem[]>(() => {
    if (!messageId) return []
    const msg = messages.find((m) => m.id === messageId)
    if (!msg?.metadata?.tool_calls?.length) return []

    // Only extract from web_search results
    const searchCall = msg.metadata.tool_calls.find(
      (tc) => tc.tool_name === 'web_search' || tc.tool === 'web_search',
    )
    if (!searchCall) return []

    const raw: unknown = (() => {
      try {
        return JSON.parse(searchCall.output) as unknown
      } catch {
        return searchCall.output
      }
    })()

    if (Array.isArray(raw) || (raw && typeof raw === 'object')) {
      // Already an array — use it directly
      if (Array.isArray(raw)) {
        return (raw as SearchResultItem[]).slice(0, 10)
      }
      // Object with a results array
      const obj = raw as Record<string, unknown>
      if (Array.isArray(obj.results)) {
        return obj.results.slice(0, 10) as SearchResultItem[]
      }
      // Not parseable to results — fall through to empty
      return []
    }

    // JSON.parse failed or raw is a string — try GroundingSearch citations
    const search = msg.search
    if (search?.citations?.length) {
      return search.citations.map(
        (c): SearchResultItem => ({
          title: c.title ?? '',
          url: c.url,
          snippet: '',
          engine: '',
        }),
      )
    }

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
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Enter') handleSearch()
    },
    [handleSearch],
  )

  return (
    <div className="flex flex-col h-full">
      {/* ── Tab bar ── */}
      <div className="flex border-b shrink-0">
        <button
          type="button"
          className={cn(
            'flex-1 text-xs font-medium py-2 text-center transition-colors',
            activeTab === 'web'
              ? 'text-foreground border-b-2 border-primary'
              : 'text-muted-foreground hover:text-foreground',
          )}
          onClick={() => setActiveTab('web')}
        >
          {t('search.panel.webTab', 'Web')}
        </button>
        <button
          type="button"
          className={cn(
            'flex-1 text-xs font-medium py-2 text-center transition-colors',
            activeTab === 'knowledge'
              ? 'text-foreground border-b-2 border-primary'
              : 'text-muted-foreground hover:text-foreground',
          )}
          onClick={() => setActiveTab('knowledge')}
        >
          {t('search.panel.knowledgeTab', 'Knowledge')}
        </button>
      </div>

      {activeTab === 'web' ? (
        <>
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
              {loading &&
                Array.from({ length: 3 }).map((_, i) => (
                  <div
                    key={i}
                    className="animate-pulse rounded-lg border bg-muted/30 p-3 space-y-2"
                  >
                    <div className="h-4 bg-muted-foreground/20 rounded w-3/4" />
                    <div className="h-3 bg-muted-foreground/10 rounded w-1/2" />
                    <div className="h-3 bg-muted-foreground/10 rounded w-full" />
                  </div>
                ))}

              {/* Error */}
              {error && (
                <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
                  {error}
                  <button
                    type="button"
                    className="ml-2 text-xs underline hover:no-underline"
                    onClick={() => doSearch(input)}
                  >
                    Retry
                  </button>
                </div>
              )}

              {/* Result cards */}
              {!loading &&
                !error &&
                results.length > 0 &&
                results.map((item, idx) => {
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
                      {/* Card header */}
                      <button
                        type="button"
                        onClick={() => toggleExpand(item.url)}
                        className="flex w-full items-start gap-2.5 p-3 text-left hover:bg-muted/30 transition-colors rounded-lg"
                      >
                        <ChevronRight
                          className={cn(
                            'w-3.5 h-3.5 mt-1 shrink-0 text-muted-foreground transition-transform',
                            expanded && 'rotate-90',
                          )}
                        />
                        {faviconUrl(item.url) && (
                          <img
                            src={faviconUrl(item.url)}
                            alt=""
                            className="w-4 h-4 mt-0.5 shrink-0 rounded"
                          />
                        )}
                        <div className="min-w-0 flex-1">
                          <p className="text-sm font-medium truncate">{item.title || item.url}</p>
                          <p className="text-xs text-muted-foreground truncate">
                            {domain(item.url)}
                          </p>
                          {item.snippet && (
                            <p className="text-xs text-muted-foreground/70 mt-1 line-clamp-2">
                              {item.snippet}
                            </p>
                          )}
                        </div>
                        <div className="flex gap-1 shrink-0 mt-0.5">
                          {cached && (
                            <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-status-success-subtle text-status-success-on-subtle">
                              Browsed
                            </span>
                          )}
                        </div>
                      </button>

                      {/* Expanded body */}
                      {expanded && (
                        <div className="border-t border-border/60 px-3 pb-3 pt-2 space-y-2">
                          {/* Screenshot preview (full-width, lazy) */}
                          <img
                            src={`/api/screenshot?url=${encodeURIComponent(item.url)}&w=800&h=600`}
                            alt=""
                            loading="lazy"
                            className="w-full max-h-64 rounded border border-border/40 object-cover object-top bg-muted/30"
                            onError={(e) => {
                              ;(e.target as HTMLImageElement).style.display = 'none'
                            }}
                          />
                          {!cached && !browsing && !browseErr && (
                            <button
                              type="button"
                              className="w-full rounded border border-border bg-muted/30 px-3 py-1.5 text-xs text-muted-foreground hover:bg-muted/50 transition-colors"
                              onClick={() => doBrowse(item.url)}
                            >
                              Read page
                            </button>
                          )}
                          {browsing && (
                            <div className="space-y-2 animate-pulse">
                              <div className="h-3 bg-muted-foreground/10 rounded w-3/4" />
                              <div className="h-3 bg-muted-foreground/10 rounded w-full" />
                              <div className="h-3 bg-muted-foreground/10 rounded w-5/6" />
                            </div>
                          )}
                          {browseErr && (
                            <div className="text-xs text-destructive">
                              {browseErr}
                              <button
                                type="button"
                                className="ml-2 underline hover:no-underline"
                                onClick={() => doBrowse(item.url)}
                              >
                                Retry
                              </button>
                            </div>
                          )}
                          {cached && (
                            <>
                              <div className="max-h-80 overflow-y-auto rounded bg-muted/40 p-2">
                                <MarkdownMessage messageId="" isStreaming={false}>
                                  {cached.markdown}
                                </MarkdownMessage>
                              </div>
                              <div className="flex justify-end gap-1">
                                <button
                                  type="button"
                                  className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-primary transition-colors px-2 py-1"
                                  onClick={() => window.open(item.url, '_blank')}
                                >
                                  <ExternalLink className="w-3 h-3" />
                                  Open
                                </button>
                                <button
                                  type="button"
                                  className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-primary transition-colors px-2 py-1"
                                  onClick={() =>
                                    saveToKnowledge(item.url, cached.title, cached.markdown)
                                  }
                                >
                                  <BookmarkPlus className="w-3 h-3" />
                                  Save
                                </button>
                              </div>
                            </>
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
        </>
      ) : (
        <KnowledgeBrowser />
      )}
    </div>
  )
}
