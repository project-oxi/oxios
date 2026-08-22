import { describe, expect, it } from 'vitest'
import { groupByCategory, mergeMountIds } from '@/components/chat/persona-picker'

describe('mergeMountIds', () => {
  it('appends missing defaults while preserving existing primary-first order', () => {
    expect(mergeMountIds('a,b', ['b', 'c'])).toBe('a,b,c')
  })

  it('returns defaults alone when no current selection exists', () => {
    expect(mergeMountIds(null, ['x', 'y'])).toBe('x,y')
    expect(mergeMountIds('', ['x'])).toBe('x')
  })

  it('drops empty and whitespace-only segments', () => {
    expect(mergeMountIds(' a , ,b ', ['c'])).toBe('a,b,c')
  })

  it('is a no-op when all defaults are already present', () => {
    expect(mergeMountIds('a,b', ['a', 'b'])).toBe('a,b')
  })
})

describe('groupByCategory', () => {
  const personas: Array<{ id: string; category?: string }> = [
    { id: 'dev', category: 'coding' },
    { id: 'novelist', category: 'writing' },
    { id: 'architect', category: 'general' },
    { id: 'normal', category: 'normal' },
    { id: 'custom', category: 'weird-new' },
    { id: 'legacy' }, // missing category → general bucket
  ]

  it('orders known categories first, unknown last', () => {
    const groups = groupByCategory(personas)
    expect(groups.map((g) => g.category)).toEqual([
      'normal',
      'coding',
      'writing',
      'general',
      'weird-new',
    ])
  })

  it('buckets a missing category into general', () => {
    const groups = groupByCategory(personas)
    const general = groups.find((g) => g.category === 'general')
    expect(general?.items.map((p) => p.id)).toEqual(['architect', 'legacy'])
  })

  it('maps label keys for known categories and empty for unknown', () => {
    const groups = groupByCategory(personas)
    expect(groups[0]?.labelKey).toBe('chat.persona.categories.normal')
    expect(groups.find((g) => g.category === 'weird-new')?.labelKey).toBe('')
  })
})
