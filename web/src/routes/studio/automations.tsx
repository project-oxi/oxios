// /studio/automations — route registration for the Automation surface.
// The page (chrome + list + editors) lives in components/automation/
// and is mounted standalone; the Studio shell (WT-3) embeds it later.

import { createFileRoute } from '@tanstack/react-router'
import { AutomationsPage } from '@/components/automation/automations-page'

export const Route = createFileRoute('/studio/automations')({
  component: AutomationsPage,
})
