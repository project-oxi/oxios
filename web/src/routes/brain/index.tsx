import { createFileRoute } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { BrainOverview } from '@/components/brain/overview'
import { StatusBanner } from '@/components/brain/status-banner'
import { PageHeader } from '@/components/shared/page-header'
import { useBrainStatus } from '@/hooks/use-brain'

export const Route = createFileRoute('/brain/')({ component: BrainOverviewPage })

function BrainOverviewPage() {
  const { t } = useTranslation()
  const { data: status } = useBrainStatus()
  return (
    <div className="space-y-6">
      <PageHeader title={t('brain.title')} subtitle={t('brain.subtitle')} />
      <StatusBanner status={status} />
      <BrainOverview />
    </div>
  )
}
