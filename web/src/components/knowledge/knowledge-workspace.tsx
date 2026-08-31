// KnowledgeWorkspace — three-pane Knowledge surface (design §7.3).
//
// 1. Context list (left)   — grouped source list (memory + library).
//                            Each row carries an explicit kind + backing
//                            id/path; rows may interleave when both groups
//                            have items, each row still labeled with its
//                            kind.
// 2. Reader/synthesis canvas (center) — renders the selected source's
//                            existing preview/read-only surface. Library
//                            documents use FilePreviewView; memory items
//                            reuse BrainEntityDetail. Defaults to first
//                            available row or a quiet empty state.
// 3. Source inspector (right) — title, publisher/path, date, excerpt,
//                            backing type, allowed next action
//                            (Add to Studio).
//
// Unbound Brain: neutral "Not connected" empty state, no recall/search
// calls fired, no implicit fallback space.
//
// Keyboard: the inspector's trigger row receives focus when the handoff
// sheet closes.

import { useQuery } from '@tanstack/react-query'
import { Brain, FileText, Plus, Search, Trash2, Unplug } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { BrainEntityDetail } from '@/components/brain/entity-detail'
import { FilePreviewView } from '@/components/portal/views/file-preview-view'
import { ErrorState } from '@/components/shared/error-state'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useKnowledgeRecursiveTree, useKnowledgeSearch } from '@/hooks/use-knowledge'
import { api } from '@/lib/api-client'
import { flattenTree } from '@/lib/tree-utils'
import { cn } from '@/lib/utils'
import { useKnowledgeStore } from '@/stores/knowledge'
import { usePortalStore } from '@/stores/portal'
import type { SearchResponse } from '@/types/brain'
import type { KnowledgeTreeNode } from '@/types/knowledge'
import { type KnowledgeContextRef, StudioHandoffSheet } from './studio-handoff'

type MemorySource = {
  kind: 'memory'
  id: string
  surface: string
  entityType?: string
  snippet?: string
  score?: number
}

type LibrarySource = {
  kind: 'library'
  path: string
  displayName: string
  hasContent: boolean
}

type Source = MemorySource | LibrarySource

export type KnowledgeWorkspaceProps = {
  /** Explicit space override (e.g. when the page is opened from a
   *  search-param). When empty the workspace surfaces a "Not
   *  connected" empty state — there is no implicit fallback space. */
  space?: string
}

function fmtScore(s: number | undefined): string {
  return typeof s === 'number' ? s.toFixed(3) : ''
}

