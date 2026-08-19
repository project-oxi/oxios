// KnowledgeBrowser — read-only knowledge note search + preview (PortalPanel view).
//
// Used by:
//   1. SearchView Knowledge tab (no initialPath)
//   2. PortalPanel KnowledgeView (with initialPath)
//
// Stateful: manages its own search + file loading. No dep on SearchPanel store.

import { useNavigate } from '@tanstack/react-router'
import { ArrowLeft, Book, ExternalLink, FileText, Loader2 } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { MarkdownMessage } from '@/components/chat/markdown-message'
import { ScrollArea } from '@/components/ui/scroll-area'
import { api } from '@/lib/api-client'
import type { KnowledgeSearchHit } from '@/types/knowledge'

/** Encode a knowledge path for safe URL interpolation (preserves '/' separators). */
function encodeFilePath(path: string): string {
  return path
    .split('/')
    .map((seg) => encodeURIComponent(seg))
    .join('/')
}

interface KnowledgeBrowserProps {
  /** Initial file path to load immediately (optional). */
  initialPath?: string
}

interface KnowledgeFileResponse {
  path: string
  content: string
}

export function KnowledgeBrowser({ initialPath }: KnowledgeBrowserProps) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const [query, setQuery] = useState('')
  const [results, setResults] = useState<KnowledgeSearchHit[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [selectedPath, setSelectedPath] = useState<string | null>(null)
  const [selectedContent, setSelectedContent] = useState<string | null>(null)
  const [contentLoading, setContentLoading] = useState(false)
  const [contentError, setContentError] = useState<string | null>(null)

  // ── Knowledge search ──────────────────────────────────────────
  const doSearch = useCallback(async (q: string) => {
    if (!q.trim()) {
      setResults([])
      return
    }
    setLoading(true)
    setError(null)
    try {
      const res = await api.post<{ results: KnowledgeSearchHit[]; count: number }>(
        '/api/knowledge/search',
        { query: q, limit: 50 },
      )
      setResults(res.results)
    } catch (e) {
      setError((e as Error).message)
    } finally {
      setLoading(false)
    }
  }, [])

  // ── File loading ──────────────────────────────────────────────
  const selectFile = useCallback(async (path: string) => {
    setSelectedPath(path)
    setContentLoading(true)
    setContentError(null)
    setSelectedContent(null)
    try {
      const res = await api.get<KnowledgeFileResponse>(
        `/api/knowledge/file/${encodeFilePath(path)}`,
      )
      setSelectedContent(res.content)
    } catch (e) {
      setContentError((e as Error).message)
    } finally {
      setContentLoading(false)
    }
  }, [])

  // ── Navigate to full editor ───────────────────────────────────
  const openInEditor = useCallback(
    (path: string) => {
      // Navigate to knowledge page; the file will be visible from there
      navigate({ to: '/brain/knowledge' })
      // Set the knowledge store's current file path for direct open
      import('@/stores/knowledge').then(({ useKnowledgeStore }) => {
        useKnowledgeStore.getState().openFile(path)
      })
    },
    [navigate],
  )

  // ── Auto-load initialPath ─────────────────────────────────────
  useEffect(() => {
    if (initialPath) selectFile(initialPath)
  }, [initialPath, selectFile])

  // ── Debounced search ──────────────────────────────────────────
  useEffect(() => {
    const timer = setTimeout(() => doSearch(query), 300)
    return () => clearTimeout(timer)
  }, [query, doSearch])

  return (
    <div className="flex flex-col h-full">
      {/* ── Header: file detail or search bar ── */}
      <div className="p-3 border-b">
        {selectedPath ? (
          <div className="flex items-center gap-2">
            <button
              type="button"
              className="text-muted-foreground hover:text-primary transition-colors shrink-0"
              onClick={() => {
                setSelectedPath(null)
                setSelectedContent(null)
              }}
            >
              <ArrowLeft className="w-4 h-4" />
            </button>
            <span className="text-sm font-medium truncate flex-1">
              {selectedPath.split('/').pop()?.replace(/\.md$/, '')}
            </span>
            <button
              type="button"
              className="text-xs text-primary hover:underline shrink-0 flex items-center gap-1"
              onClick={() => openInEditor(selectedPath)}
            >
              <ExternalLink className="w-3 h-3" />
              {t('search.panel.openInKnowledge', 'Open in Knowledge')}
            </button>
          </div>
        ) : (
          <div className="flex items-center gap-2 rounded-lg border bg-background px-3 py-1.5">
            <Book className="w-4 h-4 text-muted-foreground shrink-0" />
            <input
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t('search.panel.knowledgePlaceholder', 'Search your knowledge base…')}
              className="flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground/60"
            />
            {loading && (
              <Loader2 className="w-3.5 h-3.5 animate-spin text-muted-foreground shrink-0" />
            )}
          </div>
        )}
      </div>

      {/* ── Content area ── */}
      <ScrollArea className="flex-1">
        <div className="p-3 space-y-1">
          {/* Empty (no query) */}
          {!selectedPath && !loading && !error && results.length === 0 && !query && (
            <div className="flex flex-col items-center justify-center py-12 text-center text-muted-foreground">
              <Book className="w-8 h-8 mb-2 opacity-40" />
              <p className="text-sm">
                {t('search.panel.knowledgeEmpty', 'Search your knowledge base')}
              </p>
            </div>
          )}

          {/* Empty (no results for query) */}
          {!selectedPath && !loading && !error && results.length === 0 && query && (
            <div className="flex flex-col items-center justify-center py-12 text-center text-muted-foreground">
              <FileText className="w-8 h-8 mb-2 opacity-40" />
              <p className="text-sm">
                {t('search.panel.knowledgeNoResults', "No notes matching '{{query}}'", {
                  query,
                })}
              </p>
            </div>
          )}

          {/* Loading skeleton */}
          {!selectedPath &&
            loading &&
            Array.from({ length: 3 }).map((_, i) => (
              <div key={i} className="animate-pulse rounded-lg border bg-muted/30 p-3 space-y-2">
                <div className="h-4 bg-muted-foreground/20 rounded w-3/4" />
                <div className="h-3 bg-muted-foreground/10 rounded w-1/2" />
              </div>
            ))}

          {/* Error */}
          {!selectedPath && error && (
            <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
              {error}
              <button
                type="button"
                className="ml-2 text-xs underline"
                onClick={() => doSearch(query)}
              >
                Retry
              </button>
            </div>
          )}

          {/* Search results list */}
          {!selectedPath &&
            !loading &&
            !error &&
            results.map((hit) => (
              <button
                key={hit.path}
                type="button"
                className="w-full text-left rounded-lg border border-border/60 p-3 hover:bg-muted/30 transition-colors"
                onClick={() => selectFile(hit.path)}
              >
                <p className="text-sm font-medium truncate">{hit.name.replace(/\.md$/, '')}</p>
                {hit.snippet && (
                  <p className="text-xs text-muted-foreground/70 mt-0.5 line-clamp-2">
                    {hit.snippet}
                  </p>
                )}
                <p className="text-[10px] text-muted-foreground/50 mt-0.5 truncate font-mono">
                  {hit.path}
                </p>
              </button>
            ))}

          {/* File content loading */}
          {selectedPath && contentLoading && (
            <div className="space-y-2 animate-pulse p-3">
              <div className="h-4 bg-muted-foreground/10 rounded w-1/2" />
              <div className="h-3 bg-muted-foreground/10 rounded w-full" />
              <div className="h-3 bg-muted-foreground/10 rounded w-3/4" />
            </div>
          )}

          {/* File content error */}
          {selectedPath && contentError && (
            <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
              {contentError}
              <button
                type="button"
                className="ml-2 text-xs underline"
                onClick={() => selectFile(selectedPath)}
              >
                Retry
              </button>
            </div>
          )}

          {/* File content */}
          {selectedPath && selectedContent && !contentLoading && (
            <div className="max-h-[70vh] overflow-y-auto">
              <MarkdownMessage messageId="" isStreaming={false}>
                {selectedContent}
              </MarkdownMessage>
            </div>
          )}
        </div>
      </ScrollArea>
    </div>
  )
}
