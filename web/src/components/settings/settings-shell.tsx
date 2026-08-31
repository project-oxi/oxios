import { ChevronRight, Menu, Settings2 } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'
import { cn } from '@/lib/utils'
import { useUiPrefs } from '@/stores/ui-prefs'
import { type RailGroup, SettingsRail } from './settings-rail'

export interface SettingsShellSection {
  id: string
  labelKey: string
  /** Group id this section belongs to. */
  groupId: string
}

export interface SettingsShellGroup {
  id: string
  labelKey: string
}

interface SettingsShellProps {
  groups: SettingsShellGroup[]
  sections: SettingsShellSection[]
  activeId: string
  onNavigate: (id: string) => void
  /** Map of sectionId → number of unsaved changes (drives the rail dot/badge). */
  unsavedBySection: Record<string, number>
  /**
   * Open the save review flow. When provided, ⌘S / Ctrl+S triggers it.
   * Omit (or pass undefined) when there is nothing to save.
   */
  onReview?: () => void
  /**
   * Optional pre-rendered section header. Most callers render this
   * themselves above `<SettingsShell>` so the page can control its
   * outer padding. Defaults to `null`.
   */
  children: React.ReactNode
}

/**
 * 3-zone layout container for the `/settings` route.
 *
 * ```
 * ┌──────────┬───────────────────────────────────┐
 * │          │                                   │
 * │   Rail   │  Children (section cards)         │
 * │          │                                   │
 * │          │                                   │
 * └──────────┴───────────────────────────────────┘
 * ```
 *
 * The rail is the **single** navigation surface — it lists every
 * section grouped by category, and the active item stays visible via
 * auto `scrollIntoView`. A previous top "section tabs" bar was
 * removed because it duplicated the rail's content.
 */
