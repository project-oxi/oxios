import { Shield } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ProvidersSection } from '@/components/dashboard/providers-section'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Progress } from '@/components/ui/progress'
import { Separator } from '@/components/ui/separator'
import { useRoutingStats } from '@/hooks/use-engine'
import type { SystemStatus } from '@/types'

export interface SystemHealthCardProps {
  status?: SystemStatus
  className?: string
}

/**
 * System health card for the dashboard.
 *
 * Shows component health, providers/model switching (via ProvidersSection),
 * and model usage stats. Keeps the card slim by delegating provider
 * interaction to a separate component.
 */
export function SystemHealthCard({ status, className }: SystemHealthCardProps) {
  const { t } = useTranslation()
  const { data: routingStats } = useRoutingStats()

  if (!status) return null

  const completed = status.components?.agents?.total_completed ?? 0
  const failed = status.components?.agents?.total_failed ?? 0
  const hasUsage = !!routingStats && routingStats.totalRequests > 0

  return (
    <Card className={className}>
      <CardHeader className="flex flex-row items-center justify-between pb-2">
        <CardTitle className="flex items-center gap-2 text-base">
          <Shield className="h-4 w-4" />
          {t('dashboard.systemHealth')}
        </CardTitle>
      </CardHeader>
      <CardContent className="pt-0 space-y-3">
        {/* ── Health rows ── */}
        <div className="grid gap-2 sm:grid-cols-2 text-xs">
          {status.components?.state_store && (
            <HealthRow
              label={t('dashboard.stateStore')}
              healthy={status.components.state_store.healthy}
              detail={status.components.state_store.detail}
            />
          )}
          {status.components?.event_bus && (
            <HealthRow
              label={t('dashboard.eventBus')}
              healthy={status.components.event_bus.healthy}
              detail={status.components.event_bus.detail}
            />
          )}
          {status.components?.brain && (
            <HealthRow
              label={t('dashboard.brain')}
              healthy={status.components.brain.healthy}
              detail={
                status.components.brain.healthy
                  ? t('dashboard.brainHealthy')
                  : t('dashboard.brainDegraded')
              }
            />
          )}
          <div className="flex items-center gap-2 text-muted-foreground">
            <span className="text-foreground">⏱</span>
            <span className="text-foreground">{t('dashboard.uptime')}</span>
            <span className="truncate">{status.uptime}</span>
          </div>
        </div>

        {/* ── Agent completion/failure summary ── */}
        {(completed > 0 || failed > 0) && (
          <p className="text-xs text-muted-foreground">
            <span className="text-foreground">{completed}</span> {t('dashboard.agentsCompleted')}
            {failed > 0 && (
              <>
                {' · '}
                <span className="text-error font-medium">
                  {failed} {t('dashboard.agentsFailed')}
                </span>
              </>
            )}
          </p>
        )}

        {/* ── Providers & Model Switcher ── */}
        <Separator />
        <ProvidersSection />

        {/* ── Model usage (RFC-011 routing stats) ── */}
        {hasUsage && (
          <>
            <Separator />
            <div className="space-y-2">
              <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wide">
                {t('dashboard.modelUsage')}
              </h3>
              {Object.entries(routingStats!.modelCalls)
                .sort(([, a], [, b]) => b - a)
                .slice(0, 5)
                .map(([model, count]) => {
                  const pct = (count / routingStats!.totalRequests) * 100
                  const cost = routingStats!.modelCost[model] ?? 0
                  return (
                    <div key={model} className="space-y-1">
                      <div className="flex justify-between text-xs">
                        <span className="truncate max-w-[55%]" title={model}>
                          {model.split('/').pop()}
                        </span>
                        <span className="text-muted-foreground">
                          {pct.toFixed(0)}% ({count}) · ${cost.toFixed(3)}
                        </span>
                      </div>
                      <Progress value={pct} className="h-1.5" />
                    </div>
                  )
                })}
              {routingStats!.totalCost > 0 && (
                <p className="pt-1 text-xs text-muted-foreground">
                  {t('dashboard.totalCostAndCalls', {
                    cost: routingStats!.totalCost.toFixed(2),
                    count: routingStats!.totalRequests,
                  })}
                </p>
              )}
            </div>
          </>
        )}
      </CardContent>
    </Card>
  )
}

function HealthRow({
  label,
  healthy,
  detail,
}: {
  label: string
  healthy: boolean
  detail?: string | null
}) {
  return (
    <div className="flex items-center gap-2 text-muted-foreground">
      <div className={`h-2 w-2 rounded-full ${healthy ? 'bg-success' : 'bg-error'}`} />
      <span className="text-foreground">{label}</span>
      {detail && <span className="truncate text-xs">· {detail}</span>}
    </div>
  )
}
