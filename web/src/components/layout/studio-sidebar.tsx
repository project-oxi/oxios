import { ChatSessionNav } from './chat-session-nav'

// ── Studio sidebar (design §8.2) ───────────────────────────────
//
// Wave-1 composition: the existing ChatSessionNav project/session tree is
// the temporary Conversations data consumer (its "Unfiled" section is now
// labelled "No project"). The Automations, Personas, and Artifacts groups
// are appended by their owning workers (WT-1 route, WT-3 surface) — the
// sidebar renders no global quick links from any legacy mode.

export function StudioSidebar() {
  return <ChatSessionNav />
}