export function SettingsShell({
  groups,
  sections,
  activeId,
  onNavigate,
  unsavedBySection,
  onReview,
  children,
}: SettingsShellProps) {
  const { t } = useTranslation()
  const { showAdvancedSettings, setShowAdvancedSettings } = useUiPrefs()
  const [searchQuery, setSearchQuery] = useState('')
  const searchInputRef = useRef<HTMLInputElement>(null)

  // Flat ordered list of section ids — used for j/k navigation.
  const orderedSectionIds = useMemo(() => sections.map((s) => s.id), [sections])

  // Build rail groups from flat (group, sections) lists.
  // The 'advanced' group is hidden when showAdvancedSettings is false.
  const railGroups: RailGroup[] = useMemo(() => {
    return groups
      .filter((g) => showAdvancedSettings || g.id !== 'advanced')
      .map((g) => ({
        id: g.id,
        labelKey: g.labelKey,
        items: sections
          .filter((s) => s.groupId === g.id)
          .map((s) => ({
            id: s.id,
            labelKey: s.labelKey,
            status:
              (unsavedBySection[s.id] ?? 0) > 0 ? ('modified' as const) : ('default' as const),
            badge: unsavedBySection[s.id],
          })),
      }))
  }, [groups, sections, unsavedBySection, showAdvancedSettings])

  // Reset search when active section changes (mobile drawer UX).
  useEffect(() => {
    setSearchQuery('')
  }, [activeId])

  // ── Keyboard shortcuts ────────────────────────────────────────
  //
  //  ⌘K / Ctrl+K  → focus the search input
  //  ⌘S / Ctrl+S  → open the save review flow (when there are changes)
  //  j / k        → next / previous section
  //  g g          → first section
  //  G            → last section
  //
  // j / k / g / G are suppressed while typing in a form field so they
  // don't hijack text entry. The modifier shortcuts (⌘K, ⌘S) work
  // everywhere.
  const navigateByOffset = useCallback(
    (offset: number) => {
      const idx = orderedSectionIds.indexOf(activeId)
      if (idx === -1) return
      const next = orderedSectionIds[idx + offset]
      if (next) onNavigate(next)
    },
    [orderedSectionIds, activeId, onNavigate],
  )

  const pendingGRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey
      // ⌘K / Ctrl+K → focus search.
      if (mod && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        searchInputRef.current?.focus()
        searchInputRef.current?.select()
        return
      }
      // ⌘S / Ctrl+S → review.
      if (mod && e.key.toLowerCase() === 's') {
        e.preventDefault()
        onReview?.()
        return
      }
      // Ignore the single-key shortcuts while typing or using a modifier.
      const target = e.target as HTMLElement | null
      const isTyping =
        !!target &&
        (target.tagName === 'INPUT' ||
          target.tagName === 'TEXTAREA' ||
          target.tagName === 'SELECT' ||
          target.isContentEditable)
      if (isTyping || mod || e.altKey) return

      const key = e.key
      if (key === 'j') {
        e.preventDefault()
        navigateByOffset(1)
      } else if (key === 'k') {
        e.preventDefault()
        navigateByOffset(-1)
      } else if (key === 'G') {
        e.preventDefault()
        const last = orderedSectionIds[orderedSectionIds.length - 1]
        if (last) onNavigate(last)
      } else if (key === 'g') {
        e.preventDefault()
        // Two `g` presses within 700ms → first section. A single `g`
        // starts the pending timer; a second `g` cancels it and jumps.
        if (pendingGRef.current) {
          clearTimeout(pendingGRef.current)
          pendingGRef.current = null
          const first = orderedSectionIds[0]
          if (first) onNavigate(first)
        } else {
          pendingGRef.current = setTimeout(() => {
            pendingGRef.current = null
          }, 700)
        }
      }
    }
    window.addEventListener('keydown', handler)
    return () => {
      window.removeEventListener('keydown', handler)
      if (pendingGRef.current) clearTimeout(pendingGRef.current)
    }
  }, [navigateByOffset, onReview, onNavigate, orderedSectionIds])

  return (
    <div className="flex flex-col md:flex-row gap-6">
      {/* Desktop rail (spec §5): visible from `md` (768px) up, widening
          across three tiers — 200px → 240px → 280px. Below `md` the
          rail is hidden behind the `MobileSheet` drawer. */}
      <aside
        className={cn('hidden md:block shrink-0', 'w-[200px] lg:w-[240px] xl:w-[280px]')}
        aria-label={t('settings.title')}
      >
        <div className="sticky top-[5.5rem] max-h-[calc(100vh-6rem)] flex flex-col">
          <div className="flex min-h-0 flex-1 flex-col">
            <SettingsRail
              groups={railGroups}
              activeId={activeId}
              onNavigate={onNavigate}
              searchQuery={searchQuery}
              onSearchChange={setSearchQuery}
              searchInputRef={searchInputRef}
            />
          </div>
          {/* Advanced settings toggle */}
          <button
            type="button"
            onClick={() => setShowAdvancedSettings(!showAdvancedSettings)}
            className="flex items-center gap-2 rounded-md px-2.5 py-2 text-xs text-muted-foreground hover:bg-muted/60 hover:text-foreground transition-colors"
          >
            <Settings2 className="h-3.5 w-3.5" />
            <span className="flex-1 text-left">{t('settings.groupAdvanced')}</span>
            <ChevronRight
              className={cn(
                'h-3.5 w-3.5 transition-transform',
                showAdvancedSettings && 'rotate-90',
              )}
            />
          </button>
        </div>
      </aside>

      {/* Mobile drawer trigger — only below `md` (phones). */}
      <div className="md:hidden w-full">
        <MobileSheet
          railGroups={railGroups}
          activeId={activeId}
          onNavigate={onNavigate}
          searchQuery={searchQuery}
          onSearchChange={setSearchQuery}
        />
      </div>

      {/* Content */}
      <div className="min-w-0 flex-1">
        <div className="space-y-4 animate-stagger">{children}</div>
      </div>
    </div>
  )
}

// ─── Mobile sheet (drawer) ───────────────────────────────────────

function MobileSheet({
  railGroups,
  activeId,
  onNavigate,
  searchQuery,
  onSearchChange,
}: {
  railGroups: RailGroup[]
  activeId: string
  onNavigate: (id: string) => void
  searchQuery: string
  onSearchChange: (q: string) => void
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const active = railGroups.flatMap((g) => g.items).find((i) => i.id === activeId)
  const activeLabel = active?.label ?? (active ? t(active.labelKey) : '')

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <div className="flex items-center gap-2 mb-3">
        <DialogTrigger asChild>
          <Button variant="outline" size="sm" className="gap-2">
            <Menu className="h-3.5 w-3.5" />
            {t('settings.title')}
          </Button>
        </DialogTrigger>
        <span className="text-sm text-muted-foreground truncate">{activeLabel}</span>
      </div>
      <DialogContent
        showCloseButton={false}
        className="max-w-sm p-0 gap-0 sm:rounded-xl left-0 top-0 translate-x-0 translate-y-0 h-screen max-h-screen w-72 sm:w-72 rounded-none border-r"
      >
        <DialogHeader className="px-4 pt-4 pb-2 border-b">
          <DialogTitle>{t('settings.title')}</DialogTitle>
        </DialogHeader>
        <div className="flex flex-col px-3 pt-3 pb-4 h-[calc(100vh-4rem)]">
          <SettingsRail
            groups={railGroups}
            activeId={activeId}
            onNavigate={(id) => {
              onNavigate(id)
              setOpen(false)
            }}
            searchQuery={searchQuery}
            onSearchChange={onSearchChange}
          />
        </div>
      </DialogContent>
    </Dialog>
  )
}
