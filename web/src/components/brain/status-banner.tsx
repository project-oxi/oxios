import { AlertTriangle, Loader2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { BrainStatus } from '@/types/brain'

/** Supervisor-state-aware banner. Online → hidden. */
export function StatusBanner({ status }: { status: BrainStatus | undefined }) {
  const { t } = useTranslation()
  if (!status || status.available) return null
  const sup = status.supervisor
  if (sup && (sup.state === 'installing' || sup.state === 'starting')) {
    return (
      <div className="flex items-start gap-3 rounded-md border border-status-info bg-status-info/10 p-3 text-sm">
        <Loader2 className="h-4 w-4 mt-0.5 shrink-0 animate-spin text-status-info" />
        <p className="font-medium">{t(`brain.${sup.state}`)}</p>
      </div>
    )
  }
  if (sup && sup.state === 'failed') {
    return (
      <div className="flex items-start gap-3 rounded-md border border-status-error bg-status-error/10 p-3 text-sm">
        <AlertTriangle className="h-4 w-4 mt-0.5 shrink-0 text-status-error" />
        <div>
          <p className="font-medium text-status-error">{t('brain.failedTitle')}</p>
          <p className="text-muted-foreground">{sup.last_error ?? t('brain.failedDescription')}</p>
        </div>
      </div>
    )
  }
  return (
    <div className="flex items-start gap-3 rounded-md border border-status-error bg-status-error/10 p-3 text-sm">
      <AlertTriangle className="h-4 w-4 mt-0.5 shrink-0 text-status-error" />
      <div>
        <p className="font-medium text-status-error">{t('brain.degradedTitle')}</p>
        <p className="text-muted-foreground">{t('brain.manualDescription')}</p>
      </div>
    </div>
  )
}
