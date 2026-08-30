// PreviewStage — the workbench stage that surfaces the session's latest
// completed renderable artifact (Task 7, design §7.2).
//
// The transcript renders artifacts inline via ArtifactCard; this stage pins
// the most recent one so the user can keep it visible while the conversation
// continues. Rendering goes through the SAME ArtifactRenderer the inline
// cards use (html/svg inline, mermaid/react lazy), and "Open in panel"
// hands the artifact to the portal side panel via toggleArtifact.

import { ExternalLink } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ArtifactRenderer } from '@/components/chat/artifact/artifact-renderer'
import { Button } from '@/components/ui/button'
import { findLatestSessionArtifact } from '@/lib/session-artifacts'
import { useChatStore } from '@/stores/chat'
import { usePortalStore } from '@/stores/portal'

export function PreviewStage() {
  const { t } = useTranslation()
  const messages = useChatStore((s) => s.messages)
  const toggleArtifact = usePortalStore((s) => s.toggleArtifact)

  const latest = findLatestSessionArtifact(messages)

  return (
    <div className="flex h-full flex-col" data-testid="preview-stage">
      <div className="flex items-center justify-between border-b px-3 py-2 text-xs">
        <span className="font-medium">{t('workbench.stage.preview')}</span>
        {latest && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-6 gap-1 px-2 text-2xs"
            onClick={() => toggleArtifact(latest.meta, latest.code)}
          >
            <ExternalLink className="h-3 w-3" />
            {t('workbench.preview.openInPanel')}
          </Button>
        )}
      </div>
      <div className="flex-1 overflow-y-auto">
        {latest ? (
          <div className="p-3" data-testid="preview-stage-artifact">
            <ArtifactRenderer
              type={latest.meta.type}
              code={latest.code}
              title={latest.meta.title}
            />
          </div>
        ) : (
          <div className="p-3 text-2xs text-muted-foreground">{t('workbench.preview.empty')}</div>
        )}
      </div>
    </div>
  )
}
