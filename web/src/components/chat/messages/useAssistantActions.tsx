// useAssistantActions — action-bar handlers for assistant messages.
//
// Extracted from the legacy message-bubble.tsx so both AssistantMessage and
// future variants (supervisor, agent-council) can share the same action set.
//
// Phase 4 (2026-07-21): returns a MessageAction[] consumed by MessageActionBar,
// replacing the bespoke inline JSX.

import { Copy, RefreshCw, Trash2 } from 'lucide-react'
import { useCallback, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useShallow } from 'zustand/react/shallow'
import { useChatStore } from '@/stores/chat'
import type { ChatMessage } from '@/types'
import type { MessageAction } from './components/MessageActionBar'

interface UseAssistantActionsArgs {
  message: ChatMessage
  onRetry?: () => void
}

export interface AssistantActionsResult {
  actions: MessageAction[]
  copied: boolean
}

export function useAssistantActions({
  message,
  onRetry,
}: UseAssistantActionsArgs): AssistantActionsResult {
  const { t } = useTranslation()
  // Subscribe only to the action functions — `messages` was pulled in just to
  // locate the preceding user message for regenerate, and subscribing to it
  // re-rendered every action bar on every streaming token. Read it
  // imperatively inside the handler instead.
  const { removeMessage, sendMessage } = useChatStore(
    useShallow((s) => ({ removeMessage: s.removeMessage, sendMessage: s.sendMessage })),
  )
  const [copied, setCopied] = useState(false)

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(message.content)
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
  }, [message.content])

  const handleDelete = useCallback(() => {
    removeMessage?.(message.id)
  }, [message.id, removeMessage])

  const handleRegenerate = useCallback(() => {
    const { messages } = useChatStore.getState()
    const idx = messages.findIndex((m) => m.id === message.id)
    if (idx <= 0) return
    const precedingUser = messages[idx - 1]
    if (precedingUser?.role !== 'user') return
    removeMessage?.(message.id)
    removeMessage?.(precedingUser.id)
    sendMessage(precedingUser.content)
  }, [message.id, removeMessage, sendMessage])

  const isError = !!message.metadata?.isError

  const actions: MessageAction[] = [
    {
      id: 'copy',
      icon: <Copy className="w-3 h-3" />,
      label: copied ? t('common.copied') : t('common.copy'),
      onClick: handleCopy,
      children: copied ? <span className="text-2xs">{t('common.copied')}</span> : undefined,
    },
    {
      id: 'regenerate',
      icon: <RefreshCw className="w-3 h-3" />,
      label: t('chat.regenerate'),
      onClick: handleRegenerate,
      hidden: isError,
    },
    {
      id: 'retry',
      icon: <RefreshCw className="w-3 h-3" />,
      label: t('chat.retry'),
      onClick: onRetry ?? (() => {}),
      hidden: !isError || !onRetry,
      danger: true,
    },
    {
      id: 'delete',
      icon: <Trash2 className="w-3 h-3" />,
      label: t('common.delete'),
      onClick: handleDelete,
      danger: true,
    },
  ]

  return { actions, copied }
}
