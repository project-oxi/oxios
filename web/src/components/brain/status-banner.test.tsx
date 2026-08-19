import { render, screen } from '@testing-library/react'
import type { BrainStatus } from '@/types/brain'
import { StatusBanner } from './status-banner'

// Mock i18next — verbatim convention from existing component tests
// (see web/src/__tests__/components/shared/error-state.test.tsx).
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: 'en' },
  }),
}))

const onlineStatus = {
  available: true,
  space: 'personal',
  episodes: 1,
} as BrainStatus

const installingStatus = {
  available: false,
  space: null,
  episodes: null,
  supervisor: {
    state: 'installing',
    installed_version: null,
    daemon_version: null,
    managed_by: 'none',
    last_error: null,
  },
} as BrainStatus

const startingStatus = {
  available: false,
  space: null,
  episodes: null,
  supervisor: {
    state: 'starting',
    installed_version: null,
    daemon_version: null,
    managed_by: 'none',
    last_error: null,
  },
} as BrainStatus

const failedStatus = {
  available: false,
  space: null,
  episodes: null,
  supervisor: {
    state: 'failed',
    installed_version: null,
    daemon_version: null,
    managed_by: 'none',
    last_error: 'no-release-asset',
  },
} as BrainStatus

const noSupervisorStatus = {
  available: false,
  space: null,
  episodes: null,
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

  it('shows progress while installing', () => {
    render(<StatusBanner status={installingStatus} />)
    expect(screen.getByText('brain.installing')).toBeInTheDocument()
  })

  it('shows progress while starting', () => {
    render(<StatusBanner status={startingStatus} />)
    expect(screen.getByText('brain.starting')).toBeInTheDocument()
  })

  it('shows failure title and last_error when state is failed', () => {
    render(<StatusBanner status={failedStatus} />)
    expect(screen.getByText('brain.failedTitle')).toBeInTheDocument()
    expect(screen.getByText(/no-release-asset/)).toBeInTheDocument()
  })

  it('falls back to manual guidance when no supervisor info is present', () => {
    render(<StatusBanner status={noSupervisorStatus} />)
    expect(screen.getByText('brain.degradedTitle')).toBeInTheDocument()
    expect(screen.getByText('brain.manualDescription')).toBeInTheDocument()
  })
})
