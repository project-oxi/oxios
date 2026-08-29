// Sticky todo progress strip, shown above the composer while a plan is live.
//
// Derived from the transcript rather than from store state: the newest `todo`
// tool block already carries the authoritative snapshot, so keeping a second
// copy in the store would only create a way for the two to disagree.
import { ListChecks } from 'lucide-react'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import { cn } from '@/lib/utils'
import type { ChatMessage } from '@/types'
import type { ChatBlock } from '@/types/chat'
import { isClosed, parseTodo, type TodoSnapshot } from './tool-renders/Todo'

/**
 * The most recent todo snapshot in the transcript, or `null`.
 *
 * Walks messages newest-first and returns the first `todo` tool block that
 * carries a parseable result.
 */
export function latestTodoSnapshot(messages: ChatMessage[]): TodoSnapshot | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const blocks = (messages[i]?.blocks ?? []) as ChatBlock[]
    for (let j = blocks.length - 1; j >= 0; j--) {
      const block = blocks[j]
      if (block?.type !== 'tool') continue
      if (block.apiName !== 'todo') continue
      const snapshot = parseTodo(block.result)
      if (snapshot) return snapshot
    }
  }
  return null
}

/** True when the plan still has work the user would want to track. */
export function hasOpenWork(snapshot: TodoSnapshot | null): boolean {
  if (!snapshot || snapshot.total === 0) return false
  return snapshot.phases.some((p) => p.tasks.some((t) => !isClosed(t.status)))
}

interface TodoProgressBarProps {
  messages: ChatMessage[]
  /** Scroll the newest todo card into view. */
  onReveal?: () => void
}

export function TodoProgressBar({ messages, onReveal }: TodoProgressBarProps) {
  const { t } = useTranslation()
  const snapshot = useMemo(() => latestTodoSnapshot(messages), [messages])

  // Nothing planned, or everything finished — the strip earns its space only
  // while there is outstanding work.
  if (!hasOpenWork(snapshot) || !snapshot) return null

  const pct = snapshot.total > 0 ? Math.round((snapshot.completed / snapshot.total) * 100) : 0
  const current =
    snapshot.inProgress ??
    snapshot.phases.flatMap((p) => p.tasks).find((task) => !isClosed(task.status))?.content

  return (
    <button
      type="button"
      onClick={onReveal}
      title={t('chat.todo.revealHint')}
      className={cn(
        'group flex w-full items-center gap-2 border-t px-4 py-1.5 text-left',
        'bg-muted/40 hover:bg-muted/70 transition-colors',
      )}
    >
      <ListChecks className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
        {snapshot.completed}/{snapshot.total}
      </span>
      <span aria-hidden className="h-1 w-16 shrink-0 overflow-hidden rounded-full bg-border">
        <span
          className="block h-full rounded-full bg-primary transition-all"
          style={{ width: `${pct}%` }}
        />
      </span>
      {current && (
        <span className="min-w-0 truncate text-xs text-foreground/80 group-hover:text-foreground">
          {current}
        </span>
      )}
    </button>
  )
}
