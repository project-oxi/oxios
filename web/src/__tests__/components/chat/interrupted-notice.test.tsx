// Component test: InterruptedNotice renders a muted status footer for either
// of two reasons:
//   - cancelled (default): user pressed Stop — chat.interrupted
//   - interrupted:         socket dropped mid-stream — chat.connectionLost
// It is deliberately NOT the destructive ErrorCard styling; cancelling or
// aborting a turn is a normal action / a recoverable failure, not a fault.

import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { InterruptedNotice } from '@/components/chat/interrupted-notice'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: 'en' },
  }),
}))

describe('InterruptedNotice', () => {
  it('renders a status notice labelled with chat.interrupted and an icon (default reason)', () => {
    const { container } = render(<InterruptedNotice />)
    const status = screen.getByRole('status')
    expect(status).toHaveTextContent('chat.interrupted')
    expect(container.querySelector('svg')).not.toBeNull()
    expect(status.className).toContain('text-muted-foreground')
  })

  it('renders chat.interrupted when reason is explicitly "cancelled"', () => {
    render(<InterruptedNotice reason="cancelled" />)
    expect(screen.getByText('chat.interrupted')).toBeTruthy()
  })

  it('renders chat.connectionLost when reason is "interrupted"', () => {
    render(<InterruptedNotice reason="interrupted" />)
    expect(screen.getByText('chat.connectionLost')).toBeTruthy()
  })
})
