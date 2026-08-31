import { Link } from '@tanstack/react-router'
import { AlertTriangle, CheckCircle, CircleAlert, type LucideIcon, Plus } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { PageHeader } from '@/components/shared/page-header'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type { SystemHealth } from './runs'

const HEALTH_META: Record<
  SystemHealth['state'],
  { icon: LucideIcon; textClass: string; labelClass: string }
> = {
  healthy: {
    icon: CheckCircle,
    textClass: 'text-status-success-on-surface',
    labelClass: 'border-status-success-subtle-border bg-status-success-subtle',
  },
  attention: {
    icon: AlertTriangle,
    textClass: 'text-status-warning-on-surface',
    labelClass: 'border-status-warning-subtle-border bg-status-warning-subtle',
  },
  degraded: {
    icon: CircleAlert,
    textClass: 'text-status-error-on-surface',
    labelClass: 'border-status-error-subtle-border bg-status-error-subtle',
  },
}

/**
 * Command-center header (spec §1): contextual greeting, the single
 * system-health verdict, and New task as the primary action. Version
 * strings are deliberately NOT in the headline — they live in the
 * System details overflow (SystemLinks).
 */
export function CommandCenterHeader({ health }: { health: SystemHealth | null }) {
  const { t } = useTranslation()
  const hour = new Date().getHours()
  const greeting =
    hour < 12
      ? t('commandCenter.greeting.morning')
      : hour < 18
        ? t('commandCenter.greeting.afternoon')
        : t('commandCenter.greeting.evening')

  const meta = health ? HEALTH_META[health.state] : null
  const label = health ? t(health.labelKey) : ''
  const detail =
    health?.state === 'attention' && health.count
      ? t('commandCenter.health.attentionDetail', { count: health.count })
      : undefined

  return (
    <PageHeader
      title={greeting}
      display
      titleMeta={
        health && meta ? (
          <span
            title={detail}
            className={cn(
              'inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-2xs font-medium',
              meta.labelClass,
              meta.textClass,
            )}
            aria-label={detail ? `${label} — ${detail}` : label}
          >
            <meta.icon className="h-3 w-3" aria-hidden="true" />
            {label}
          </span>
        ) : null
      }
      actions={
        <Button asChild>
          <Link to="/chat">
            <Plus className="h-4 w-4" aria-hidden="true" />
            {t('commandCenter.newTask')}
          </Link>
        </Button>
      }
    />
  )
}
