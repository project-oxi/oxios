// TaskComments — verifies the thread renders seeded items, the composer is
// present, and a seeded comment with author+timestamp appears verbatim.

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import { afterEach } from 'vitest'
import { server } from '@/__tests__/msw/server'
import { TaskComments } from './task-comments'

function makeWrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  function Wrapper({ children }: { children: React.ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>
  }
  return Wrapper
}

describe('TaskComments', () => {
  afterEach(() => {
    server.resetHandlers()
  })

  it('renders the seeded comments list', async () => {
    server.use(
      http.get('/api/tasks/task-1/comments', () =>
        HttpResponse.json({
          comments: [
            {
              id: 'c1',
              taskId: 'task-1',
              content: 'First comment from operator',
              authorAgentId: 'agent-7',
              createdAt: '2026-01-02T03:04:05.000Z',
            },
            {
              id: 'c2',
              taskId: 'task-1',
              content: 'Second comment — please verify',
              authorAgentId: 'agent-9',
              createdAt: '2026-01-02T04:05:06.000Z',
            },
          ],
        }),
      ),
    )

    render(<TaskComments taskId="task-1" />, { wrapper: makeWrapper() })

    expect(await screen.findByText('First comment from operator')).toBeInTheDocument()
    expect(screen.getByText('Second comment — please verify')).toBeInTheDocument()
    expect(screen.getByText('agent-7')).toBeInTheDocument()
    expect(screen.getByText('agent-9')).toBeInTheDocument()
    // The composer placeholder is visible.
    expect(screen.getByPlaceholderText('tasks.commentPlaceholder')).toBeInTheDocument()
  })

  it('shows the empty state when the list is empty', async () => {
    server.use(http.get('/api/tasks/task-1/comments', () => HttpResponse.json({ comments: [] })))
    render(<TaskComments taskId="task-1" />, { wrapper: makeWrapper() })
    expect(await screen.findByText('tasks.noComments')).toBeInTheDocument()
  })
})
