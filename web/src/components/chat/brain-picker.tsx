import { Brain, Check, ChevronDown } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { useBrainSpaces } from '@/hooks/use-brain'
import { resolveBrainSpace } from '@/lib/brain-binding'
import { cn } from '@/lib/utils'
import { useChatStore } from '@/stores/chat'

// ─── Component ────────────────────────────────────────────────

interface BrainPickerProps {
  /** Spaces the brain daemon exposes (roster). */
  spaces: Array<{ id: string; name: string }>
  /** Explicit session-scoped selection (null = off; defaults may apply). */
  activeBrainSpace: string | null
  setActiveBrainSpace: (name: string | null) => void
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

  // The pill and the checked rows reflect the RESOLVED binding (explicit
  // pick, else project/global default); an inherited — non-explicit —
  // binding is marked with the "(default)" suffix so its origin is obvious.
  const resolved = resolveBrainSpace(activeBrainSpace, projectDefault, globalDefault)
  const inherited = activeBrainSpace === null && resolved !== null
  const active = spaces.find((s) => s.name === resolved) ?? null

  const pick = (name: string | null) => {
    setActiveBrainSpace(name)
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
            resolved && 'bg-accent text-foreground',
          )}
        >
          <Brain className="h-3.5 w-3.5" />
          <span className="max-w-28 truncate">
            {active ? active.name : t('chat.brain.toolsOff')}
          </span>
          {inherited && <span className="opacity-60">{t('chat.brain.defaultSuffix')}</span>}
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
              resolved === null && 'bg-accent/60',
            )}
          >
            <span>{t('chat.brain.none')}</span>
            {resolved === null && <Check className="h-3.5 w-3.5 shrink-0" />}
          </button>

          {spaces.map((s) => (
            <button
              key={s.id}
              type="button"
              onClick={() => pick(s.name)}
              className={cn(
                'flex w-full items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-left text-sm',
                'hover:bg-accent',
                s.name === resolved && 'bg-accent/60',
              )}
            >
              <span className="truncate">{s.name}</span>
              {s.name === resolved && <Check className="h-3.5 w-3.5" />}
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
  const brainDefaults = useChatStore((s) => s.brainDefaults)

  const spacesQuery = useBrainSpaces()
  const spaces = Array.isArray(spacesQuery.data) ? spacesQuery.data : []

  // Stale-binding normalization: a rehydrated binding may name a space that
  // no longer exists (renamed/deleted in oxibrain). Display AND resolve as
  // null in that case; useBrainBindingSync clears the stored binding too.
  const bound = spaces.some((s) => s.name === activeBrainSpace) ? activeBrainSpace : null

  if (spaces.length === 0) return null

  return (
    <BrainPicker
      spaces={spaces}
      activeBrainSpace={bound}
      setActiveBrainSpace={setActiveBrainSpace}
      projectDefault={brainDefaults.project}
      globalDefault={brainDefaults.global}
    />
  )
}
