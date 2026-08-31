import { useQuery } from '@tanstack/react-query'
import { Check, ChevronDown, Sparkles } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { api } from '@/lib/api-client'
import { cn } from '@/lib/utils'
import { useChatStore } from '@/stores/chat'
import type { Persona } from '@/types'

// ─── Props ────────────────────────────────────────────────────

export interface PersonaPickerProps {
  personas: Persona[]
  /** Global active persona name (shown on the "auto" row). */
  globalPersonaName: string | null
  activePersonaId: string | null
  setActivePersona: (id: string | null) => void
}

// ─── Component ────────────────────────────────────────────────

/**
 * PersonaPicker — the chat-input pill for session-scoped persona selection.
 *
 * The first row ("auto") clears the override so the turn inherits the
 * global active persona (set on the personas page / command palette).
 * Rows are rendered as a flat list in roster order. Keep the surface
 * small: no editing here — management stays on the personas page.
 */
export function PersonaPicker({
  personas,
  globalPersonaName,
  activePersonaId,
  setActivePersona,
}: PersonaPickerProps) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const listRef = useRef<HTMLDivElement>(null)
  const enabled = useMemo(() => personas.filter((p) => p.enabled !== false), [personas])
  const active = enabled.find((p) => p.id === activePersonaId) ?? null

  // Reset scroll on open — the picker is short-lived per interaction.
  useEffect(() => {
    if (open) listRef.current?.scrollTo({ top: 0 })
  }, [open])

  const pick = (persona: Persona | null) => {
    setActivePersona(persona?.id ?? null)
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
          {/* Show the effective persona: session override, else the global
              active persona. "Auto" was misleading — there is no detection. */}
          <span className="max-w-28 truncate">
            {active ? active.name : (globalPersonaName ?? t('chat.persona.auto'))}
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
              <span>{t('chat.persona.globalDefault')}</span>
              {globalPersonaName && (
                <span className="text-xs text-muted-foreground">
                  {t('chat.persona.globalIs', { name: globalPersonaName })}
                </span>
              )}
            </span>
            {!activePersonaId && <Check className="h-3.5 w-3.5 shrink-0" />}
          </button>

          {enabled.map((p) => {
            const selected = p.id === activePersonaId
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
                    <span className="truncate text-xs text-muted-foreground">{p.description}</span>
                  )}
                </span>
                {selected && <Check className="h-3.5 w-3.5 shrink-0" />}
              </button>
            )
          })}

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
  const activePersonaId = useChatStore((s) => s.activePersonaId)
  const setActivePersona = useChatStore((s) => s.setActivePersona)

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
    />
  )
}
