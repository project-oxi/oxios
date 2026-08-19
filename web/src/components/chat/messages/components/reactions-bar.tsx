// ReactionsBar — displays existing emoji reactions on a message + the
// ReactionPicker trigger. Toggling an existing reaction re-toggles it.
//
// Also hosts the 👍/👎 answer rating (Task 22): the rating is stored in
// ChatMessage.metadata.rating via the chat store — in-memory only, mirroring
// the localStorage emoji reactions' single-user scope.

import { ThumbsDown, ThumbsUp } from 'lucide-react'
import { useCallback } from 'react'
import { useTranslation } from 'react-i18next'
import { listReactions, toggleReaction } from '@/lib/reactions-storage'
import { cn } from '@/lib/utils'
import { useChatStore } from '@/stores/chat'
import { ReactionPicker } from './reaction-picker'

interface ReactionsBarProps {
  messageId: string
  /** Force re-render when localStorage changes (parent provides a version counter). */
  version: number
  className?: string
}

export function ReactionsBar({ messageId, version, className }: ReactionsBarProps) {
  const { t } = useTranslation()
  const rateMessage = useChatStore((s) => s.rateMessage)
  const rating = useChatStore(
    useCallback((s) => s.messages.find((m) => m.id === messageId)?.metadata?.rating, [messageId]),
  )

  const handleSelect = useCallback(
    (emoji: string) => {
      toggleReaction(messageId, emoji)
      // Trigger parent re-render via a custom event since we don't own the state.
      window.dispatchEvent(new CustomEvent('reactions-changed'))
    },
    [messageId],
  )

  const handleToggleExisting = useCallback(
    (emoji: string) => {
      toggleReaction(messageId, emoji)
      window.dispatchEvent(new CustomEvent('reactions-changed'))
    },
    [messageId],
  )

  const handleRate = useCallback(
    (value: 1 | -1) => {
      // Toggle: re-clicking the active rating clears it.
      rateMessage(messageId, rating === value ? null : value)
    },
    [messageId, rating, rateMessage],
  )

  // `version` is consumed here so the parent re-render propagates to this bar.
  void version
  const reactions = listReactions(messageId)

  const ratingButton =
    'inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs transition-colors hover:bg-muted'

  return (
    <div className={cn('flex flex-wrap items-center gap-1', className)}>
      <button
        type="button"
        onClick={() => handleRate(1)}
        aria-label={t('chat.rateUp')}
        aria-pressed={rating === 1}
        className={cn(ratingButton, rating === 1 && 'border-primary/40 bg-primary/10 text-primary')}
      >
        <ThumbsUp className="h-3 w-3" />
      </button>
      <button
        type="button"
        onClick={() => handleRate(-1)}
        aria-label={t('chat.rateDown')}
        aria-pressed={rating === -1}
        className={cn(
          ratingButton,
          rating === -1 && 'border-destructive/40 bg-destructive/10 text-destructive',
        )}
      >
        <ThumbsDown className="h-3 w-3" />
      </button>
      {reactions.map(({ emoji }) => (
        <button
          key={emoji}
          type="button"
          onClick={() => handleToggleExisting(emoji)}
          className="flex items-center gap-1 rounded-full border bg-muted/40 px-2 py-0.5 text-xs transition-colors hover:bg-muted"
          aria-label={`Toggle reaction ${emoji}`}
        >
          <span>{emoji}</span>
        </button>
      ))}
      <ReactionPicker onSelect={handleSelect} />
    </div>
  )
}
