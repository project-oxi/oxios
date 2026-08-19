import { useRouterState } from '@tanstack/react-router'
import { ExternalLink, FileText, Folder, Globe, Loader2, Search } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useKnowledgeSearch, useKnowledgeTree } from '@/hooks/use-knowledge'
import { cn } from '@/lib/utils'
import { useKnowledgeStore } from '@/stores/knowledge'
import { usePortalStore } from '@/stores/portal'
import type { KnowledgeSearchHit, KnowledgeTreeEntry } from '@/types/knowledge'

interface SearchModalProps {
  /** When opened externally (e.g. from chat message action) */
  forceOpen?: boolean
  /** When set, modal acts as a file picker for moving a chat message */
  selectedMessageText?: string | null
  /** Called in moveMessage mode when a file is selected */
  onMoveToFile?: (path: string) => void
  /** Called in moveMessage mode when a directory is selected */
  onMoveToDir?: (dir: string) => void
  /** Called when the modal closes */
  onClose?: () => void
}

/** A single item that can appear in the results list (search hit, file, or directory). */
interface ResultItem {
  path: string
  name: string
  isDir: boolean
}

const MAX_RECENT_FILES = 15

export function SearchModal({
  forceOpen,
  selectedMessageText,
  onMoveToFile,
  onMoveToDir,
  onClose,
}: SearchModalProps) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [focusedIndex, setFocusedIndex] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLUListElement>(null)

  const openFile = useKnowledgeStore((s) => s.openFile)
  const searchMutation = useKnowledgeSearch()
  const { data: treeEntries } = useKnowledgeTree()

  const isMoveMode = selectedMessageText != null
  const [searchResults, setSearchResults] = useState<KnowledgeSearchHit[]>([])

  // ── Web tab state ────────────────────────────────────────────
  const [searchTab, setSearchTab] = useState<'files' | 'web'>('files')
  const [webQuery, setWebQuery] = useState('')
  const [webResults, setWebResults] = useState<{ title: string; url: string; snippet: string }[]>(
    [],
  )
  const [webLoading, setWebLoading] = useState(false)

  // ── Build the display list ──────────────────────────────────
  const recentFiles: ResultItem[] = (treeEntries ?? [])
    .filter((e: KnowledgeTreeEntry) => !e.is_dir)
    .slice(0, MAX_RECENT_FILES)
    .map((e: KnowledgeTreeEntry) => ({ path: e.name, name: e.name, isDir: false }))

  const treeDirs: ResultItem[] = (treeEntries ?? [])
    .filter((e: KnowledgeTreeEntry) => e.is_dir)
    .map((e: KnowledgeTreeEntry) => ({ path: e.name, name: e.name, isDir: true }))

  const displayItems: ResultItem[] = (() => {
    if (query.trim() && searchResults.length > 0) {
      return searchResults.map((h: KnowledgeSearchHit) => ({
        path: h.path,
        name: h.name,
        isDir: false,
      }))
    }
    // In move mode, show directories first then files
    if (isMoveMode) {
      return [...treeDirs, ...recentFiles]
    }
    // Normal empty state: just show recent files
    return recentFiles
  })()

  // ── Global ⌘K / ⌘P listener (M5: pathname via ref) ───────────
  const router = useRouterState()
  const pathnameRef = useRef(router.location.pathname)
  pathnameRef.current = router.location.pathname

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!pathnameRef.current.startsWith('/brain/knowledge')) return
      if ((e.metaKey || e.ctrlKey) && e.key === 'p') {
        e.preventDefault()
        e.stopPropagation()
        setOpen(true)
        setQuery('')
        setFocusedIndex(0)
        setSearchResults([])
      }
    }
    window.addEventListener('keydown', handler, true)
    return () => window.removeEventListener('keydown', handler, true)
  }, [])

  // ── Focus input when opening ────────────────────────────────
  useEffect(() => {
    if (open) {
      // Small delay so the DOM mounts before focusing
      const id = requestAnimationFrame(() => inputRef.current?.focus())
      return () => cancelAnimationFrame(id)
    }
  }, [open])

  // ── Respond to forceOpen prop ───────────────────────────────
  useEffect(() => {
    if (forceOpen) {
      setOpen(true)
      setQuery('')
      setFocusedIndex(0)
      setSearchResults([])
    }
  }, [forceOpen])

  // ── Auto-search on query change (debounced) ─────────────────
  useEffect(() => {
    if (!open) return
    if (!query.trim()) {
      setSearchResults([])
      setFocusedIndex(0)
      return
    }
    const timer = setTimeout(() => {
      searchMutation.mutate(
        { query, limit: 20 },
        {
          onSuccess: (data) => {
            setSearchResults(data.results)
            setFocusedIndex(0)
          },
          onError: () => {
            setSearchResults([])
          },
        },
      )
    }, 150)
    return () => clearTimeout(timer)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, open, searchMutation.mutate])

  // ── Web search on web query change (debounced) ──────────────
  useEffect(() => {
    if (!open || searchTab !== 'web') return
    if (!webQuery.trim()) {
      setWebResults([])
      return
    }
    const timer = setTimeout(async () => {
      setWebLoading(true)
      try {
        const res = await fetch('/api/search', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ query: webQuery, engines: 'ddg,wiki', limit: 10 }),
        })
        if (!res.ok) throw new Error(`Search failed: ${res.status}`)
        const data = await res.json()
        setWebResults(data.results)
      } catch {
        setWebResults([])
      } finally {
        setWebLoading(false)
      }
    }, 300)
    return () => clearTimeout(timer)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [webQuery, open, searchTab])

  // ── Close handler ───────────────────────────────────────────
  const close = useCallback(() => {
    setOpen(false)
    onClose?.()
  }, [onClose])

  // ── Select handler ──────────────────────────────────────────
  const handleSelect = useCallback(
    (item: ResultItem) => {
      if (isMoveMode) {
        if (item.isDir && onMoveToDir) {
          onMoveToDir(item.path)
        } else if (!item.isDir && onMoveToFile) {
          onMoveToFile(item.path)
        }
      } else {
        openFile(item.path)
      }
      close()
    },
    [isMoveMode, onMoveToDir, onMoveToFile, openFile, close],
  )

  // ── Keyboard navigation inside modal ────────────────────────
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      const count = Math.max(displayItems.length, 1)

      if (e.key === 'Escape') {
        e.preventDefault()
        close()
        return
      }

      if (e.key === 'ArrowDown') {
        e.preventDefault()
        setFocusedIndex((i) => (i + 1) % count)
        return
      }

      if (e.key === 'ArrowUp') {
        e.preventDefault()
        setFocusedIndex((i) => (i - 1 + count) % count)
        return
      }

      if (e.key === 'Enter') {
        e.preventDefault()
        const item = displayItems[focusedIndex]
        if (item) {
          handleSelect(item)
        }
        return
      }
    },
    [displayItems, focusedIndex, close, handleSelect],
  )

  // ── Scroll focused item into view ───────────────────────────
  useEffect(() => {
    if (!listRef.current) return
    const focusedEl = listRef.current.children[focusedIndex] as HTMLElement | undefined
    focusedEl?.scrollIntoView({ block: 'nearest' })
  }, [focusedIndex])

  // ── Early return when closed ────────────────────────────────
  if (!open) return null

  const hasQuery = query.trim().length > 0
  const isSearching = searchMutation.isPending && hasQuery

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center pt-[20vh]">
      {/* Backdrop */}
      <div className="fixed inset-0 bg-black/50" onClick={close} />

      {/* Dialog */}
      <div className="relative w-full max-w-lg bg-background border rounded-lg shadow-lg overflow-hidden">
        {/* Search input row */}
        <div className="flex items-center gap-2 px-3 py-2.5 border-b">
          {searchTab === 'web' ? (
            <Globe className="h-4 w-4 text-muted-foreground shrink-0" />
          ) : (
            <Search className="h-4 w-4 text-muted-foreground shrink-0" />
          )}
          <input
            ref={searchTab === 'files' ? inputRef : undefined}
            value={searchTab === 'web' ? webQuery : query}
            onChange={(e) =>
              searchTab === 'web' ? setWebQuery(e.target.value) : setQuery(e.target.value)
            }
            onKeyDown={searchTab === 'files' ? handleKeyDown : undefined}
            placeholder={
              searchTab === 'web'
                ? 'Search the web…'
                : isMoveMode
                  ? t('knowledge.searchOrSelectDestination')
                  : t('knowledge.searchFiles')
            }
            className="flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
          />
          {webLoading && searchTab === 'web' && (
            <Loader2 className="h-4 w-4 animate-spin text-muted-foreground shrink-0" />
          )}
          <kbd className="text-2xs text-muted-foreground border rounded px-1.5 py-0.5 font-mono">
            ESC
          </kbd>
        </div>

        {/* Tab bar */}
        {!isMoveMode && (
          <div className="flex border-b">
            <button
              type="button"
              className={cn(
                'flex-1 text-xs font-medium py-1.5 text-center transition-colors',
                searchTab === 'files'
                  ? 'text-foreground border-b-2 border-primary'
                  : 'text-muted-foreground hover:text-foreground',
              )}
              onClick={() => setSearchTab('files')}
            >
              <FileText className="w-3 h-3 inline mr-1" />
              Files
            </button>
            <button
              type="button"
              className={cn(
                'flex-1 text-xs font-medium py-1.5 text-center transition-colors',
                searchTab === 'web'
                  ? 'text-foreground border-b-2 border-primary'
                  : 'text-muted-foreground hover:text-foreground',
              )}
              onClick={() => setSearchTab('web')}
            >
              <Globe className="w-3 h-3 inline mr-1" />
              Web
            </button>
          </div>
        )}

        {/* Results */}
        {searchTab === 'files' ? (
          <ul ref={listRef} className="max-h-80 overflow-y-auto p-1">
            {displayItems.length > 0 ? (
              displayItems.map((item, i) => (
                <li
                  key={item.path + (item.isDir ? '-dir' : '-file')}
                  className={cn(
                    'flex items-center gap-2.5 px-3 py-2 text-sm cursor-pointer select-none rounded-md transition-colors',
                    i === focusedIndex ? 'bg-accent text-accent-foreground' : 'hover:bg-accent/50',
                  )}
                  onClick={() => handleSelect(item)}
                  onMouseEnter={() => setFocusedIndex(i)}
                >
                  {item.isDir ? (
                    <Folder className="h-4 w-4 shrink-0 text-muted-foreground" />
                  ) : (
                    <FileText className="h-4 w-4 shrink-0 text-muted-foreground" />
                  )}
                  <span className="font-medium truncate">{item.name.replace(/\.md$/, '')}</span>
                  {item.path.includes('/') && (
                    <span className="ml-auto text-xs text-muted-foreground shrink-0">
                      {item.path.replace(/\/[^/]+$/, '')}
                    </span>
                  )}
                  {item.isDir && (
                    <span className="ml-auto text-xs text-muted-foreground shrink-0">dir</span>
                  )}
                </li>
              ))
            ) : hasQuery ? (
              <li className="px-4 py-6 text-sm text-muted-foreground text-center">
                {isSearching ? t('knowledge.searching') : t('knowledge.noSearchResults')}
              </li>
            ) : (
              <li className="px-4 py-6 text-sm text-muted-foreground text-center">
                {isMoveMode ? 'Select a file or directory' : 'Type to search files'}
              </li>
            )}
          </ul>
        ) : (
          <div className="max-h-80 overflow-y-auto p-1">
            {webLoading && webQuery.trim() && (
              <div className="space-y-2 p-3">
                {Array.from({ length: 3 }).map((_, i) => (
                  <div key={i} className="animate-pulse h-12 bg-muted/30 rounded" />
                ))}
              </div>
            )}
            {!webLoading && webResults.length === 0 && webQuery.trim() && (
              <p className="px-4 py-6 text-sm text-muted-foreground text-center">
                No web results found
              </p>
            )}
            {!webLoading && !webQuery.trim() && (
              <p className="px-4 py-6 text-sm text-muted-foreground text-center">Search the web…</p>
            )}
            {webResults.map((r, i) => (
              <div
                key={`${r.url}-${i}`}
                className="flex items-start gap-2.5 px-3 py-2 text-sm cursor-pointer select-none rounded-md transition-colors hover:bg-accent/50"
              >
                <Globe className="h-4 w-4 shrink-0 text-muted-foreground mt-0.5" />
                <div className="min-w-0 flex-1">
                  <p className="font-medium truncate">{r.title || r.url}</p>
                  <p className="text-xs text-muted-foreground/70 line-clamp-1">{r.snippet}</p>
                  <p className="text-[10px] text-muted-foreground/50 truncate mt-0.5">{r.url}</p>
                </div>
                <button
                  type="button"
                  className="shrink-0 text-xs text-primary hover:underline"
                  onClick={() => {
                    usePortalStore.getState().pushView({ type: 'search', query: r.title || r.url })
                    close()
                  }}
                >
                  <ExternalLink className="w-3 h-3 inline mr-0.5" />
                  Panel
                </button>
              </div>
            ))}
          </div>
        )}

        {/* Footer hint */}
        <div className="flex items-center justify-between border-t px-3 py-1.5 text-2xs text-muted-foreground">
          <span>
            <kbd className="font-mono border rounded px-1">↑↓</kbd> navigate
            {' · '}
            <kbd className="font-mono border rounded px-1">↵</kbd> select
          </span>
          <span>
            <kbd className="font-mono border rounded px-1">⌘K</kbd> to open
          </span>
        </div>
      </div>
    </div>
  )
}
