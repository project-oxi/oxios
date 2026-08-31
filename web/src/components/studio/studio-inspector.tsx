// StudioInspector — on-demand right pane (IA design §8.5).
//
// Tabs are driven ONLY by data actually present in the transcript or the
// session bindings: Context (bindings), Sources (citations/grounding the
// turn recorded), Activity (tool/reasoning blocks), Outline (real
// markdown headings), Review (pending approval / session changes).
// A tab with no backing data never renders — the inspector does not
// invent health, provenance, or structure. The default state is closed;
// open/close is explicit and closing returns focus to the toggle.

import { PanelRightClose, X } from 'lucide-react'
import { useEffect, useRef } from 'react'
import { useTranslation } from 'react-i18next'
import { useSessionChanges } from '@/hooks/use-session-changes'
import { useChatStore } from '@/stores/chat'
import type { ChatMessage } from '@/types'

export type StudioInspectorTab = 'context' | 'sources' | 'activity' | 'outline' | 'review'

interface StudioInspectorProps {
  open: boolean
  tab: StudioInspectorTab | null
  onTabChange: (tab: StudioInspectorTab) => void
  onClose: () => void
  /** Registration point for the toggle button so close can return focus. */
  toggleRef: React.RefObject<HTMLButtonElement | null>
}

/** Real `##`/`#` markdown headings of the latest assistant prose. */
function outlineOf(message: ChatMessage | undefined): string[] {
  if (message?.role !== 'assistant') return []
  return message.content
    .split('\n')
    .filter((line) => /^#{1,3}\s+\S/.test(line))
    .map((line) => line.replace(/^#+\s+/, '').trim())
    .slice(0, 30)
}

export function useInspectorAvailability(messages: ChatMessage[]) {
  const activeToolApproval = useChatStore((s) => s.activeToolApproval)
  const activePathAccess = useChatStore((s) => s.activePathAccess)
  const changes = useSessionChanges()
  const last = [...messages].reverse().find((m) => m.role === 'assistant')
  const sources = (last?.search?.citations?.length ?? 0) + (last?.chunksList?.length ?? 0) > 0
  const activity =
    last?.blocks?.some(
      (b) => b.type === 'tool' || b.type === 'reasoning' || b.type === 'subagent',
    ) ?? false
  const outline = outlineOf(last).length > 0
  const review =
    !!activeToolApproval || !!activePathAccess || changes.some((c) => c.status === 'pending')
  return { sources, activity, outline, review }
}

export function StudioInspector({
  open,
  tab,
  onTabChange,
  onClose,
  toggleRef,
}: StudioInspectorProps) {
  const { t } = useTranslation()
  const messages = useChatStore((s) => s.messages)
  const activeProjectId = useChatStore((s) => s.activeProjectId)
  const activePersonaId = useChatStore((s) => s.activePersonaId)
  const activeModelId = useChatStore((s) => s.activeModelId)
  const activeBrainSpace = useChatStore((s) => s.activeBrainSpace)
  const activeToolApproval = useChatStore((s) => s.activeToolApproval)
  const activePathAccess = useChatStore((s) => s.activePathAccess)
  const changes = useSessionChanges()
  const closeRef = useRef<HTMLButtonElement>(null)

  // Focus return: the close button takes focus, and focus moves back to
  // the toggle after the pane unmounts.
  useEffect(() => {
    if (!open) closeRef.current = null
  }, [open])

  if (!open || !tab) return null

  const last = [...messages].reverse().find((m) => m.role === 'assistant')
  const outline = outlineOf(last)
  const citations = last?.search?.citations ?? []
  const chunks = last?.chunksList ?? []
  const activityBlocks = (last?.blocks ?? []).filter(
    (b) => b.type === 'tool' || b.type === 'reasoning' || b.type === 'subagent',
  )
  const pendingChanges = changes.filter((c) => c.status === 'pending')

  const tabs: { id: StudioInspectorTab; label: string }[] = [
    { id: 'context', label: t('studio.inspector.context') },
    ...(citations.length + chunks.length > 0
      ? [{ id: 'sources' as const, label: t('studio.inspector.sources') }]
      : []),
    ...(activityBlocks.length > 0
      ? [{ id: 'activity' as const, label: t('studio.inspector.activity') }]
      : []),
    ...(outline.length > 0
      ? [{ id: 'outline' as const, label: t('studio.inspector.outline') }]
      : []),
    ...(activeToolApproval || activePathAccess || pendingChanges.length > 0
      ? [{ id: 'review' as const, label: t('studio.inspector.review') }]
      : []),
  ]

  const handleClose = () => {
    onClose()
    // Focus return to the inspector toggle (IA design §12.5).
    toggleRef.current?.focus()
  }

  return (
    <aside
      data-testid="studio-inspector"
      aria-label={t('studio.inspector.label')}
      className="flex w-80 shrink-0 flex-col overflow-hidden border-l bg-background"
    >
      <div className="flex h-10 shrink-0 items-center gap-1 border-b px-2">
        <PanelRightClose className="mr-1 h-3.5 w-3.5 text-muted-foreground" />
        {tabs.map((item) => (
          <button
            key={item.id}
            type="button"
            onClick={() => onTabChange(item.id)}
            className={`rounded-md px-2 py-1 text-xs transition-colors ${
              tab === item.id
                ? 'bg-accent font-medium text-accent-foreground'
                : 'text-muted-foreground hover:bg-accent/50'
            }`}
          >
            {item.label}
          </button>
        ))}
        <button
          ref={closeRef}
          type="button"
          onClick={handleClose}
          aria-label={t('studio.inspector.close')}
          data-testid="studio-inspector-close"
          className="ml-auto rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-3 text-sm">
        {tab === 'context' && (
          <dl data-testid="studio-inspector-context" className="space-y-2">
            {[
              {
                k: t('studio.context.project'),
                v: activeProjectId ?? t('studio.context.noProject'),
              },
              {
                k: t('studio.context.persona'),
                v: activePersonaId ?? t('chat.persona.auto'),
              },
              {
                k: t('studio.context.model'),
                v: activeModelId ?? t('studio.context.defaultModel'),
              },
              {
                k: t('studio.context.brain'),
                v: activeBrainSpace ?? t('studio.context.brainNotConnected'),
              },
            ].map((row) => (
              <div key={row.k} className="flex items-start justify-between gap-2">
                <dt className="shrink-0 text-xs text-muted-foreground">{row.k}</dt>
                <dd className="truncate text-xs">{row.v}</dd>
              </div>
            ))}
          </dl>
        )}
        {tab === 'sources' && (
          <ul data-testid="studio-inspector-sources" className="space-y-1.5">
            {citations.map((c, i) => (
              <li key={`c-${i}`} className="truncate text-xs">
                {c.title || c.url || t('studio.inspector.unnamedSource')}
              </li>
            ))}
            {chunks.map((c) => (
              <li key={c.id} className="truncate text-xs">
                {c.filename || c.id}
              </li>
            ))}
          </ul>
        )}
        {tab === 'activity' && (
          <ul data-testid="studio-inspector-activity" className="space-y-1.5">
            {activityBlocks.map((b, i) => (
              <li key={i} className="truncate text-xs text-muted-foreground">
                {b.type === 'tool'
                  ? `${t('studio.inspector.toolRun')}: ${'toolName' in b ? String(b.toolName) : ''}`
                  : b.type === 'subagent'
                    ? `${t('studio.inspector.subagent')}: ${b.name}`
                    : t('studio.inspector.reasoning')}
              </li>
            ))}
          </ul>
        )}
        {tab === 'outline' && (
          <ol data-testid="studio-inspector-outline" className="list-decimal space-y-1 pl-4">
            {outline.map((heading, i) => (
              <li key={i} className="text-xs">
                {heading}
              </li>
            ))}
          </ol>
        )}
        {tab === 'review' && (
          <ul data-testid="studio-inspector-review" className="space-y-1.5">
            {activeToolApproval && (
              <li className="text-xs">
                {t('studio.inspector.pendingApproval')}: {activeToolApproval.toolName}
              </li>
            )}
            {activePathAccess && (
              <li className="text-xs">
                {t('studio.inspector.pendingPathAccess')}: {activePathAccess.path}
              </li>
            )}
            {pendingChanges.map((c) => (
              <li key={c.id} className="truncate text-xs">
                {t('studio.inspector.pendingChange')}: {c.path}
              </li>
            ))}
          </ul>
        )}
      </div>
    </aside>
  )
}
