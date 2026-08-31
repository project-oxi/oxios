import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { BrainSearch } from '@/components/brain/search'
import { SpaceSelect } from '@/components/brain/space-select'
import { StatusBanner } from '@/components/brain/status-banner'
import { PageHeader } from '@/components/shared/page-header'
import { useBrainStatus } from '@/hooks/use-brain'

export const Route = createFileRoute('/knowledge/memory/search')({
  component: MemorySearchPage,
  validateSearch: (search: Record<string, unknown>) => ({
    // Search is space-scoped server-side ('' = prompt for a space first).
    space: (search.space as string) ?? '',
  }),
})

function MemorySearchPage() {
  const { t } = useTranslation()
  const { data: status } = useBrainStatus()
  const { space } = Route.useSearch()
  const navigate = useNavigate({ from: Route.id })

  const selectSpace = (next: string) =>
    navigate({ search: (prev) => ({ ...prev, space: next }), replace: true })

  return (
    <div className="space-y-6">
      <PageHeader title={t('brain.search')} subtitle={t('knowledge.surface.memorySubtitle')} />
      <StatusBanner status={status} />
      <SpaceSelect value={space} onValueChange={selectSpace} className="max-w-xs" />
      {space ? (
        <BrainSearch
          space={space}
          onSelectEntity={(id) =>
            navigate({ to: '/knowledge/memory/entities', search: { id, space } })
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
