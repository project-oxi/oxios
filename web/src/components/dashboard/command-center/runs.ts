import type { SystemStatus } from '@/types'
import type { AgentListItem } from '@/types/agent'

/**
 * Command-center run helpers — pure functions over the `/api/agents` wire
 * shape (`AgentListItem`). No React, no fetching: everything here is
 * unit-testable and deliberately dumb so the components stay presentational.
 *
 * Wire statuses come from the kernel `AgentStatus` Display impl and are
 * lowercase: starting | running | idle | stopped | failed | completed.
 * The spec's "waiting for input" state is not modeled by the backend, so it
 * simply never matches — no fabrication.
 */

/** Failed runs stay visible on the dashboard for 24h after completion. */
const RECENT_FAILED_WINDOW_MS = 24 * 60 * 60 * 1000

const MIN_S = 60
const HOUR_S = 3_600

/**
 * Priority order for the Active execution list (spec §3): running first,
 * then starting, then failed, then everything else. Unknown statuses sort
 * last so a new backend state degrades instead of hijacking the top.
 */
export function statusRank(status: string): number {
  const s = status.toLowerCase()
  switch (s) {
    case 'running':
      return 0
    case 'starting':
      return 1
    case 'failed':
      return 2
    case 'idle':
      return 3
    default:
      return 4 // stopped, completed, unknown
  }
}

function isLiveStatus(s: string): boolean {
  return s === 'running' || s === 'starting'
}

/** Best-known "when did something last happen" timestamp, as epoch ms. */
export function lastActivityAt(run: {
  completed_at?: string | null
  started_at?: string | null
  created_at?: string | null
}): number {
  const iso = run.completed_at ?? run.started_at ?? run.created_at
  const ms = iso ? Date.parse(iso) : Number.NaN
  return Number.isNaN(ms) ? 0 : ms
}

/**
 * The dashboard's Active execution set: running + starting agents, plus
 * agents that failed recently (inside the 24h window). Sorted by the spec's
 * priority order — status rank first, most recent activity within a rank.
 */
export function selectActiveRuns(items: AgentListItem[], now: number): AgentListItem[] {
  const active = items.filter((r) => {
    const s = r.status.toLowerCase()
    if (isLiveStatus(s)) return true
    if (s === 'failed') return now - lastActivityAt(r) < RECENT_FAILED_WINDOW_MS
    return false
  })
  return active.sort((a, b) => {
    const byRank = statusRank(a.status) - statusRank(b.status)
    if (byRank !== 0) return byRank
    return lastActivityAt(b) - lastActivityAt(a)
  })
}

/** Only running/starting agents can be stopped (spec §3: stop action for a running agent). */
export function isStoppable(run: { status: string }): boolean {
  return isLiveStatus(run.status.toLowerCase())
}

/**
 * Real step progress only (spec: never fabricate a percentage). Returns
 * null when the backend does not report a positive total, and clamps a
 * runaway completed counter to the total so the fraction stays ≤ 1.
 */
export function progressOf(run: {
  steps_completed: number
  steps_total: number | null
}): { completed: number; total: number } | null {
  if (typeof run.steps_total !== 'number' || run.steps_total <= 0) return null
  const total = run.steps_total
  const completed = Math.min(Math.max(run.steps_completed, 0), total)
  return { completed, total }
}

/**
 * Human elapsed time: "42s", "4m 12s", "1h 23m". When the run has
 * completed, elapsed caps at the completion time instead of growing forever.
 */
export function formatElapsed(fromMs: number, nowMs: number, completedMs?: number | null): string {
  const end = typeof completedMs === 'number' && !Number.isNaN(completedMs) ? completedMs : nowMs
  const totalSec = Math.max(0, Math.floor((end - fromMs) / 1000))
  if (totalSec < 60) return `${totalSec}s`
  const totalMin = Math.floor(totalSec / 60)
  if (totalMin < 60) return `${totalMin}m ${totalSec % 60}s`
  return `${Math.floor(totalSec / HOUR_S)}h ${Math.floor((totalSec % HOUR_S) / MIN_S)}m`
}

export type HealthState = 'healthy' | 'attention' | 'degraded'

export interface SystemHealth {
  state: HealthState
  labelKey: string
  /** Number of unhealthy components (attention state only). */
  count?: number
}

/**
 * Header health summary (spec §1): a single System healthy / Degraded /
 * Attention needed verdict derived from `/api/status` component health.
 * A failed or absent status query is "degraded" — the system's health is
 * unknown, which is not the same as healthy.
 */
export function systemHealth(
  status: SystemStatus | undefined,
  statusFailed: boolean,
): SystemHealth {
  if (statusFailed || !status) {
    return { state: 'degraded', labelKey: 'commandCenter.health.degraded' }
  }
  const components = status.components
  if (components) {
    const checks: Array<boolean | undefined> = [
      components.state_store?.healthy,
      components.event_bus?.healthy,
      components.git?.healthy,
      components.brain?.healthy,
    ]
    const defined = checks.filter((c): c is boolean => typeof c === 'boolean')
    const unhealthy = defined.filter((c) => !c).length
    if (unhealthy > 0) {
      return { state: 'attention', labelKey: 'commandCenter.health.attention', count: unhealthy }
    }
  }
  return { state: 'healthy', labelKey: 'commandCenter.health.healthy' }
}

export interface RunStatusMeta {
  labelKey: string
  /** Status-colored text on a neutral surface — always the `-on-surface` token. */
  textClass: string
  /** Solid status token for the icon glyph. */
  iconClass: string
  icon: 'activity' | 'loader' | 'alert' | 'pause' | 'check' | 'square'
  spin?: boolean
  pulse?: boolean
}

/**
 * Status = icon + text label + semantic color (spec: never color alone).
 * Motion is gated with `motion-safe:` so reduced-motion users get static
 * glyphs; the text label carries the meaning without animation either way.
 */
export function statusMeta(status: string): RunStatusMeta {
  const s = status.toLowerCase()
  switch (s) {
    case 'running':
      return {
        labelKey: 'commandCenter.status.running',
        textClass: 'text-status-success-on-surface',
        iconClass: 'text-status-success',
        icon: 'activity',
        pulse: true,
      }
    case 'starting':
      return {
        labelKey: 'commandCenter.status.starting',
        textClass: 'text-status-info-on-surface',
        iconClass: 'text-status-info',
        icon: 'loader',
        spin: true,
      }
    case 'failed':
      return {
        labelKey: 'commandCenter.status.failed',
        textClass: 'text-status-error-on-surface',
        iconClass: 'text-status-error',
        icon: 'alert',
      }
    case 'idle':
      return {
        labelKey: 'commandCenter.status.idle',
        textClass: 'text-status-info-on-surface',
        iconClass: 'text-status-info',
        icon: 'pause',
      }
    case 'completed':
      return {
        labelKey: 'commandCenter.status.completed',
        textClass: 'text-status-success-on-surface',
        iconClass: 'text-status-success',
        icon: 'check',
      }
    default: // stopped and unknown terminal states
      return {
        labelKey: 'commandCenter.status.stopped',
        textClass: 'text-muted-foreground',
        iconClass: 'text-muted-foreground',
        icon: 'square',
      }
  }
}
