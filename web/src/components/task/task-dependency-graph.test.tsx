// TaskDependencyGraph — verifies the contract: render null with no deps.

import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import type { Task } from '@/types/task'
import { TaskDependencyGraph } from './task-dependency-graph'

function makeTask(): Task {
  return {
    id: 'task-1',
    identifier: 'task-1',
    name: 'Task 1',
    description: undefined,
    instruction: 'do the thing',
    status: 'backlog',
    priority: 0,
    automationMode: null,
    schedulePattern: null,
    scheduleTimezone: null,
    heartbeatIntervalSecs: null,
    maxExecutions: null,
    executionCount: 0,
    verifyEnabled: false,
    verifyMaxIterations: 3,
    createdAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:00:00Z',
    consecutiveFailures: 0,
    dependencies: [],
  }
}

describe('TaskDependencyGraph', () => {
  it('renders null when there are no dependencies', () => {
    const { container } = render(<TaskDependencyGraph task={makeTask()} dependencies={[]} />)
    // No SVG, no wrapper — empty render.
    expect(container.querySelector('svg')).toBeNull()
    expect(container.firstChild).toBeNull()
  })

  it('renders an svg when there is at least one dependency', () => {
    const dep: Task = { ...makeTask(), id: 'task-2', name: 'Task 2', status: 'completed' }
    const { container } = render(<TaskDependencyGraph task={makeTask()} dependencies={[dep]} />)
    expect(container.querySelector('svg')).not.toBeNull()
  })
})
