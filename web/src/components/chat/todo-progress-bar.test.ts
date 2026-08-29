import { describe, expect, it } from 'vitest'
import type { ChatMessage } from '@/types'
import { hasOpenWork, latestTodoSnapshot } from './todo-progress-bar'
import { isClosed, parseTodo } from './tool-renders/Todo'

function todoBlock(id: string, result: unknown) {
  return {
    type: 'tool' as const,
    id,
    identifier: 'kernel',
    apiName: 'todo',
    arguments: {},
    result,
    status: 'success' as const,
  }
}

function message(blocks: unknown[]): ChatMessage {
  return {
    id: 'm1',
    role: 'assistant',
    content: '',
    timestamp: new Date().toISOString(),
    blocks,
  } as unknown as ChatMessage
}

const snapshot = (tasks: Array<{ content: string; status: string }>) => ({
  phases: [{ name: 'Work', tasks }],
  total: tasks.length,
  completed: tasks.filter((t) => t.status === 'completed' || t.status === 'abandoned').length,
  inProgress: tasks.find((t) => t.status === 'in_progress')?.content ?? null,
})

describe('parseTodo', () => {
  it('rejects payloads that are not todo snapshots', () => {
    // The same `tool_end.results` channel carries web-search hits and issue
    // payloads, so the parser must not claim them.
    expect(parseTodo(null)).toBeNull()
    expect(parseTodo('done')).toBeNull()
    expect(parseTodo({ kind: 'issue', issue: { number: 1 } })).toBeNull()
    expect(parseTodo([{ title: 'a result' }])).toBeNull()
  })

  it('keeps an unknown status rather than dropping the task', () => {
    const parsed = parseTodo({
      phases: [{ name: 'P', tasks: [{ content: 'x', status: 'deferred' }] }],
      total: 1,
      completed: 0,
    })
    expect(parsed?.phases[0]?.tasks[0]?.status).toBe('deferred')
  })

  it('tolerates missing counters', () => {
    const parsed = parseTodo({ phases: [] })
    expect(parsed).toEqual({ phases: [], total: 0, completed: 0, inProgress: null })
  })
})

describe('isClosed', () => {
  it('treats abandoned as closed and blocked as open', () => {
    expect(isClosed('completed')).toBe(true)
    expect(isClosed('abandoned')).toBe(true)
    expect(isClosed('blocked')).toBe(false)
    expect(isClosed('pending')).toBe(false)
    expect(isClosed('in_progress')).toBe(false)
  })
})

describe('latestTodoSnapshot', () => {
  it('returns the newest snapshot, not the first', () => {
    const messages = [
      message([todoBlock('t1', snapshot([{ content: 'a', status: 'pending' }]))]),
      message([
        todoBlock(
          't2',
          snapshot([
            { content: 'a', status: 'completed' },
            { content: 'b', status: 'in_progress' },
          ]),
        ),
      ]),
    ]
    expect(latestTodoSnapshot(messages)?.total).toBe(2)
    expect(latestTodoSnapshot(messages)?.inProgress).toBe('b')
  })

  it('ignores non-todo tool blocks sharing the results channel', () => {
    const searchBlock = { ...todoBlock('s1', [{ title: 'hit' }]), apiName: 'web_search' }
    const messages = [message([searchBlock])]
    expect(latestTodoSnapshot(messages)).toBeNull()
  })

  it('skips a todo block whose result never arrived', () => {
    const messages = [
      message([todoBlock('t1', snapshot([{ content: 'a', status: 'pending' }]))]),
      message([todoBlock('t2', undefined)]),
    ]
    expect(latestTodoSnapshot(messages)?.total).toBe(1)
  })

  it('returns null for a transcript with no blocks', () => {
    expect(latestTodoSnapshot([])).toBeNull()
    expect(latestTodoSnapshot([message([])])).toBeNull()
  })
})

describe('hasOpenWork', () => {
  it('is false once every task is closed, so the strip disappears', () => {
    expect(
      hasOpenWork(
        parseTodo(
          snapshot([
            { content: 'a', status: 'completed' },
            { content: 'b', status: 'abandoned' },
          ]),
        ),
      ),
    ).toBe(false)
  })

  it('is true while a task is blocked — blocked work is still outstanding', () => {
    expect(hasOpenWork(parseTodo(snapshot([{ content: 'a', status: 'blocked' }])))).toBe(true)
  })

  it('is false for an empty or absent plan', () => {
    expect(hasOpenWork(null)).toBe(false)
    expect(hasOpenWork(parseTodo(snapshot([])))).toBe(false)
  })
})
