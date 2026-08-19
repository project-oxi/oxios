import { Route as KnowledgeGraph } from './graph'
import { Route as KnowledgeIndex } from './index'

function catchRedirect(fn: () => unknown): { to: string; search: unknown } {
  try {
    fn()
    throw new Error('expected redirect')
  } catch (e: unknown) {
    const err = e as { options?: { to?: string; search?: unknown }; to?: string; search?: unknown }
    const to = err.options?.to ?? err.to
    const search = err.options?.search ?? err.search
    if (!to) throw e
    return { to, search }
  }
}

describe('knowledge redirects', () => {
  it('redirects /knowledge to /brain/knowledge preserving search', () => {
    const r = catchRedirect(() =>
      KnowledgeIndex.options.beforeLoad!({ search: { path: 'notes/a.md' } } as never),
    )
    expect(r.to).toBe('/brain/knowledge')
    expect(r.search).toEqual({ path: 'notes/a.md' })
  })
  it('redirects /knowledge/graph', () => {
    const r = catchRedirect(() => KnowledgeGraph.options.beforeLoad!({} as never))
    expect(r.to).toBe('/brain/knowledge/graph')
  })
})
