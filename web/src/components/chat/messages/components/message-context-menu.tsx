// MessageContextMenu — native right-click context menu for chat messages.
//
// LobeHub analogue: hooks/useChatItemContextMenu.tsx (edit/copy/delete/regenerate).
// Oxios version uses a native positioned menu (no radix dependency needed).
// Wraps message content; on right-click shows Copy / Regenerate (assistant) /
// Delete actions.

import { Copy, GitBranch, RefreshCw, Trash2 } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { cn } from '@/lib/utils'
import { useChatStore } from '@/stores/chat'
import { usePortalStore } from '@/stores/portal'
import type { ChatMessage } from '@/types'

interface MenuState {
  x: number
  y: number
}

interface MessageContextMenuProps {
  message: ChatMessage
  onRetry?: () => void
  children: React.ReactNode
}

export function MessageContextMenu({ message, onRetry, children }: MessageContextMenuProps) {
  const { t } = useTranslation()
  const [menu, setMenu] = useState<MenuState | null>(null)
  const removeMessage = useChatStore((s) => s.removeMessage)
  const sendMessage = useChatStore((s) => s.sendMessage)
  const messages = useChatStore((s) => s.messages)

  const open = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    e.stopPropagation()
    setMenu({ x: e.clientX, y: e.clientY })
  }, [])

  const close = useCallback(() => setMenu(null), [])

  useEffect(() => {
    if (!menu) return
    window.addEventListener('click', close)
    window.addEventListener('resize', close)
    window.addEventListener('scroll', close, true)
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && close()
    document.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('click', close)
      window.removeEventListener('resize', close)
      window.removeEventListener('scroll', close, true)
      document.removeEventListener('keydown', onKey)
    }
  }, [menu, close])

  const handleCopy = () => {
    navigator.clipboard.writeText(message.content)
    close()
  }

  const handleRegenerate = () => {
    const idx = messages.findIndex((m) => m.id === message.id)
    if (idx <= 0) return
    const precedingUser = messages[idx - 1]
    if (precedingUser?.role !== 'user') return
    removeMessage?.(message.id)
    removeMessage?.(precedingUser.id)
    sendMessage(precedingUser.content)
    close()
  }

  const handleDelete = () => {
    removeMessage?.(message.id)
    close()
  }

  const isError = !!message.metadata?.isError
  const canRegenerate = message.role === 'assistant' && !isError

  // Clamp menu position to viewport
  const top = menu ? Math.min(menu.y, window.innerHeight - 160) : 0
  const left = menu ? Math.min(menu.x, window.innerWidth - 180) : 0

  return (
    <>
      <div onContextMenu={open}>{children}</div>
      {menu && (
        <div
          className="fixed z-50 min-w-[160px] rounded-lg border bg-popover p-1 shadow-lg"
          style={{ top, left }}
          onClick={(e) => e.stopPropagation()}
        >
          <ContextItem icon={Copy} label={t('common.copy')} onClick={handleCopy} />
          <ContextItem
            icon={GitBranch}
            label={t('chat.branchHere')}
            onClick={() => {
              useChatStore.getState().branchFrom(message.id)
              close()
            }}
          />
          <ContextItem
            icon={GitBranch}
            label={t('portal.createThread')}
            onClick={() => {
              usePortalStore
                .getState()
                .pushView({ type: 'thread', sessionId: null, parentId: message.id })
              close()
            }}
          />
          {canRegenerate && (
            <ContextItem icon={RefreshCw} label={t('chat.regenerate')} onClick={handleRegenerate} />
          )}
          {message.role === 'assistant' && isError && onRetry && (
            <ContextItem
              icon={RefreshCw}
              label={t('chat.retry')}
              onClick={() => {
                onRetry()
                close()
              }}
              danger
            />
          )}
          <ContextItem icon={Trash2} label={t('common.delete')} onClick={handleDelete} danger />
        </div>
      )}
    </>
  )
}

function ContextItem({
  icon: Icon,
  label,
  onClick,
  danger,
}: {
  icon: typeof Copy
  label: string
  onClick: () => void
  danger?: boolean
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-xs transition-colors',
        danger ? 'text-destructive hover:bg-destructive/10' : 'text-foreground hover:bg-accent',
      )}
    >
      <Icon className="size-3.5 shrink-0" />
      {label}
    </button>
  )
}
