import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { useBrainEntity, useBrainTimeline, useBrainWhy } from '@/hooks/use-brain'

/** JSON pretty-printer with a stable key for memo-friendly rendering. */
function JsonBlock({ label, value }: { label: string; value: unknown }) {
  return (
    <div>
      <h4 className="mb-1 text-sm font-medium">{label}</h4>
      <pre className="max-h-96 overflow-auto rounded bg-muted p-3 text-xs">
        {value == null ? '—' : JSON.stringify(value, null, 2)}
      </pre>
    </div>
  )
}

/** Entity drill-down: beliefs (entity), timeline, and per-statement why. */
export function BrainEntityDetail() {
  const { t } = useTranslation()
  const [entityId, setEntityId] = useState('')
  const [submitted, setSubmitted] = useState('')
  const [statementId, setStatementId] = useState('')
  // Why lookup is gated by an explicit submit — `useBrainWhy` would refetch
  // on every keystroke otherwise.
  const [submittedStatement, setSubmittedStatement] = useState('')

  const { data: entity, isLoading: entityLoading } = useBrainEntity(submitted || null)
  const { data: timeline, isLoading: timelineLoading } = useBrainTimeline(submitted || null)
  const { data: why, isLoading: whyLoading } = useBrainWhy(submittedStatement || null)

  return (
    <div className="space-y-4">
      <div className="flex gap-2">
        <Input
          value={entityId}
          onChange={(e) => setEntityId(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') setSubmitted(entityId)
          }}
          placeholder={t('brain.entityPlaceholder')}
          aria-label={t('brain.entityPlaceholder')}
          className="flex-1"
        />
        <Button onClick={() => setSubmitted(entityId)}>{t('brain.inspect')}</Button>
      </div>

      {submitted && (
        <div className="space-y-4">
          {entityLoading || timelineLoading ? (
            <p className="text-sm text-muted-foreground">{t('brain.loading')}</p>
          ) : (
            <>
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">{t('brain.beliefs')}</CardTitle>
                </CardHeader>
                <CardContent>
                  <JsonBlock label={submitted} value={entity ?? null} />
                </CardContent>
              </Card>
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">{t('brain.timeline')}</CardTitle>
                </CardHeader>
                <CardContent>
                  <JsonBlock label={submitted} value={timeline ?? null} />
                </CardContent>
              </Card>
            </>
          )}

          <div className="flex gap-2">
            <Input
              value={statementId}
              onChange={(e) => setStatementId(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && statementId.trim().length > 0) {
                  setSubmittedStatement(statementId.trim())
                }
              }}
              placeholder={t('brain.whyPlaceholder')}
              aria-label={t('brain.whyPlaceholder')}
              className="flex-1"
            />
            <Button
              variant="outline"
              onClick={() => setSubmittedStatement(statementId.trim())}
              disabled={statementId.trim().length === 0}
            >
              {t('brain.why')}
            </Button>
          </div>
          {whyLoading && <p className="text-sm text-muted-foreground">{t('brain.loading')}</p>}
          {submittedStatement && !whyLoading && (
            <Card>
              <CardHeader>
                <CardTitle className="text-base">{t('brain.provenance')}</CardTitle>
              </CardHeader>
              <CardContent>
                <JsonBlock label={submittedStatement} value={why ?? null} />
              </CardContent>
            </Card>
          )}
        </div>
      )}
    </div>
  )
}
