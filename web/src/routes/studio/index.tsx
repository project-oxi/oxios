import { createFileRoute } from '@tanstack/react-router'
import { MessageSquare } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { EmptyState } from '@/components/shared/empty-state'

/**
 * Studio landing route — a shell-safe placeholder. The conversation-first
 * Studio workbench is owned by the wave-2 Studio worker, which recomposes
 * the legacy chat route into this file.
 */
function StudioIndex() {
  const { t } = useTranslation()
  return (
    <EmptyState
      icon={<MessageSquare className="h-10 w-10" />}
      title={t('sidebar.studio')}
      description={t('common.surfacePlaceholderBody')}
    />
  )
}

export const Route = createFileRoute('/studio/')({
  component: StudioIndex,
})
