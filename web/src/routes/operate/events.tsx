import { createFileRoute } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { LiveActivityFeed } from '@/components/dashboard/live-activity-feed'
import { PageHeader } from '@/components/shared/page-header'

export const Route = createFileRoute('/operate/events')({ component: EventsPage })

/**
 * Full event timeline — the deep-investigation surface the dashboard's
 * Live timeline panel links to (command-center spec §4). Reuses the
 * standalone LiveActivityFeed (card variant) with its pause, filter, and
 * capped-list behavior; no second event transport.
 */
function EventsPage() {
  const { t } = useTranslation()

  return (
    <div className="space-y-4 animate-fade-in-up">
      <PageHeader title={t('events.title')} subtitle={t('events.subtitle')} />
      <div className="h-[calc(100dvh-190px)] min-h-[420px]">
        <LiveActivityFeed />
      </div>
    </div>
  )
}
