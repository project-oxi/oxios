import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { BrainOperations } from '@/components/brain/operations'
import { SpaceSelect } from '@/components/brain/space-select'
import { StatusBanner } from '@/components/brain/status-banner'
import { PageHeader } from '@/components/shared/page-header'
import { useBrainStatus } from '@/hooks/use-brain'

/**
 * Chronological composition of the memory-operations data the existing
 * `BrainOperations` view already renders (merges/failures/sources via the
 * same use-brain hooks). Reuses the existing operations component; the
 * heading is "Timeline" because the design calls this surface the
 * Memory/Timeline view.
 */
export const Route = createFileRoute('/knowledge/memory/timeline')({
  component: MemoryTimelinePage,
  validateSearch: (search: Record<string, unknown>) => ({
    space: (search.space as string) ?? '',
  }),
})

function MemoryTimelinePage() {
  const { t } = useTranslation()
  const { data: status } = useBrainStatus()
  const { space } = Route.useSearch()
  const navigate = useNavigate({ from: Route.id })

  const selectSpace = (next: string) =>
    navigate({ search: (prev) => ({ ...prev, space: next }), replace: true })

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('knowledge.surface.timeline')}
        subtitle={t('brain.operationsSubtitle')}
      />
      <StatusBanner status={status} />
      <SpaceSelect value={space} onValueChange={selectSpace} className="max-w-xs" />
      {space ? (
        <BrainOperations space={space} />
      ) : (
        <p className="text-sm text-muted-foreground" role="status">
          {t('brain.selectSpace')}
        </p>
      )}
    </div>
  )
}
