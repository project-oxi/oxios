import { createFileRoute } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { BrainContradictions } from '@/components/brain/contradictions'
import { BrainEntityDetail } from '@/components/brain/entity-detail'
import { BrainOverview } from '@/components/brain/overview'
import { BrainSearch } from '@/components/brain/search'
import { StatusBanner } from '@/components/brain/status-banner'
import { PageHeader } from '@/components/shared/page-header'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { useBrainStatus } from '@/hooks/use-brain'

export const Route = createFileRoute('/brain')({ component: BrainPage })

function BrainPage() {
  const { t } = useTranslation()
  const { data: status } = useBrainStatus()

  return (
    <div className="space-y-6">
      <PageHeader title={t('brain.title')} subtitle={t('brain.subtitle')} />
      <StatusBanner status={status} />
      <Tabs defaultValue="overview">
        <TabsList>
          <TabsTrigger value="overview">{t('brain.overview')}</TabsTrigger>
          <TabsTrigger value="search">{t('brain.search')}</TabsTrigger>
          <TabsTrigger value="entity">{t('brain.entity')}</TabsTrigger>
          <TabsTrigger value="contradictions">{t('brain.contradictions')}</TabsTrigger>
        </TabsList>
        <TabsContent value="overview" className="mt-4">
          <BrainOverview />
        </TabsContent>
        <TabsContent value="search" className="mt-4">
          <BrainSearch />
        </TabsContent>
        <TabsContent value="entity" className="mt-4">
          <BrainEntityDetail />
        </TabsContent>
        <TabsContent value="contradictions" className="mt-4">
          <BrainContradictions />
        </TabsContent>
      </Tabs>
    </div>
  )
}
