// WorktreeComparePanel — compare N fan-out agents' changes and merge the winner.
//
// RFC-044 Phase 4: when a fan-out group completes (all agents done/failed),
// this panel fetches each agent's git diff, shows them side by side, and lets
// the user pick a winner to merge into the target branch.
//
// The panel is a Dialog overlay triggered from the AgentFanoutCardGrid when
// all agents have settled. Each agent shows a diff stat card (files changed,
// +/- lines). Clicking a card expands the full diff below. A "Merge" button
// on each card runs `git merge` of that worktree's branch into main.

import {
  CheckCircle2,
  ChevronRight,
  GitMerge,
  GitPullRequest,
  Loader2,
  XCircle,
} from 'lucide-react'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { api } from '@/lib/api-client'
import { cn } from '@/lib/utils'
import type { FanoutGroup } from '@/stores/fanout'

// ── API types ──

interface DiffFile {
  path: string
  insertions: number
  deletions: number
}

interface DiffStat {
  files_changed: number
  insertions: number
  deletions: number
  files: DiffFile[]
  diff_text: string
}

interface MergeResult {
  merged: boolean
  conflicts: string[]
  target_branch: string
}

// ── Diff text renderer ──

/** One classified line of raw `git diff` output. */
interface RawDiffLine {
  text: string
  kind: 'hunk' | 'add' | 'del' | 'file' | 'context'
}

/** Classify a raw git diff line for coloring. */
function classifyDiffLine(line: string): RawDiffLine['kind'] {
  if (line.startsWith('@@')) return 'hunk'
  if (line.startsWith('+') && !line.startsWith('+++')) return 'add'
  if (line.startsWith('-') && !line.startsWith('---')) return 'del'
  if (
    line.startsWith('diff ') ||
    line.startsWith('index ') ||
    line.startsWith('---') ||
    line.startsWith('+++') ||
    line.startsWith('Binary files')
  ) {
    return 'file'
  }
  return 'context'
}

const LINE_STYLE: Record<RawDiffLine['kind'], string> = {
  hunk: 'text-diff-hunk bg-diff-hunk/5',
  add: 'text-diff-add bg-diff-add/10',
  del: 'text-diff-del bg-diff-del/10',
  file: 'text-muted-foreground font-medium',
  context: 'text-foreground/70',
}

/** Render raw git diff output as a colored <pre>. */
function DiffTextRenderer({ text }: { text: string }) {
  const lines = text.split('\n').slice(0, 500) // cap for performance
  return (
    <pre className="overflow-x-auto rounded-lg border bg-muted/30 p-3 text-xs font-mono leading-relaxed">
      {lines.map((line, i) => {
        const kind = classifyDiffLine(line)
        return (
          <div key={i} className={cn('px-1', LINE_STYLE[kind])}>
            {line || ' '}
          </div>
        )
      })}
      {text.split('\n').length > 500 && (
        <div className="px-1 py-2 text-muted-foreground italic">
          … ({text.split('\n').length - 500} more lines truncated)
        </div>
      )}
    </pre>
  )
}

// ── Diff stat card ──

function DiffStatCard({
  name,
  status,
  diff,
  selected,
  mergeResult,
  merging,
  onSelect,
  onMerge,
}: {
  name: string
  status: string
  diff: DiffStat | null
  selected: boolean
  mergeResult: MergeResult | null
  merging: boolean
  onSelect: () => void
  onMerge: () => void
}) {
  const canMerge = status === 'done' && !mergeResult?.merged
  return (
    <div
      className={cn(
        'rounded-lg border bg-card p-3 transition-colors',
        selected ? 'border-primary ring-1 ring-primary/30' : 'border-border/70',
      )}
    >
      <button type="button" onClick={onSelect} className="flex w-full items-center gap-2 text-left">
        <span
          className={cn(
            'inline-block h-2 w-2 shrink-0 rounded-full',
            status === 'done' ? 'bg-status-success' : 'bg-status-error',
          )}
        />
        <span className="truncate text-sm font-medium">{name}</span>
        {diff && (
          <span className="ml-auto shrink-0 text-2xs text-muted-foreground tabular-nums">
            {diff.files_changed} file{diff.files_changed !== 1 ? 's' : ''}
          </span>
        )}
      </button>
      {diff ? (
        <div className="mt-2 flex items-center gap-3 text-2xs font-mono">
          <span className="text-status-success-on-surface">+{diff.insertions}</span>
          <span className="text-status-error-on-surface">-{diff.deletions}</span>
          <div className="ml-auto flex items-center gap-1">
            <button
              type="button"
              onClick={onSelect}
              className="flex items-center gap-0.5 text-muted-foreground hover:text-foreground transition-colors"
            >
              {selected ? 'Hide' : 'View'}
              <ChevronRight
                className={cn('h-3 w-3 transition-transform', selected && 'rotate-90')}
              />
            </button>
          </div>
        </div>
      ) : (
        <p className="mt-2 text-2xs text-muted-foreground">No changes</p>
      )}
      {/* File list */}
      {diff && diff.files.length > 0 && (
        <div className="mt-2 space-y-0.5">
          {diff.files.slice(0, 5).map((f) => (
            <div
              key={f.path}
              className="flex items-center gap-1 truncate text-2xs text-muted-foreground"
            >
              <span className="truncate font-mono">{f.path}</span>
              <span className="ml-auto shrink-0 font-mono text-status-success-on-surface">
                +{f.insertions}
              </span>
              <span className="shrink-0 font-mono text-status-error-on-surface">
                -{f.deletions}
              </span>
            </div>
          ))}
          {diff.files.length > 5 && (
            <p className="text-2xs text-muted-foreground italic">+{diff.files.length - 5} more</p>
          )}
        </div>
      )}
      {/* Merge button / result */}
      <div className="mt-3">
        {mergeResult?.merged ? (
          <div className="flex items-center gap-1.5 text-2xs text-status-success-on-surface">
            <CheckCircle2 className="h-3.5 w-3.5" />
            Merged into {mergeResult.target_branch}
          </div>
        ) : mergeResult && mergeResult.conflicts.length > 0 ? (
          <div className="flex items-center gap-1.5 text-2xs text-status-error-on-surface">
            <XCircle className="h-3.5 w-3.5" />
            {mergeResult.conflicts.length} conflict
            {mergeResult.conflicts.length !== 1 ? 's' : ''}
          </div>
        ) : canMerge ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={merging}
            onClick={onMerge}
            className="h-7 w-full gap-1.5 text-2xs"
          >
            {merging ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <GitMerge className="h-3 w-3" />
            )}
            Merge
          </Button>
        ) : null}
      </div>
    </div>
  )
}

