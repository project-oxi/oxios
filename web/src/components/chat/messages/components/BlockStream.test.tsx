import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render } from '@testing-library/react'
import type { ReactNode } from 'react'
import { describe, expect, it } from 'vitest'
import type { ChatBlock } from '@/types'
import { BlockStream } from './BlockStream'

function renderBlockStream(blocks: ChatBlock[], messageId: string, generating = false) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  )
  return render(
    <BlockStream blocks={blocks} messageId={messageId} generating={generating} />,
    { wrapper },
  )
}

describe('BlockStream', () => {
  it('renders tool before text (execution order, not inverted)', () => {
    const blocks: ChatBlock[] = [
      {
        type: 'tool',
        id: 't1',
        identifier: 'kernel',
        apiName: 'grep',
        arguments: {},
        status: 'success',
      },
      { type: 'text', id: 'x1', text: 'Final answer' },
    ]
    const { container } = renderBlockStream(blocks, 'm1')
    const text = container.textContent ?? ''
    expect(text).toContain('grep')
    expect(text).toContain('Final answer')
    // The tool must appear BEFORE the answer — the core fix over the old
    // "tool list below the answer" categorized layout.
    expect(text.indexOf('grep')).toBeLessThan(text.indexOf('Final answer'))
  })

  it('renders a lone text block (simple Q&A turn)', () => {
    const blocks: ChatBlock[] = [{ type: 'text', id: 'x1', text: 'Just an answer' }]
    const { container } = renderBlockStream(blocks, 'm1')
    expect(container.textContent).toContain('Just an answer')
  })

  it('renders a reasoning block through the Thinking + markdown body', () => {
    // Exercises the reasoning → Thinking → MarkdownMessage path (the body was
    // monospace <pre> before the hierarchy redesign). A streaming span is
    // auto-expanded so its content is in the DOM.
    const blocks: ChatBlock[] = [
      {
        type: 'reasoning',
        id: 'r1',
        text: 'Considering the options first',
        status: 'streaming',
        startedAt: Date.now(),
      },
      { type: 'text', id: 'x1', text: 'Answer' },
    ]
    const { container } = renderBlockStream(blocks, 'm1')
    const text = container.textContent ?? ''
    expect(text).toContain('Considering the options first')
    // Answer still renders after the reasoning span (flow order).
    expect(text.indexOf('Considering')).toBeLessThan(text.indexOf('Answer'))
  })

  describe('working tail', () => {
    it('shows the pulse when generating and all blocks are settled', () => {
      const blocks: ChatBlock[] = [
        {
          type: 'tool',
          id: 't1',
          identifier: 'kernel',
          apiName: 'exec',
          arguments: {},
          status: 'success',
        },
      ]
      const { getByTestId } = renderBlockStream(blocks, 'm1', true)
      expect(getByTestId('working-tail')).toBeDefined()
    })

    it('hides the pulse while the trailing block streams its own affordance', () => {
      const blocks: ChatBlock[] = [
        {
          type: 'reasoning',
          id: 'r1',
          text: 'thinking hard',
          status: 'streaming',
          startedAt: Date.now(),
        },
      ]
      const { queryByTestId } = renderBlockStream(blocks, 'm1', true)
      expect(queryByTestId('working-tail')).toBeNull()
    })

    it('hides the pulse when the turn is not generating', () => {
      const blocks: ChatBlock[] = [
        { type: 'text', id: 'x1', text: 'done answer' },
      ]
      const { queryByTestId } = renderBlockStream(blocks, 'm1', false)
      expect(queryByTestId('working-tail')).toBeNull()
    })
  })
})
