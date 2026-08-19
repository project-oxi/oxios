import { createFileRoute } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { BrainEntityDetail } from '@/components/brain/entity-detail'
import { PageHeader } from '@/components/shared/page-header'

export const Route = createFileRoute('/brain/entity')({ component: EntityPage })

function EntityPage() {
  const { t } = useTranslation()
  return (
    <div className="space-y-6">
      <PageHeader title={t('brain.entity')} subtitle={t('brain.subtitle')} />
      <BrainEntityDetail />
    </div>
  )
}
