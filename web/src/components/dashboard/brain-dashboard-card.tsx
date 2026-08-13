import { BrainCircuit } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { useBrainStatus } from '@/hooks/use-brain'

/** Dashboard tile: brain daemon availability + episode count (RFC-047). */
export function BrainDashboardCard({ className }: { className?: string }) {
  const { t } = useTranslation()
  const { data: status } = useBrainStatus()

  return (
    <Card className={className}>
      <CardHeader className="flex flex-row items-center justify-between pb-2">
        <CardTitle className="flex items-center gap-2 text-base">
          <BrainCircuit className="h-4 w-4" />
          {t('brain.title')}
        </CardTitle>
      </CardHeader>
      <CardContent className="pt-0">
        {status?.available ? (
          <div className="space-y-1 text-xs">
            <div className="flex items-center justify-between">
              <span className="text-muted-foreground">{t('brain.episodes')}</span>
              <span className="font-medium">{status.episodes ?? 0}</span>
            </div>
            <div className="flex items-center justify-between">
              <span className="text-muted-foreground">{t('brain.available')}</span>
              <span className="font-medium text-status-success">{t('brain.online')}</span>
            </div>
          </div>
        ) : (
          <p className="text-xs text-muted-foreground py-1">{t('brain.degradedTitle')}</p>
        )}
      </CardContent>
    </Card>
  )
}
