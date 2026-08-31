// AutomationPersonaPicker — explicit persona binding for an automation.
//
// Persisted on the definition as `personaId: string | null` (null = global
// default). Selection is independent of the active chat session — operators
// choose the persona that will run the automation when it fires.

import { useQuery } from '@tanstack/react-query'
import { Check, ChevronDown, Sparkles } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { api } from '@/lib/api-client'
import { cn } from '@/lib/utils'

export interface AutomationPersonaPickerProps {
  /** Persona roster (passed in — caller already loaded it). */
  personas: Array<{ id: string; name: string; description?: string }>
  /** Persisted automation binding; `null` = no explicit persona. */
  value: string | null
  onChange: (id: string | null) => void
  /** Test/aria label. */
  ariaLabel?: string
}

export function AutomationPersonaPicker({
  personas,
  value,
  onChange,
  ariaLabel,
}: AutomationPersonaPickerProps) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const enabled = personas.filter((p) => p !== null)
  const active = enabled.find((p) => p.id === value) ?? null

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={ariaLabel ?? t('automations.personaPickerLabel')}
          data-testid="automation-persona-picker"
          className={cn(
            'flex h-8 items-center gap-1 rounded-md border bg-card px-2 text-xs font-medium text-muted-foreground',
            'transition-colors hover:bg-accent hover:text-foreground',
            active && 'border-primary/40 text-foreground',
          )}
        >
          <Sparkles className="h-3.5 w-3.5" />
          <span className="max-w-40 truncate">
            {active ? active.name : t('automations.noPersona')}
          </span>
          <ChevronDown className="h-3 w-3 opacity-60" />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-72 p-0" collisionPadding={8}>
        <div className="border-b px-2 py-1.5 text-xs font-medium text-muted-foreground">
          {t('automations.personaPickerTitle')}
        </div>
        <div className="max-h-80 overflow-y-auto p-1">
          <button
            type="button"
            onClick={() => {
              onChange(null)
              setOpen(false)
            }}
            data-testid="automation-persona-picker-none"
            className={cn(
              'flex w-full items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-left text-sm',
              'hover:bg-accent',
              value === null && 'bg-accent/60',
            )}
          >
            <span className="text-muted-foreground italic">{t('automations.noPersona')}</span>
            {value === null && <Check className="h-3.5 w-3.5 shrink-0" />}
          </button>
          {enabled.map((p) => (
            <button
              key={p.id}
              type="button"
              onClick={() => {
                onChange(p.id)
                setOpen(false)
              }}
              data-testid={`automation-persona-picker-option-${p.id}`}
              className={cn(
                'flex w-full items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-left text-sm',
                'hover:bg-accent',
                p.id === value && 'bg-accent/60',
              )}
            >
              <span className="flex min-w-0 flex-col">
                <span className="truncate">{p.name}</span>
                {p.description && (
                  <span className="truncate text-xs text-muted-foreground">{p.description}</span>
                )}
              </span>
              {p.id === value && <Check className="h-3.5 w-3.5 shrink-0" />}
            </button>
          ))}
          {enabled.length === 0 && (
            <div className="px-2 py-3 text-center text-xs text-muted-foreground">
              {t('automations.noPersonas')}
            </div>
          )}
        </div>
      </PopoverContent>
    </Popover>
  )
}

/** Loads the persona roster from `/api/personas`. */
export function useAutomationPersonaRoster() {
  return useQuery({
    queryKey: ['personas'],
    queryFn: async (): Promise<Array<{ id: string; name: string; description?: string }>> => {
      const res = await api.get<
        | Array<{ id: string; name: string; description?: string }>
        | { items: Array<{ id: string; name: string; description?: string }> }
      >('/api/personas')
      if (Array.isArray(res)) return res
      return res.items ?? []
    },
    staleTime: 60_000,
  })
}