export function KnowledgeWorkspace({ space: spaceOverride = '' }: KnowledgeWorkspaceProps) {
  const { t } = useTranslation()

  const resolvedSpace = spaceOverride || ''

  // ─── Memory source: useBrainSearch (only when space is bound) ─────
  const [memoryQuery, setMemoryQuery] = useState('')
  const [submittedMemoryQuery, setSubmittedMemoryQuery] = useState('')
  const memorySearch = useBrainSearchMemory(resolvedSpace, submittedMemoryQuery)

  const memorySources: MemorySource[] = useMemo(() => {
    if (!resolvedSpace) return []
    const hits = memorySearch.data?.memory ?? []
    return hits.map((h) => ({
      kind: 'memory' as const,
      id: h.entity_id,
      surface: h.entity_surface,
      entityType: h.entity_type,
      snippet: h.snippet,
      score: h.score,
    }))
  }, [memorySearch.data, resolvedSpace])

  // ─── Library source: recursive tree + optional search ─────────────
  const [libraryQuery, setLibraryQuery] = useState('')
  const { data: tree } = useKnowledgeRecursiveTree()
  const librarySearch = useKnowledgeSearch()
  const [libraryHits, setLibraryHits] = useState<KnowledgeTreeNode[]>([])
  const [searchingLibrary, setSearchingLibrary] = useState(false)

  const submitLibrarySearch = useCallback(
    async (q: string) => {
      const trimmed = q.trim()
      if (!trimmed) {
        setLibraryHits([])
        return
      }
      setSearchingLibrary(true)
      try {
        const res = await librarySearch.mutateAsync({ query: trimmed, limit: 20 })
        const results = res?.results ?? []
        const indexByPath = new Map<string, KnowledgeTreeNode>()
        if (tree)
          flattenTree(tree).forEach((n) => {
            if (!n.is_dir) indexByPath.set(n.path, n)
          })
        setLibraryHits(
          results
            .map((r) => indexByPath.get(r.path))
            .filter((n): n is KnowledgeTreeNode => Boolean(n)),
        )
      } catch {
        setLibraryHits([])
      } finally {
        setSearchingLibrary(false)
      }
    },
    [librarySearch, tree],
  )

  const librarySources: LibrarySource[] = useMemo(() => {
    const query = libraryQuery.trim()
    if (query && libraryHits.length > 0) {
      return libraryHits.map((n) => ({
        kind: 'library' as const,
        path: n.path,
        displayName: n.display_name || n.name,
        hasContent: n.has_content,
      }))
    }
    if (!tree) return []
    return flattenTree(tree)
      .filter((n) => !n.is_dir)
      .slice(0, 30)
      .map((n) => ({
        kind: 'library' as const,
        path: n.path,
        displayName: n.display_name || n.name,
        hasContent: n.has_content,
      }))
  }, [libraryHits, libraryQuery, tree])

  // ─── Combined source list (preserves per-row kind) ────────────────
  const allSources: Source[] = useMemo(() => {
    return [...memorySources, ...librarySources]
  }, [memorySources, librarySources])

  const [selectedSource, setSelectedSource] = useState<Source | undefined>(undefined)
  useEffect(() => {
    if (selectedSource) return
    if (allSources.length === 0) return
    setSelectedSource(allSources[0])
  }, [allSources, selectedSource])

  const openFile = useKnowledgeStore((s) => s.openFile)
  const pushView = usePortalStore((s) => s.pushView)

  const handleSelect = useCallback(
    (src: Source) => {
      setSelectedSource(src)
      if (src.kind === 'library') {
        openFile(src.path)
      }
    },
    [openFile],
  )

  const handleOpenInPortal = useCallback(
    (src: Source) => {
      if (src.kind !== 'library') return
      pushView({ type: 'filePreview', path: src.path, content: undefined })
    },
    [pushView],
  )

  // ─── Handoff intent ───────────────────────────────────────────────
  const [handoffOpen, setHandoffOpen] = useState(false)
  const triggerRef = useRef<HTMLElement | null>(null)
  const triggerButtonRefs = useRef<Map<string, HTMLElement>>(new Map())

  const setTriggerRef = useCallback(
    (key: string, el: HTMLButtonElement | null) => {
      if (el) {
        triggerButtonRefs.current.set(key, el)
        if (selectedSource && keyOf(selectedSource) === key) {
          triggerRef.current = el
        }
      } else {
        triggerButtonRefs.current.delete(key)
      }
    },
    [selectedSource],
  )

  const contextRefs: KnowledgeContextRef[] = useMemo(() => {
    if (!selectedSource) return []
    if (selectedSource.kind === 'memory') {
      return [{ kind: 'brain-memory', id: selectedSource.id }]
    }
    return [{ kind: 'knowledge-document', path: selectedSource.path }]
  }, [selectedSource])

  const isAvailable = useCallback(
    (ref: KnowledgeContextRef) => {
      if (ref.kind === 'brain-memory') {
        return memorySources.some((s) => s.id === ref.id)
      }
      if (ref.kind === 'knowledge-document') {
        return librarySources.some((s) => s.path === ref.path)
      }
      return false
    },
    [memorySources, librarySources],
  )

  return (
    <div className="flex h-full min-h-[calc(100vh-8rem)] gap-4" data-testid="knowledge-workspace">
      {/* ── Pane 1: Context list ──────────────────────────────── */}
      <aside
        className="w-72 shrink-0 flex flex-col gap-3 overflow-hidden"
        aria-label={t('knowledge.workspace.contextList')}
      >
        {/* Memory group */}
        <section className="flex flex-col gap-2 min-h-0">
          <div className="flex items-center justify-between gap-2">
            <h2 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground flex items-center gap-1.5">
              <Brain className="h-3.5 w-3.5" />
              {t('knowledge.workspace.contextListMemory')}
            </h2>
            <span className="text-2xs text-muted-foreground tabular-nums">
              {memorySources.length}
            </span>
          </div>
          {!resolvedSpace ? (
            <BrainNotConnected />
          ) : (
            <MemorySearchInput
              value={memoryQuery}
              onChange={setMemoryQuery}
              onSubmit={(q) => setSubmittedMemoryQuery(q.trim())}
              loading={memorySearch.isFetching}
            />
          )}
          {resolvedSpace && (
            <div
              className="min-h-0 flex-1 overflow-auto"
              aria-label={t('knowledge.workspace.contextListMemory')}
            >
              {memorySearch.isError ? (
                <ErrorState className="py-6" />
              ) : memorySources.length === 0 ? (
                <p className="text-xs text-muted-foreground py-2" role="status">
                  {submittedMemoryQuery
                    ? t('brain.noSearchResults')
                    : t('knowledge.workspace.emptyMemory')}
                </p>
              ) : (
                <ul className="space-y-1">
                  {memorySources.map((src) => (
                    <SourceRow
                      key={keyOf(src)}
                      src={src}
                      selected={selectedSource?.kind === 'memory' && selectedSource.id === src.id}
                      onSelect={() => handleSelect(src)}
                      registerRef={(el) => setTriggerRef(keyOf(src), el)}
                    />
                  ))}
                </ul>
              )}
            </div>
          )}
        </section>

        {/* Library group */}
        <section className="flex flex-col gap-2 min-h-0">
          <div className="flex items-center justify-between gap-2">
            <h2 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground flex items-center gap-1.5">
              <FileText className="h-3.5 w-3.5" />
              {t('knowledge.workspace.contextListLibrary')}
            </h2>
            <span className="text-2xs text-muted-foreground tabular-nums">
              {librarySources.length}
            </span>
          </div>
          <form
            onSubmit={(e) => {
              e.preventDefault()
              void submitLibrarySearch(libraryQuery)
            }}
            className="flex gap-1"
          >
            <Input
              value={libraryQuery}
              onChange={(e) => setLibraryQuery(e.target.value)}
              placeholder={t('knowledge.typeToSearch')}
              aria-label={t('knowledge.typeToSearch')}
            />
            <Button
              type="submit"
              variant="outline"
              size="sm"
              disabled={searchingLibrary}
              aria-label={t('brain.search')}
            >
              <Search className="h-3.5 w-3.5" />
            </Button>
          </form>
          <div
            className="min-h-0 flex-1 overflow-auto"
            aria-label={t('knowledge.workspace.contextListLibrary')}
          >
            {librarySources.length === 0 ? (
              <p className="text-xs text-muted-foreground py-2" role="status">
                {t('knowledge.workspace.emptyLibrary')}
              </p>
            ) : (
              <ul className="space-y-1">
                {librarySources.map((src) => (
                  <SourceRow
                    key={keyOf(src)}
                    src={src}
                    selected={
                      selectedSource?.kind === 'library' && selectedSource.path === src.path
                    }
                    onSelect={() => handleSelect(src)}
                    onOpenPreview={() => handleOpenInPortal(src)}
                    registerRef={(el) => setTriggerRef(keyOf(src), el)}
                  />
                ))}
              </ul>
            )}
          </div>
        </section>
      </aside>

      {/* ── Pane 2: Reader/synthesis canvas ──────────────────── */}
      <section
        className="flex-1 min-w-0 flex flex-col overflow-hidden"
        aria-label={t('knowledge.workspace.reader')}
      >
        <div className="flex-1 min-h-0 overflow-auto rounded-lg border bg-card">
          {selectedSource ? (
            selectedSource.kind === 'library' ? (
              <div className="h-full">
                <FilePreviewView
                  view={{ type: 'filePreview', path: selectedSource.path, content: undefined }}
                />
              </div>
            ) : (
              resolvedSpace && (
                <div className="p-4">
                  <BrainEntityDetail
                    space={resolvedSpace}
                    initialEntityId={selectedSource.id}
                    onNavigateEntity={(id) => {
                      setSelectedSource({ kind: 'memory', id, surface: id, score: undefined })
                    }}
                  />
                </div>
              )
            )
          ) : (
            <p className="p-8 text-sm text-muted-foreground text-center" role="status">
              {resolvedSpace
                ? t('knowledge.workspace.noSources')
                : t('knowledge.workspace.emptyReader')}
            </p>
          )}
        </div>
      </section>

      {/* ── Pane 3: Source inspector ──────────────────────────── */}
      <aside
        className="w-72 shrink-0 flex flex-col gap-3 overflow-hidden"
        aria-label={t('knowledge.workspace.inspector')}
      >
        <h2 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          {t('knowledge.workspace.inspector')}
        </h2>
        {selectedSource ? (
          <InspectorCard
            src={selectedSource}
            onAddToStudio={() => {
              setHandoffOpen(true)
              triggerRef.current = triggerButtonRefs.current.get(keyOf(selectedSource)) ?? null
            }}
            onClear={() => setSelectedSource(undefined)}
          />
        ) : (
          <p className="text-xs text-muted-foreground">{t('knowledge.workspace.emptyReader')}</p>
        )}
      </aside>

      <StudioHandoffSheet
        open={handoffOpen}
        onOpenChange={setHandoffOpen}
        contextRefs={contextRefs}
        isAvailable={isAvailable}
        triggerRef={triggerRef}
      />
    </div>
  )
}

