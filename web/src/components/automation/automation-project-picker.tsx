// AutomationProjectPicker — explicit project binding for an automation.
//
// The first row ("No project") is a required option — automation bindings
// are persisted on the definition (projectId: null = projectless), NOT
// inferred from the active Studio conversation. This selector renders
// only the persisted choices; the chat-inferred selection is irrelevant.

import { useQuery } from '@tanstack/react-query'
import { Check, ChevronDown, Folder } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { api } from '@/lib/api-client'
import { cn } from '@/lib/utils'

export interface AutomationProjectPickerProps {
  /** Project roster (passed in — caller already loaded it). */
  projects: Array<{ id: string; name: string }>
  /** Persisted automation binding; `null` = projectless. */
  value: string | null
  onChange: (id: string | null) => void
  /** Test/aria label. */
  ariaLabel?: string
}

/**
 * Renders an inline picker pill that lists every persisted project plus a
 * leading "No project" option. Selecting a project calls `onChange` with
 * the project's id; choosing "No project" calls `onChange(null)`.
 */
export function AutomationProjectPicker({
  projects,
  value,
  onChange,
  ariaLabel,
}: AutomationProjectPickerProps) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const active = projects.find((p) => p.id === value) ?? null

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={ariaLabel ?? t('automations.projectPickerLabel')}
          data-testid="automation-project-picker"
          className={cn(
            'flex h-8 items-center gap-1 rounded-md border bg-card px-2 text-xs font-medium text-muted-foreground',
            'transition-colors hover:bg-accent hover:text-foreground',
            active && 'border-primary/40 text-foreground',
          )}
        >
          <Folder className="h-3.5 w-3.5" />
          <span className="max-w-40 truncate">
            {active ? active.name : t('automations.noProject')}
          </span>
          <ChevronDown className="h-3 w-3 opacity-60" />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-72 p-0" collisionPadding={8}>
        <div className="border-b px-2 py-1.5 text-xs font-medium text-muted-foreground">
          {t('automations.projectPickerTitle')}
        </div>
        <div className="max-h-80 overflow-y-auto p-1">
          <button
            type="button"
            onClick={() => {
              onChange(null)
              setOpen(false)
            }}
            data-testid="automation-project-picker-none"
            className={cn(
              'flex w-full items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-left text-sm',
              'hover:bg-accent',
              value === null && 'bg-accent/60',
            )}
          >
            <span className="text-muted-foreground italic">{t('automations.noProject')}</span>
            {value === null && <Check className="h-3.5 w-3.5 shrink-0" />}
          </button>
          {projects.map((p) => (
            <button
              key={p.id}
              type="button"
              onClick={() => {
                onChange(p.id)
                setOpen(false)
              }}
              data-testid={`automation-project-picker-option-${p.id}`}
              className={cn(
                'flex w-full items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-left text-sm',
                'hover:bg-accent',
                p.id === value && 'bg-accent/60',
              )}
            >
              <span className="truncate">{p.name}</span>
              {p.id === value && <Check className="h-3.5 w-3.5 shrink-0" />}
            </button>
          ))}
          {projects.length === 0 && (
            <div className="px-2 py-3 text-center text-xs text-muted-foreground">
              {t('automations.noProjects')}
            </div>
          )}
        </div>
      </PopoverContent>
    </Popover>
  )
}

/**
 * Loads the project roster via the shared `/api/projects` endpoint. The
 * endpoint returns `{ projects: Project[]; count: number }` (see
 * crates/oxios-kernel/src/api/routes/project_routes.rs) but the web
 * store upstream accepts `{ items, total }` — we tolerate both shapes.
 */
export function useAutomationProjectRoster() {
  return useQuery({
    // Namespaced so it never shares cache shape with the layout's
    // ['projects'] ({items,total}) — React Query serves stale data on
    // refetch failure, and a shape mix would crash projects.find.
    queryKey: ['automations', 'roster', 'projects'],
    queryFn: async (): Promise<Array<{ id: string; name: string }>> => {
      const res = await api.get<
        | { projects: Array<{ id: string; name: string }> }
        | { items: Array<{ id: string; name: string }> }
      >('/api/projects')
      if ('projects' in res) return res.projects
      return res.items ?? []
    },
    staleTime: 60_000,
  })
}
