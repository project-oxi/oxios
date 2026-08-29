// PreviewStage — renderable-artifact preview placeholder (Task 7).
//
// The active session transcript already renders artifacts inline; this
// stage is a quick-access surface so the user can pin a preview while
// continuing the conversation. It also doubles as the persistence target
// for any "preview" rail click in case the workbench wants to surface
// the same artifact in a side panel without going through the portal.

import { useTranslation } from 'react-i18next'
import { useSessionChanges } from '@/hooks/use-session-changes'

export function PreviewStage() {
  const { t } = useTranslation()
  // The current build renders an empty placeholder; this file exists so the
  // stage panel can be selected without throwing. Renderable artifacts
  // continue to live inline in the transcript — the preview rail is the
  // pinned spotlight for the most recent one (Task 8 will surface the
  // artifact-store here).
  const changes = useSessionChanges()
  return (
    <div className="flex h-full flex-col" data-testid="preview-stage">
      <div className="flex items-center justify-between border-b px-3 py-2 text-xs">
        <span className="font-medium">{t('workbench.stage.preview')}</span>
      </div>
      <div className="flex-1 overflow-y-auto p-3 text-2xs text-muted-foreground">
        {changes.length === 0
          ? t('workbench.preview.empty')
          : t('workbench.preview.recentCount', { count: changes.length })}
      </div>
    </div>
  )
}