// ── Sub-components ─────────────────────────────────────────────────

function keyOf(src: Source): string {
  return src.kind === 'memory' ? `memory:${src.id}` : `library:${src.path}`
}

function SourceRow({
  src,
  selected,
  onSelect,
  onOpenPreview,
  registerRef,
}: {
  src: Source
  selected: boolean
  onSelect: () => void
  onOpenPreview?: () => void
  registerRef: (el: HTMLButtonElement | null) => void
}) {
  const { t } = useTranslation()
  const isMemory = src.kind === 'memory'
  const badge = t(
    isMemory ? 'knowledge.workspace.kindBadge.memory' : 'knowledge.workspace.kindBadge.library',
  )
  return (
    <li>
      <button
        ref={registerRef}
        type="button"
        onClick={onSelect}
        aria-current={selected ? 'true' : undefined}
        data-testid={`source-row-${keyOf(src)}`}
        data-kind={src.kind}
        className={cn(
          'w-full text-left rounded-md border px-2.5 py-1.5 transition-colors',
          selected ? 'border-primary bg-primary/5' : 'border-transparent hover:bg-accent/40',
        )}
      >
        <div className="flex items-center justify-between gap-2">
          <span className="truncate text-sm font-medium">
            {isMemory ? src.surface : src.displayName}
          </span>
          <span className="shrink-0 rounded border px-1 py-0.5 text-[10px] font-mono uppercase tracking-wide text-muted-foreground">
            {badge}
          </span>
        </div>
        {isMemory && src.snippet && (
          <p className="mt-1 line-clamp-2 text-2xs text-muted-foreground">{src.snippet}</p>
        )}
        {!isMemory && !src.hasContent && (
          <p className="mt-1 text-2xs text-muted-foreground">
            {t('knowledge.workspace.missingMeta')}
          </p>
        )}
        {isMemory && src.score !== undefined && (
          <span className="mt-1 inline-block text-2xs text-muted-foreground/70 tabular-nums">
            {fmtScore(src.score)}
          </span>
        )}
      </button>
      {selected && onOpenPreview && (
        <div className="mt-1 ml-1">
          <Button variant="ghost" size="sm" onClick={onOpenPreview} className="h-7 px-2 text-xs">
            <Plus className="h-3 w-3" />
            {t('knowledge.workspace.preview')}
          </Button>
        </div>
      )}
    </li>
  )
}

