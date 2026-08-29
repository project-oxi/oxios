import { describe, expect, it } from 'vitest'
import { groupByCategory } from '@/components/chat/persona-picker'

describe('groupByCategory', () => {
  it('groups personas by category in display order', () => {
    const personas = [
      { id: '1', category: 'coding' },
      { id: '2', category: 'writing' },
      { id: '3', category: 'coding' },
    ]
    const groups = groupByCategory(personas)
    expect(groups.map((g) => g.category)).toEqual(['coding', 'writing'])
    expect(groups[0]?.items.map((p) => p.id)).toEqual(['1', '3'])
    expect(groups[1]?.items.map((p) => p.id)).toEqual(['2'])
  })

  it('falls back to other for unknown categories, ordered last', () => {
    const personas = [
      { id: '1', category: 'poetry' },
      { id: '2', category: 'coding' },
    ]
    const groups = groupByCategory(personas)
    expect(groups.map((g) => g.category)).toEqual(['coding', 'poetry'])
    expect(groups[1]?.labelKey).toBe('')
  })
})
