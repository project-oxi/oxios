// SubAgentBlock — a fork in the agent's own timeline.
//
// Same visual tier as a tool card (process, not answer), with the child's
// name and terminal state. Surfaces sub-agent delegations (Task 21) so the
// user sees when the main agent forks work out to another agent.

import { GitFork } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { SubAgentBlockData } from '@/types/chat'

export function SubAgentBlock({ block }: { block: SubAgentBlockData }) {
  const { t } = useTranslation()
  return (
    <div className="flex items-center gap-1.5 text-xs text-muted-foreground" role="status">
      <GitFork className="h-3 w-3 shrink-0" />
      <span className="truncate">{t('chat.subagent', { name: block.name })}</span>
      {block.status === 'running' && <span className="animate-pulse">…</span>}
      {block.status === 'failed' && <span className="text-destructive">✕</span>}
    </div>
  )
}
