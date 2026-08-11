import { createFileRoute } from '@tanstack/react-router'
import { FolderPlus, Pencil, RefreshCw, Sparkles, Trash2, Wrench } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { CreateMountDialog } from '@/components/mount/create-mount-dialog'
import { EditMountDialog } from '@/components/mount/edit-mount-dialog'
import { EmptyState } from '@/components/shared/empty-state'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { PageHeader } from '@/components/shared/page-header'
import { RefreshButton } from '@/components/shared/refresh-button'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { useDeleteMount, useMounts, useRescanMount } from '@/hooks/use-mounts'
import type { Mount } from '@/types'

export const Route = createFileRoute('/mounts/')({ component: MountsPage })

function MountsPage() {
  const { t } = useTranslation()
  const [search, setSearch] = useState('')
  const [showCreate, setShowCreate] = useState(false)
  const [editingMount, setEditingMount] = useState<Mount | null>(null)
  const [deleteTarget, setDeleteTarget] = useState<Mount | null>(null)
  const { data, isLoading, isError, refetch } = useMounts(search || undefined)
  const deleteMount = useDeleteMount()
  const rescanMount = useRescanMount()

  const mounts = Array.isArray(data?.items) ? data.items : []

  const handleDelete = async (mount: Mount) => {
    try {
      await deleteMount.mutateAsync(mount.id)
      toast.success(t('mounts.deleted'))
    } catch (err) {
      toast.error(err instanceof Error ? err.message : t('mounts.deleteFailed'))
    }
  }

  const handleRescan = async (mount: Mount) => {
    try {
      await rescanMount.mutateAsync(mount.id)
      toast.success(t('mounts.rescanned'))
    } catch (err) {
      toast.error(err instanceof Error ? err.message : t('mounts.rescanFailed'))
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('mounts.title')}
        subtitle={t('mounts.desc')}
        actions={
          <Button onClick={() => setShowCreate(true)}>
            <FolderPlus className="h-4 w-4" />
            {t('mounts.create')}
          </Button>
        }
      />

      {/* Search */}
      <div className="flex items-center gap-2">
        <Input
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={t('mounts.searchPlaceholder')}
          className="max-w-xs"
        />
        <RefreshButton onClick={() => refetch()} />
      </div>

      {/* Content */}
      {isLoading ? (
        <LoadingCards />
      ) : isError ? (
        <ErrorState onRetry={() => refetch()} />
      ) : mounts.length === 0 ? (
        <EmptyState
          icon={<FolderPlus className="h-8 w-8" />}
          title={t('mounts.empty')}
          description={t('mounts.emptyDesc')}
          action={
            <Button onClick={() => setShowCreate(true)}>
              <FolderPlus className="h-4 w-4" />
              {t('mounts.create')}
            </Button>
          }
        />
      ) : (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {mounts.map((mount) => (
            <div
              key={mount.id}
              className="group relative rounded-lg border bg-card p-4 transition-all hover:shadow-sm"
            >
              {/* Action buttons: rescan + delete (top-right) */}

              <div className="absolute right-2 top-2 flex items-center gap-0.5">
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-8 w-8"
                  onClick={() => setEditingMount(mount)}
                  aria-label={t('common.edit')}
                >
                  <Pencil className="h-3.5 w-3.5" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-8 w-8"
                  onClick={() => handleRescan(mount)}
                  aria-label={t('mounts.rescan')}
                >
                  <RefreshCw className="h-3.5 w-3.5" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-8 w-8 text-destructive hover:text-destructive"
                  onClick={() => setDeleteTarget(mount)}
                  aria-label={t('common.delete')}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </div>

              {/* Name */}
              <div className="mb-2 flex items-center gap-2">
                {mount.source === 'auto_promoted' ? (
                  <Sparkles className="h-4 w-4 text-hue-purple" />
                ) : (
                  <Wrench className="h-4 w-4" />
                )}
                <h3 className="font-semibold truncate">{mount.name}</h3>
                {mount.source === 'auto_promoted' && (
                  <span className="rounded-full bg-hue-purple/10 px-2 py-0.5 text-xs text-hue-purple">
                    {t('mounts.autoPromoted')}
                  </span>
                )}
                {mount.enrichment_pending && (
                  <button
                    type="button"
                    onClick={() => handleRescan(mount)}
                    className="rounded-full bg-status-warning/10 px-2 py-0.5 text-xs text-status-warning-on-surface hover:bg-status-warning/20 transition-colors"
                    title={t('mounts.rescan')}
                  >
                    {t('mounts.needsRefresh')} ↻
                  </button>
                )}
              </div>

              {/* Path */}
              <p className="mb-2 text-xs text-muted-foreground truncate font-mono">
                {mount.paths[0] ?? '(no path)'}
              </p>

              {/* Auto-description */}
              {mount.auto_description && (
                <p className="mb-2 text-sm text-muted-foreground line-clamp-2">
                  {mount.auto_description}
                </p>
              )}

              {/* Languages + stack */}
              <div className="flex flex-wrap gap-1">
                {mount.auto_meta.languages.map((lang) => (
                  <span
                    key={lang}
                    className="rounded bg-primary/10 px-1.5 py-0.5 text-xs text-primary"
                  >
                    {lang}
                  </span>
                ))}
                {mount.auto_meta.stack.slice(0, 4).map((s) => (
                  <span
                    key={s}
                    className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground"
                  >
                    {s}
                  </span>
                ))}
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Create dialog */}
      <CreateMountDialog open={showCreate} onOpenChange={setShowCreate} />
      {/* Edit dialog */}
      <EditMountDialog mount={editingMount} onOpenChange={setEditingMount} />

      {/* Delete confirm */}
      <Dialog open={deleteTarget !== null} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('mounts.deleteConfirmTitle')}</DialogTitle>
            <DialogDescription>{t('mounts.deleteConfirmDescription')}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setDeleteTarget(null)}
              disabled={deleteMount.isPending}
            >
              {t('common.cancel')}
            </Button>
            <Button
              size="sm"
              variant="destructive"
              onClick={() => {
                if (!deleteTarget) return
                handleDelete(deleteTarget)
                setDeleteTarget(null)
              }}
              disabled={deleteMount.isPending}
            >
              {t('common.delete')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