// ── Main panel ──

interface WorktreeComparePanelProps {
  group: FanoutGroup
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function WorktreeComparePanel({ group, open, onOpenChange }: WorktreeComparePanelProps) {
  const { t } = useTranslation()
  const [diffs, setDiffs] = useState<Record<string, DiffStat>>({})
  const [loading, setLoading] = useState(true)
  const [selectedAgent, setSelectedAgent] = useState<string | null>(null)
  const [merging, setMerging] = useState<string | null>(null)
  const [mergeResults, setMergeResults] = useState<Record<string, MergeResult>>({})

  // Fetch diffs for completed agents on open.
  useEffect(() => {
    if (!open) return
    setLoading(true)
    const completed = group.agents.filter((a) => a.status === 'done' && a.worktreePath)
    if (completed.length === 0) {
      setLoading(false)
      return
    }
    // Auto-select the first completed agent.
    if (!selectedAgent) setSelectedAgent(completed[0]!.agentId)

    Promise.all(
      completed.map(async (agent) => {
        try {
          const res = await api.post<DiffStat>('/api/worktree/diff', {
            worktree_path: agent.worktreePath,
          })
          return [agent.agentId, res] as const
        } catch {
          return [agent.agentId, null] as const
        }
      }),
    ).then((results) => {
      const map: Record<string, DiffStat> = {}
      for (const [id, stat] of results) {
        if (stat) map[id] = stat
      }
      setDiffs(map)
      setLoading(false)
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  const handleMerge = async (agentId: string) => {
    const agent = group.agents.find((a) => a.agentId === agentId)
    if (!agent?.worktreePath) return
    setMerging(agentId)
    try {
      const res = await api.post<MergeResult>('/api/worktree/merge', {
        worktree_path: agent.worktreePath,
      })
      setMergeResults((prev) => ({ ...prev, [agentId]: res }))
      if (res.merged) {
        toast.success(
          t('chat.fanout.merged', {
            defaultValue: 'Merged into {{branch}}',
            branch: res.target_branch,
          }),
        )
      } else if (res.conflicts.length > 0) {
        toast.error(
          t('chat.fanout.conflicts', {
            defaultValue: '{{count}} merge conflicts',
            count: res.conflicts.length,
          }),
        )
      }
    } catch {
      toast.error(t('chat.fanout.mergeFailed', { defaultValue: 'Merge failed' }))
    } finally {
      setMerging(null)
    }
  }

  const completedAgents = group.agents.filter((a) => a.status === 'done')
  const selectedDiff = selectedAgent ? diffs[selectedAgent] : null

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] max-w-4xl overflow-hidden">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <GitPullRequest className="h-4 w-4" />
            {t('chat.fanout.compareTitle', { defaultValue: 'Fan-out Results' })}
          </DialogTitle>
        </DialogHeader>

        {/* Prompt preview */}
        <p className="truncate text-sm text-muted-foreground">{group.prompt}</p>

        {/* Loading state */}
        {loading ? (
          <div className="flex items-center justify-center py-12">
            <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
          </div>
        ) : completedAgents.length === 0 ? (
          <p className="py-8 text-center text-sm text-muted-foreground">
            {t('chat.fanout.noCompleted', {
              defaultValue: 'No completed agents to compare.',
            })}
          </p>
        ) : (
          <>
            {/* Agent diff stat grid */}
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
              {completedAgents.map((agent) => (
                <DiffStatCard
                  key={agent.agentId}
                  name={agent.name ?? agent.agentId.slice(0, 8)}
                  status={agent.status}
                  diff={diffs[agent.agentId] ?? null}
                  selected={selectedAgent === agent.agentId}
                  mergeResult={mergeResults[agent.agentId] ?? null}
                  merging={merging === agent.agentId}
                  onSelect={() =>
                    setSelectedAgent(selectedAgent === agent.agentId ? null : agent.agentId)
                  }
                  onMerge={() => handleMerge(agent.agentId)}
                />
              ))}
            </div>

            {/* Selected agent's full diff */}
            {selectedAgent && selectedDiff && (
              <div className="mt-3 max-h-[40vh] overflow-y-auto rounded-lg border p-2">
                <div className="mb-2 flex items-center gap-2">
                  <span className="text-xs font-medium">
                    {group.agents.find((a) => a.agentId === selectedAgent)?.name ??
                      selectedAgent.slice(0, 8)}
                  </span>
                  <span className="text-2xs text-muted-foreground">
                    {selectedDiff.files_changed} files · +{selectedDiff.insertions} −
                    {selectedDiff.deletions}
                  </span>
                </div>
                <DiffTextRenderer text={selectedDiff.diff_text} />
              </div>
            )}
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
