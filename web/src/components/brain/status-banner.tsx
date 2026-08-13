import { AlertTriangle } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { BrainStatus } from '@/types/brain'

/** Degraded-mode banner: shown when the brain daemon is unreachable. */
export function StatusBanner({ status }: { status: BrainStatus | undefined }) {
  const { t } = useTranslation()
  if (!status || status.available) return null
  return (
    <div className="flex items-start gap-3 rounded-md border border-status-error bg-status-error/10 p-3 text-sm">
      <AlertTriangle className="h-4 w-4 mt-0.5 shrink-0 text-status-error" />
      <div>
        <p className="font-medium text-status-error">{t('brain.degradedTitle')}</p>
        <p className="text-muted-foreground">{t('brain.degradedDescription')}</p>
      </div>
    </div>
  )
}
