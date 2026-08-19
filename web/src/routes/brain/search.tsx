import { createFileRoute } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { BrainSearch } from '@/components/brain/search'
import { PageHeader } from '@/components/shared/page-header'

export const Route = createFileRoute('/brain/search')({ component: SearchPage })

function SearchPage() {
  const { t } = useTranslation()
  return (
    <div className="space-y-6">
      <PageHeader title={t('brain.search')} subtitle={t('brain.subtitle')} />
      <BrainSearch />
    </div>
  )
}
