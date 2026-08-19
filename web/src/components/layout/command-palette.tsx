import { useRouter, useRouterState } from '@tanstack/react-router'
import { PanelLeftClose, PanelLeftOpen, Search } from 'lucide-react'
import { useEffect, useMemo, useRef } from 'react'
import { useTranslation } from 'react-i18next'
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import { useCommandPaletteStore } from '@/stores/command-palette'
import { deriveSidebarMode, useSidebarStore } from '@/stores/sidebar'
import { useCaptureProvider } from './command-palette/capture'
import { useControlProvider } from './command-palette/control'
import { buildContext } from './command-palette/lexer'
import { useNavProvider } from './command-palette/nav'
import { useNewProvider } from './command-palette/new'
import { RecencyLog, rank } from './command-palette/ranker'
import { CommandRegistry } from './command-palette/registry'
import { useRunProvider } from './command-palette/run'
import { useSwitchProvider } from './command-palette/switch'
import type { PaletteItem, Verb } from './command-palette/types'

const RECENCY_KEY = 'oxios-cmd-palette-recency'

/** Heading i18n key for each verb's CommandGroup. */
const VERB_HEADING: Partial<Record<Verb, string>> = {
  go: 'commandPalette.sectionNavigation',
  capture: 'commandPalette.sectionCapture',
  control: 'commandPalette.sectionActions',
  run: 'commandPalette.sectionRun',
  switch: 'commandPalette.sectionSwitch',
  new: 'commandPalette.sectionNew',
}

/** Group ranked items by verb, preserving first-appearance order. */
function groupByVerb(items: PaletteItem[]): Array<[string, PaletteItem[]]> {
  const order: Verb[] = []
  const map = new Map<Verb, PaletteItem[]>()
  for (const it of items) {
    if (!map.has(it.verb)) {
      map.set(it.verb, [])
      order.push(it.verb)
    }
    map.get(it.verb)!.push(it)
  }
  return order.map((v) => [VERB_HEADING[v] ?? 'commandPalette.sectionActions', map.get(v)!])
}

export function CommandPalette() {
  const { t } = useTranslation()
  const router = useRouter()
  const routerState = useRouterState()
  const mode = deriveSidebarMode(routerState.location.pathname)

  const open = useCommandPaletteStore((s) => s.open)
  const query = useCommandPaletteStore((s) => s.query)
  const openPalette = useCommandPaletteStore((s) => s.openPalette)
  const closePalette = useCommandPaletteStore((s) => s.closePalette)
  const setQuery = useCommandPaletteStore((s) => s.setQuery)
  const togglePalette = useCommandPaletteStore((s) => s.togglePalette)

  const sidebarCollapsed = useSidebarStore((s) => s.collapsed)
  const toggleSidebar = useSidebarStore((s) => s.toggle)

  const navProvider = useNavProvider()
  const captureProvider = useCaptureProvider()
  const runProvider = useRunProvider()
  const switchProvider = useSwitchProvider()
  const controlProvider = useControlProvider()
  const newProvider = useNewProvider()
  const registry = useMemo(() => {
    const r = new CommandRegistry()
    r.register(navProvider)
    r.register(captureProvider)
    r.register(runProvider)
    r.register(switchProvider)
    r.register(controlProvider)
    r.register(newProvider)
    return r
  }, [navProvider, captureProvider, runProvider, switchProvider, controlProvider, newProvider])

  // Recency log, persisted to localStorage (design §7).
  const recency = useRef(new RecencyLog()).current
  useEffect(() => {
    try {
      const saved = localStorage.getItem(RECENCY_KEY)
      if (saved) recency.load(JSON.parse(saved) as string[])
    } catch {
      /* ignore corrupt storage */
    }
  }, [recency])
  const recordSelection = (id: string) => {
    recency.record(id)
    try {
      localStorage.setItem(RECENCY_KEY, JSON.stringify(recency.snapshot()))
    } catch {
      /* ignore quota errors */
    }
  }

  // Global ⌘K — owns the shortcut app-wide.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        togglePalette()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [togglePalette])

  const ctx = useMemo(() => buildContext(query, mode), [query, mode])

  const items = useMemo(() => {
    let resolved = registry.resolve(ctx)
    // Empty-state host conveniences (toggle sidebar, search notes) — kept from
    // the legacy palette until the empty-state redesign in P5.
    if (!query.trim()) {
      resolved = [
        ...resolved,
        {
          id: 'action-toggle-sidebar',
          verb: 'control',
          icon: sidebarCollapsed ? (
            <PanelLeftOpen className="h-4 w-4" />
          ) : (
            <PanelLeftClose className="h-4 w-4" />
          ),
          title: sidebarCollapsed
            ? t('commandPalette.actionExpandSidebar')
            : t('commandPalette.actionCollapseSidebar'),
          score: 0,
          onSelect: () => toggleSidebar(),
        },
        {
          id: 'action-search-notes',
          verb: 'go',
          icon: <Search className="h-4 w-4" />,
          title: t('commandPalette.actionSearchNotes'),
          hint: <kbd className="text-[10px] text-muted-foreground">⌘P</kbd>,
          score: 0,
          onSelect: () => router.history.push('/brain/knowledge'),
        },
      ]
    }
    return rank(resolved, ctx, recency)
  }, [registry, ctx, query, recency, t, router, sidebarCollapsed, toggleSidebar])

  const groups = useMemo(() => groupByVerb(items), [items])

  const handleSelect = async (item: PaletteItem) => {
    if (item.compose) {
      setQuery(item.compose)
      return
    }
    recordSelection(item.id)
    try {
      await item.onSelect()
    } finally {
      setQuery('')
      closePalette()
    }
  }

  const kbd = (children: React.ReactNode) => (
    <kbd className="rounded border border-border bg-muted/50 px-1.5 py-0.5 font-mono text-[10px]">
      {children}
    </kbd>
  )

  return (
    <CommandDialog
      open={open}
      onOpenChange={(o) => {
        if (o) openPalette()
        else {
          setQuery('')
          closePalette()
        }
      }}
    >
      <CommandInput
        placeholder={t('commandPalette.placeholder')}
        value={query}
        onValueChange={setQuery}
      />
      <CommandList>
        <CommandEmpty>{t('commandPalette.empty')}</CommandEmpty>
        {groups.map(([heading, groupItems]) => (
          <CommandGroup key={heading} heading={t(heading)}>
            {groupItems.map((item) => (
              <CommandItem key={item.id} value={item.id} onSelect={() => handleSelect(item)}>
                {item.icon}
                <span className="flex-1">
                  {item.title}
                  {item.subtitle != null && (
                    <span className="text-muted-foreground"> · {item.subtitle}</span>
                  )}
                </span>
                {item.hint}
              </CommandItem>
            ))}
          </CommandGroup>
        ))}
      </CommandList>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-t px-3 py-2 text-[11px] text-muted-foreground">
        <span className="inline-flex items-center gap-1">
          {kbd('↑↓')}
          {t('commandPalette.hintNavigate')}
        </span>
        <span className="inline-flex items-center gap-1">
          {kbd('⏎')}
          {t('commandPalette.hintSelect')}
        </span>
        <span className="inline-flex items-center gap-1">
          {kbd('> ! ~ / +')}
          {t('commandPalette.hintVerbs')}
        </span>
        <span className="inline-flex items-center gap-1">
          {kbd('@')}
          {t('commandPalette.hintEntity')}
        </span>
        <span className="ml-auto inline-flex items-center gap-1">
          {kbd('esc')}
          {t('commandPalette.hintClose')}
        </span>
      </div>
    </CommandDialog>
  )
}
