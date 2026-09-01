import {
  AlertTriangle,
  Bot,
  Brain,
  CheckCircle,
  ChevronRight,
  Database,
  ListChecks,
  type LucideIcon,
  MessageSquare,
  PlayCircle,
  Rocket,
  Settings,
  Sparkles,
  Square,
  Timer,
  Wrench,
  XCircle,
} from 'lucide-react'
import type { OxiosEvent } from '@/types'

/**
 * Map a SSE event type to a one-line summary, icon, and accent color.
 *
 * Used by the Live Activity Feed on the dashboard to display incoming
 * events in a human-friendly format. Only event types the RFC marks as
 * "interesting" are listed here; others get a generic fallback.
 */
export interface FormattedEvent {
  /** Display label (lowercase) for the event type. */
  label: string
  /** Short human-readable summary. */
  summary: string
  /** Lucide icon component. */
  icon: LucideIcon
  /** Tailwind text-color class for the icon. */
  color: string
  /** Optional link to the resource this event refers to. */
  href?: string
}

/** Route hint extracted from a raw event payload. */
function routeFromEvent(event: OxiosEvent): string | undefined {
  const type = event.type
  if (type === 'approval_requested' || type === 'approval_resolved') {
    return '/operate'
  }
  if (
    type === 'agent_created' ||
    type === 'agent_started' ||
    type === 'agent_stopped' ||
    type === 'agent_failed' ||
    type === 'agent_output'
  ) {
    return '/operate/runs'
  }
  if (type === 'memory_stored' || type === 'memory_recalled') {
    return '/memory'
  }
  if (type === 'tool_started' || type === 'tool_finished') {
    return '/studio'
  }
  return undefined
}

function safeStr(v: unknown, max = 60): string {
  if (typeof v !== 'string') return ''
  return v.length > max ? `${v.slice(0, max)}…` : v
}