function MemorySearchInput({
  value,
  onChange,
  onSubmit,
  loading,
}: {
  value: string
  onChange: (next: string) => void
  onSubmit: (q: string) => void
  loading: boolean
}) {
  const { t } = useTranslation()
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault()
        onSubmit(value)
      }}
      className="flex gap-1"
    >
      <Input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        aria-label={t('brain.searchPlaceholder')}
        className="flex-1 h-8 text-sm"
      />
      <Button
        type="submit"
        variant="outline"
        size="sm"
        disabled={loading}
        aria-label={t('brain.search')}
      >
        <Search className="h-3.5 w-3.5" />
      </Button>
    </form>
  )
}

function BrainNotConnected() {
  const { t } = useTranslation()
  return (
    <div
      className="rounded-md border border-status-warning-subtle-border bg-status-warning-subtle p-3 flex items-start gap-2"
      data-testid="brain-not-connected"
      role="status"
    >
      <Unplug className="h-4 w-4 mt-0.5 shrink-0 text-status-warning" aria-hidden="true" />
      <div className="space-y-0.5">
        <p className="text-xs font-medium text-status-warning-on-subtle">
          {t('knowledge.workspace.brainNotConnected')}
        </p>
        <p className="text-2xs text-muted-foreground">
          {t('knowledge.workspace.brainNotConnectedHint')}
        </p>
      </div>
    </div>
  )
}

