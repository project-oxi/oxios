// messages/UserMessage — right-aligned plain text (no bubble) with edit + delete.

import { Pencil, Trash2 } from 'lucide-react'
import { memo, useCallback, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ChatItem } from '@/components/chat/chat-item'
import { useChatStore } from '@/stores/chat'
import type { ChatMessage } from '@/types'
import type { MessageAction } from './components/MessageActionBar'
import { MessageActionBar } from './components/MessageActionBar'

interface UserMessageProps {
  message: ChatMessage
}

function UserMessageImpl({ message }: UserMessageProps) {
  const { t } = useTranslation()
  const { removeMessage, sendMessage } = useChatStore()
  const [editing, setEditing] = useState(false)
  const [editValue, setEditValue] = useState('')

  const startEdit = useCallback(() => {
    setEditValue(message.content)
    setEditing(true)
  }, [message.content])

  const saveEdit = useCallback(() => {
    setEditing(false)
    if (editValue.trim() && editValue !== message.content) {
      removeMessage?.(message.id)
      sendMessage(editValue)
    }
  }, [editValue, message.id, removeMessage, sendMessage])

  const handleDelete = useCallback(() => {
    removeMessage?.(message.id)
  }, [message.id, removeMessage])

  const actions: MessageAction[] = [
    {
      id: 'edit',
      icon: <Pencil className="w-3 h-3" />,
      label: t('common.edit'),
      onClick: startEdit,
    },
    {
      id: 'delete',
      icon: <Trash2 className="w-3 h-3" />,
      label: t('common.delete'),
      onClick: handleDelete,
      danger: true,
    },
  ]

  return (
    <ChatItem
      placement="right"
      time={message.timestamp ? new Date(message.timestamp).getTime() : undefined}
      showTitle={false}
      actions={<MessageActionBar actions={actions} />}
    >
      {editing ? (
        <div className="flex flex-col gap-2">
          <textarea
            value={editValue}
            onChange={(e) => setEditValue(e.target.value)}
            className="w-full min-w-[300px] rounded-lg border bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary/30 resize-none"
            rows={Math.min(editValue.split('\n').length, 10)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault()
                saveEdit()
              }
              if (e.key === 'Escape') setEditing(false)
            }}
          />
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={saveEdit}
              className="inline-flex items-center gap-1 px-2.5 py-1 rounded-md bg-primary text-primary-foreground text-xs"
            >
              Save &amp; Resend
            </button>
            <button
              type="button"
              onClick={() => setEditing(false)}
              className="inline-flex items-center gap-1 px-2.5 py-1 rounded-md border text-xs"
            >
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <div className="max-w-[80%] text-sm whitespace-pre-wrap leading-relaxed">
          {message.content}
        </div>
      )}
    </ChatItem>
  )
}

export const UserMessage = memo(UserMessageImpl)
