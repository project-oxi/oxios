import {
  Cable,
  CheckCircle2,
  CircleAlert,
  CircleX,
  Cpu,
  ExternalLink,
  ShieldCheck,
  Sparkles,
  Zap,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ErrorState } from '@/components/shared/error-state'
import { PageHeader } from '@/components/shared/page-header'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import { type OperateCapability, useOperateCapabilities } from '@/hooks/use-operate'
import { cn } from '@/lib/utils'

/**
 * Family order follows the frozen contract (mcp → skill → engine →
 * security → channel). A family with zero rows renders no panel.
 */
const FAMILY_ORDER = ['mcp', 'skill', 'engine', 'security', 'channel'] as const

const FAMILY_ICON: Record<string, React.ReactNode> = {
  mcp: <Zap className="h-4 w-4" aria-hidden="true" />,
  skill: <Sparkles className="h-4 w-4" aria-hidden="true" />,
  engine: <Cpu className="h-4 w-4" aria-hidden="true" />,
  security: <ShieldCheck className="h-4 w-4" aria-hidden="true" />,
  channel: <Cable className="h-4 w-4" aria-hidden="true" />,
}
/**
 * Capability hub (design §6) — every real capability the system actually
 * has, one row per source record, grouped by family. Each row is
 * source-labelled (family + scope where the source records one) and
 * deep-links to its canonical /operate/system/* route. Status is text +
 * icon + semantic color; no invented states.
 */
export function CapabilityHub() {
  const { t } = useTranslation()
  const capabilities = useOperateCapabilities()
  const families: Record<string, OperateCapability[]> = {}
  for (const row of capabilities.data?.families ?? []) {
    const list = families[row.family] ?? []
    list.push(row)
    families[row.family] = list
  }

  return (
    <div className="space-y-4 animate-fade-in-up">
      <PageHeader title={t('operate.capabilities')} subtitle={t('operate.capabilitiesSubtitle')} />

      {capabilities.isLoading && (
        <div className="space-y-2" aria-busy="true">
          {[0, 1, 2].map((i) => (
            <Skeleton key={i} className="h-16 w-full rounded-lg" />
          ))}
        </div>
      )}

      {capabilities.isError && <ErrorState onRetry={() => capabilities.refetch()} />}

      {FAMILY_ORDER.map((family) => {
        const rows = families[family] ?? []
        if (rows.length === 0) return null
        return (
          <Card key={family}>
            <CardHeader className="pb-2">
              <CardTitle className="flex items-center gap-2 text-base">
                {FAMILY_ICON[family]}
                {t(`operate.family.${family}`)}
              </CardTitle>
            </CardHeader>
            <CardContent className="pt-0">
              <ul className="space-y-1" data-testid={`capability-family-${family}`}>
                {rows.map((row) => (
                  <li key={row.id}>
                    <a
                      href={row.deepRoute}
                      className="flex flex-wrap items-center gap-2 rounded-md border px-2.5 py-2 text-sm transition-colors hover:bg-accent/50"
                    >
                      <span className="min-w-0 flex-1 truncate font-medium">{row.name}</span>
                      {row.scope && (
                        <span className="shrink-0 text-2xs text-muted-foreground">{row.scope}</span>
                      )}
                      <CapabilityStatus status={row.status} />
                      <ExternalLink
                        className="h-3 w-3 shrink-0 text-muted-foreground"
                        aria-hidden="true"
                      />
                    </a>
                  </li>
                ))}
              </ul>
            </CardContent>
          </Card>
        )
      })}
    </div>
  )
}

/** Status chip: icon + raw source status text + semantic color. */
function CapabilityStatus({ status }: { status: string }) {
  const meta = capabilityStatusMeta(status)
  const Icon = meta.icon
  return (
    <span
      className={cn(
        'inline-flex shrink-0 items-center gap-1 rounded-md border px-1.5 py-0.5 text-2xs font-medium',
        meta.classes,
      )}
    >
      <Icon className="h-3 w-3" aria-hidden="true" />
      {status}
    </span>
  )
}

/** Map the exact backend status strings to semantic tokens. */
function capabilityStatusMeta(status: string): {
  icon: typeof CheckCircle2
  classes: string
} {
  const s = status.toLowerCase()
  if (['connected', 'ready', 'configured', 'running', 'enabled'].includes(s))
    return {
      icon: CheckCircle2,
      classes:
        'border-status-success-subtle-border bg-status-success-subtle text-status-success-on-surface',
    }
  if (s === 'needs_setup')
    return {
      icon: CircleAlert,
      classes:
        'border-status-warning-subtle-border bg-status-warning-subtle text-status-warning-on-surface',
    }
  if (['disabled', 'unconfigured'].includes(s))
    return { icon: CircleX, classes: 'bg-muted text-muted-foreground' }
  // Approval modes (manual | allow-list | auto-run) and channel availability — config facts, not health.
  return {
    icon: CheckCircle2,
    classes: 'border-status-info-subtle-border bg-status-info-subtle text-status-info-on-surface',
  }
}
