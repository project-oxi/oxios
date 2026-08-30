import { Boxes, FileText, History } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { BriefMarkdown } from '@/components/brain/brief-markdown'
import { WhyDetails } from '@/components/brain/why-details'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { useBrainBrief, useBrainEntity, useBrainTimeline } from '@/hooks/use-brain'
import type { Belief } from '@/types/brain'
import { isSentinelTime } from '@/types/brain'

function fmtDate(ms: number | undefined | null): string {
  if (ms == null || isSentinelTime(ms)) return '—'
  return new Date(ms).toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  })
}

function fmtWindow(from: number, to: number): string {
  const start = isSentinelTime(from) ? null : from
  const end = isSentinelTime(to) ? null : to
  if (start === null && end === null) return '—'
  if (start === null) return `→ ${fmtDate(end)}`
  if (end === null) return `${fmtDate(start)} →`
  return `${fmtDate(start)} → ${fmtDate(end)}`
}

const statusTone: Record<string, string> = {
  active: 'text-status-success',
  superseded: 'text-muted-foreground',
  contradicted: 'text-status-error',
  retracted: 'text-status-warning',
}

function BeliefStatus({ status }: { status: string }) {
  return <span className={statusTone[status] ?? 'text-muted-foreground'}>{status}</span>
}

