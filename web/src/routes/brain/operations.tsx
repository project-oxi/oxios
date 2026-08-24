import { createFileRoute } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { BrainOperations } from '@/components/brain/operations'
import { StatusBanner } from '@/components/brain/status-banner'
import { PageHeader } from '@/components/shared/page-header'
import { useBrainStatus } from '@/hooks/use-brain'

export const Route = createFileRoute('/brain/operations')({
  component: OperationsPage,
})

function OperationsPage() {
  const { t } = useTranslation()
  const { data: status } = useBrainStatus()
  return (
    <div className="space-y-6">
      <PageHeader title={t('brain.operations')} subtitle={t('brain.operationsSubtitle')} />
      <StatusBanner status={status} />
      <BrainOperations />
    </div>
  )
}
