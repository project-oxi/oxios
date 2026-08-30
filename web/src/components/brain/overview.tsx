import { BrainCircuit, Database, GitCompareArrows, Layers } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import {
  useBrainSpaceOverview,
  useBrainSpaces,
  useBrainStats,
  useBrainStatus,
} from '@/hooks/use-brain'
import { cn } from '@/lib/utils'
import { spaceParam } from './space-select'

function StatCard({ icon, label, value }: { icon: React.ReactNode; label: string; value: string }) {
  return (
    <Card>
      <CardContent className="flex items-center gap-3 pt-6">
        {icon}
        <div>
          <p className="text-xs text-muted-foreground">{label}</p>
          <p className="text-xl font-semibold tabular-nums">{value}</p>
        </div>
      </CardContent>
    </Card>
  )
}

/** Overview: daemon availability, scoped space cards, and the space selector. */
export function BrainOverview({
  space,
  onSelectEntity,
  onSelectSpace,
}: {
  space: string
  onSelectEntity?: (id: string) => void
  onSelectSpace?: (space: string) => void
}) {
  const { t } = useTranslation()
  const { data: status, isLoading, isError, refetch } = useBrainStatus()
  const { data: stats } = useBrainStats(space)
  const { data: overview } = useBrainSpaceOverview(space)
  const { data: spaces } = useBrainSpaces()

  if (isLoading) return <LoadingCards count={4} />
  if (isError) return <ErrorState onRetry={() => refetch()} />

  const s = stats ?? { episodes: null, entities: null, statements: null, contradictions: null }
  const fmt = (n: number | null | undefined) => (n == null ? '—' : String(n))

  const spacesCard = (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">{t('brain.spaces')}</CardTitle>
      </CardHeader>
      <CardContent>
        {(spaces ?? []).length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t('brain.spaceLabel')}: {t('brain.unknown')}
          </p>
        ) : (
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b text-left text-xs text-muted-foreground">
                <th className="py-2 pr-4 font-medium">{t('brain.spaceName')}</th>
                <th className="py-2 pr-4 font-medium text-right">{t('brain.entities')}</th>
                <th className="py-2 font-medium text-right">{t('brain.episodes')}</th>
              </tr>
            </thead>
            <tbody>
              {(spaces ?? []).map((sp) => {
                const value = spaceParam(sp)
                const selected = value === space
                return (
                  <tr
                    key={sp.id}
                    className={cn(
                      'cursor-pointer border-b transition-colors hover:bg-accent/50 last:border-0',
                      selected && 'bg-accent/30 font-medium',
                    )}
                    onClick={() => onSelectSpace?.(value)}
                  >
                    <td className="py-2 pr-4">
                      {sp.name || (
                        <span className="text-muted-foreground">({sp.id.slice(0, 8)}…)</span>
                      )}
                      {selected && (
                        <span className="ml-2 text-2xs text-muted-foreground/70">
                          {t('brain.spaceCurrent')}
                        </span>
                      )}
                    </td>
                    <td className="py-2 pr-4 text-right tabular-nums">{sp.entity_count}</td>
                    <td className="py-2 text-right tabular-nums">{sp.episode_count}</td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        )}
      </CardContent>
    </Card>
  )

  // No space selected: the global spaces table doubles as the selector and
  // the scoped cards stay hidden until a row is picked.
  if (!space) {
    return (
      <div className="space-y-4">
        <p className="text-sm text-muted-foreground" role="status">
          {t('brain.selectSpace')}
        </p>
        {spacesCard}
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <StatCard
          icon={<BrainCircuit className="h-4 w-4 text-muted-foreground" />}
          label={t('brain.available')}
          value={status?.available ? t('brain.online') : t('brain.offline')}
        />
        <StatCard
          icon={<Layers className="h-4 w-4 text-muted-foreground" />}
          label={t('brain.episodes')}
          value={fmt(s.episodes)}
        />
        <StatCard
          icon={<Database className="h-4 w-4 text-muted-foreground" />}
          label={t('brain.entities')}
          value={fmt(s.entities)}
        />
        <StatCard
          icon={<GitCompareArrows className="h-4 w-4 text-muted-foreground" />}
          label={t('brain.contradictions')}
          value={fmt(s.contradictions)}
        />
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle className="text-base">{t('brain.recentEntities')}</CardTitle>
          </CardHeader>
          <CardContent>
            {(overview?.recent_entities ?? []).length === 0 ? (
              <p className="text-sm text-muted-foreground">{t('brain.noEntitiesYet')}</p>
            ) : (
              <div className="flex flex-wrap gap-2">
                {(overview?.recent_entities ?? []).map((e) => (
                  <button
                    key={e.id}
                    type="button"
                    onClick={() => onSelectEntity?.(e.id)}
                    className="rounded-lg border px-3 py-1.5 text-left transition-colors hover:bg-accent/50"
                  >
                    <span className="block text-sm font-medium">{e.surface}</span>
                    <span className="block text-2xs text-muted-foreground/70">{e.type}</span>
                  </button>
                ))}
              </div>
            )}
          </CardContent>
        </Card>

        {spacesCard}
      </div>
    </div>
  )
}