export function formatEvent(event: OxiosEvent): FormattedEvent {
  const type = event.type
  const base: FormattedEvent = {
    label: type,
    summary: '',
    icon: Settings,
    color: 'text-muted-foreground',
    href: routeFromEvent(event),
  }

  switch (type) {
    case 'agent_created': {
      const name = safeStr(event.name, 32)
      return {
        ...base,
        label: 'fork',
        summary: name ? `forked "${name}"` : 'agent forked',
        icon: Rocket,
        color: 'text-status-success',
      }
    }
    case 'agent_started':
      return {
        ...base,
        label: 'start',
        summary: 'agent started',
        icon: PlayCircle,
        color: 'text-status-info',
      }
    case 'agent_stopped':
      return {
        ...base,
        label: 'kill',
        summary: 'agent stopped',
        icon: Square,
        color: 'text-status-warning',
      }
    case 'agent_failed': {
      const err = safeStr(event.error, 50)
      return {
        ...base,
        label: 'failed',
        summary: err ? `agent failed: ${err}` : 'agent failed',
        icon: XCircle,
        color: 'text-status-error',
      }
    }
    case 'agent_output':
      return {
        ...base,
        label: 'output',
        summary: 'agent output',
        icon: MessageSquare,
        color: 'text-status-info',
      }
    case 'tool_started': {
      const tool = safeStr(event.tool_name, 30)
      return {
        ...base,
        label: 'tool',
        summary: tool ? `▶ ${tool}` : 'tool started',
        icon: Wrench,
        color: 'text-status-info',
      }
    }
    case 'tool_finished': {
      const tool = safeStr(event.tool_name, 30)
      const isError = event.is_error === true
      return {
        ...base,
        label: 'tool',
        summary: tool ? (isError ? `✗ ${tool}` : `✓ ${tool}`) : 'tool finished',
        icon: isError ? XCircle : CheckCircle,
        color: isError ? 'text-status-error' : 'text-status-success',
      }
    }
    case 'memory_recalled': {
      const count = typeof event.count === 'number' ? event.count : 0
      const q = safeStr(event.query, 30)
      return {
        ...base,
        label: 'memory',
        summary: q
          ? `recalled ${count} for "${q}"`
          : `recalled ${count} memor${count === 1 ? 'y' : 'ies'}`,
        icon: Brain,
        color: 'text-hue-purple',
      }
    }
    case 'memory_stored':
      return {
        ...base,
        label: 'memory',
        summary: 'memory stored',
        icon: Database,
        color: 'text-hue-purple',
      }
    case 'approval_requested': {
      const action = safeStr(event.action, 24)
      const resource = safeStr(event.resource, 40)
      return {
        ...base,
        label: 'approval',
        summary: resource ? `${action} → ${resource}` : action || 'approval requested',
        icon: AlertTriangle,
        color: 'text-status-warning',
      }
    }
    case 'approval_resolved': {
      const approved = event.approved === true
      return {
        ...base,
        label: 'approval',
        summary: approved ? 'approved' : 'rejected',
        icon: approved ? CheckCircle : XCircle,
        color: approved ? 'text-status-success' : 'text-status-error',
      }
    }
    case 'phase_started':
    case 'phase_completed': {
      const phase = safeStr(event.phase, 30)
      return {
        ...base,
        label: 'phase',
        summary: phase ? `${type === 'phase_started' ? '→' : '✓'} ${phase}` : 'phase',
        icon: ListChecks,
        color: 'text-hue-blue',
      }
    }
    case 'evaluation_complete': {
      const passed = event.passed === true
      return {
        ...base,
        label: 'eval',
        summary: passed ? 'evaluation passed' : 'evaluation failed',
        icon: passed ? CheckCircle : XCircle,
        color: passed ? 'text-status-success' : 'text-status-error',
      }
    }
    case 'project_created':
    case 'project_activated': {
      const name = safeStr(event.name, 30)
      return {
        ...base,
        label: 'project',
        summary: name
          ? `${type === 'project_activated' ? 'activated' : 'created'} "${name}"`
          : 'project',
        icon: Bot,
        color: 'text-status-info',
      }
    }
    case 'agent_group_created':
    case 'agent_group_member_completed':
      return {
        ...base,
        label: 'group',
        summary: 'agent group update',
        icon: ChevronRight,
        color: 'text-hue-teal',
      }
    case 'evolution_started':
    case 'evolution_max_reached':
      return {
        ...base,
        label: 'evolve',
        summary: type === 'evolution_max_reached' ? 'evolution max reached' : 'evolution started',
        icon: Sparkles,
        color: 'text-hue-purple',
      }
    case 'token_usage_update': {
      const inT = Number(event.input_tokens ?? 0)
      const outT = Number(event.output_tokens ?? 0)
      const total = inT + outT
      return {
        ...base,
        label: 'tokens',
        summary: total ? `tokens ${total.toLocaleString()}` : 'token update',
        icon: Timer,
        color: 'text-status-info',
      }
    }
    case 'reasoning_fragment':
      return {
        ...base,
        label: 'think',
        summary: 'reasoning fragment',
        icon: Sparkles,
        color: 'text-hue-purple',
      }
    case 'memory_recalled_used': {
      const count = typeof event.count === 'number' ? event.count : 0
      const q = safeStr(event.query, 30)
      return {
        ...base,
        label: 'memory',
        summary: q
          ? `recalled ${count} for "${q}"`
          : `recalled ${count} memor${count === 1 ? 'y' : 'ies'}`,
        icon: Brain,
        color: 'text-hue-purple',
      }
    }
    default: {
      // Generic fallback for any other interesting event
      return {
        ...base,
        label: 'event',
        summary: type,
        icon: Timer,
        color: 'text-muted-foreground',
      }
    }
  }
}

/**
 * The set of event types the dashboard considers worth surfacing in the
 * Live Activity Feed. Filtering is applied client-side so the feed stays
 * small even when the SSE stream is busy.
 */
export const INTERESTING_EVENT_TYPES = new Set<string>([
  'agent_created',
  'agent_started',
  'agent_stopped',
  'agent_failed',
  'agent_output',
  'tool_started',
  'tool_finished',
  'memory_recalled',
  'memory_recalled_used',
  'memory_stored',
  'approval_requested',
  'approval_resolved',
  'phase_started',
  'phase_completed',
  'evaluation_complete',
  'project_created',
  'project_activated',
  'agent_group_created',
  'agent_group_member_completed',
  'evolution_started',
  'evolution_max_reached',
  'token_usage_update',
  'reasoning_fragment',
])

export function isInterestingEvent(event: OxiosEvent): boolean {
  return INTERESTING_EVENT_TYPES.has(event.type)
}
