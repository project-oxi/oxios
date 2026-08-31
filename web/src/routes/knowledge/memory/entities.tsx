import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { BrainEntityDetail } from '@/components/brain/entity-detail'
import { SpaceSelect } from '@/components/brain/space-select'
import { StatusBanner } from '@/components/brain/status-banner'
import { PageHeader } from '@/components/shared/page-header'
import { useBrainStatus } from '@/hooks/use-brain'

export const Route = createFileRoute('/knowledge/memory/entities')({
  component: MemoryEntityPage,
  validateSearch: (search: Record<string, unknown>) => ({
    id: (search.id as string) || undefined,
    space: (search.space as string) ?? '',
  }),
})

function MemoryEntityPage() {
  const { t } = useTranslation()
  const { data: status } = useBrainStatus()
  const { id, space } = Route.useSearch()
  const navigate = useNavigate({ from: Route.id })

  // Deep-link recovery: /knowledge/memory/entities without a `space` param is
  // a dead end (every scoped query is disabled) — same SpaceSelect affordance
  // as the other memory routes; picking one navigates with the param set.
  const selectSpace = (next: string) =>
    navigate({ search: (prev) => ({ ...prev, space: next }), replace: true })

  return (
    <div className="space-y-6">
      <PageHeader title={t('brain.entity')} subtitle={t('knowledge.surface.memorySubtitle')} />
      <StatusBanner status={status} />
      <SpaceSelect value={space} onValueChange={selectSpace} className="max-w-xs" />
      {space ? (
        <BrainEntityDetail
          space={space}
          initialEntityId={id}
          onNavigateEntity={(entityId) =>
            navigate({
              to: '/knowledge/memory/entities',
              search: { id: entityId, space },
              replace: true,
            })
          }
        />
      ) : (
        <p className="text-sm text-muted-foreground" role="status">
          {t('brain.selectSpace')}
        </p>
      )}
    </div>
  )
}
