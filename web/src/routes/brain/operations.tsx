import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { BrainOperations } from '@/components/brain/operations'
import { SpaceSelect } from '@/components/brain/space-select'
import { StatusBanner } from '@/components/brain/status-banner'
import { PageHeader } from '@/components/shared/page-header'
import { useBrainStatus } from '@/hooks/use-brain'

export const Route = createFileRoute('/brain/operations')({
  component: OperationsPage,
  validateSearch: (search: Record<string, unknown>) => ({
    space: (search.space as string) ?? '',
  }),
})

function OperationsPage() {
  const { t } = useTranslation()
  const { data: status } = useBrainStatus()
  const { space } = Route.useSearch()
  const navigate = useNavigate({ from: Route.id })

  const selectSpace = (next: string) =>
    navigate({ search: (prev) => ({ ...prev, space: next }), replace: true })

  return (
    <div className="space-y-6">
      <PageHeader title={t('brain.operations')} subtitle={t('brain.operationsSubtitle')} />
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
