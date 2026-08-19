import { createFileRoute } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { BrainContradictions } from '@/components/brain/contradictions'
import { PageHeader } from '@/components/shared/page-header'

export const Route = createFileRoute('/brain/contradictions')({
  component: ContradictionsPage,
})

function ContradictionsPage() {
  const { t } = useTranslation()
  return (
    <div className="space-y-6">
      <PageHeader title={t('brain.contradictions')} subtitle={t('brain.subtitle')} />
      <BrainContradictions />
    </div>
  )
}
