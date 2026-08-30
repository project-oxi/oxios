// §8.2 action provenance: tool cards distinguish denial from failure and the
// permission reason remains visible in the EXPANDED view. The backend puts
// the gate's denial text (format_denied) into the tool error message; the
// collapsed header shows status + tool name, the expanded body must show
// the reason — never collapse it to a generic "Tool error".

import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { ToolCallList } from '@/components/chat/messages/components/ToolCallList'
import type { ChatToolPayload } from '@/types/chat'

const DENIED: ChatToolPayload = {
  id: 'c1',
  identifier: 'kernel',
  apiName: 'bash',
  arguments: { command: 'rm -rf /tmp/x' },
  status: 'error',
  error: {
    type: 'tool_error',
    message:
      '🔒 Access denied: path /tmp/x is outside the project roots — widen the project roots or request approval',
    severity: 'error',
  },
}

const FAILED: ChatToolPayload = {
  id: 'c2',
  identifier: 'kernel',
  apiName: 'read_file',
  arguments: { path: '/tmp/x' },
  status: 'error',
  error: { type: 'tool_error', message: 'No such file or directory', severity: 'error' },
}

describe('tool card denial reason visibility (§8.2)', () => {
  it('hides the reason while the card is collapsed', () => {
    render(<ToolCallList calls={[DENIED]} />)
    expect(screen.queryByText(/Access denied/)).toBeNull()
  })

  it('shows the permission reason in the expanded view', () => {
    render(<ToolCallList calls={[DENIED]} />)
    fireEvent.click(screen.getByRole('button'))
    expect(
      screen.getByText(/Access denied: path \/tmp\/x is outside the project roots/),
    ).toBeTruthy()
  })

  it('keeps sibling failure reasons independent from denial text', () => {
    render(
      <ToolCallList
        calls={[DENIED, FAILED]}
        // Expand both cards.
        defaultExpanded
      />,
    )
    expect(screen.getByText(/Access denied: path \/tmp\/x/)).toBeTruthy()
    expect(screen.getByText('No such file or directory')).toBeTruthy()
  })
})
