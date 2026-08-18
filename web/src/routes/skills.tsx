import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import {
  Check,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  CircleCheck,
  CircleX,
  Download,
  FileUp,
  Globe,
  Link2,
  PackagePlus,
  Pencil,
  Plus,
  Power,
  Search,
  Store,
  Trash2,
  Type,
  X,
  Zap,
} from 'lucide-react'

import { useCallback, useDeferredValue, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { EmptyState } from '@/components/shared/empty-state'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { PageHeader } from '@/components/shared/page-header'
import { ImportDialog } from '@/components/skills/import-dialog'
import { MarketplaceDetail } from '@/components/skills/marketplace-detail'
import { SkillDetail } from '@/components/skills/skill-detail'
import { SkillEditorDialog } from '@/components/skills/skill-editor-dialog'
import { SkillSummaryPill } from '@/components/skills/skill-summary-pill'
import { UpdateBadge, useSkillUpdates } from '@/components/skills/update-badge'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import { api } from '@/lib/api-client'
import { cn } from '@/lib/utils'
import type { ClawHubSearchResult, Skill, SkillFormat, SkillStatus, SkillsShSkill } from '@/types'

export const Route = createFileRoute('/skills')({
  component: SkillsPage,
  validateSearch: (search: Record<string, unknown>) => ({
    tab: (search.tab as string) || undefined,
  }),
})

type Tab = 'installed' | 'marketplace'
type MarketplaceSource = 'clawhub' | 'skills-sh'

const STATUS_DISPLAY: Record<
  SkillStatus,
  { icon: React.ReactNode; label: string; variant: 'success' | 'warning' | 'destructive' }
> = {
  ready: { icon: <CircleCheck className="h-3 w-3" />, label: 'ready', variant: 'success' },
  needs_setup: {
    icon: <CircleAlert className="h-3 w-3" />,
    label: 'needs-setup',
    variant: 'warning',
  },
  disabled: { icon: <CircleX className="h-3 w-3" />, label: 'disabled', variant: 'destructive' },
}

const SOURCE_VARIANT: Record<string, 'outline' | 'secondary' | 'default'> = {
  managed: 'outline',
  bundled: 'secondary',
  workspace: 'default',
}

const FORMAT_META: Record<
  SkillFormat,
  { label: string; variant: 'default' | 'secondary' | 'outline'; description: string }
> = {
  oxios: { label: 'Oxios', variant: 'default', description: 'Oxios native skill' },
  openclaw: { label: 'OpenClaw', variant: 'secondary', description: 'ClawHub marketplace skill' },
  claude_code: {
    label: 'Claude',
    variant: 'outline',
    description: 'Claude Code skill — core instructions compatible, some features may not apply',
  },
  agent_skills: {
    label: 'Standard',
    variant: 'outline',
    description: 'Agent Skills standard (agentskills.io)',
  },
}

function FormatBadge({ format }: { format: SkillFormat }) {
  const m = FORMAT_META[format]
  return (
    <Badge variant={m.variant} className="text-xs" title={m.description}>
      {m.label}
    </Badge>
  )
}

// ─── Page ─────────────────────────────────────────────────────

function SkillsPage() {
  const { t } = useTranslation()
  const search = Route.useSearch()
  const navigate = useNavigate({ from: Route.id })
  const tab: Tab = search.tab === 'marketplace' ? 'marketplace' : 'installed'
  const [mktSource, setMktSource] = useState<MarketplaceSource>('clawhub')
  const [filter, setFilter] = useState<'all' | SkillStatus>('all')
  const [searchQuery, setSearchQuery] = useState('' /* icon fallback */)
  const [mktQuery, setMktQuery] = useState('' /* icon fallback */)
  const deferredQuery = useDeferredValue(mktQuery)

  // Selected skill for detail panel
  const [selectedSkill, setSelectedSkill] = useState<Skill | null>(null)
  const [selectedMktSlug, setSelectedMktSlug] = useState<string | null>(null)
  const [selectedSkillsShId, setSelectedSkillsShId] = useState<string | null>(null)

  // Editor + import dialog state (F1/F2/F3)
  const [editorState, setEditorState] = useState<
    { mode: 'create' } | { mode: 'edit'; skill: Skill } | null
  >(null)
  const [importState, setImportState] = useState<{ open: boolean; mode: 'file' | 'text' | 'url' }>({
    open: false,
    mode: 'file',
  })

  // Fetch the raw SKILL.md when editing, so the editor opens pre-filled.
  const editingName = editorState?.mode === 'edit' ? editorState.skill.name : null
  const { data: editContent } = useQuery({
    queryKey: ['skill', editingName, 'content'],
    queryFn: async () => {
      const r = await api.get<{ content: string }>(
        `/api/skills/${encodeURIComponent(editingName!)}/content`,
      )
      return r?.content ?? ''
    },
    enabled: !!editingName,
  })

  const handleEditSkill = useCallback((skill: Skill) => {
    setEditorState({ mode: 'edit', skill })
  }, [])

  const {
    data: skills,
    isLoading: sl,
    isError: se,
    refetch: sr,
  } = useQuery({
    queryKey: ['skills'],
    queryFn: async () => {
      const r = await api.get<{ skills: Skill[] }>('/api/skills')
      return Array.isArray(r?.skills) ? r.skills : []
    },
    refetchInterval: 30000,
  })
  const {
    data: mktResults,
    isLoading: ml,
    isError: me,
    refetch: mr,
  } = useQuery({
    queryKey: ['marketplace', 'search', deferredQuery],
    queryFn: async () => {
      const r = await api.get<ClawHubSearchResult[]>('/api/marketplace/search', {
        q: deferredQuery,
      })
      return Array.isArray(r) ? r : []
    },
    enabled: tab === 'marketplace' && mktSource === 'clawhub' && deferredQuery.trim().length > 0,
    refetchOnWindowFocus: false,
  })
  // Skills.sh search
  const {
    data: skillsShResults,
    isLoading: ssl,
    isError: sse,
    refetch: ssr,
  } = useQuery({
    queryKey: ['skills-sh', 'search', deferredQuery],
    queryFn: async () => {
      const r = await api.get<{ data: SkillsShSkill[] }>('/api/marketplace/skills-sh/search', {
        q: deferredQuery,
      })
      return Array.isArray(r?.data) ? r.data : []
    },
    enabled: tab === 'marketplace' && mktSource === 'skills-sh' && deferredQuery.trim().length > 0,
    refetchOnWindowFocus: false,
  })
  // Skills.sh trending list (loaded when tab is open and source is skills-sh)
  const { data: skillsShTrending } = useQuery({
    queryKey: ['skills-sh', 'trending'],
    queryFn: async () => {
      const r = await api.get<{ data: SkillsShSkill[] }>('/api/marketplace/skills-sh/list', {
        view: 'trending',
        per_page: '20',
      })
      return Array.isArray(r?.data) ? r.data : []
    },
    enabled: tab === 'marketplace' && mktSource === 'skills-sh',
    refetchOnWindowFocus: false,
    staleTime: 60_000,
  })

  // Updates check
  const { data: updates } = useSkillUpdates()
  const updateCount = updates?.length ?? 0

  const counts = useMemo(() => {
    const l: Skill[] = Array.isArray(skills) ? skills : []
    return {
      all: l.length,
      ready: l.filter((s) => s.status === 'ready').length,
      needs_setup: l.filter((s) => s.status === 'needs_setup').length,
      disabled: l.filter((s) => s.status === 'disabled').length,
    }
  }, [skills])

  const filtered = useMemo(() => {
    let l: Skill[] = Array.isArray(skills) ? skills : []
    if (filter !== 'all') l = l.filter((s) => s.status === filter)
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase()
      l = l.filter(
        (s) => s.name.toLowerCase().includes(q) || s.description.toLowerCase().includes(q),
      )
    }
    return l
  }, [skills, filter, searchQuery])

  // Close detail panel when switching tabs
  const handleTabChange = useCallback(
    (newTab: Tab) => {
      navigate({
        search: (prev) => ({ ...prev, tab: newTab === 'installed' ? undefined : newTab }),
        replace: true,
      })
      setSelectedSkill(null)
      setSelectedMktSlug(null)
      setSelectedSkillsShId(null)
    },
    [navigate],
  )

  // Gate the editor so edit mode opens only after SKILL.md content has loaded
  // (avoids seeding a blank body when the content query is still fetching).
  const editorOpen = !!editorState && (editorState.mode === 'create' || editContent !== undefined)

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('skills.title')}
        subtitle={t('skills.subtitle')}
        actions={<SkillSummaryPill counts={counts} filter={filter} onFilterChange={setFilter} />}
      />

      {/* F1/F2: primary actions — create & import are always-visible entry points */}
      <div className="flex items-center gap-2">
        <Button onClick={() => setEditorState({ mode: 'create' })}>
          <Plus className="h-4 w-4" />
          {t('skills.create')}
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="secondary">
              <Download className="h-4 w-4" />
              {t('skills.import')}
              <ChevronDown className="h-3.5 w-3.5" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start">
            <DropdownMenuItem onClick={() => setImportState({ open: true, mode: 'file' })}>
              <FileUp className="h-4 w-4" />
              {t('skills.importFromFile')}
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => setImportState({ open: true, mode: 'text' })}>
              <Type className="h-4 w-4" />
              {t('skills.importFromText')}
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => setImportState({ open: true, mode: 'url' })}>
              <Link2 className="h-4 w-4" />
              {t('skills.importFromUrl')}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      {/* Tab switcher */}
      <div className="inline-flex h-9 items-center rounded-lg bg-muted p-1 text-muted-foreground gap-0.5">
        <button
          type="button"
          onClick={() => handleTabChange('installed')}
          className={cn(
            'inline-flex items-center justify-center whitespace-nowrap rounded-md px-3 py-1 text-sm font-medium transition-all gap-1.5',
            tab === 'installed' ? 'bg-background text-foreground shadow' : 'hover:bg-background/50',
          )}
        >
          <Zap className="h-3.5 w-3.5" /> {t('skills.installed')}{' '}
          <span className="text-xs text-muted-foreground">{counts.all}</span>
          <UpdateBadge count={updateCount} />
        </button>
        <button
          type="button"
          onClick={() => handleTabChange('marketplace')}
          className={cn(
            'inline-flex items-center justify-center whitespace-nowrap rounded-md px-3 py-1 text-sm font-medium transition-all gap-1.5',
            tab === 'marketplace'
              ? 'bg-background text-foreground shadow'
              : 'hover:bg-background/50',
          )}
        >
          <Store className="h-3.5 w-3.5" /> {t('skills.marketplace')}
          <UpdateBadge count={updateCount} />
        </button>
      </div>

      {/* Main content area with optional side panel */}
      <div
        className={cn(
          'grid gap-6',
          selectedSkill || selectedMktSlug || selectedSkillsShId
            ? 'grid-cols-1 lg:grid-cols-[1fr_380px]'
            : 'grid-cols-1',
        )}
      >
        <div>
          {tab === 'installed' ? (
            <InstalledTab
              filtered={filtered}
              allSkills={Array.isArray(skills) ? skills : []}
              search={searchQuery}
              setSearch={setSearchQuery}
              isLoading={sl}
              isError={se}
              refetch={sr}
              selectedSkill={selectedSkill}
              onSelectSkill={setSelectedSkill}
              onEditSkill={handleEditSkill}
              updates={updates}
            />
          ) : (
            <MarketplaceTab
              source={mktSource}
              onSourceChange={setMktSource}
              clawhubResults={mktResults}
              skillsShResults={deferredQuery.trim() ? skillsShResults : skillsShTrending}
              query={mktQuery}
              setQuery={setMktQuery}
              deferredQuery={deferredQuery}
              isLoading={mktSource === 'clawhub' ? ml : ssl}
              isError={mktSource === 'clawhub' ? me : sse}
              refetch={() => {
                mr()
                ssr()
              }}
              selectedClawhubSlug={selectedMktSlug}
              onSelectClawhubSlug={setSelectedMktSlug}
              selectedSkillsShId={selectedSkillsShId}
              onSelectSkillsShId={setSelectedSkillsShId}
            />
          )}
        </div>

        {/* Side panel */}
        {selectedSkill && (
          <div className="border rounded-lg p-4 h-fit sticky top-6">
            <SkillDetail
              skill={selectedSkill}
              onClose={() => setSelectedSkill(null)}
              onEdit={() => handleEditSkill(selectedSkill)}
            />
          </div>
        )}
        {selectedMktSlug && (
          <div className="border rounded-lg p-4 h-fit sticky top-6">
            <MarketplaceDetail slug={selectedMktSlug} onClose={() => setSelectedMktSlug(null)} />
          </div>
        )}
        {selectedSkillsShId && (
          <div className="border rounded-lg p-4 h-fit sticky top-6">
            <SkillsShDetail id={selectedSkillsShId} onClose={() => setSelectedSkillsShId(null)} />
          </div>
        )}
      </div>

      {/* F1/F2/F3 dialogs */}
      <SkillEditorDialog
        open={editorOpen}
        onOpenChange={(o) => {
          if (!o) setEditorState(null)
        }}
        skill={editorState?.mode === 'edit' ? editorState.skill : null}
        initialContent={editorState?.mode === 'edit' ? (editContent ?? '') : ''}
      />
      <ImportDialog
        open={importState.open}
        onOpenChange={(o) => setImportState((s) => ({ ...s, open: o }))}
        initialMode={importState.mode}
      />
    </div>
  )
}

