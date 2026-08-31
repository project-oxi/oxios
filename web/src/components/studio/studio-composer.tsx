// StudioComposer — the Studio composer area (IA design §8.3).
//
// Owns the composer zone layout (todo progress + input shell) and wires
// ChatInput in its `'studio'` variant: NO persistent project/persona/
// model/Brain selectors here — StudioContextBar is their single home.
// The one-turn `Turn options` popover rides the toolbar slot and resets
// after each send (chat store contract).

import type { AttachedFile, ContextAttachment } from '@/components/chat/chat-input'
import { ChatInput } from '@/components/chat/chat-input'
import { TodoProgressBar } from '@/components/chat/todo-progress-bar'
import { TurnOptionsPopover } from '@/components/studio/turn-options-popover'
import type { ChatMessage } from '@/types'

interface StudioComposerProps {
  messages: ChatMessage[]
  input: string
  setInput: (value: string) => void
  onSend: (content: string, contextItems: ContextAttachment[], files: AttachedFile[]) => void
  onCancel: () => void
  isStreaming: boolean
  connected: boolean
  queuedCount: number
  roles: { name: string; model: string }[]
  setActiveRole: (role: string | null) => void
  activeModelId: string | null
  setActiveModelId: (id: string | null) => void
  onRevealTodo: () => void
}

export function StudioComposer({
  messages,
  input,
  setInput,
  onSend,
  onCancel,
  isStreaming,
  connected,
  queuedCount,
  roles,
  setActiveRole,
  activeModelId,
  setActiveModelId,
  onRevealTodo,
}: StudioComposerProps) {
  return (
    <div className="bg-background/95 backdrop-blur-sm shrink-0">
      {/* Live plan progress. Renders nothing unless a todo plan with
          outstanding work exists in this session's transcript. */}
      <TodoProgressBar messages={messages} onReveal={onRevealTodo} />
      <ChatInput
        value={input}
        onChange={setInput}
        onSend={onSend}
        onCancel={onCancel}
        isStreaming={isStreaming}
        connected={connected}
        queuedCount={queuedCount}
        roles={roles}
        setActiveRole={setActiveRole}
        activeModelId={activeModelId}
        setActiveModelId={setActiveModelId}
        toolbarSlot={<TurnOptionsPopover />}
      />
    </div>
  )
}
