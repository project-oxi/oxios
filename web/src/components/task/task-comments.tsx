// TaskComments — comment thread inside the task detail dialog (RFC-043 §D4)

import { MessageSquare, Send, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { EmptyState } from '@/components/shared/empty-state'
import { LoadingCards } from '@/components/shared/loading'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import {
  useAddTaskComment,
  useDeleteTaskComment,
  useTaskComments,
  useUpdateTaskComment,
} from '@/hooks/use-tasks'
import { cn } from '@/lib/utils'
import type { TaskComment } from '@/types/task'

export interface TaskCommentsProps {
  taskId: string
}

export function TaskComments({ taskId }: TaskCommentsProps) {
  const { t } = useTranslation()
  const { data, isLoading } = useTaskComments(taskId)
  const add = useAddTaskComment(taskId)
  const update = useUpdateTaskComment(taskId)
  const remove = useDeleteTaskComment(taskId)
  const [draft, setDraft] = useState('')
  const [editingId, setEditingId] = useState<string | null>(null)
  const [editingDraft, setEditingDraft] = useState('')

  const comments = data?.comments ?? []

  const handleSubmit = () => {
    const content = draft.trim()
    if (!content) return
    add.mutate(
      { content },
      {
        onSuccess: () => {
          setDraft('')
          toast.success(t('tasks.commentAdded'))
        },
        onError: () => toast.error(t('tasks.commentAddFailed')),
      },
    )
  }

  const handleSaveEdit = (commentId: string) => {
    const content = editingDraft.trim()
    if (!content) return
    update.mutate(
      { commentId, content },
      {
        onSuccess: () => {
          setEditingId(null)
          setEditingDraft('')
          toast.success(t('tasks.commentUpdated'))
        },
        onError: () => toast.error(t('tasks.commentUpdateFailed')),
      },
    )
  }

  const handleDelete = (commentId: string) => {
    remove.mutate(commentId, {
      onSuccess: () => toast.success(t('tasks.commentDeleted')),
      onError: () => toast.error(t('tasks.commentDeleteFailed')),
    })
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-1.5">
        <MessageSquare className="h-3.5 w-3.5 text-muted-foreground" />
        <Label className="text-xs font-medium text-muted-foreground">
          {t('tasks.commentsTitle')}
        </Label>
      </div>

      {/* Composer */}
      <div className="space-y-2">
        <Textarea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder={t('tasks.commentPlaceholder')}
          rows={2}
        />
        <div className="flex justify-end">
          <Button
            size="sm"
            variant="outline"
            className="gap-1.5"
            onClick={handleSubmit}
            disabled={!draft.trim() || add.isPending}
          >
            <Send className="h-3 w-3" />
            {add.isPending ? t('tasks.commentSending') : t('tasks.commentSend')}
          </Button>
        </div>
      </div>

      {/* Thread */}
      {isLoading ? (
        <LoadingCards count={1} />
      ) : comments.length === 0 ? (
        <EmptyState
          size="compact"
          title={t('tasks.noComments')}
          description={t('tasks.noCommentsDescription')}
        />
      ) : (
        <div className="space-y-2 max-h-48 overflow-y-auto">
          {comments.map((c) => (
            <CommentItem
              key={c.id}
              comment={c}
              editing={editingId === c.id}
              draft={editingDraft}
              isPending={update.isPending || remove.isPending}
              onStartEdit={() => {
                setEditingId(c.id)
                setEditingDraft(c.content)
              }}
              onCancelEdit={() => {
                setEditingId(null)
                setEditingDraft('')
              }}
              onChangeDraft={setEditingDraft}
              onSaveEdit={() => handleSaveEdit(c.id)}
              onDelete={() => handleDelete(c.id)}
            />
          ))}
        </div>
      )}
    </div>
  )
}

function CommentItem({
  comment,
  editing,
  draft,
  isPending,
  onStartEdit,
  onCancelEdit,
  onChangeDraft,
  onSaveEdit,
  onDelete,
}: {
  comment: TaskComment
  editing: boolean
  draft: string
  isPending: boolean
  onStartEdit: () => void
  onCancelEdit: () => void
  onChangeDraft: (v: string) => void
  onSaveEdit: () => void
  onDelete: () => void
}) {
  const { t } = useTranslation()
  return (
    <div className="rounded-lg border p-2 text-xs">
      <div className="flex items-center justify-between gap-1 text-muted-foreground">
        <span className="font-mono">
          {comment.authorAgentId ?? t('tasks.commentAuthorUnknown')}
        </span>
        <span>{new Date(comment.createdAt).toLocaleString()}</span>
      </div>
      {editing ? (
        <div className="mt-1 space-y-1">
          <Textarea value={draft} onChange={(e) => onChangeDraft(e.target.value)} rows={2} />
          <div className="flex justify-end gap-1">
            <Button size="sm" variant="ghost" onClick={onCancelEdit}>
              {t('tasks.cancel')}
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={onSaveEdit}
              disabled={!draft.trim() || isPending}
            >
              {t('tasks.commentSave')}
            </Button>
          </div>
        </div>
      ) : (
        <>
          <p className={cn('mt-1 whitespace-pre-wrap text-foreground/90')}>{comment.content}</p>
          <div className="mt-1 flex justify-end gap-1">
            <Button size="sm" variant="ghost" className="h-6 text-2xs" onClick={onStartEdit}>
              {t('common.edit')}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="h-6 text-2xs text-muted-foreground hover:text-destructive"
              onClick={onDelete}
              aria-label={t('tasks.commentDelete')}
            >
              <Trash2 className="h-3 w-3" />
            </Button>
          </div>
        </>
      )}
    </div>
  )
}