// ─── Installed Tab ────────────────────────────────────────────

interface SkillUpdate {
  slug: string
  currentVersion: string
  latestVersion: string
  changelog?: string
}

function InstalledTab({
  filtered,
  allSkills,
  search,
  setSearch,
  isLoading,
  isError,
  refetch,
  selectedSkill,
  onSelectSkill,
  onEditSkill,
  updates,
}: {
  filtered: Skill[]
  allSkills: Skill[]
  search: string
  setSearch: (s: string) => void
  isLoading: boolean
  isError: boolean
  refetch: () => void
  selectedSkill: Skill | null
  onSelectSkill: (s: Skill | null) => void
  onEditSkill: (s: Skill) => void
  updates?: SkillUpdate[]
}) {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const [deleteTarget, setDeleteTarget] = useState<Skill | null>(null)

  const toggleMutation = useMutation({
    mutationFn: ({ name, enable }: { name: string; enable: boolean }) => {
      const endpoint = enable
        ? `/api/skills/${encodeURIComponent(name)}/enable`
        : `/api/skills/${encodeURIComponent(name)}/disable`
      return api.post(endpoint)
    },
    onSuccess: () => {
      toast.success(t('skills.toggleSuccess'))
      qc.invalidateQueries({ queryKey: ['skills'] })
    },
    onError: (err: unknown) => {
      toast.error(err instanceof Error ? err.message : t('common.error'))
    },
  })

  const deleteMutation = useMutation({
    mutationFn: (name: string) => api.delete(`/api/skills/${encodeURIComponent(name)}`),
    onSuccess: () => {
      toast.success(t('skills.deleteSuccess', { name: deleteTarget?.name }))
      qc.invalidateQueries({ queryKey: ['skills'] })
      setDeleteTarget(null)
      if (selectedSkill && deleteTarget && selectedSkill.name === deleteTarget.name) {
        onSelectSkill(null)
      }
    },
    onError: (err: unknown) => {
      toast.error(err instanceof Error ? err.message : t('common.error'))
    },
  })

  if (isLoading) return <LoadingCards count={4} />
  if (isError) return <ErrorState onRetry={() => refetch()} />

  return (
    <>
      {/* Status filter is unified in the header SkillSummaryPill (F5). */}
      <div className="flex items-center justify-end">
        <Input
          placeholder={t('skills.searchInstalled')}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="max-w-xs"
        />
      </div>
      {filtered.length === 0 ? (
        <EmptyState
          icon={<Zap className="h-10 w-10" />}
          title={allSkills.length === 0 ? t('skills.noSkills') : t('skills.noMatching')}
          description={
            allSkills.length === 0
              ? t('skills.noSkillsDescription')
              : t('skills.noMatchingDescription')
          }
        />
      ) : (
        <div className="grid gap-4">
          {filtered.map((s) => {
            const hasUpdate = updates?.some((u) => u.slug === s.name)
            return (
              <SkillCard
                key={s.name}
                skill={s}
                isSelected={selectedSkill?.name === s.name}
                hasUpdate={hasUpdate}
                onSelect={() => onSelectSkill(selectedSkill?.name === s.name ? null : s)}
                onToggle={() =>
                  toggleMutation.mutate({ name: s.name, enable: s.status === 'disabled' })
                }
                onDelete={() => setDeleteTarget(s)}
                onEdit={() => onEditSkill(s)}
                isToggling={toggleMutation.isPending}
              />
            )
          })}
        </div>
      )}

      {/* Delete confirmation dialog */}
      <Dialog
        open={!!deleteTarget}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null)
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('skills.deleteConfirm')}</DialogTitle>
            <DialogDescription>
              {t('skills.deleteDescription', {
                name: deleteTarget?.name ?? '' /* icon fallback */,
              })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" size="sm" onClick={() => setDeleteTarget(null)}>
              {t('common.cancel')}
            </Button>
            <Button
              variant="destructive"
              size="sm"
              onClick={() => deleteTarget && deleteMutation.mutate(deleteTarget.name)}
              disabled={deleteMutation.isPending}
            >
              {t('common.delete')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}

// ─── Marketplace Tab ──────────────────────────────────────────

function MarketplaceTab({
  source,
  onSourceChange,
  clawhubResults,
  skillsShResults,
  query,
  setQuery,
  deferredQuery,
  isLoading,
  isError,
  refetch,
  selectedClawhubSlug,
  onSelectClawhubSlug,
  selectedSkillsShId,
  onSelectSkillsShId,
}: {
  source: MarketplaceSource
  onSourceChange: (s: MarketplaceSource) => void
  clawhubResults?: ClawHubSearchResult[]
  skillsShResults?: SkillsShSkill[]
  query: string
  setQuery: (s: string) => void
  deferredQuery: string
  isLoading: boolean
  isError: boolean
  refetch: () => void
  selectedClawhubSlug: string | null
  onSelectClawhubSlug: (s: string | null) => void
  selectedSkillsShId: string | null
  onSelectSkillsShId: (s: string | null) => void
}) {
  const { t } = useTranslation()
  const qc = useQueryClient()

  // ClawHub install mutation
  const clawhubMut = useMutation({
    mutationFn: ({ slug, version }: { slug: string; version?: string }) =>
      api.post(`/api/marketplace/skills/${slug}/install`, { version }),
    onSuccess: (_: unknown, v: { slug: string; version?: string }) => {
      toast.success(t('skills.installSuccess', { slug: v.slug }))
      qc.invalidateQueries({ queryKey: ['skills'] })
    },
    onError: (e: unknown) => {
      toast.error(e instanceof Error ? e.message : t('skills.installFailed'))
    },
  })

  // Skills.sh install mutation
  const skillsShMut = useMutation({
    mutationFn: (id: string) =>
      api.post(`/api/marketplace/skills-sh/skill/${encodeURIComponent(id)}/install`),
    onSuccess: (_: unknown, id: string) => {
      toast.success(t('skills.installSuccess', { slug: id }))
      qc.invalidateQueries({ queryKey: ['skills'] })
    },
    onError: (e: unknown) => {
      toast.error(e instanceof Error ? e.message : t('skills.installFailed'))
    },
  })

  const hasQ = deferredQuery.trim().length > 0

  return (
    <>
      {/* Source toggle */}
      <div className="flex items-center gap-3">
        <div className="relative flex-1">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground pointer-events-none" />
          <Input
            placeholder={t('skills.searchMarketplace')}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="pl-10"
            autoFocus
          />
        </div>
        <div className="inline-flex h-9 items-center rounded-lg bg-muted p-1 text-muted-foreground gap-0.5">
          <button
            type="button"
            onClick={() => onSourceChange('clawhub')}
            className={cn(
              'inline-flex items-center justify-center whitespace-nowrap rounded-md px-2.5 py-1 text-xs font-medium transition-all gap-1',
              source === 'clawhub'
                ? 'bg-background text-foreground shadow'
                : 'hover:bg-background/50',
            )}
          >
            ClawHub
          </button>
          <button
            type="button"
            onClick={() => onSourceChange('skills-sh')}
            className={cn(
              'inline-flex items-center justify-center whitespace-nowrap rounded-md px-2.5 py-1 text-xs font-medium transition-all gap-1',
              source === 'skills-sh'
                ? 'bg-background text-foreground shadow'
                : 'hover:bg-background/50',
            )}
          >
            Skills.sh
            <span className="text-2xs text-muted-foreground">npx</span>
          </button>
        </div>
      </div>

      {/* Results */}
      {source === 'clawhub' ? (
        !hasQ ? (
          <EmptyState
            icon={<PackagePlus className="h-10 w-10" />}
            title={t('skills.discover')}
            description={t('skills.discoverDescription')}
          />
        ) : isLoading ? (
          <LoadingCards count={4} />
        ) : isError ? (
          <ErrorState onRetry={() => refetch()} />
        ) : clawhubResults?.length === 0 ? (
          <EmptyState
            icon={<Search className="h-10 w-10" />}
            title={t('skills.noResults')}
            description={t('skills.noResultsFor', { query: deferredQuery })}
          />
        ) : (
          <div className="grid gap-4">
            {clawhubResults!.map((s) => (
              <MarketplaceCard
                key={s.slug}
                skill={s}
                isSelected={selectedClawhubSlug === s.slug}
                isInstalling={clawhubMut.isPending}
                onSelect={() => onSelectClawhubSlug(selectedClawhubSlug === s.slug ? null : s.slug)}
                onInstall={(sl, v) => clawhubMut.mutate({ slug: sl, version: v })}
              />
            ))}
          </div>
        )
      ) : // Skills.sh
      !hasQ && !skillsShResults?.length ? (
        <EmptyState
          icon={<PackagePlus className="h-10 w-10" />}
          title={t('skills.discover')}
          description={t('skills.discoverDescription')}
        />
      ) : isLoading ? (
        <LoadingCards count={4} />
      ) : isError ? (
        <ErrorState onRetry={() => refetch()} />
      ) : skillsShResults?.length === 0 ? (
        <EmptyState
          icon={<Search className="h-10 w-10" />}
          title={t('skills.noResults')}
          description={t('skills.noResultsFor', { query: deferredQuery })}
        />
      ) : (
        <div className="grid gap-4">
          {skillsShResults!.map((s) => (
            <SkillsShCard
              key={s.id}
              skill={s}
              isSelected={selectedSkillsShId === s.id}
              isInstalling={skillsShMut.isPending}
              onSelect={() => onSelectSkillsShId(selectedSkillsShId === s.id ? null : s.id)}
              onInstall={(id) => skillsShMut.mutate(id)}
            />
          ))}
        </div>
      )}
    </>
  )
}

// ─── Skill Card (enhanced) ────────────────────────────────────

function SkillCard({
  skill,
  isSelected,
  hasUpdate,
  onSelect,
  onToggle,
  onDelete,
  onEdit,
  isToggling,
}: {
  skill: Skill
  isSelected: boolean
  hasUpdate?: boolean
  onSelect: () => void
  onToggle: () => void
  onDelete: () => void
  onEdit?: () => void
  isToggling: boolean
}) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(isSelected)
  const sd = STATUS_DISPLAY[skill.status]
  const isClaude = skill.format === 'claude_code'
  const isDisabled = skill.status === 'disabled'
  const hasMissing =
    skill.missing.bins.length > 0 ||
    skill.missing.anyBins.length > 0 ||
    skill.missing.env.length > 0 ||
    skill.missing.config.length > 0 ||
    (skill.missing.integrations?.length ?? 0) > 0 ||
    (skill.missing.anyIntegrations?.length ?? 0) > 0

  return (
    <Card
      className={cn(
        'transition-shadow hover:shadow-md cursor-pointer select-none',
        isSelected && 'ring-2 ring-primary',
      )}
      role="button"
      tabIndex={0}
      aria-expanded={expanded}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          onSelect()
          setExpanded((v) => !v)
        }
      }}
    >
      <CardContent className="p-5 space-y-3">
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-start gap-2 min-w-0" onClick={onSelect}>
            <span className="text-lg leading-none mt-0.5 shrink-0"> </span>
            <div className="min-w-0">
              <h3 className="font-semibold text-base leading-tight">{skill.name}</h3>
              {skill.description && (
                <p className="text-sm text-muted-foreground mt-0.5 line-clamp-2">
                  {skill.description}
                </p>
              )}
            </div>
          </div>
          <div className="flex items-center gap-2 shrink-0">
            {hasUpdate && (
              <Badge variant="outline" className="text-xs gap-1 border-warning/50 text-warning">
                {t('skills.updateAvailable')}
              </Badge>
            )}
            <FormatBadge format={skill.format} />
            {skill.always && (
              <Badge variant="outline" className="text-xs">
                {t('skills.always')}
              </Badge>
            )}
            <Badge variant={sd.variant} className="text-xs gap-1">
              {sd.icon} {sd.label}
            </Badge>
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation()
                setExpanded((v) => !v)
              }}
              className="ml-1 rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
              aria-label={expanded ? t('skills.collapse') : t('skills.expand')}
            >
              {expanded ? (
                <ChevronDown className="h-4 w-4" />
              ) : (
                <ChevronRight className="h-4 w-4" />
              )}
            </button>
          </div>
        </div>
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          {skill.version && <span className="font-mono">v{skill.version}</span>}
          <Badge variant={SOURCE_VARIANT[skill.source] ?? 'outline'} className="text-xs">
            {skill.source}
          </Badge>
          {skill.author && (
            <span>
              {t('skills.by')} {skill.author}
            </span>
          )}
        </div>
        {isClaude && (
          <div className="rounded-md bg-info/10 border border-info/20 px-3 py-2">
            <p className="text-xs text-info">{t('skills.claudeCompatible')}</p>
          </div>
        )}
        {expanded && (
          <>
            {(skill.requirements.bins.length > 0 ||
              skill.requirements.anyBins.length > 0 ||
              skill.requirements.env.length > 0 ||
              skill.requirements.config.length > 0) && (
              <div className="space-y-1.5">
                <p className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
                  {t('skills.requires')}
                </p>
                <div className="space-y-1 pl-2">
                  {skill.requirements.bins.length > 0 && (
                    <ReqRow
                      labelKey="skills.bins"
                      items={skill.requirements.bins}
                      missing={skill.missing.bins}
                    />
                  )}
                  {skill.requirements.anyBins.length > 0 && (
                    <ReqRow
                      labelKey="skills.anyBins"
                      items={skill.requirements.anyBins}
                      missing={skill.missing.anyBins}
                    />
                  )}
                  {skill.requirements.env.length > 0 && (
                    <ReqRow
                      labelKey="skills.env"
                      items={skill.requirements.env}
                      missing={skill.missing.env}
                    />
                  )}
                  {skill.requirements.config.length > 0 && (
                    <ReqRow
                      labelKey="skills.config"
                      items={skill.requirements.config}
                      missing={skill.missing.config}
                    />
                  )}
                </div>
              </div>
            )}
            {skill.install.length > 0 && (
              <div className="space-y-1.5">
                <p className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
                  {t('skills.install')}
                </p>
                <div className="pl-2 space-y-1">
                  {skill.install.map((sp, i) => (
                    <div
                      key={`${sp.kind}-${i}`}
                      className="flex items-center gap-2 text-sm text-muted-foreground"
                    >
                      <span className="text-xs font-mono bg-muted px-1.5 py-0.5 rounded">
                        {sp.kind}
                      </span>
                      <span>{sp.label ?? sp.bins.join(', ')}</span>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </>
        )}
        {hasMissing && skill.status === 'needs_setup' && (
          <div className="rounded-md bg-warning/10 border border-warning/20 px-3 py-2">
            <p className="text-xs text-warning">
              {t('skills.missingWarning', {
                missing: [
                  ...skill.missing.bins.map((b) => `bin:${b}`),
                  ...skill.missing.env.map((e) => `env:${e}`),
                  ...skill.missing.config.map((c) => `config:${c}`),
                  ...skill.missing.anyBins.map((b) => `any_bin:${b}`),
                  ...(skill.missing.integrations ?? []).map((i) => `integration:${i}`),
                  ...(skill.missing.anyIntegrations ?? []).map((i) => `any_integration:${i}`),
                ].join(', '),
              })}
            </p>
          </div>
        )}

        {/* Inline actions */}
        <div className="flex items-center gap-2 pt-2 border-t" onClick={(e) => e.stopPropagation()}>
          <Button
            variant={isDisabled ? 'default' : 'outline'}
            size="sm"
            onClick={onToggle}
            disabled={isToggling}
            className="gap-1.5"
          >
            <Power className="h-3.5 w-3.5" />
            {isDisabled ? t('skills.enable') : t('skills.disable')}
          </Button>
          {onEdit && (
            <Button variant="ghost" size="sm" onClick={onEdit} className="gap-1.5">
              <Pencil className="h-3.5 w-3.5" />
              {t('skills.edit')}
            </Button>
          )}
          <Button
            variant="ghost"
            size="sm"
            onClick={onDelete}
            className="gap-1.5 text-destructive hover:text-destructive"
          >
            <Trash2 className="h-3.5 w-3.5" />
            {t('skills.delete')}
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}

// ─── Marketplace Card (enhanced) ──────────────────────────────

function MarketplaceCard({
  skill,
  isSelected,
  isInstalling,
  onSelect,
  onInstall,
}: {
  skill: ClawHubSearchResult
  isSelected: boolean
  isInstalling: boolean
  onSelect: () => void
  onInstall: (s: string, v?: string) => void
}) {
  const { t } = useTranslation()
  const v = skill.version
  const dn = skill.displayName || skill.slug
  const rt = useMemo(() => {
    if (!skill.updatedAt) return null
    const d = Math.floor((Date.now() - skill.updatedAt) / 86_400_000)
    if (d === 0) return 'today'
    if (d === 1) return '1d ago'
    if (d < 30) return `${d}d ago`
    const w = Math.floor(d / 7)
    if (w < 4) return `${w}w ago`
    return `${Math.floor(d / 30)}mo ago`
  }, [skill.updatedAt])
  return (
    <Card
      className={cn(
        'transition-shadow hover:shadow-md cursor-pointer select-none',
        isSelected && 'ring-2 ring-primary',
      )}
    >
      <CardContent className="p-5 space-y-3">
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-start gap-2 min-w-0" onClick={onSelect}>
            <Search className="h-4 w-4 shrink-0" />
            <div className="min-w-0">
              <h3 className="font-semibold text-base leading-tight">{dn}</h3>
              {skill.summary && (
                <p className="text-sm text-muted-foreground mt-0.5 line-clamp-2">{skill.summary}</p>
              )}
            </div>
          </div>
          <div className="flex items-center gap-2 shrink-0">
            <FormatBadge format="openclaw" />
            {v && (
              <Badge variant="outline" className="text-xs font-mono">
                v{v}
              </Badge>
            )}
            <Button
              size="sm"
              onClick={(e) => {
                e.stopPropagation()
                onInstall(skill.slug, v)
              }}
              disabled={isInstalling}
              className="gap-1.5"
            >
              {t('skills.install')}
            </Button>
          </div>
        </div>
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <span className="font-mono text-muted-foreground/80">{skill.slug}</span>
          {rt && (
            <>
              <span>·</span>
              <span>{rt}</span>
            </>
          )}
        </div>
      </CardContent>
    </Card>
  )
}

// ─── Skills.sh Card ────────────────────────────────────────────

function SkillsShCard({
  skill,
  isSelected,
  isInstalling,
  onSelect,
  onInstall,
}: {
  skill: SkillsShSkill
  isSelected: boolean
  isInstalling: boolean
  onSelect: () => void
  onInstall: (id: string) => void
}) {
  const { t } = useTranslation()
  return (
    <Card
      className={cn(
        'transition-shadow hover:shadow-md cursor-pointer select-none',
        isSelected && 'ring-2 ring-primary',
      )}
    >
      <CardContent className="p-5 space-y-3">
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-start gap-2 min-w-0" onClick={onSelect}>
            <Globe className="h-4 w-4 shrink-0" />
            <div className="min-w-0">
              <h3 className="font-semibold text-base leading-tight">{skill.name}</h3>
              <p className="text-sm text-muted-foreground mt-0.5 line-clamp-2">{skill.source}</p>
            </div>
          </div>
          <div className="flex items-center gap-2 shrink-0">
            <Badge variant="secondary" className="text-xs">
              Skills.sh
            </Badge>
            <Badge variant="outline" className="text-xs font-mono">
              {skill.installs.toLocaleString()}
            </Badge>
            <Button
              size="sm"
              onClick={(e) => {
                e.stopPropagation()
                onInstall(skill.id)
              }}
              disabled={isInstalling}
              className="gap-1.5"
            >
              {t('skills.install')}
            </Button>
          </div>
        </div>
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <span className="font-mono text-muted-foreground/80">{skill.slug}</span>
          <span>·</span>
          <span>{skill.source}</span>
          {skill.sourceType === 'github' && (
            <Badge variant="outline" className="text-2xs px-1 py-0">
              GitHub
            </Badge>
          )}
          {skill.isDuplicate && (
            <Badge variant="outline" className="text-2xs px-1 py-0 border-warning/50 text-warning">
              fork
            </Badge>
          )}
        </div>
      </CardContent>
    </Card>
  )
}

// ─── Skills.sh Detail Panel ────────────────────────────────────────

function SkillsShDetail({ id, onClose }: { id: string; onClose: () => void }) {
  const { t } = useTranslation()
  const qc = useQueryClient()

  const { data, isLoading, isError } = useQuery({
    queryKey: ['skills-sh', 'detail', id],
    queryFn: async () => {
      const r = await api.get<{
        id: string
        source: string
        slug: string
        installs: number
        hash?: string
        files?: Array<{ path: string; contents: string }>
      }>(`/api/marketplace/skills-sh/skill/${encodeURIComponent(id)}`)
      return r
    },
    refetchOnWindowFocus: false,
  })

  const installMut = useMutation({
    mutationFn: () =>
      api.post(`/api/marketplace/skills-sh/skill/${encodeURIComponent(id)}/install`),
    onSuccess: () => {
      toast.success(t('skills.installSuccess', { slug: id }))
      qc.invalidateQueries({ queryKey: ['skills'] })
    },
    onError: (e: unknown) => {
      toast.error(e instanceof Error ? e.message : t('skills.installFailed'))
    },
  })

  if (isLoading) return <div className="text-sm text-muted-foreground">{t('common.loading')}</div>
  if (isError || !data)
    return <div className="text-sm text-destructive">{t('skills.loadDetailFailed')}</div>

  const skillMd = data.files?.find(
    (f) => f.path === 'SKILL.md' || f.path.toLowerCase() === 'skill.md',
  )

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="font-semibold text-lg">{data.slug}</h3>
        <Button variant="ghost" size="sm" onClick={onClose}>
          ✕
        </Button>
      </div>

      <div className="space-y-2">
        <div className="flex items-center gap-2 text-sm">
          <Badge variant="secondary">Skills.sh</Badge>
          <span className="text-muted-foreground">{data.source}</span>
        </div>
        <div className="text-sm text-muted-foreground">
          {data.installs.toLocaleString()} installs
          {data.hash && (
            <span className="ml-2 font-mono text-xs">({data.hash.slice(0, 8)}...)</span>
          )}
        </div>
      </div>

      {data.files && data.files.length > 0 && (
        <div className="space-y-1">
          <p className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
            Files
          </p>
          <div className="space-y-1">
            {data.files.map((f) => (
              <div key={f.path} className="text-xs font-mono bg-muted px-2 py-1 rounded">
                {f.path} <span className="text-muted-foreground">({f.contents.length} chars)</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {skillMd && (
        <div className="space-y-1">
          <p className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
            SKILL.md Preview
          </p>
          <pre className="text-xs bg-muted p-3 rounded-md overflow-auto max-h-64 whitespace-pre-wrap">
            {skillMd.contents.slice(0, 2000)}
            {skillMd.contents.length > 2000 ? '\n...' : '' /* icon fallback */}
          </pre>
        </div>
      )}

      <Button
        className="w-full"
        onClick={() => installMut.mutate()}
        disabled={installMut.isPending}
      >
        {t('skills.install')}
      </Button>
    </div>
  )
}

// ─── Helpers ─────────────────────────────────────────────────

function ReqRow({
  labelKey,
  items,
  missing,
}: {
  labelKey: string
  items: string[]
  missing: string[]
}) {
  const { t } = useTranslation()
  return (
    <div className="flex items-start gap-2 text-xs">
      <span className="text-muted-foreground w-16 shrink-0 pt-px">{t(labelKey)}</span>
      <div className="flex flex-wrap gap-x-3 gap-y-0.5">
        {items.map((item) => {
          const m = missing.includes(item)
          return (
            <span
              key={item}
              className={cn('flex items-center gap-1', m ? 'text-error' : 'text-success')}
            >
              {m ? <X className="h-3 w-3" /> : <Check className="h-3 w-3" />}
              {item}
              {m && <span className="text-error">{t('skills.missing')}</span>}
            </span>
          )
        })}
      </div>
    </div>
  )
}
