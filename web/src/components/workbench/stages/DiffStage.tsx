// DiffStage — review panel for the aggregated file changes from the
// active session (Task 7, design §7.2).
//
// The hook `useSessionChanges` derives the flat list; this component
// renders each entry with a unified-diff snippet, plus a per-row
// Reviewed / Revert pair. The pending count drives the pin state.

import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { useSessionChanges } from '@/hooks/use-session-changes'
import { useWorkbenchStore } from '@/stores/workbench'

export function DiffStage() {
  const { t } = useTranslation()
  const changes = useSessionChanges()
  const markReviewed = useWorkbenchStore((s) => s.markReviewed)
  const revertChange = useWorkbenchStore((s) => s.revertChange)
  const reviewedChangeIds = useWorkbenchStore((s) => s.reviewedChangeIds)
  const revertedChangeIds = useWorkbenchStore((s) => s.revertedChangeIds)

  const pendingCount = useMemo(
    () => changes.filter((c) => c.status === 'pending').length,
    [changes],
  )

  return (
    <div className="flex h-full flex-col" data-testid="diff-stage">
      <div className="flex items-center justify-between border-b px-3 py-2 text-xs">
        <span className="font-medium">{t('workbench.stage.changes')}</span>
        <span className="text-2xs text-muted-foreground">
          {t('workbench.changes.reviewPending', { count: pendingCount })}
        </span>
      </div>
      <div className="flex-1 overflow-y-auto">
        {changes.length === 0 ? (
          <div className="p-3 text-2xs text-muted-foreground">{t('workbench.changes.empty')}</div>
        ) : (
          <ul className="divide-y">
            {changes.map((c) => {
              // F6: pass the post-action pending count so the store can
              // release the pin when the last pending change is resolved.
              const pendingAfter = c.status === 'pending' ? pendingCount - 1 : pendingCount
              const statusLabel =
                c.status === 'reviewed'
                  ? t('workbench.changes.reviewed')
                  : c.status === 'reverted'
                    ? t('workbench.changes.reverted')
                    : t('workbench.changes.pending')
              return (
                <li key={c.id} className="flex flex-col gap-1 px-3 py-2">
                  <div className="flex items-center justify-between gap-2 text-2xs">
                    <span className="truncate font-mono">{c.path}</span>
                    <span className="shrink-0 text-muted-foreground">{statusLabel}</span>
                  </div>
                  <pre className="max-h-32 overflow-auto rounded bg-muted/40 p-2 text-2xs font-mono">
                    {c.diff}
                  </pre>
                  <div className="flex gap-1">
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      disabled={reviewedChangeIds.has(c.id) || revertedChangeIds.has(c.id)}
                      onClick={() => markReviewed(c.id, pendingAfter)}
                      className="h-6 px-2 text-2xs"
                    >
                      {t('workbench.changes.reviewed')}
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      disabled={revertedChangeIds.has(c.id)}
                      onClick={() => revertChange(c.id, pendingAfter)}
                      className="h-6 px-2 text-2xs"
                    >
                      {t('workbench.changes.revert')}
                    </Button>
                  </div>
                </li>
              )
            })}
          </ul>
        )}
      </div>
    </div>
  )
}
