import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { BrainEntityDetail } from '@/components/brain/entity-detail'
import { StatusBanner } from '@/components/brain/status-banner'
import { PageHeader } from '@/components/shared/page-header'
import { useBrainStatus } from '@/hooks/use-brain'

export const Route = createFileRoute('/brain/entity')({
  component: EntityPage,
  validateSearch: (search: Record<string, unknown>) => ({
    id: (search.id as string) || undefined,
  }),
})

function EntityPage() {
  const { t } = useTranslation()
  const { data: status } = useBrainStatus()
  const { id } = Route.useSearch()
  const navigate = useNavigate({ from: Route.id })
  return (
    <div className="space-y-6">
      <PageHeader title={t('brain.entity')} subtitle={t('brain.subtitle')} />
      <StatusBanner status={status} />
      <BrainEntityDetail
        initialEntityId={id}
        onNavigateEntity={(entityId) =>
          navigate({ to: '/brain/entity', search: { id: entityId }, replace: true })
        }
      />
    </div>
  )
}
