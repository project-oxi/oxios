// ArtifactView — portal view body for an artifact (code source / live preview).
//
// Extracted from the former artifact-panel.tsx. The shared portal chrome
// (back / title / close) lives in portal-panel.tsx; this component owns the
// artifact-specific actions (code↔preview toggle, copy, download, version
// switcher) and the body renderer.

import { Check, ChevronLeft, ChevronRight, Code, Copy, Download, Eye } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ArtifactRenderer } from '@/components/chat/artifact/artifact-renderer'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { type PortalView, usePortalStore } from '@/stores/portal'
import { ArtifactType } from '@/types/artifact'

const TYPE_LABEL: Record<ArtifactType, string> = {
  [ArtifactType.Html]: 'HTML',
  [ArtifactType.Svg]: 'SVG',
  [ArtifactType.Mermaid]: 'Mermaid',
  [ArtifactType.React]: 'React',
}

const FILE_EXT: Record<ArtifactType, string> = {
  [ArtifactType.Html]: 'html',
  [ArtifactType.Svg]: 'svg',
  [ArtifactType.Mermaid]: 'mmd',
  [ArtifactType.React]: 'tsx',
}

interface ArtifactViewProps {
  view: Extract<PortalView, { type: 'artifact' }>
}

export function ArtifactView({ view }: ArtifactViewProps) {
  const { t } = useTranslation()
  const setDisplayMode = usePortalStore((s) => s.setArtifactDisplayMode)
  const setActiveVersion = usePortalStore((s) => s.setActiveVersion)
  const [copied, setCopied] = useState(false)

  const { meta, content, displayMode, key, versions, activeVersion } = view
  const { type, title } = meta
  const label = title ?? TYPE_LABEL[type]

  const copy = async () => {
    await navigator.clipboard.writeText(content)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  const download = () => {
    const blob = new Blob([content], { type: 'text/plain;charset=utf-8' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `${label.replace(/\s+/g, '-').toLowerCase() || 'artifact'}.${FILE_EXT[type]}`
    a.click()
    URL.revokeObjectURL(url)
  }

  return (
    <div className="flex h-full flex-col">
      {/* Artifact actions sub-header */}
      <div className="flex items-center justify-between gap-1 border-b px-2 py-1.5">
        {versions.length > 1 && (
          <div className="flex items-center gap-1 text-xs text-muted-foreground">
            <button
              type="button"
              disabled={activeVersion === 0}
              onClick={() => setActiveVersion(key, activeVersion - 1)}
              aria-label={t('artifact.versionPrev')}
              className="flex h-6 w-6 items-center justify-center rounded hover:bg-muted disabled:opacity-40 disabled:hover:bg-transparent"
            >
              <ChevronLeft className="h-3 w-3" />
            </button>
            <span>{t('artifact.version', { n: activeVersion + 1, total: versions.length })}</span>
            <button
              type="button"
              disabled={activeVersion === versions.length - 1}
              onClick={() => setActiveVersion(key, activeVersion + 1)}
              aria-label={t('artifact.versionNext')}
              className="flex h-6 w-6 items-center justify-center rounded hover:bg-muted disabled:opacity-40 disabled:hover:bg-transparent"
            >
              <ChevronRight className="h-3 w-3" />
            </button>
          </div>
        )}
        <div className="ms-auto flex items-center gap-1">
          <div className="flex items-center rounded-md border p-0.5">
            <button
              type="button"
              onClick={() => setDisplayMode(key, 'code')}
              className={cn(
                'flex h-7 items-center gap-1 rounded px-2 text-xs transition-colors',
                displayMode === 'code'
                  ? 'bg-secondary text-secondary-foreground'
                  : 'text-muted-foreground hover:text-foreground',
              )}
            >
              <Code className="size-3.5" />
              {t('artifact.code')}
            </button>
            <button
              type="button"
              onClick={() => setDisplayMode(key, 'preview')}
              className={cn(
                'flex h-7 items-center gap-1 rounded px-2 text-xs transition-colors',
                displayMode === 'preview'
                  ? 'bg-secondary text-secondary-foreground'
                  : 'text-muted-foreground hover:text-foreground',
              )}
            >
              <Eye className="size-3.5" />
              {t('artifact.preview')}
            </button>
          </div>

          <Button size="icon" variant="ghost" onClick={copy} aria-label={t('artifact.copy')}>
            {copied ? <Check className="size-4" /> : <Copy className="size-4" />}
          </Button>
          <Button
            size="icon"
            variant="ghost"
            onClick={download}
            aria-label={t('artifact.download')}
          >
            <Download className="size-4" />
          </Button>
        </div>
      </div>

      {/* Body */}
      <div className="min-h-0 flex-1">
        {displayMode === 'code' ? (
          <pre className="h-full overflow-auto bg-muted/40 p-4 font-mono text-xs leading-relaxed">
            <code>{content}</code>
          </pre>
        ) : (
          <ArtifactRenderer type={type} code={content} title={title} />
        )}
      </div>
    </div>
  )
}
