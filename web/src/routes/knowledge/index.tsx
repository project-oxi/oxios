import { createFileRoute } from '@tanstack/react-router'
import { KnowledgeWorkspace } from '@/components/knowledge/knowledge-workspace'

export const Route = createFileRoute('/knowledge/')({
  component: KnowledgeHomeRoute,
  validateSearch: (search: Record<string, unknown>) => ({
    space: (search.space as string) ?? '',
  }),
})

/**
 * Knowledge home (design §7.3): the unified memory+library workspace.
 * Three responsive panes spanning BOTH domains; the inspector's
 * "Add to Studio" action is wired internally to the StudioHandoffSheet.
 */
function KnowledgeHomeRoute() {
  const { space } = Route.useSearch()
  return <KnowledgeWorkspace space={space} />
}
