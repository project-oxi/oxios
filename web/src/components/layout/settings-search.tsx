// SettingsSearch — CMD+K / Ctrl+K search across all settings (LobeHub-inspired)
// Global keyboard shortcut that opens a command-palette-style search over
// settings sections, provider configs, and engine options.

import { useNavigate } from '@tanstack/react-router'
import { CornerDownLeft, Search } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { cn } from '@/lib/utils'

// ── Search item ──

interface SearchItem {
  id: string
  label: string
  keywords: string[]
  section: string
  action: () => void
}

// ── Build search index ──

function buildSearchIndex(navigate: (to: string) => void): SearchItem[] {
  return [
    // Settings sections
    {
      id: 'settings-engine',
      label: 'Engine & Providers',
      keywords: [
        'provider',
        'model',
        'api key',
        'engine',
        'llm',
        'openai',
        'anthropic',
        'claude',
        'gpt',
      ],
      section: 'Settings',
      action: () => navigate('/settings'),
    },
    {
      id: 'settings-general',
      label: 'General Settings',
      keywords: ['general', 'config', 'language', 'theme'],
      section: 'Settings',
      action: () => navigate('/settings'),
    },
    {
      id: 'settings-security',
      label: 'Security',
      keywords: ['rbac', 'access', 'permissions', 'tools'],
      section: 'Settings',
      action: () => navigate('/settings'),
    },
    {
      id: 'settings-memory',
      label: 'Memory',
      keywords: ['memory', 'dream', 'consolidation', 'hnsw'],
      section: 'Settings',
      action: () => navigate('/settings'),
    },
    {
      id: 'settings-channels',
      label: 'Channels',
      keywords: ['channels', 'web', 'cli', 'telegram'],
      section: 'Settings',
      action: () => navigate('/settings'),
    },
    {
      id: 'settings-notifications',
      label: 'Notifications',
      keywords: ['notifications', 'email', 'alerts'],
      section: 'Settings',
      action: () => navigate('/settings'),
    },
    // Navigation
    {
      id: 'nav-agents',
      label: 'Agents',
      keywords: ['agents', 'agent', 'create', 'configure'],
      section: 'Navigation',
      action: () => navigate('/agents'),
    },
    {
      id: 'nav-sessions',
      label: 'Sessions',
      keywords: ['sessions', 'chat', 'history', 'conversation'],
      section: 'Navigation',
      action: () => navigate('/sessions'),
    },
    {
      id: 'nav-skills',
      label: 'Skills',
      keywords: ['skills', 'skill', 'marketplace', 'clawhub'],
      section: 'Navigation',
      action: () => navigate('/skills'),
    },
    {
      id: 'nav-personas',
      label: 'Personas',
      keywords: ['personas', 'persona', 'roles'],
      section: 'Navigation',
      action: () => navigate('/personas'),
    },
    {
      id: 'nav-knowledge',
      label: 'Knowledge Base',
      keywords: ['knowledge', 'kb', 'notes', 'markdown', 'wiki'],
      section: 'Navigation',
      action: () => navigate('/knowledge'),
    },
    {
      id: 'nav-memory',
      label: 'Memory Browser',
      keywords: ['memory', 'browser', 'dreams'],
      section: 'Navigation',
      action: () => navigate('/brain'),
    },
    {
      id: 'nav-mcp',
      label: 'MCP Servers',
      keywords: ['mcp', 'servers', 'tools', 'external'],
      section: 'Navigation',
      action: () => navigate('/mcp'),
    },
    {
      id: 'nav-budget',
      label: 'Budget',
      keywords: ['budget', 'cost', 'spend', 'quota'],
      section: 'Navigation',
      action: () => navigate('/budget'),
    },
    {
      id: 'nav-security',
      label: 'Security Audit',
      keywords: ['security', 'audit', 'rbac', 'access'],
      section: 'Navigation',
      action: () => navigate('/security'),
    },
    {
      id: 'nav-cron',
      label: 'Cron Jobs',
      keywords: ['cron', 'schedule', 'jobs', 'tasks'],
      section: 'Navigation',
      action: () => navigate('/cron-jobs'),
    },
    {
      id: 'nav-resources',
      label: 'Resources',
      keywords: ['resources', 'cpu', 'memory', 'disk'],
      section: 'Navigation',
      action: () => navigate('/resources'),
    },
    {
      id: 'nav-email',
      label: 'Email',
      keywords: ['email', 'smtp', 'resend'],
      section: 'Navigation',
      action: () => navigate('/email'),
    },
    {
      id: 'nav-git',
      label: 'Git',
      keywords: ['git', 'commits', 'branches'],
      section: 'Navigation',
      action: () => navigate('/git'),
    },
    {
      id: 'nav-token',
      label: 'Token Maxing',
      keywords: ['token', 'maxing', 'quota', 'subscription'],
      section: 'Navigation',
      action: () => navigate('/token-maxing'),
    },
  ]
}

