import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { BrainSearch } from '@/components/brain/search'
import { StatusBanner } from '@/components/brain/status-banner'
import { PageHeader } from '@/components/shared/page-header'
import { useBrainStatus } from '@/hooks/use-brain'

export const Route = createFileRoute('/brain/search')({ component: SearchPage })

function SearchPage() {
  const { t } = useTranslation()
  const { data: status } = useBrainStatus()
  const navigate = useNavigate()
  return (
    <div className="space-y-6">
      <PageHeader title={t('brain.search')} subtitle={t('brain.subtitle')} />
      <StatusBanner status={status} />
      <BrainSearch onSelectEntity={(id) => navigate({ to: '/brain/entity', search: { id } })} />
    </div>
  )
}
