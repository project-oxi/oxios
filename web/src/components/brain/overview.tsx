import { BrainCircuit, Database, GitCompareArrows, Layers } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { useBrainStats, useBrainStatus } from '@/hooks/use-brain'

function StatCard({
  icon,
  label,
  value,
}: {
  icon: React.ReactNode
  label: string
  value: string | number
}) {
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
        <CardTitle className="text-sm font-medium">{label}</CardTitle>
        {icon}
      </CardHeader>
      <CardContent>
        <div className="text-2xl font-bold">{value}</div>
      </CardContent>
    </Card>
  )
}

/** Overview: daemon availability + space counts. */
export function BrainOverview() {
  const { t } = useTranslation()
  const { data: status, isLoading, isError, refetch } = useBrainStatus()
  const { data: stats } = useBrainStats()

  if (isLoading) return <LoadingCards count={4} />
  if (isError) return <ErrorState onRetry={() => refetch()} />

  const s = {
    episodes: stats?.episodes ?? null,
    entities: stats?.entities ?? null,
    statements: stats?.statements ?? null,
    contradictions: stats?.contradictions ?? null,
  }

  const fmt = (n: number | null) => (n == null ? '—' : n.toLocaleString())

  return (
    <div className="space-y-6">
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
      <Card>
        <CardHeader>
          <CardTitle className="text-base">{t('brain.spaceLabel')}</CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground">
            {status?.space ?? t('brain.unknown')} · {t('brain.statements')}: {fmt(s.statements)}
          </p>
        </CardContent>
      </Card>
    </div>
  )
}