function InspectorCard({
  src,
  onAddToStudio,
  onClear,
}: {
  src: Source
  onAddToStudio: () => void
  onClear: () => void
}) {
  const { t } = useTranslation()
  const isMemory = src.kind === 'memory'
  return (
    <div className="rounded-lg border bg-card p-3 space-y-3 text-sm" data-testid="source-inspector">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <p className="font-semibold truncate">{isMemory ? src.surface : src.displayName}</p>
          {!isMemory && (
            <p className="font-mono text-2xs text-muted-foreground truncate">{src.path}</p>
          )}
          {isMemory && src.entityType && (
            <p className="text-2xs text-muted-foreground">{src.entityType}</p>
          )}
        </div>
        <button
          type="button"
          onClick={onClear}
          className="text-muted-foreground hover:text-foreground"
          aria-label={t('knowledge.workspace.removeUnavailable')}
        >
          <Trash2 className="h-3.5 w-3.5" />
        </button>
      </div>

      <dl className="space-y-1.5 text-xs">
        <div className="flex items-center justify-between">
          <dt className="text-muted-foreground">
            {t('knowledge.workspace.kindBadge.memory').split(' /')[0]}
          </dt>
          <dd>
            <span
              className={cn(
                'inline-flex items-center rounded-full border px-2 py-0.5 text-2xs',
                isMemory
                  ? 'border-status-info-subtle-border bg-status-info-subtle text-status-info-on-subtle'
                  : 'border-status-success-subtle-border bg-status-success-subtle text-status-success-on-subtle',
              )}
            >
              {t(
                isMemory
                  ? 'knowledge.workspace.kindBadge.memory'
                  : 'knowledge.workspace.kindBadge.library',
              )}
            </span>
          </dd>
        </div>
        {isMemory && src.snippet && (
          <div>
            <dt className="text-muted-foreground">{t('knowledge.workspace.excerpt')}</dt>
            <dd className="line-clamp-3 text-foreground/90">{src.snippet}</dd>
          </div>
        )}
        {isMemory && src.score !== undefined && (
          <div className="flex items-center justify-between">
            <dt className="text-muted-foreground">{t('knowledge.workspace.score')}</dt>
            <dd className="tabular-nums">{fmtScore(src.score)}</dd>
          </div>
        )}
      </dl>

      <Button
        onClick={onAddToStudio}
        className="w-full"
        size="sm"
        data-testid="inspector-add-to-studio"
      >
        <Plus className="h-4 w-4" />
        {t('knowledge.workspace.addToStudio')}
      </Button>
    </div>
  )
}

// ── Hooks ──────────────────────────────────────────────────────────

function useBrainSearchMemory(space: string, query: string) {
  return useQuery({
    queryKey: ['brain', 'search', query, 'memory', 'workspace', space],
    queryFn: () =>
      api.get<SearchResponse | null>('/api/brain/search', {
        q: query,
        space,
        mode: 'hybrid',
        limit: '20',
        planes: 'memory',
      }),
    enabled: space.length > 0 && query.trim().length > 0,
    placeholderData: (prev) => prev,
  })
}
