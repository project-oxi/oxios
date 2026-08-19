// AgentFanoutCard — inline card for a single spawned worktree agent.
//
// RFC-044 Phase 3: when a persona exposes the `worktree-fanout` capability,
// the user can launch N parallel agents against separate worktrees. This
// card renders inline in the chat transcript and shows the agent's status:
//   • green dot  → done
//   • yellow dot → working
//   • red dot    → failed
//
// The card is intentionally compact — it's stacked next to its peers in a
// grid that follows the composer's fan-out submission, so the user can see
// at a glance which agents are still alive.

import { CircleAlert, GitBranch, Loader2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { formatRelativeTime } from '@/lib/relative-time'
import { cn } from '@/lib/utils'

export type AgentFanoutStatus = 'working' | 'done' | 'failed'

export interface AgentFanoutCardProps {
  /** Stable agent id (from the worktree-fanout API). */
  agentId: string
  /** Display name; falls back to the id when absent. */
  name?: string
  /** Worktree path the agent is operating in. */
  worktreePath?: string
  /** Last update from the agent. */
  status: AgentFanoutStatus
  /** Short status text (e.g. "running tests…", "3 files changed"). */
  detail?: string
  /** Epoch-ms timestamp of the last status update. */
  updatedAt?: number
  /** Optional click handler — wire up to "open agent detail" later. */
  onSelect?: (agentId: string) => void
}

/** Color token + icon for each status. */
const STATUS_PRESENTATION: Record<
  AgentFanoutStatus,
  { dot: string; text: string; Icon: typeof Loader2 }
> = {
  working: {
    dot: 'bg-status-warning animate-pulse',
    text: 'text-status-warning-on-subtle',
    Icon: Loader2,
  },
  done: {
    dot: 'bg-status-success',
    text: 'text-status-success-on-subtle',
    Icon: GitBranch,
  },
  failed: {
    dot: 'bg-destructive',
    text: 'text-destructive',
    Icon: CircleAlert,
  },
}

export function AgentFanoutCard({
  agentId,
  name,
  worktreePath,
  status,
  detail,
  updatedAt,
  onSelect,
}: AgentFanoutCardProps) {
  const { t } = useTranslation()
  const pres = STATUS_PRESENTATION[status]
  const StatusIcon = pres.Icon
  const displayName = name ?? agentId.slice(0, 8)
  const ago =
    updatedAt == null || !Number.isFinite(updatedAt)
      ? ''
      : formatRelativeTime(new Date(updatedAt).toISOString(), t)

  return (
    <button
      type="button"
      onClick={() => onSelect?.(agentId)}
      className={cn(
        'group flex w-full items-start gap-2.5 rounded-lg border bg-card px-3 py-2.5 text-left',
        'transition-colors hover:bg-accent/40 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring',
        'border-border/70',
      )}
      data-agent-id={agentId}
      data-agent-status={status}
    >
      {/* Status dot */}
      <span
        aria-hidden="true"
        className={cn('mt-1 inline-block h-2 w-2 shrink-0 rounded-full', pres.dot)}
      />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <StatusIcon
            className={cn(
              'h-3.5 w-3.5 shrink-0',
              status === 'working' && 'animate-spin',
              pres.text,
            )}
          />
          <span className="truncate text-sm font-medium text-foreground">{displayName}</span>
          {ago && (
            <span className="ml-auto shrink-0 text-2xs text-muted-foreground tabular-nums">
              {ago}
            </span>
          )}
        </div>
        {worktreePath && (
          <p className="mt-0.5 truncate font-mono text-2xs text-muted-foreground">{worktreePath}</p>
        )}
        {detail && <p className={cn('mt-1 truncate text-xs', pres.text)}>{detail}</p>}
      </div>
    </button>
  )
}

/** Grid wrapper — renders multiple AgentFanoutCards as a responsive grid. */
export function AgentFanoutCardGrid({
  agents,
  onSelect,
}: {
  agents: AgentFanoutCardProps[]
  onSelect?: (agentId: string) => void
}) {
  if (agents.length === 0) return null
  return (
    <div
      className={cn(
        'grid gap-2',
        agents.length === 1
          ? 'grid-cols-1'
          : agents.length === 2
            ? 'grid-cols-1 sm:grid-cols-2'
            : 'grid-cols-1 sm:grid-cols-2 lg:grid-cols-3',
      )}
    >
      {agents.map((a) => (
        <AgentFanoutCard key={a.agentId} {...a} onSelect={onSelect} />
      ))}
    </div>
  )
}
