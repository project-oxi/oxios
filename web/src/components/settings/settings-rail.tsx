import { Search, SearchX, X } from 'lucide-react'
import { type RefObject, useEffect, useMemo, useRef } from 'react'
import { useTranslation } from 'react-i18next'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'

export interface RailItem {
  id: string
  labelKey: string
  /** Optional pre-translated label (overrides labelKey). */
  label?: string
  /** Optional badge count, e.g. number of unsaved changes. */
  badge?: number
  /** Optional status: 'modified' shows a small dot. */
  status?: 'modified' | 'default'
}

export interface RailGroup {
  id: string
  labelKey: string
  /** Optional pre-translated group label. */
  label?: string
  items: RailItem[]
}

interface SettingsRailProps {
  groups: RailGroup[]
  activeId: string
  onNavigate: (id: string) => void
  searchQuery: string
  onSearchChange: (q: string) => void
  /** Optional external ref to the search input (used by ⌘K shortcut). */
  searchInputRef?: RefObject<HTMLInputElement | null>
}

/**
 * Left rail navigation. Renders a search box at the top followed by
 * grouped navigation items. Items can carry an optional badge (e.g. the
 * number of unsaved changes in that section) and a "modified" status
 * dot.
 *
 * The component is `position: sticky` on `lg+`. Below `lg`, callers are
 * expected to render the rail inside a `Sheet`/`Dialog` drawer.
 */
export function SettingsRail({
  groups,
  activeId,
  onNavigate,
  searchQuery,
  onSearchChange,
  searchInputRef,
}: SettingsRailProps) {
  const { t } = useTranslation()
  const internalRef = useRef<HTMLInputElement>(null)
  const searchRef = searchInputRef ?? internalRef

  const filteredGroups = useMemo(
    () => filterGroups(groups, searchQuery, t),
    [groups, searchQuery, t],
  )

  const noMatches = searchQuery.trim().length > 0 && filteredGroups.length === 0

  // Keep the active item scrolled into view inside the rail. Without
  // this, j/k navigation can land on an item hidden below the fold of
  // the scrollable rail.
  const activeItemRef = useRef<HTMLButtonElement>(null)
  useEffect(() => {
    const el = activeItemRef.current
    if (!el) return
    el.scrollIntoView({ block: 'nearest' })
  }, [activeId])

  return (
    <nav aria-label={t('settings.title')} className="flex w-full min-h-0 flex-1 flex-col gap-2">
      {/* Search */}
      <div className="relative">
        <Search
          className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground"
          aria-hidden
        />
        <Input
          ref={searchRef}
          type="search"
          value={searchQuery}
          onChange={(e) => onSearchChange(e.target.value)}
          placeholder={t('settings.searchPlaceholder')}
          className="h-8 pl-8 pr-7 text-xs"
          aria-label={t('settings.searchPlaceholder')}
        />
        {searchQuery && (
          <button
            type="button"
            onClick={() => {
              onSearchChange('')
              searchRef.current?.focus()
            }}
            aria-label={t('common.clear')}
            className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground transition-colors"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        )}
      </div>

      {/* Groups */}
      <div className="flex-1 overflow-y-auto -mx-1 px-1 pb-4 space-y-0.5">
        {noMatches ? (
          <div className="flex flex-col items-center justify-center gap-2 px-3 py-8 text-center text-muted-foreground">
            <SearchX className="h-5 w-5 opacity-60" />
            <p className="text-xs">{t('settings.noMatches', { query: searchQuery })}</p>
          </div>
        ) : (
          filteredGroups.map((group) => (
            <div key={group.id} className="pt-3 first:pt-1">
              <div className="px-2.5 mb-1">
                <span className="text-2xs font-semibold uppercase tracking-wider text-muted-foreground/70">
                  {group.label ?? t(group.labelKey)}
                </span>
              </div>
              {group.items.map((item) => {
                const isActive = item.id === activeId
                const isModified = item.status === 'modified'
                return (
                  <button
                    key={item.id}
                    ref={isActive ? activeItemRef : null}
                    type="button"
                    onClick={() => onNavigate(item.id)}
                    aria-current={isActive ? 'page' : undefined}
                    data-section-id={item.id}
                    data-modified={isModified ? 'true' : undefined}
                    className={cn(
                      'group/rail-item relative flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-sm transition-colors',
                      isActive
                        ? 'bg-primary/10 text-primary font-medium'
                        : 'text-foreground/70 hover:bg-muted/60 hover:text-foreground',
                    )}
                  >
                    {/* Active indicator bar */}
                    {isActive && (
                      <span
                        aria-hidden
                        className="absolute left-0 top-1.5 bottom-1.5 w-0.5 rounded-full bg-primary"
                      />
                    )}
                    {/* Modified dot (when not active) */}
                    {isModified && !isActive && (
                      <span
                        aria-hidden
                        className="absolute left-1.5 top-1/2 -translate-y-1/2 h-1.5 w-1.5 rounded-full bg-primary"
                      />
                    )}
                    <span className="flex-1 truncate pl-1.5">{item.label ?? t(item.labelKey)}</span>
                    {typeof item.badge === 'number' && item.badge > 0 && (
                      <span
                        className={cn(
                          'ml-auto inline-flex h-4 min-w-4 items-center justify-center rounded-full px-1.5 text-2xs font-semibold tabular-nums',
                          isActive
                            ? 'bg-primary text-primary-foreground'
                            : 'bg-muted text-muted-foreground group-hover/rail-item:bg-background',
                        )}
                      >
                        {item.badge}
                      </span>
                    )}
                  </button>
                )
              })}
            </div>
          ))
        )}
      </div>
    </nav>
  )
}

// ─── Search filter ───────────────────────────────────────────────

function filterGroups(groups: RailGroup[], query: string, t: (k: string) => string): RailGroup[] {
  const q = query.trim().toLowerCase()
  if (!q) return groups
  const out: RailGroup[] = []
  for (const g of groups) {
    const groupLabel = (g.label ?? t(g.labelKey)).toLowerCase()
    const items = g.items.filter((i) => {
      const itemLabel = (i.label ?? t(i.labelKey)).toLowerCase()
      return itemLabel.includes(q) || groupLabel.includes(q)
    })
    if (items.length > 0) {
      out.push({ ...g, items })
    }
  }
  return out
}
