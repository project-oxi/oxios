import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { KnowledgeWorkspace } from '@/components/knowledge/knowledge-workspace'
import { PageHeader } from '@/components/shared/page-header'

export const Route = createFileRoute('/knowledge/library/workspace')({
  component: LibraryWorkspacePage,
  validateSearch: (search: Record<string, unknown>) => ({
    space: (search.space as string) ?? '',
  }),
})

/** Unified memory+library workspace (design §7.3). The inspector's
 *  "Add to Studio" action is wired internally to the StudioHandoffSheet. */
function LibraryWorkspacePage() {
  const { t } = useTranslation()
  const { space } = Route.useSearch()
  // Touch useNavigate so the page keeps parity with the rest of the
  // library surface — future cross-pane navigation will land here.
  useNavigate({ from: Route.id })
  return (
    <div className="space-y-4">
      <PageHeader
        title={t('knowledge.workspace.title')}
        subtitle={t('knowledge.workspace.subtitle')}
      />
      <KnowledgeWorkspace space={space} />
    </div>
  )
}