function BeliefsTable({
  beliefs,
  onWhy,
}: {
  beliefs: Belief[]
  onWhy: (statementId: string) => void
}) {
  const { t } = useTranslation()
  if (beliefs.length === 0)
    return <p className="text-sm text-muted-foreground">{t('brain.noBeliefs')}</p>
  return (
    <table className="w-full text-sm">
      <thead>
        <tr className="border-b text-left text-xs text-muted-foreground">
          <th className="py-2 pr-4 font-medium">{t('brain.statement')}</th>
          <th className="py-2 pr-4 font-medium">{t('brain.confidence')}</th>
          <th className="py-2 pr-4 font-medium">{t('brain.support')}</th>
          <th className="py-2 pr-4 font-medium">{t('brain.validity')}</th>
          <th className="py-2 pr-4 font-medium">{t('brain.status')}</th>
          <th className="py-2 font-medium" />
        </tr>
      </thead>
      <tbody>
        {beliefs.map((b) => (
          <tr key={b.statement} className="border-b last:border-0 align-top">
            <td className="py-2 pr-4 font-mono text-xs break-all">{b.statement.slice(0, 12)}…</td>
            <td className="py-2 pr-4 tabular-nums">{b.confidence.toFixed(2)}</td>
            <td className="py-2 pr-4 tabular-nums whitespace-nowrap">
              <span className="text-status-success">+{b.support.affirm_count}</span>{' '}
              <span className="text-status-error">−{b.support.deny_count}</span>{' '}
              <span className="text-muted-foreground">({b.support.distinct_episodes} ep)</span>
            </td>
            <td className="py-2 pr-4 whitespace-nowrap text-muted-foreground">
              {fmtWindow(b.valid_from, b.valid_to)}
            </td>
            <td className="py-2 pr-4">
              <BeliefStatus status={b.status} />
            </td>
            <td className="py-2 text-right">
              <Button variant="ghost" size="sm" onClick={() => onWhy(b.statement)}>
                {t('brain.why')}
              </Button>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

/**
 * Entity drill-down: readable page (brief), beliefs table, timeline table,
 * and per-statement provenance. Entity links from search/overview/graph
 * arrive via `initialEntityId`; `onNavigateEntity` mirrors internal
 * entity:// link follows back to the caller (route search param).
 */
export function BrainEntityDetail({
  space,
  initialEntityId,
  onNavigateEntity,
}: {
  /** Space all scoped brain calls are bound to (required server-side). */
  space: string
  initialEntityId?: string
  onNavigateEntity?: (entityId: string) => void
}) {
  const { t } = useTranslation()
  const [entityId, setEntityId] = useState(initialEntityId ?? '')
  const [submitted, setSubmitted] = useState(initialEntityId ?? '')
  const [whyStatement, setWhyStatement] = useState('')

  // Route-driven id changes (search page links) load without retyping.
  useEffect(() => {
    if (initialEntityId && initialEntityId !== submitted) {
      setEntityId(initialEntityId)
      setSubmitted(initialEntityId)
      setWhyStatement('')
    }
  }, [initialEntityId, submitted])

  const { data: brief, isLoading: briefLoading } = useBrainBrief(
    'entity',
    submitted || null,
    null,
    space,
  )
  const { data: beliefs, isLoading: beliefsLoading } = useBrainEntity(submitted || null, space)
  const { data: timeline, isLoading: timelineLoading } = useBrainTimeline(submitted || null, space)

  const loadEntity = (id: string) => {
    setSubmitted(id)
    setEntityId(id)
    setWhyStatement('')
    onNavigateEntity?.(id)
  }

  return (
    <div className="space-y-4">
      <div className="flex gap-2">
        <Input
          value={entityId}
          onChange={(e) => setEntityId(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && loadEntity(entityId.trim())}
          placeholder={t('brain.entityPlaceholder')}
          aria-label={t('brain.entityPlaceholder')}
          className="flex-1"
        />
        <Button onClick={() => loadEntity(entityId.trim())}>{t('brain.inspect')}</Button>
      </div>

      {submitted && (
        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2 text-base">
                <FileText className="h-4 w-4" />
                {t('brain.page')}
              </CardTitle>
            </CardHeader>
            <CardContent>
              {briefLoading ? (
                <p className="text-sm text-muted-foreground">{t('brain.loading')}</p>
              ) : brief?.markdown ? (
                <BriefMarkdown onEntityLink={loadEntity}>{brief.markdown}</BriefMarkdown>
              ) : (
                <p className="text-sm text-muted-foreground">{t('brain.pageUnavailable')}</p>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2 text-base">
                <Boxes className="h-4 w-4" />
                {t('brain.beliefs')}
              </CardTitle>
            </CardHeader>
            <CardContent>
              {beliefsLoading ? (
                <p className="text-sm text-muted-foreground">{t('brain.loading')}</p>
              ) : (
                <BeliefsTable beliefs={beliefs ?? []} onWhy={setWhyStatement} />
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2 text-base">
                <History className="h-4 w-4" />
                {t('brain.timeline')}
              </CardTitle>
            </CardHeader>
            <CardContent>
              {timelineLoading ? (
                <p className="text-sm text-muted-foreground">{t('brain.loading')}</p>
              ) : (timeline ?? []).length === 0 ? (
                <p className="text-sm text-muted-foreground">{t('brain.noTimeline')}</p>
              ) : (
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b text-left text-xs text-muted-foreground">
                      <th className="py-2 pr-4 font-medium">{t('brain.predicate')}</th>
                      <th className="py-2 pr-4 font-medium">{t('brain.object')}</th>
                      <th className="py-2 pr-4 font-medium">{t('brain.validity')}</th>
                      <th className="py-2 pr-4 font-medium">{t('brain.recordedAt')}</th>
                      <th className="py-2 font-medium">{t('brain.status')}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {(timeline ?? []).map((e) => (
                      <tr
                        key={`${e.statement_id}-${e.recorded_at}`}
                        className="border-b last:border-0"
                      >
                        <td className="py-2 pr-4 font-medium">{e.predicate}</td>
                        <td className="py-2 pr-4">
                          {e.object_entity ? (
                            <button
                              type="button"
                              className="text-primary underline underline-offset-2 hover:opacity-80 transition-opacity"
                              onClick={() => loadEntity(e.object_entity as string)}
                            >
                              {e.object_repr}
                            </button>
                          ) : (
                            e.object_repr
                          )}
                        </td>
                        <td className="py-2 pr-4 whitespace-nowrap text-muted-foreground">
                          {fmtWindow(e.valid_from, e.valid_to)}
                        </td>
                        <td className="py-2 pr-4 whitespace-nowrap text-muted-foreground">
                          {fmtDate(e.recorded_at)}
                        </td>
                        <td className="py-2">
                          <BeliefStatus status={e.status} />
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </CardContent>
          </Card>

          {whyStatement && (
            <Card>
              <CardHeader>
                <CardTitle className="text-base">
                  {t('brain.provenance')} ·{' '}
                  <span className="font-mono text-xs">{whyStatement.slice(0, 16)}…</span>
                </CardTitle>
              </CardHeader>
              <CardContent>
                <WhyDetails statementId={whyStatement} space={space} />
              </CardContent>
            </Card>
          )}
        </div>
      )}
    </div>
  )
}
