import { render, screen } from '@testing-library/react'
import type { BrainStatus } from '@/types/brain'
import { StatusBanner } from './status-banner'

// Mock i18next — verbatim convention from existing component tests
// (see web/src/__tests__/components/shared/error-state.test.tsx).
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, params?: Record<string, unknown>) => {
      if (params && 'roots' in params) return `${key}:${String(params.roots)}`
      return key
    },
    i18n: { language: 'en' },
  }),
}))

const onlineStatus = {
  available: true,
  pending_extraction: 0,
  binary: { installed: true, path: '/usr/bin/oxibrain', version: '0.8.0' },
} as BrainStatus

const binaryMissingStatus = {
  available: false,
  pending_extraction: null,
  binary: { installed: false, path: null, version: null },
} as BrainStatus

const degradedStatus = {
  available: false,
  pending_extraction: 3,
  binary: { installed: true, path: '/usr/bin/oxibrain', version: '0.8.0' },
} as BrainStatus

describe('StatusBanner', () => {
  it('renders nothing when online', () => {
    const { container } = render(<StatusBanner status={onlineStatus} />)
    expect(container).toBeEmptyDOMElement()
  })

  it('renders nothing when status is undefined', () => {
    const { container } = render(<StatusBanner status={undefined} />)
    expect(container).toBeEmptyDOMElement()
  })

  it('shows the binary-missing install hint when the brain binary is absent', () => {
    render(<StatusBanner status={binaryMissingStatus} />)
    expect(screen.getByText('brain.notInstalled')).toBeInTheDocument()
    expect(screen.getByText('brain.notInstalledHint')).toBeInTheDocument()
  })

  it('falls back to the degraded banner when the binary is installed but no session is active', () => {
    render(<StatusBanner status={degradedStatus} />)
    expect(screen.getByText('brain.degradedTitle')).toBeInTheDocument()
    expect(screen.getByText('brain.manualDescription')).toBeInTheDocument()
  })
})
