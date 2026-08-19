import { describe, expect, it } from 'vitest'
import { deriveSidebarMode } from './sidebar'

describe('deriveSidebarMode', () => {
  it.each([
    ['/', 'console'],
    ['/agents', 'console'],
    ['/brain', 'brain'],
    ['/brain/search', 'brain'],
    ['/brain/knowledge', 'knowledge'],
    ['/brain/knowledge/graph', 'knowledge'],
    ['/chat', 'chat'],
  ] as const)('%s → %s', (path, mode) => {
    expect(deriveSidebarMode(path)).toBe(mode)
  })
})
