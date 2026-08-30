// message-bubble.tsx — backward-compat thin wrapper.
//
// Phase 2 (2026-07-21): the monolithic renderer was split into role-specific
// components under messages/. This file preserves the public MessageBubble
// export so existing consumers (routes/chat.tsx, oxios-copilot-dialog.tsx) keep
// working without changes. New code should import MessageView directly.
//
// See docs/designs/2026-07-21-lobehub-chat-port-design.md §7 Phase 2.

import type { MessageViewProps } from './messages/MessageView'
import { MessageView } from './messages/MessageView'

export type MessageBubbleProps = MessageViewProps

/** Delegate to the new role-dispatched MessageView. */
export function MessageBubble(props: MessageBubbleProps) {
  return <MessageView {...props} />
}

export { MessageView }
