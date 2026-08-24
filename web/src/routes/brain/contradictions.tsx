import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { BrainContradictions } from '@/components/brain/contradictions'
import { StatusBanner } from '@/components/brain/status-banner'
import { PageHeader } from '@/components/shared/page-header'
import { useBrainStatus } from '@/hooks/use-brain'

export const Route = createFileRoute('/brain/contradictions')({
  component: ContradictionsPage,
})

function ContradictionsPage() {
  const { t } = useTranslation()
  const { data: status } = useBrainStatus()
  const navigate = useNavigate()
  return (
    <div className="space-y-6">
      <PageHeader title={t('brain.contradictions')} subtitle={t('brain.subtitle')} />
      <StatusBanner status={status} />
      <BrainContradictions
        onSelectEntity={(id) => navigate({ to: '/brain/entity', search: { id } })}
      />
    </div>
  )
}
