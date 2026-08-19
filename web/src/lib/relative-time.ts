// Single relative-time formatter. Two hardcoded English copies existed
// (empty-chat-state.tsx, AgentFanoutCard.tsx) — both bypassed i18n.
import type { TFunction } from 'i18next'

// Uses the existing `common.*` time keys (en: "just now", "{{count}}m ago",
// …; ko: "방금", "{{count}}분 전", …) — the brief proposed a new `time.*`
// namespace, but the identical translations already live under `common.*` in
// both locales, so adding a parallel namespace would duplicate them.

export function formatRelativeTime(iso: string, t: TFunction): string {
  const deltaMs = Date.now() - new Date(iso).getTime()
  const s = Math.max(0, Math.floor(deltaMs / 1000))
  if (s < 60) return t('common.justNow')
  const m = Math.floor(s / 60)
  if (m < 60) return t('common.minutesAgo', { count: m })
  const h = Math.floor(m / 60)
  if (h < 24) return t('common.hoursAgo', { count: h })
  return t('common.daysAgo', { count: Math.floor(h / 24) })
}
