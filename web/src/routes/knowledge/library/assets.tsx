import { createFileRoute } from '@tanstack/react-router'
import { File, FileAudio, FileText, FileVideo, Image as ImageIcon } from 'lucide-react'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import { EmptyState } from '@/components/shared/empty-state'
import { PageHeader } from '@/components/shared/page-header'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { useKnowledgeRecursiveTree } from '@/hooks/use-knowledge'
import { flattenTree } from '@/lib/tree-utils'
import { useKnowledgeStore } from '@/stores/knowledge'
import type { KnowledgeTreeNode } from '@/types/knowledge'

export const Route = createFileRoute('/knowledge/library/assets')({
  component: LibraryAssetsPage,
})

const ASSET_EXTENSIONS: Record<string, true> = {
  png: true,
  jpg: true,
  jpeg: true,
  gif: true,
  svg: true,
  webp: true,
  bmp: true,
  mp3: true,
  wav: true,
  m4a: true,
  flac: true,
  ogg: true,
  mp4: true,
  mov: true,
  webm: true,
  mkv: true,
  pdf: true,
  zip: true,
  tar: true,
  gz: true,
}

const MARKDOWN_EXTENSIONS: Record<string, true> = {
  md: true,
  markdown: true,
}

const IMAGE_EXTS: Record<string, true> = {
  png: true,
  jpg: true,
  jpeg: true,
  gif: true,
  svg: true,
  webp: true,
  bmp: true,
}

const AUDIO_EXTS: Record<string, true> = {
  mp3: true,
  wav: true,
  m4a: true,
  flac: true,
  ogg: true,
}

const VIDEO_EXTS: Record<string, true> = {
  mp4: true,
  mov: true,
  webm: true,
  mkv: true,
}

function iconFor(ext: string): React.ReactNode {
  if (IMAGE_EXTS[ext]) return <ImageIcon className="h-4 w-4" />
  if (AUDIO_EXTS[ext]) return <FileAudio className="h-4 w-4" />
  if (VIDEO_EXTS[ext]) return <FileVideo className="h-4 w-4" />
  if (ext === 'pdf') return <FileText className="h-4 w-4" />
  return <File className="h-4 w-4" />
}

function basename(path: string): string {
  const idx = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'))
  return idx >= 0 ? path.slice(idx + 1) : path
}

function extOf(name: string): string {
  const dot = name.lastIndexOf('.')
  return dot >= 0 ? name.slice(dot + 1).toLowerCase() : ''
}

/** Compose the EXISTING recursive tree into a non-markdown asset listing. */
function LibraryAssetsPage() {
  const { t } = useTranslation()
  const { data: tree } = useKnowledgeRecursiveTree()
  const openFile = useKnowledgeStore((s) => s.openFile)

  const assets = useMemo(() => {
    if (!tree) return [] as KnowledgeTreeNode[]
    return flattenTree(tree).filter((node) => {
      if (node.is_dir) return false
      const ext = extOf(node.name)
      return Boolean(ext) && Boolean(ASSET_EXTENSIONS[ext]) && !MARKDOWN_EXTENSIONS[ext]
    })
  }, [tree])

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('knowledge.surface.assets')}
        subtitle={t('knowledge.surface.librarySubtitle')}
      />
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <FileText className="h-4 w-4" />
            {t('knowledge.files')}
          </CardTitle>
        </CardHeader>
        <CardContent>
          {assets.length === 0 ? (
            <EmptyState
              title={t('knowledge.workspace.emptyLibrary')}
              description={t('knowledge.noFilesYet')}
              size="compact"
            />
          ) : (
            <ul className="divide-y">
              {assets.map((asset) => {
                const ext = extOf(asset.name)
                return (
                  <li key={asset.path}>
                    <button
                      type="button"
                      onClick={() => openFile(asset.path)}
                      className="w-full flex items-center gap-3 py-2 px-2 text-left rounded transition-colors hover:bg-accent/40 focus-visible:bg-accent/40"
                    >
                      <span className="shrink-0 text-muted-foreground">{iconFor(ext)}</span>
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-sm font-medium">
                          {asset.display_name || basename(asset.path)}
                        </span>
                        <span className="block truncate text-2xs text-muted-foreground font-mono">
                          {asset.path}
                        </span>
                      </span>
                      <span className="shrink-0 rounded border px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wide text-muted-foreground">
                        {ext || '?'}
                      </span>
                    </button>
                  </li>
                )
              })}
            </ul>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
