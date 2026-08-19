// PortalPanel — right-side panel with stack-based navigation.
//
// LobeHub analogue: features/Portal (router.tsx + components/Header.tsx +
// per-view Body/Title). Oxios version: a shared header (back / title / close)
// above a view router that dispatches on the top PortalView's `type`.
//
// The panel is mounted by the chat route when the portal stack is non-empty.
// It reads the entire stack from usePortalStore and renders the top view.

import { ArrowLeft, X } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { useFocusTrap } from '@/hooks/use-focus-trap'
import { cn } from '@/lib/utils'
import { type PortalView, usePortalStore } from '@/stores/portal'
import { ArtifactView } from './views/artifact-view'
import { DocumentView } from './views/document-view'
import { KnowledgeBrowser } from './views/knowledge-browser'
import { SearchView } from './views/search-view'
import { ThreadView } from './views/thread-view'

/** Resolve a human title for a view (shown in the shared header). */
function viewTitle(view: PortalView): string {
  switch (view.type) {
    case 'artifact':
      return view.meta.title ?? view.meta.type.toUpperCase()
    case 'filePreview':
      return basename(view.path)
    case 'document':
      return basename(view.path)
    case 'thread':
      return `Thread from ${view.parentId.slice(0, 8)}`
    case 'search':
      return view.query ?? 'Search'
    case 'knowledge':
      return view.title ?? basename(view.path)
  }
}

/** Last path segment (file name) for the portal header. */
function basename(path: string): string {
  if (!path) return ''
  const idx = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'))
  return idx >= 0 ? path.slice(idx + 1) : path
}

function PortalHeader() {
  const { t } = useTranslation()
  const stack = usePortalStore((s) => s.stack)
  const popView = usePortalStore((s) => s.popView)
  const clearStack = usePortalStore((s) => s.clearStack)
  const top = stack[stack.length - 1]
  const canGoBack = stack.length > 1

  return (
    <div className="flex items-center gap-1 border-b px-2 py-1.5">
      {canGoBack && (
        <Button size="icon" variant="ghost" onClick={popView} aria-label={t('portal.back')}>
          <ArrowLeft className="size-4" />
        </Button>
      )}
      <div className="mx-1 min-w-0 flex-1">
        <div className="truncate text-sm font-medium">{top ? viewTitle(top) : ''}</div>
      </div>
      <Button size="icon" variant="ghost" onClick={clearStack} aria-label={t('portal.close')}>
        <X className="size-4" />
      </Button>
    </div>
  )
}

/** Dispatch the top view to its renderer. */
function ViewBody() {
  const top = usePortalStore((s) => s.stack[s.stack.length - 1])
  if (!top) return null
  switch (top.type) {
    case 'artifact':
      return <ArtifactView view={top} />
    case 'document':
      return <DocumentView view={top} />
    case 'thread':
      return <ThreadView view={top} />
    case 'search':
      return <SearchView query={top.query} messageId={top.messageId} />
    case 'knowledge':
      return <KnowledgeBrowser initialPath={top.path} />
  }
}

const MIN_WIDTH = 360
const MAX_WIDTH_RATIO = 0.85
const DEFAULT_WIDTH = 640
const STORAGE_KEY = 'oxios:portal-width'

function useResizableWidth() {
  const [width, setWidth] = useState(() => {
    const stored = typeof window !== 'undefined' ? window.localStorage.getItem(STORAGE_KEY) : null
    return stored ? Number(stored) : DEFAULT_WIDTH
  })
  const dragging = useRef(false)

  const onPointerDown = useCallback((e: React.PointerEvent) => {
    e.preventDefault()
    dragging.current = true
    document.body.style.userSelect = 'none'
    document.body.style.cursor = 'col-resize'
  }, [])

  useEffect(() => {
    const onMove = (e: PointerEvent) => {
      if (!dragging.current) return
      const max = window.innerWidth * MAX_WIDTH_RATIO
      // Panel is on the right; width grows as the handle moves left.
      const next = Math.min(max, Math.max(MIN_WIDTH, window.innerWidth - e.clientX))
      setWidth(next)
    }
    const onUp = () => {
      if (!dragging.current) return
      dragging.current = false
      document.body.style.userSelect = ''
      document.body.style.cursor = ''
      setWidth((w) => {
        window.localStorage.setItem(STORAGE_KEY, String(Math.round(w)))
        return w
      })
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    return () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
    }
  }, [])

  return { width, onPointerDown }
}

interface PortalPanelProps {
  className?: string
}

export function PortalPanel({ className }: PortalPanelProps) {
  const { t } = useTranslation()
  const stack = usePortalStore((s) => s.stack)
  const clearStack = usePortalStore((s) => s.clearStack)
  const panelRef = useRef<HTMLDivElement>(null)
  useFocusTrap(panelRef, stack.length > 0, clearStack)
  const { width, onPointerDown } = useResizableWidth()
  if (stack.length === 0) return null

  return (
    <div
      ref={panelRef}
      role="dialog"
      aria-modal="true"
      aria-label={t('portal.panelLabel')}
      tabIndex={-1}
      className={cn('relative flex h-full flex-col border-l bg-background', className)}
      style={{ width }}
    >
      {/* Drag handle on the left edge */}
      <div
        onPointerDown={onPointerDown}
        className="absolute inset-y-0 left-0 z-10 w-1 -translate-x-1/2 cursor-col-resize hover:bg-primary/30 transition-colors"
        aria-hidden
      />
      <PortalHeader />
      <div className="min-h-0 flex-1">
        <ViewBody />
      </div>
    </div>
  )
}