// ── Component ──

export function SettingsSearch() {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [focusIndex, setFocusIndex] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)
  const navigateFn = useNavigate()

  const searchIndex = useRef<SearchItem[]>([])
  useEffect(() => {
    searchIndex.current = buildSearchIndex((to: string) => {
      navigateFn({ to } as never)
      setOpen(false)
      setQuery('')
    })
  }, [navigateFn])

  // Filter results
  const results = query.trim()
    ? searchIndex.current.filter((item) => {
        const q = query.toLowerCase()
        return (
          item.label.toLowerCase().includes(q) ||
          item.keywords.some((k) => k.toLowerCase().includes(q))
        )
      })
    : searchIndex.current

  // Reset focus when results change
  useEffect(() => {
    setFocusIndex(0)
  }, [query])

  // CMD+K / Ctrl+K to toggle
  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault()
        setOpen((v) => !v)
        setQuery('')
        setTimeout(() => inputRef.current?.focus(), 50)
      }
      if (e.key === 'Escape' && open) {
        setOpen(false)
        setQuery('')
      }
    },
    [open],
  )

  useEffect(() => {
    document.addEventListener('keydown', handleKeyDown)
    return () => document.removeEventListener('keydown', handleKeyDown)
  }, [handleKeyDown])

  const handleSelect = useCallback((item: SearchItem) => {
    item.action()
    setOpen(false)
    setQuery('')
  }, [])

  if (!open) return null

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center pt-[15vh]">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/40 backdrop-blur-sm"
        onClick={() => setOpen(false)}
      />

      {/* Dialog */}
      <div className="relative w-full max-w-lg rounded-xl border bg-popover shadow-2xl overflow-hidden">
        {/* Search input */}
        <div className="flex items-center gap-2 px-4 py-3 border-b">
          <Search className="h-4 w-4 text-muted-foreground shrink-0" />
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'ArrowDown') {
                e.preventDefault()
                setFocusIndex((i) => Math.min(i + 1, results.length - 1))
              }
              if (e.key === 'ArrowUp') {
                e.preventDefault()
                setFocusIndex((i) => Math.max(i - 1, 0))
              }
              if (e.key === 'Enter' && results[focusIndex]) {
                handleSelect(results[focusIndex])
              }
            }}
            placeholder="Search settings, pages, and tools..."
            className="flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground/60"
          />
          <kbd className="hidden sm:inline-flex items-center gap-0.5 rounded border bg-muted px-1.5 py-0.5 text-2xs font-mono text-muted-foreground">
            <span className="text-xs">⌘</span>K
          </kbd>
        </div>

        {/* Results */}
        <div className="max-h-80 overflow-y-auto p-2">
          {results.length === 0 ? (
            <p className="px-3 py-6 text-sm text-muted-foreground text-center">
              No results for "{query}"
            </p>
          ) : (
            results.map((item, i) => (
              <button
                key={item.id}
                type="button"
                onClick={() => handleSelect(item)}
                onMouseEnter={() => setFocusIndex(i)}
                className={cn(
                  'flex items-center gap-3 w-full rounded-md px-3 py-2.5 text-left transition-colors',
                  i === focusIndex ? 'bg-accent text-accent-foreground' : 'hover:bg-accent/50',
                )}
              >
                <div className="flex-1 min-w-0">
                  <p className="text-sm font-medium truncate">{item.label}</p>
                  <p className="text-xs text-muted-foreground">{item.section}</p>
                </div>
                {i === focusIndex && (
                  <CornerDownLeft className="h-4 w-4 text-muted-foreground shrink-0" />
                )}
              </button>
            ))
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center gap-4 px-4 py-2 border-t text-2xs text-muted-foreground">
          <span className="flex items-center gap-1">
            <kbd className="rounded border bg-muted px-1 py-0.5 font-mono">↑↓</kbd> Navigate
          </span>
          <span className="flex items-center gap-1">
            <kbd className="rounded border bg-muted px-1 py-0.5 font-mono">↵</kbd> Select
          </span>
          <span className="flex items-center gap-1">
            <kbd className="rounded border bg-muted px-1 py-0.5 font-mono">Esc</kbd> Close
          </span>
        </div>
      </div>
    </div>
  )
}
