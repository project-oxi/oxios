import { describe, expect, it } from 'vitest'
import { formatRelativeTime } from '@/lib/relative-time'

describe('formatRelativeTime', () => {
  it('formats relative times through i18n', () => {
    const t = ((k: string, o?: Record<string, unknown>) => `${k}:${o?.count ?? ''}`) as never
    const now = Date.now()
    expect(formatRelativeTime(new Date(now - 5_000).toISOString(), t)).toBe('common.justNow:')
    expect(formatRelativeTime(new Date(now - 120_000).toISOString(), t)).toBe('common.minutesAgo:2')
    expect(formatRelativeTime(new Date(now - 7_200_000).toISOString(), t)).toBe('common.hoursAgo:2')
    expect(formatRelativeTime(new Date(now - 172_800_000).toISOString(), t)).toBe(
      'common.daysAgo:2',
    )
  })

  it('clamps negative deltas (clock skew) to just now', () => {
    const t = ((k: string) => k) as never
    expect(formatRelativeTime(new Date(Date.now() + 10_000).toISOString(), t)).toBe(
      'common.justNow',
    )
  })
})
