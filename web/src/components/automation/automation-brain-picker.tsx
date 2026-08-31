// AutomationBrainPicker — explicit brain-space binding for an automation.
//
// Persisted on the definition as `brainSpace: string | null`. Selection is
// independent of the active chat session — operators choose which brain
// space the automation reads/writes when it fires. The picker hides itself
// when the brain daemon has no spaces (degraded / unconfigured).

import { useQuery } from '@tanstack/react-query'
import { Brain, Check, ChevronDown } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { api } from '@/lib/api-client'
import { cn } from '@/lib/utils'

export interface AutomationBrainPickerProps {
  /** Brain-space roster (passed in — caller already loaded it). */
  spaces: Array<{ id: string; name: string }>
  /** Persisted automation binding by space NAME; `null` = no binding. */
  value: string | null
  onChange: (name: string | null) => void
  /** Test/aria label. */
  ariaLabel?: string
}

export function AutomationBrainPicker({
  spaces,
  value,
  onChange,
  ariaLabel,
}: AutomationBrainPickerProps) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const named = spaces.filter((s) => !!s.name)
  const active = named.find((s) => s.name === value) ?? null

  // Stale binding normalization: if a rehydrated value names a space that
  // no longer exists, we still render the pill but show "off" rather than
  // a dead name (mirrors BrainPickerContainer's behavior).
  const bound = named.some((s) => s.name === value) ? value : null

  if (named.length === 0) return null

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={ariaLabel ?? t('automations.brainPickerLabel')}
          data-testid="automation-brain-picker"
          className={cn(
            'flex h-8 items-center gap-1 rounded-md border bg-card px-2 text-xs font-medium text-muted-foreground',
            'transition-colors hover:bg-accent hover:text-foreground',
            active && 'border-primary/40 text-foreground',
          )}
        >
          <Brain className="h-3.5 w-3.5" />
          <span className="max-w-40 truncate">
            {active ? active.name : t('automations.noBrain')}
          </span>
          <ChevronDown className="h-3 w-3 opacity-60" />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-72 p-0" collisionPadding={8}>
        <div className="border-b px-2 py-1.5 text-xs font-medium text-muted-foreground">
          {t('automations.brainPickerTitle')}
        </div>
        <div className="max-h-80 overflow-y-auto p-1">
          <button
            type="button"
            onClick={() => {
              onChange(null)
              setOpen(false)
            }}
            data-testid="automation-brain-picker-none"
            className={cn(
              'flex w-full items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-left text-sm',
              'hover:bg-accent',
              bound === null && 'bg-accent/60',
            )}
          >
            <span className="text-muted-foreground italic">{t('automations.noBrain')}</span>
            {bound === null && <Check className="h-3.5 w-3.5 shrink-0" />}
          </button>
          {named.map((s) => (
            <button
              key={s.id}
              type="button"
              onClick={() => {
                onChange(s.name)
                setOpen(false)
              }}
              data-testid={`automation-brain-picker-option-${s.id}`}
              className={cn(
                'flex w-full items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-left text-sm',
                'hover:bg-accent',
                s.name === bound && 'bg-accent/60',
              )}
            >
              <span className="truncate">{s.name}</span>
              {s.name === bound && <Check className="h-3.5 w-3.5 shrink-0" />}
            </button>
          ))}
        </div>
      </PopoverContent>
    </Popover>
  )
}

/** Loads the brain-space roster from `/api/brain/spaces`. */
export function useAutomationBrainSpaces() {
  return useQuery({
    queryKey: ['brain', 'spaces'],
    queryFn: async (): Promise<Array<{ id: string; name: string }>> => {
      const res = await api.get<
        Array<{ id: string; name: string }> | { items: Array<{ id: string; name: string }> }
      >('/api/brain/spaces')
      // The endpoint returns JSON `null` when the brain daemon is absent;
      // dereferencing null would throw inside the queryFn and poison the
      // shared ['brain','spaces'] cache for useBrainSpaces consumers.
      if (Array.isArray(res)) return res
      return res?.items ?? []
    },
    staleTime: 60_000,
  })
}
