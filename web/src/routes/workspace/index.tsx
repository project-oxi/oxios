import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import {
  ChevronDown,
  ChevronRight,
  Eye,
  File,
  Folder,
  FolderOpen,
  Pencil,
  Plus,
  RefreshCw,
  Trash2,
  Upload,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { EmptyState } from '@/components/shared/empty-state'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { PageHeader } from '@/components/shared/page-header'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { CreateFileDialog } from '@/components/workspace/create-file-dialog'
import { FileBreadcrumb } from '@/components/workspace/file-breadcrumb'
import { FileEditor } from '@/components/workspace/file-editor'
import { FileViewer } from '@/components/workspace/file-viewer'
import { UploadDropZone } from '@/components/workspace/upload-drop-zone'
import { useCreateFile, useDeleteFile, useSaveFile } from '@/hooks/use-state-workspace'
import { api } from '@/lib/api-client'
import type { TreeEntry } from '@/types'
import { isEditable, isImage } from '@/types/workspace'

export const Route = createFileRoute('/workspace/')({ component: WorkspacePage })

type SelectedFile = {
  path: string
  mode: 'view' | 'edit'
}

/** Tailwind text tint for a file node by extension category — at-a-glance type ID. */
function fileTint(name: string): string {
  const ext = name.split('.').pop()?.toLowerCase() ?? ''
  if (['png', 'jpg', 'jpeg', 'gif', 'svg', 'webp'].includes(ext)) return 'text-hue-teal'
  if (['rs', 'ts', 'tsx', 'js', 'jsx', 'py', 'go', 'java', 'c', 'cpp', 'rb'].includes(ext))
    return 'text-hue-blue'
  if (['md', 'txt', 'json', 'toml', 'yaml', 'yml'].includes(ext)) return 'text-hue-amber'
  return 'text-muted-foreground'
}

function WorkspacePage() {
  const { t } = useTranslation()

  // Tree state: path-based expansion tracking
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set())
  const [selectedFile, setSelectedFile] = useState<SelectedFile | null>(null)
  const [showCreateDialog, setShowCreateDialog] = useState(false)
  const [showUpload, setShowUpload] = useState(false)
  const [editedContent, setEditedContent] = useState<string | null>(null)
  const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false)

  // Current directory context for breadcrumb + create
  const currentDir = useMemo(() => {
    if (!selectedFile) return ''
    const parts = selectedFile.path.split('/')
    parts.pop()
    return parts.join('/')
  }, [selectedFile])

  // --- Data fetching ---

  // Root tree
  const {
    data: rootEntries,
    isLoading: rootLoading,
    isError: rootError,
    refetch: refetchRoot,
    isFetching: rootFetching,
  } = useQuery({
    queryKey: ['workspace-tree'],
    queryFn: async () => {
      const res = await api.get<TreeEntry[]>('/api/workspace/tree')
      return Array.isArray(res) ? res : []
    },
    refetchInterval: 15000,
  })

  // Children for each expanded directory
  const expandedArr = useMemo(() => [...expandedPaths], [expandedPaths])
  const { data: childrenMap } = useQuery({
    queryKey: ['workspace-children', expandedArr],
    queryFn: async () => {
      const result: Record<string, TreeEntry[]> = {}
      for (const dir of expandedArr) {
        try {
          const res = await api.get<TreeEntry[]>(
            `/api/workspace/tree?dir=${encodeURIComponent(dir)}`,
          )
          result[dir] = Array.isArray(res) ? res : []
        } catch {
          result[dir] = []
        }
      }
      return result
    },
    enabled: expandedArr.length > 0,
  })

  // Selected file content
  const { data: fileData, isLoading: fileLoading } = useQuery({
    queryKey: ['workspace-file', selectedFile?.path],
    queryFn: async () => {
      if (!selectedFile) return null
      const res = await api.get<string>(
        `/api/workspace/file/${encodeURIComponent(selectedFile.path)}`,
      )
      return res
    },
    enabled: !!selectedFile,
  })

  // --- Mutations ---
  const saveFile = useSaveFile()
  const createFile = useCreateFile()
  const deleteFile = useDeleteFile()

  // --- Handlers ---

  const toggleExpand = useCallback((path: string) => {
    setExpandedPaths((prev) => {
      const next = new Set(prev)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      return next
    })
  }, [])

  const handleFileClick = useCallback(
    (entry: TreeEntry, parentPath: string) => {
      if (entry.is_dir) {
        const fullPath = parentPath ? `${parentPath}/${entry.name}` : entry.name
        toggleExpand(fullPath)
      } else {
        const fullPath = parentPath ? `${parentPath}/${entry.name}` : entry.name
        setSelectedFile({ path: fullPath, mode: 'view' })
      }
    },
    [toggleExpand],
  )

  const handleDoubleClick = useCallback((entry: TreeEntry, parentPath: string) => {
    if (entry.is_dir) return
    const fullPath = parentPath ? `${parentPath}/${entry.name}` : entry.name
    if (isEditable(fullPath)) {
      setSelectedFile({ path: fullPath, mode: 'edit' })
    }
  }, [])

  const handleBreadcrumbNavigate = useCallback(
    (dir: string) => {
      setSelectedFile(null)
      if (dir) {
        toggleExpand(dir)
      }
    },
    [toggleExpand],
  )

  const handleSave = useCallback(
    (content: string) => {
      if (!selectedFile) return
      saveFile.mutate({ path: selectedFile.path, content })
    },
    [selectedFile, saveFile],
  )
  // Reset the lifted editor buffer whenever the open file changes, so the
  // toolbar Save never writes a previous file's content into the new one.
  useEffect(() => {
    setEditedContent(null)
  }, [selectedFile?.path])

  const handleCreate = useCallback(
    (fullPath: string, isDir: boolean) => {
      createFile.mutate({ path: fullPath, isDir }, { onSuccess: () => refetchRoot() })
    },
    [createFile, refetchRoot],
  )

  const handleDelete = useCallback(() => {
    if (!selectedFile) return
    setDeleteConfirmOpen(true)
  }, [selectedFile])

  const confirmDelete = () => {
    if (!selectedFile) return
    deleteFile.mutate(selectedFile.path, {
      onSuccess: () => {
        setSelectedFile(null)
        setDeleteConfirmOpen(false)
        refetchRoot()
      },
    })
  }

  // --- Render helpers ---

  const renderEntry = (entry: TreeEntry, parentPath: string, depth: number = 0) => {
    const fullPath = parentPath ? `${parentPath}/${entry.name}` : entry.name
    const isExpanded = expandedPaths.has(fullPath)
    const isSelected = selectedFile?.path === fullPath

    return (
      <div key={fullPath}>
        <div
          role="treeitem"
          tabIndex={0}
          className={`flex items-center gap-2 py-1.5 px-2 hover:bg-muted/50 rounded cursor-pointer text-sm ${
            isSelected ? 'bg-primary/10 text-primary' : ''
          }`}
          style={{ paddingLeft: `${depth * 16 + 8}px` }}
          onClick={() => handleFileClick(entry, parentPath)}
          onDoubleClick={() => handleDoubleClick(entry, parentPath)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault()
              handleFileClick(entry, parentPath)
            }
          }}
        >
          {entry.is_dir ? (
            <>
              {isExpanded ? (
                <ChevronDown className="h-4 w-4 shrink-0" />
              ) : (
                <ChevronRight className="h-4 w-4 shrink-0" />
              )}
              <Folder className="h-4 w-4 text-warning shrink-0" />
            </>
          ) : (
            <>
              <span className="w-4" />
              <File className={`h-4 w-4 ${fileTint(entry.name)} shrink-0`} />
            </>
          )}
          <span className="truncate">{entry.name}</span>
          {!entry.is_dir && entry.size > 0 && (
            <span className="ml-auto text-xs text-muted-foreground">
              {entry.size > 1024 ? `${(entry.size / 1024).toFixed(1)}KB` : `${entry.size}B`}
            </span>
          )}
        </div>
        {isExpanded &&
          entry.is_dir &&
          (Array.isArray(childrenMap?.[fullPath]) ? childrenMap[fullPath] : []).map((child) =>
            renderEntry(child, fullPath, depth + 1),
          )}
      </div>
    )
  }

  // --- Main render ---

  if (rootLoading) return <LoadingCards count={4} />
  if (rootError) return <ErrorState onRetry={() => refetchRoot()} />

  return (
    <div className="flex flex-col h-full gap-4">
      {/* Header */}
      <PageHeader
        title={t('workspace.title')}
        subtitle={t('workspace.description')}
        actions={
          <>
            <Button variant="outline" size="sm" onClick={() => setShowCreateDialog(true)}>
              <Plus className="h-4 w-4" /> {t('common.create')}
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => setShowUpload(!showUpload)}
              aria-label={t('workspace.uploadFile')}
            >
              <Upload className="h-4 w-4" /> {t('workspace.uploadFile')}
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => refetchRoot()}
              disabled={rootFetching}
            >
              <RefreshCw className={`h-4 w-4 ${rootFetching ? 'animate-spin' : ''}`} />
            </Button>
          </>
        }
      />

      {/* Split layout */}
      <div className="flex flex-1 gap-4 min-h-0">
        {/* Left panel: File tree */}
        <div className="w-72 shrink-0 border rounded-lg overflow-y-auto">
          <div className="p-2">
            <div className="flex items-center justify-between px-2 py-1.5 mb-1">
              <span className="text-sm font-medium flex items-center gap-2">
                <FolderOpen className="h-4 w-4" /> {t('workspace.files')}
              </span>
            </div>
            {!rootEntries || rootEntries.length === 0 ? (
              <EmptyState
                icon={<FolderOpen className="h-8 w-8" />}
                title={t('workspace.noWorkspace')}
                description={t('workspace.description')}
                className="py-6"
              />
            ) : (
              <div className="space-y-0">{rootEntries.map((entry) => renderEntry(entry, ''))}</div>
            )}
          </div>
          {/* Upload zone */}
          {showUpload && (
            <div className="px-2 pb-2">
              <UploadDropZone currentDir={currentDir} onUploaded={() => refetchRoot()} />
            </div>
          )}
        </div>

        {/* Right panel: File viewer/editor */}
        <div className="flex-1 flex flex-col min-w-0 border rounded-lg overflow-hidden">
          {selectedFile ? (
            <>
              {/* Toolbar */}
              <div className="flex items-center justify-between px-3 py-2 border-b shrink-0">
                <FileBreadcrumb path={selectedFile.path} onNavigate={handleBreadcrumbNavigate} />
                <div className="flex items-center gap-1">
                  {isEditable(selectedFile.path) && (
                    <>
                      <Button
                        variant={selectedFile.mode === 'view' ? 'secondary' : 'ghost'}
                        size="sm"
                        className="h-7 px-2"
                        onClick={() => setSelectedFile((s) => s && { ...s, mode: 'view' })}
                        aria-label={t('common.view')}
                      >
                        <Eye className="h-3.5 w-3.5" />
                      </Button>
                      <Button
                        variant={selectedFile.mode === 'edit' ? 'secondary' : 'ghost'}
                        size="sm"
                        className="h-7 px-2"
                        onClick={() => setSelectedFile((s) => s && { ...s, mode: 'edit' })}
                        aria-label={t('common.edit')}
                      >
                        <Pencil className="h-3.5 w-3.5" />
                      </Button>
                    </>
                  )}
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-7 px-2 text-destructive hover:text-destructive"
                    onClick={handleDelete}
                    aria-label={t('common.delete')}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                  {selectedFile.mode === 'edit' && (
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-7 px-2 text-xs"
                      onClick={() => {
                        if (editedContent != null) handleSave(editedContent)
                      }}
                      disabled={editedContent == null || saveFile.isPending}
                    >
                      Save
                    </Button>
                  )}
                </div>
              </div>

              {/* Content */}
              <div className="flex-1 min-h-0">
                {fileLoading ? (
                  <div className="flex items-center justify-center h-full text-muted-foreground text-sm">
                    Loading...
                  </div>
                ) : selectedFile.mode === 'edit' && isEditable(selectedFile.path) ? (
                  <FileEditor
                    path={selectedFile.path}
                    content={fileData ?? ''}
                    onSave={handleSave}
                    onChange={setEditedContent}
                  />
                ) : isImage(selectedFile.path) ? (
                  <div className="flex items-center justify-center h-full p-4">
                    <img
                      src={`/api/workspace/file/${encodeURIComponent(selectedFile.path)}`}
                      alt={selectedFile.path}
                      className="max-w-full max-h-full object-contain"
                    />
                  </div>
                ) : (
                  <FileViewer path={selectedFile.path} content={fileData ?? ''} />
                )}
              </div>
            </>
          ) : (
            <div className="flex items-center justify-center h-full text-muted-foreground">
              <div className="text-center space-y-2">
                <File className="h-10 w-10 mx-auto opacity-50" />
                <p className="text-sm">{t('workspace.selectFile')}</p>
                <p className="text-xs">{t('workspace.doubleClickEdit')}</p>
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Create File Dialog */}
      <CreateFileDialog
        open={showCreateDialog}
        onOpenChange={setShowCreateDialog}
        currentDir={currentDir}
        onSubmit={handleCreate}
      />
      <Dialog open={deleteConfirmOpen} onOpenChange={setDeleteConfirmOpen}>
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>{t('workspace.deleteConfirmTitle')}</DialogTitle>
            <DialogDescription>
              {t('workspace.deleteConfirmDesc', { path: selectedFile?.path ?? '' })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setDeleteConfirmOpen(false)}
              disabled={deleteFile.isPending}
            >
              {t('common.cancel')}
            </Button>
            <Button
              variant="destructive"
              size="sm"
              onClick={confirmDelete}
              disabled={deleteFile.isPending}
            >
              {t('common.delete')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
