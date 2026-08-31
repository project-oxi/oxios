import { describe, expect, it } from 'vitest'
import {
  formatElapsed,
  isStoppable,
  lastActivityAt,
  progressOf,
  selectActiveRuns,
  statusRank,
  systemHealth,
} from '@/components/dashboard/command-center/runs'
import type { AgentListItem } from '@/types/agent'

function run(overrides: Partial<AgentListItem> = {}): AgentListItem {
  return {
    id: 'agent_1',
    name: 'Release QA',
    status: 'running',
    created_at: '2026-08-31T05:00:00Z',
    started_at: '2026-08-31T05:00:10Z',
    completed_at: null,
    project_id: null,
    session_id: null,
    error: null,
    steps_completed: 3,
    steps_total: 5,
    tokens_used: 1200,
    cost_usd: 0.04,
    model_id: 'provider/model',
    duration_secs: null,
    ...overrides,
  }
}

const T0 = Date.parse('2026-08-31T06:00:00Z')

describe('statusRank', () => {
  it('orders running before starting before failed before idle before terminal states', () => {
    expect(statusRank('running')).toBeLessThan(statusRank('starting'))
    expect(statusRank('starting')).toBeLessThan(statusRank('failed'))
    expect(statusRank('failed')).toBeLessThan(statusRank('idle'))
    expect(statusRank('idle')).toBeLessThan(statusRank('stopped'))
    expect(statusRank('stopped')).toBe(statusRank('completed'))
  })

  it('is case-insensitive and treats unknown statuses as least priority', () => {
    expect(statusRank('RUNNING')).toBe(0)
    expect(statusRank('mystery')).toBeGreaterThan(statusRank('idle'))
  })
})

describe('lastActivityAt', () => {
  it('prefers completed_at, then started_at, then created_at', () => {
    expect(
      lastActivityAt({
        completed_at: '2026-08-31T05:30:00Z',
        started_at: '2026-08-31T05:00:00Z',
        created_at: '2026-08-31T04:00:00Z',
      }),
    ).toBe(Date.parse('2026-08-31T05:30:00Z'))
    expect(
      lastActivityAt({
        completed_at: null,
        started_at: '2026-08-31T05:00:00Z',
        created_at: '2026-08-31T04:00:00Z',
      }),
    ).toBe(Date.parse('2026-08-31T05:00:00Z'))
    expect(
      lastActivityAt({ completed_at: null, started_at: null, created_at: '2026-08-31T04:00:00Z' }),
    ).toBe(Date.parse('2026-08-31T04:00:00Z'))
  })
})

describe('selectActiveRuns', () => {
  it('keeps running and starting agents plus recently failed agents', () => {
    const runs = selectActiveRuns(
      [
        run({ id: 'a', status: 'running' }),
        run({ id: 'b', status: 'starting' }),
        run({ id: 'c', status: 'failed', completed_at: '2026-08-31T05:59:00Z' }),
        run({ id: 'd', status: 'idle' }),
        run({ id: 'e', status: 'completed' }),
        run({ id: 'f', status: 'failed', completed_at: '2026-08-30T05:00:00Z' }), // >24h old
      ],
      T0,
    )
    expect(runs.map((r) => r.id)).toEqual(['a', 'b', 'c'])
  })

  it('sorts by status rank, then most recent activity first within a rank', () => {
    const runs = selectActiveRuns(
      [
        run({ id: 'old-running', started_at: '2026-08-31T05:00:00Z' }),
        run({ id: 'new-running', started_at: '2026-08-31T05:50:00Z' }),
        run({ id: 'starting', status: 'starting', started_at: '2026-08-31T05:55:00Z' }),
      ],
      T0,
    )
    expect(runs.map((r) => r.id)).toEqual(['new-running', 'old-running', 'starting'])
  })
})

describe('isStoppable', () => {
  it('is true only for running and starting agents', () => {
    expect(isStoppable(run({ status: 'running' }))).toBe(true)
    expect(isStoppable(run({ status: 'starting' }))).toBe(true)
    expect(isStoppable(run({ status: 'failed' }))).toBe(false)
    expect(isStoppable(run({ status: 'completed' }))).toBe(false)
  })
})

describe('progressOf', () => {
  it('returns completed/total only when steps_total is a positive number', () => {
    expect(progressOf(run({ steps_completed: 3, steps_total: 5 }))).toEqual({
      completed: 3,
      total: 5,
    })
    expect(progressOf(run({ steps_completed: 0, steps_total: null }))).toBeNull()
    expect(progressOf(run({ steps_completed: 2, steps_total: 0 }))).toBeNull()
  })

  it('clamps completed to total to keep a truthful fraction', () => {
    expect(progressOf(run({ steps_completed: 7, steps_total: 5 }))).toEqual({
      completed: 5,
      total: 5,
    })
  })
})

describe('formatElapsed', () => {
  it('formats seconds, minutes, and hours without fabricating precision', () => {
    expect(formatElapsed(T0 - 12_000, T0)).toBe('12s')
    expect(formatElapsed(T0 - 252_000, T0)).toBe('4m 12s')
    expect(formatElapsed(T0 - 4_980_000, T0)).toBe('1h 23m')
  })

  it('caps at completed_at when the run has finished', () => {
    expect(formatElapsed(T0 - 120_000, T0, T0 - 60_000)).toBe('1m 0s')
  })
})

describe('systemHealth', () => {
  const healthy = {
    service: 'oxios',
    status: 'ok',
    version: '1.0.0',
    channels: [],
    uptime: '1h',
    components: {
      state_store: { healthy: true },
      event_bus: { healthy: true },
      brain: { healthy: true },
    },
  }

  it('reports healthy when every component is healthy', () => {
    expect(systemHealth(healthy, false).state).toBe('healthy')
  })

  it('reports attention with a count when a component is unhealthy', () => {
    const degraded = {
      ...healthy,
      components: { ...healthy.components, event_bus: { healthy: false } },
    }
    const health = systemHealth(degraded, false)
    expect(health.state).toBe('attention')
    expect(health.count).toBe(1)
  })

  it('reports degraded when the status query failed or data is absent', () => {
    expect(systemHealth(undefined, true).state).toBe('degraded')
    expect(systemHealth(undefined, false).state).toBe('degraded')
  })
})
