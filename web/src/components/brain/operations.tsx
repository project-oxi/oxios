import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Card, CardContent } from '@/components/ui/card'
import { useBrainFailures, useBrainMerges, useBrainSources } from '@/hooks/use-brain'
import type { OperationsSection } from '@/types/brain'

const SECTIONS: OperationsSection[] = ['merges', 'failures', 'sources']

function fmtDate(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  })
}

function MergesTable({ space }: { space: string }) {
  const { t } = useTranslation()
  const { data, isLoading } = useBrainMerges(space)
  if (isLoading) return <p className="text-sm text-muted-foreground">{t('brain.loading')}</p>
  const items = data ?? []
  if (items.length === 0)
    return <p className="text-sm text-muted-foreground">{t('brain.noMerges')}</p>
  return (
    <table className="w-full text-sm">
      <thead>
        <tr className="border-b text-left text-xs text-muted-foreground">
          <th className="py-2 pr-4 font-medium">{t('brain.mergeLoser')}</th>
          <th className="py-2 pr-4 font-medium">{t('brain.mergeWinner')}</th>
          <th className="py-2 pr-4 font-medium">{t('brain.decidedBy')}</th>
          <th className="py-2 pr-4 font-medium">{t('brain.decidedAt')}</th>
          <th className="py-2 font-medium">{t('brain.status')}</th>
        </tr>
      </thead>
      <tbody>
        {items.map((m) => (
          <tr key={m.id} className="border-b last:border-0">
            <td className="py-2 pr-4 font-mono text-xs break-all">{m.loser.slice(0, 12)}…</td>
            <td className="py-2 pr-4 font-mono text-xs break-all">{m.winner.slice(0, 12)}…</td>
            <td className="py-2 pr-4">{m.decided_by.kind}</td>
            <td className="py-2 pr-4 whitespace-nowrap text-muted-foreground">
              {fmtDate(m.decided_at)}
            </td>
            <td className="py-2">
              {m.undone_at ? (
                <span className="text-status-warning">{t('brain.mergeUndone')}</span>
              ) : (
                <span className="text-status-success">{t('brain.mergeActive')}</span>
              )}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

function FailuresTable({ space }: { space: string }) {
  const { t } = useTranslation()
  const { data, isLoading } = useBrainFailures(space)
  if (isLoading) return <p className="text-sm text-muted-foreground">{t('brain.loading')}</p>
  const items = data ?? []
  if (items.length === 0)
    return <p className="text-sm text-muted-foreground">{t('brain.noFailures')}</p>
  return (
    <div className="space-y-2">
      {items.map((f) => (
        <details key={f.id} className="rounded-lg border p-3">
          <summary className="cursor-pointer text-sm">
            <span className="font-medium">#{f.id}</span>{' '}
            <span className="text-muted-foreground">
              {fmtDate(f.created_at)} · {t('brain.extractor')} {f.extractor_id.slice(0, 8)}…
            </span>
          </summary>
          <div className="mt-2 space-y-2">
            <p className="text-xs text-muted-foreground font-mono break-all">
              {t('brain.episode')}: {f.episode_id}
            </p>
            <pre className="max-h-64 overflow-auto rounded bg-muted p-3 text-xs whitespace-pre-wrap">
              {f.raw_response}
            </pre>
            {f.errors_json && f.errors_json !== '[]' && (
              <pre className="max-h-32 overflow-auto rounded bg-destructive/10 p-3 text-xs whitespace-pre-wrap">
                {f.errors_json}
              </pre>
            )}
          </div>
        </details>
      ))}
    </div>
  )
}

function SourcesTable({ space }: { space: string }) {
  const { t } = useTranslation()
  const { data, isLoading } = useBrainSources(space)
  if (isLoading) return <p className="text-sm text-muted-foreground">{t('brain.loading')}</p>
  const items = data ?? []
  if (items.length === 0)
    return <p className="text-sm text-muted-foreground">{t('brain.noSources')}</p>
  return (
    <table className="w-full text-sm">
      <thead>
        <tr className="border-b text-left text-xs text-muted-foreground">
          <th className="py-2 pr-4 font-medium">{t('brain.sourceName')}</th>
          <th className="py-2 pr-4 font-medium">{t('brain.sourceKind')}</th>
          <th className="py-2 pr-4 font-medium">{t('brain.sourceMode')}</th>
          <th className="py-2 font-medium">{t('brain.created')}</th>
        </tr>
      </thead>
      <tbody>
        {items.map((s) => (
          <tr key={s.id} className="border-b last:border-0">
            <td className="py-2 pr-4 font-mono text-xs break-all">{s.name}</td>
            <td className="py-2 pr-4">{s.kind}</td>
            <td className="py-2 pr-4">
              <span className={s.mode === 'pull' ? 'text-status-success' : ''}>{s.mode}</span>
            </td>
            <td className="py-2 whitespace-nowrap text-muted-foreground">
              {fmtDate(s.created_at)}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

/**
 * Daemon operations console: entity merges, extraction failures, and
 * registered pull sources (`review_merges` sections). Failures are the
 * quarantine queue — the daemon's own console surfaces the same data.
 */
export function BrainOperations({ space }: { space: string }) {
  const { t } = useTranslation()
  const [section, setSection] = useState<OperationsSection>('failures')
  return (
    <div className="space-y-4">
      <div className="flex gap-1 rounded-lg border p-1 w-fit" role="tablist">
        {SECTIONS.map((s) => (
          <button
            key={s}
            type="button"
            role="tab"
            aria-selected={section === s}
            onClick={() => setSection(s)}
            className={
              'px-3 py-1.5 text-sm rounded-md transition-colors ' +
              (section === s
                ? 'bg-accent text-accent-foreground font-medium'
                : 'text-muted-foreground hover:text-foreground')
            }
          >
            {t(`brain.section_${s}`)}
          </button>
        ))}
      </div>
      <Card>
        <CardContent className="pt-6">
          {section === 'merges' && <MergesTable space={space} />}
          {section === 'failures' && <FailuresTable space={space} />}
          {section === 'sources' && <SourcesTable space={space} />}
        </CardContent>
      </Card>
    </div>
  )
}
