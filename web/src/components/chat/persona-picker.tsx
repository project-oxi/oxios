import { useQuery } from '@tanstack/react-query'
import { Check, ChevronDown, Sparkles } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { api } from '@/lib/api-client'
import { cn } from '@/lib/utils'
import { useChatStore } from '@/stores/chat'
import type { Persona } from '@/types'

// ─── Category taxonomy ────────────────────────────────────────

/** Display order for known categories; unknown values fall into 'other'. */
export const CATEGORY_ORDER = [
  'normal',
  'coding',
  'writing',
  'research',
  'operations',
  'general',
] as const

const CATEGORY_LABEL_KEYS: Record<string, string> = {
  normal: 'chat.persona.categories.normal',
  coding: 'chat.persona.categories.coding',
  writing: 'chat.persona.categories.writing',
  research: 'chat.persona.categories.research',
  operations: 'chat.persona.categories.operations',
  general: 'chat.persona.categories.general',
}

const GENRE_LABEL_KEYS: Record<string, string> = {
  novel: 'chat.persona.genres.novel',
  scenario: 'chat.persona.genres.scenario',
  essay: 'chat.persona.genres.essay',
  blog: 'chat.persona.genres.blog',
}

/** Group personas by category in display order; unknown categories last. */
export function groupByCategory<T extends { category?: string | null }>(
  personas: T[],
): Array<{ category: string; labelKey: string; items: T[] }> {
  const buckets = new Map<string, T[]>()
  for (const p of personas) {
    const cat = p.category ?? 'general'
    const list = buckets.get(cat) ?? []
    list.push(p)
    buckets.set(cat, list)
  }
  const groups: Array<{ category: string; labelKey: string; items: T[] }> = []
  for (const cat of CATEGORY_ORDER) {
    const items = buckets.get(cat)
    if (items?.length)
      groups.push({ category: cat, labelKey: CATEGORY_LABEL_KEYS[cat] ?? '', items })
    buckets.delete(cat)
  }
  // Unknown/user-defined categories render under their raw name, last.
  for (const [cat, items] of buckets) {
    groups.push({ category: cat, labelKey: '', items })
  }
  return groups
}

/**
 * Merge a persona's `default_mount_ids` into the current mount selection.
 * Pure helper (unit-tested): keeps existing selection order (primary first),
 * appends missing defaults, de-duplicates, drops empties.
 */
export function mergeMountIds(current: string | null, defaults: string[]): string {
  const existing = (current ?? '')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)
  const merged = [...existing]
  for (const id of defaults) {
    if (id && !merged.includes(id)) merged.push(id)
  }
  return merged.join(',')
}

// ─── Props ────────────────────────────────────────────────────

export interface PersonaPickerProps {
  personas: Persona[]
  /** Global active persona name (shown on the "auto" row). */
  globalPersonaName: string | null
  activePersonaId: string | null
  setActivePersona: (id: string | null) => void
  /**
   * Called after a persona with `default_mount_ids` is selected so the
   * caller can attach mounts (composer chips). Receives the merged ids.
   */
  onAutoAttachMounts?: (mergedMountIds: string) => void
  currentMountIds?: string | null
}

// ─── Component ────────────────────────────────────────────────

/**
 * PersonaPicker — the chat-input pill for session-scoped persona selection.
 *
 * The first row ("auto") clears the override so the turn inherits the
 * global active persona (set on the personas page / command palette).
 * Rows are grouped by category with genre badges for writing personas.
 * Keep the surface small: no editing here — management stays on the
 * personas page.
 */
