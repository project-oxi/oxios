// Component test: PortalPanel exposes dialog semantics, focuses inside the
// panel when it opens, and Escape clears the portal stack (closes the panel).

import { fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { PortalPanel } from '@/components/portal/portal-panel'
import { usePortalStore } from '@/stores/portal'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}))

describe('PortalPanel', () => {
  beforeEach(() => {
    usePortalStore.setState({ stack: [] })
  })

  afterEach(() => {
    usePortalStore.setState({ stack: [] })
  })

  it('is an accessible dialog that traps focus and closes on Escape', () => {
    usePortalStore.getState().pushView({ type: 'search' })
    render(<PortalPanel />)
    const panel = screen.getByRole('dialog')
    expect(panel.getAttribute('aria-modal')).toBe('true')
    expect(panel.contains(document.activeElement)).toBe(true)
    fireEvent.keyDown(panel, { key: 'Escape' })
    expect(usePortalStore.getState().stack).toHaveLength(0)
  })
})
