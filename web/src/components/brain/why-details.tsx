import { useTranslation } from 'react-i18next'
import { useBrainWhy } from '@/hooks/use-brain'

function fmtDate(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  })
}

/**
 * Provenance details for one statement: confidence breakdown + assertions
 * (polarity, confidence, mention, recorded_at, episode). Shared by the
 * entity drill-down and the contradiction inbox.
 */
export function WhyDetails({ statementId, space }: { statementId: string; space: string }) {
  const { t } = useTranslation()
  const { data: why, isLoading } = useBrainWhy(statementId, space)
  if (isLoading) return <p className="text-sm text-muted-foreground">{t('brain.loading')}</p>
  if (!why) return <p className="text-sm text-muted-foreground">{t('brain.whyUnavailable')}</p>
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm">
        <span className="font-medium">{why.statement.predicate}</span>
        <span className="text-muted-foreground">{t('brain.status')}:</span>
        <span>{why.status}</span>
        <span className="text-muted-foreground">{t('brain.rawConfidence')}:</span>
        <span className="tabular-nums">{why.confidence_breakdown.raw_confidence.toFixed(2)}</span>
        <span className="text-muted-foreground">
          {t('brain.supportCount', {
            support: why.confidence_breakdown.support_count,
            contradictions: why.confidence_breakdown.contradiction_count,
          })}
        </span>
      </div>
      {why.assertions.length > 0 && (
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b text-left text-xs text-muted-foreground">
              <th className="py-2 pr-4 font-medium">{t('brain.polarity')}</th>
              <th className="py-2 pr-4 font-medium">{t('brain.confidence')}</th>
              <th className="py-2 pr-4 font-medium">{t('brain.mention')}</th>
              <th className="py-2 pr-4 font-medium">{t('brain.recordedAt')}</th>
              <th className="py-2 font-medium">{t('brain.episode')}</th>
            </tr>
          </thead>
          <tbody>
            {why.assertions.map((a) => (
              <tr key={a.assertion_id} className="border-b last:border-0">
                <td
                  className={
                    'py-2 pr-4 ' +
                    (a.polarity === 'affirm' ? 'text-status-success' : 'text-status-error')
                  }
                >
                  {a.polarity}
                </td>
                <td className="py-2 pr-4 tabular-nums">{a.confidence.toFixed(2)}</td>
                <td className="py-2 pr-4">{a.mention ?? '—'}</td>
                <td className="py-2 pr-4 whitespace-nowrap text-muted-foreground">
                  {fmtDate(a.recorded_at)}
                </td>
                <td className="py-2 font-mono text-xs break-all">{a.episode_id.slice(0, 12)}…</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  )
}
