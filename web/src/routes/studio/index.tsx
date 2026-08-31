// Studio — the conversation-first creation surface (IA design §8).
// `/studio` resolves to the last open Studio session when one exists,
// otherwise to a fresh projectless conversation. A new conversation
// receives NO implicit project binding.

import { createFileRoute } from '@tanstack/react-router'
import { StudioShell } from '@/components/studio/studio-shell'

export const Route = createFileRoute('/studio/')({
  component: StudioShell,
})
