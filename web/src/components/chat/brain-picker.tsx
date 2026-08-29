import { Brain, Check, ChevronDown } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { useBrainSpaces } from '@/hooks/use-brain'
import { cn } from '@/lib/utils'
import { useChatStore } from '@/stores/chat'

// ─── Default resolution ───────────────────────────────────────

/**
 * Chat brain binding resolution order: the explicit session picker choice
 * wins; otherwise the project default, then the global default. An empty
 * global default means "no default" → null (unconnected turn).
 *
 * Pure helper (unit-tested); Task 9 feeds the resolved value into the
 * sendMessage payload instead of the raw explicit selection.
 */
export function resolveBrainSpace(
  explicit: string | null,
  projectDefault: string | null | undefined,
  globalDefault: string,
): string | null {
  return explicit ?? projectDefault ?? (globalDefault || null)
}

// ─── Component ────────────────────────────────────────────────

interface BrainPickerProps {
  /** Spaces the brain daemon exposes (roster). */
  spaces: Array<{ id: string; name: string }>
  /** Explicit session-scoped selection (null = off; defaults may apply). */
  activeBrainSpace: string | null
  setActiveBrainSpace: (id: string | null) => void
  /** Project default space — drives the "(default)" suffix (Task 9 wires it). */
  projectDefault?: string | null
  /** Global default space; '' = none (Task 9 wires it). */
  globalDefault?: string
}

/**
 * BrainPicker — the chat-input pill for session-scoped brain-space binding.
 *
 * The first row ("none") unbinds the session so the turn runs unconnected.
 * Keep the surface small: no space management here — that lives on the
 * brain pages.
 */
export function BrainPicker({
  spaces,
  activeBrainSpace,
  setActiveBrainSpace,
  projectDefault = null,
  globalDefault = '',
}: BrainPickerProps) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)

  const active = spaces.find((s) => s.id === activeBrainSpace) ?? null
  // Inherited binding (project/global default) while nothing is picked
  // explicitly → mark the pill so the user can tell where the binding came
  // from. Task 9 starts passing the defaults; until then this stays quiet.
  const inherited = resolveBrainSpace(activeBrainSpace, projectDefault, globalDefault)
  const showDefaultSuffix = activeBrainSpace === null && inherited !== null

  const pick = (id: string | null) => {
    setActiveBrainSpace(id)
    setOpen(false)
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={t('chat.brain.label')}
          title={
            active ? t('chat.brain.connected', { name: active.name }) : t('chat.brain.toolsOff')
          }
          className={cn(
            'flex h-7 items-center gap-1 rounded-md px-2 text-xs font-medium text-muted-foreground',
            'transition-colors hover:bg-accent hover:text-foreground',
            activeBrainSpace && 'bg-accent text-foreground',
          )}
        >
          <Brain className="h-3.5 w-3.5" />
          <span className="max-w-28 truncate">
            {active ? active.name : t('chat.brain.toolsOff')}
          </span>
          {showDefaultSuffix && <span className="opacity-60">{t('chat.brain.defaultSuffix')}</span>}
          <ChevronDown className="h-3 w-3 opacity-60" />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-64 p-0" collisionPadding={8}>
        <div className="border-b px-2 py-1.5 text-xs font-medium text-muted-foreground">
          {t('chat.brain.label')}
        </div>
        <div className="max-h-80 overflow-y-auto p-1">
          <button
            type="button"
            onClick={() => pick(null)}
            className={cn(
              'flex w-full items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-left text-sm',
              'hover:bg-accent',
              activeBrainSpace === null && 'bg-accent/60',
            )}
          >
            <span>{t('chat.brain.none')}</span>
            {activeBrainSpace === null && <Check className="h-3.5 w-3.5 shrink-0" />}
          </button>

          {spaces.map((s) => (
            <button
              key={s.id}
              type="button"
              onClick={() => pick(s.id)}
              className={cn(
                'flex w-full items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-left text-sm',
                'hover:bg-accent',
                s.id === activeBrainSpace && 'bg-accent/60',
              )}
            >
              <span className="truncate">{s.name}</span>
              {s.id === activeBrainSpace && <Check className="h-3.5 w-3.5" />}
            </button>
          ))}

          {spaces.length === 0 && (
            <div className="px-2 py-3 text-center text-xs text-muted-foreground">
              {t('chat.brain.none')}
            </div>
          )}
        </div>
      </PopoverContent>
    </Popover>
  )
}

// ─── Container ────────────────────────────────────────────────

/**
 * Resolves the brain-space roster + chat-store selection and forwards them
 * to [`BrainPicker`]. Selecting a space here is session-scoped (WS
 * `brain_space`); the turn binds to it and the backend persists it on the
 * session. Hidden while the roster is empty or unavailable (degraded /
 * unconfigured brain) — there is nothing to bind to.
 */
export function BrainPickerContainer() {
  const activeBrainSpace = useChatStore((s) => s.activeBrainSpace)
  const setActiveBrainSpace = useChatStore((s) => s.setActiveBrainSpace)

  const spacesQuery = useBrainSpaces()
  const spaces = Array.isArray(spacesQuery.data) ? spacesQuery.data : []

  if (spaces.length === 0) return null

  return (
    <BrainPicker
      spaces={spaces}
      activeBrainSpace={activeBrainSpace}
      setActiveBrainSpace={setActiveBrainSpace}
    />
  )
}
