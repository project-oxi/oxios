// Todo render — live checklist card, Claude-Desktop style.
//
// The kernel's `todo` tool publishes a phase snapshot on `tool_end.results`
// (see crates/oxios-kernel/src/tools/todo.rs::phases_to_json), so this card
// shows the plan as it stands after the call rather than parsing the text
// summary the model sees.
import { ChevronDown, ListChecks } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { cn } from '@/lib/utils'
import type { ToolRenderComponent } from './registry'

/** One task inside a phase. Mirrors the kernel's flat snapshot shape. */
export interface TodoTask {
  content: string
  /** pending | in_progress | completed | abandoned | blocked */
  status: string
  icon?: string
  blockReason?: string | null
}

export interface TodoPhase {
  name: string
  tasks: TodoTask[]
}

export interface TodoSnapshot {
  phases: TodoPhase[]
  total: number
  completed: number
  inProgress?: string | null
}

/** Tasks shown before the card collapses the rest behind a toggle. */
const COLLAPSED_COUNT = 8

/**
 * Narrow an arbitrary tool result to a todo snapshot.
 *
 * Exported because the sticky progress bar reads the same payload out of the
 * transcript — one parser, so the card and the bar can never disagree.
 */
export function parseTodo(result: unknown): TodoSnapshot | null {
  if (!result || typeof result !== 'object') return null
  const raw = result as Record<string, unknown>
  if (!Array.isArray(raw.phases)) return null
  const phases: TodoPhase[] = raw.phases
    .filter((p): p is Record<string, unknown> => !!p && typeof p === 'object')
    .map((p) => ({
      name: typeof p.name === 'string' ? p.name : '',
      tasks: Array.isArray(p.tasks)
        ? p.tasks
            .filter((t): t is Record<string, unknown> => !!t && typeof t === 'object')
            .map((t) => ({
              content: typeof t.content === 'string' ? t.content : '',
              status: typeof t.status === 'string' ? t.status : 'pending',
              icon: typeof t.icon === 'string' ? t.icon : undefined,
              blockReason: typeof t.blockReason === 'string' ? t.blockReason : null,
            }))
        : [],
    }))
  return {
    phases,
    total: typeof raw.total === 'number' ? raw.total : 0,
    completed: typeof raw.completed === 'number' ? raw.completed : 0,
    inProgress: typeof raw.inProgress === 'string' ? raw.inProgress : null,
  }
}

/** A task is closed when no further work is expected on it. */
export function isClosed(status: string): boolean {
  return status === 'completed' || status === 'abandoned'
}

// Fallback glyphs. The kernel sends `icon` alongside `status`; these cover a
// status added upstream before this file learns about it.
const FALLBACK_ICON: Record<string, string> = {
  pending: '☐',
  in_progress: '▶',
  completed: '☑',
  abandoned: '✗',
  blocked: '⏸',
}

function TaskRow({ task }: { task: TodoTask }) {
  const { t } = useTranslation()
  const closed = isClosed(task.status)
  return (
    <li className="flex items-start gap-2 py-0.5">
      <span
        aria-hidden
        className={cn(
          'select-none leading-5 shrink-0',
          task.status === 'in_progress' && 'text-primary',
          task.status === 'completed' && 'text-success',
          task.status === 'abandoned' && 'text-muted-foreground',
          task.status === 'blocked' && 'text-warning',
        )}
      >
        {task.icon || FALLBACK_ICON[task.status] || '·'}
      </span>
      <span className="min-w-0">
        <span
          className={cn(
            'text-xs',
            task.status === 'in_progress' && 'font-medium text-foreground',
            closed && 'line-through text-muted-foreground',
          )}
        >
          {task.content}
        </span>
        {task.status === 'blocked' && task.blockReason && (
          <span className="ml-1.5 text-2xs text-warning">
            {t('chat.todo.blocked', { reason: task.blockReason })}
          </span>
        )}
      </span>
    </li>
  )
}

export const TodoRender: ToolRenderComponent = ({ result, isRunning }) => {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const snapshot = parseTodo(result)

  if (isRunning && !snapshot) {
    return (
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <span className="inline-block w-2 h-2 rounded-full bg-status-warning animate-pulse" />
        {t('chat.todo.updating')}
      </div>
    )
  }
  if (!snapshot || snapshot.phases.length === 0) {
    return <div className="text-xs text-muted-foreground">{t('chat.todo.empty')}</div>
  }

  // Collapse by task count across phases, not by phase, so a single long
  // phase collapses too.
  let budget = expanded ? Number.POSITIVE_INFINITY : COLLAPSED_COUNT
  const visible = snapshot.phases
    .map((phase) => {
      const tasks = phase.tasks.slice(0, Math.max(0, budget))
      budget -= tasks.length
      return { ...phase, tasks }
    })
    .filter((phase) => phase.tasks.length > 0)
  const hiddenCount = snapshot.total - visible.reduce((n, p) => n + p.tasks.length, 0)

  // A plan with one unnamed phase is a flat list; don't show a heading for it.
  const showPhaseNames = snapshot.phases.length > 1 || !!snapshot.phases[0]?.name

  return (
    <div className="space-y-2 text-sm">
      <div className="flex items-center gap-2 text-xs">
        <ListChecks className="w-3.5 h-3.5 text-muted-foreground" />
        <span className="text-muted-foreground">
          {t('chat.todo.progress', { completed: snapshot.completed, total: snapshot.total })}
        </span>
      </div>

      <div className="space-y-2">
        {visible.map((phase, i) => (
          <div key={`${phase.name}-${i}`}>
            {showPhaseNames && phase.name && (
              <div className="text-2xs uppercase tracking-wide text-muted-foreground mb-0.5">
                {phase.name}
              </div>
            )}
            <ul className="space-y-0">
              {phase.tasks.map((task, j) => (
                <TaskRow key={`${task.content}-${j}`} task={task} />
              ))}
            </ul>
          </div>
        ))}
      </div>

      {hiddenCount > 0 && (
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
        >
          <ChevronDown className={cn('w-3 h-3 transition-transform', expanded && 'rotate-180')} />
          {expanded ? t('chat.todo.showLess') : t('chat.todo.showMore', { count: hiddenCount })}
        </button>
      )}
    </div>
  )
}
