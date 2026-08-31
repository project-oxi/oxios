import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { BrainOverview } from '@/components/brain/overview'
import { DefaultBrainSelect, SpaceSelect } from '@/components/brain/space-select'
import { StatusBanner } from '@/components/brain/status-banner'
import { PageHeader } from '@/components/shared/page-header'
import { useBrainStatus } from '@/hooks/use-brain'

export const Route = createFileRoute('/knowledge/memory/')({
  component: MemoryOverviewPage,
  validateSearch: (search: Record<string, unknown>) => ({
    // Selected space for the scoped brain queries ('' = none picked yet).
    space: (search.space as string) ?? '',
  }),
})

function MemoryOverviewPage() {
  const { t } = useTranslation()
  const { data: status } = useBrainStatus()
  const { space } = Route.useSearch()
  const navigate = useNavigate({ from: Route.id })

  const selectSpace = (next: string) =>
    navigate({ search: (prev) => ({ ...prev, space: next }), replace: true })

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('knowledge.surface.memory')}
        subtitle={t('knowledge.surface.memorySubtitle')}
        actions={<DefaultBrainSelect className="w-44" />}
      />
      <StatusBanner status={status} />
      <SpaceSelect value={space} onValueChange={selectSpace} className="max-w-xs" />
      <BrainOverview
        space={space}
        onSelectSpace={selectSpace}
        onSelectEntity={(id) =>
          navigate({ to: '/knowledge/memory/entities', search: { id, space } })
        }
      />
    </div>
  )
}
