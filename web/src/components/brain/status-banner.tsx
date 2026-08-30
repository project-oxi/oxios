import { AlertTriangle } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { BrainStatus } from '@/types/brain'

/** Daemonless status banner. Online → hidden. Binary missing → install hint. */
export function StatusBanner({ status }: { status: BrainStatus | undefined }) {
  const { t } = useTranslation()
  if (!status || status.available) return null
  if (!status.binary.installed) {
    return (
      <div className="flex items-start gap-3 rounded-md border border-status-error bg-status-error/10 p-3 text-sm">
        <AlertTriangle className="h-4 w-4 mt-0.5 shrink-0 text-status-error" />
        <div>
          <p className="font-medium text-status-error">{t('brain.notInstalled')}</p>
          <p className="text-muted-foreground">{t('brain.notInstalledHint')}</p>
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