export function PersonaPicker({
  personas,
  globalPersonaName,
  activePersonaId,
  setActivePersona,
  onAutoAttachMounts,
  currentMountIds,
}: PersonaPickerProps) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const listRef = useRef<HTMLDivElement>(null)

  const enabled = useMemo(() => personas.filter((p) => p.enabled !== false), [personas])
  const groups = useMemo(() => groupByCategory(enabled), [enabled])
  const active = enabled.find((p) => p.id === activePersonaId) ?? null

  // Reset scroll on open — the picker is short-lived per interaction.
  useEffect(() => {
    if (open) listRef.current?.scrollTo({ top: 0 })
  }, [open])

  const pick = (persona: Persona | null) => {
    setActivePersona(persona?.id ?? null)
    if (persona?.default_mount_ids?.length && onAutoAttachMounts) {
      onAutoAttachMounts(mergeMountIds(currentMountIds ?? null, persona.default_mount_ids))
    }
    setOpen(false)
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={t('chat.persona.label')}
          className={cn(
            'flex h-7 items-center gap-1 rounded-md px-2 text-xs font-medium text-muted-foreground',
            'transition-colors hover:bg-accent hover:text-foreground',
            activePersonaId && 'bg-accent text-foreground',
          )}
        >
          <Sparkles className="h-3.5 w-3.5" />
          <span className="max-w-28 truncate">
            {active ? active.name : t('chat.persona.auto')}
          </span>
          <ChevronDown className="h-3 w-3 opacity-60" />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-64 p-0" collisionPadding={8}>
        <div className="border-b px-2 py-1.5 text-xs font-medium text-muted-foreground">
          {t('chat.persona.label')}
        </div>
        <div ref={listRef} className="max-h-80 overflow-y-auto p-1">
          <button
            type="button"
            onClick={() => pick(null)}
            className={cn(
              'flex w-full items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-left text-sm',
              'hover:bg-accent',
              !activePersonaId && 'bg-accent/60',
            )}
          >
            <span className="flex flex-col">
              <span>{t('chat.persona.auto')}</span>
              {globalPersonaName && (
                <span className="text-xs text-muted-foreground">
                  {t('chat.persona.globalIs', { name: globalPersonaName })}
                </span>
              )}
            </span>
            {!activePersonaId && <Check className="h-3.5 w-3.5 shrink-0" />}
          </button>

          {groups.map((group) => (
            <div key={group.category} className="mt-1">
              <div className="px-2 py-1 text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
                {group.labelKey ? t(group.labelKey) : group.category}
              </div>
              {group.items.map((p) => {
                const selected = p.id === activePersonaId
                const genreKey = p.genre != null ? GENRE_LABEL_KEYS[p.genre] : undefined
                return (
                  <button
                    key={p.id}
                    type="button"
                    onClick={() => pick(p)}
                    className={cn(
                      'flex w-full items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-left text-sm',
                      'hover:bg-accent',
                      selected && 'bg-accent/60',
                    )}
                  >
                    <span className="flex min-w-0 flex-col">
                      <span className="truncate">{p.name}</span>
                      {p.description && (
                        <span className="truncate text-xs text-muted-foreground">
                          {p.description}
                        </span>
                      )}
                    </span>
                    <span className="flex shrink-0 items-center gap-1">
                      {genreKey && (
                        <span className="rounded bg-info-muted px-1.5 py-0.5 text-2xs font-medium text-info">
                          {t(genreKey)}
                        </span>
                      )}
                      {selected && <Check className="h-3.5 w-3.5" />}
                    </span>
                  </button>
                )
              })}
            </div>
          ))}

          {enabled.length === 0 && (
            <div className="px-2 py-3 text-center text-xs text-muted-foreground">
              {t('chat.persona.none')}
            </div>
          )}
        </div>
      </PopoverContent>
    </Popover>
  )
}

// ─── Container ────────────────────────────────────────────────

interface ActivePersonaName {
  id: string
  name?: string
}

/**
 * Resolves the persona roster + global active name + chat-store selection
 * and forwards them to [`PersonaPicker`]. Selecting a persona here is
 * session-scoped (WS `persona_id`); the global default stays on the
 * personas page / command palette.
 */
export function PersonaPickerContainer() {
  const { t } = useTranslation()
  const activePersonaId = useChatStore((s) => s.activePersonaId)
  const setActivePersona = useChatStore((s) => s.setActivePersona)
  const activeMountIds = useChatStore((s) => s.activeMountIds)

  const personasQuery = useQuery({
    queryKey: ['personas'],
    queryFn: () => api.get<Persona[]>('/api/personas'),
    staleTime: 60_000,
  })

  const activePersonaQuery = useQuery({
    queryKey: ['persona', 'active'],
    queryFn: async (): Promise<ActivePersonaName | null> => {
      try {
        return await api.get<ActivePersonaName>('/api/personas/active')
      } catch {
        return null
      }
    },
    staleTime: 60_000,
    retry: false,
  })

  const personas = Array.isArray(personasQuery.data) ? personasQuery.data : []

  return (
    <PersonaPicker
      personas={personas}
      globalPersonaName={activePersonaQuery.data?.name ?? null}
      activePersonaId={activePersonaId}
      setActivePersona={setActivePersona}
      currentMountIds={activeMountIds}
      onAutoAttachMounts={(merged) => {
        if (merged === (activeMountIds ?? '')) return
        useChatStore.setState({ activeMountIds: merged })
        toast.info(t('chat.persona.mountsAttached'))
      }}
    />
  )
}
