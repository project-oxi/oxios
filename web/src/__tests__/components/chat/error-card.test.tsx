// Component test: ErrorCard renders kind-specific copy via the i18n lookup
// over the real backend error kinds (snake_case, from KNOWN_ERROR_KINDS).
// Task 2 rewrote the store to emit these kinds; Task 6 wires the i18n copy.
//
// Setup: t(key) returns the key so tests can assert *which* kind-key was
// resolved even when the locale table is loaded asynchronously.

import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ErrorCard } from '@/components/chat/messages/components/ErrorCard'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue ?? key,
    i18n: { language: 'en' },
  }),
}))

describe('ErrorCard', () => {
  it('renders provider_error copy for the backend kind', () => {
    render(<ErrorCard error={{ type: 'stream_error', category: 'provider_error' }} />)
    // i18n key is the canonical surface — verifies the lookup hit the
    // providerError entry, not the legacy client-only 'unknown' fallback.
    expect(screen.getByText('chat.error.providerError.title')).toBeTruthy()
  })

  it('renders each backend kind with its own i18n key', () => {
    const cases: Array<{ kind: string; expectedKey: string }> = [
      { kind: 'execution_failed', expectedKey: 'chat.error.executionFailed.title' },
      { kind: 'api_key_missing', expectedKey: 'chat.error.apiKeyMissing.title' },
      { kind: 'provider_error', expectedKey: 'chat.error.providerError.title' },
      { kind: 'timeout', expectedKey: 'chat.error.timeout.title' },
      { kind: 'permission_denied', expectedKey: 'chat.error.permissionDenied.title' },
      { kind: 'validation_error', expectedKey: 'chat.error.validationError.title' },
      { kind: 'internal', expectedKey: 'chat.error.internal.title' },
      { kind: 'cancelled', expectedKey: 'chat.error.cancelled.title' },
    ]
    for (const { kind, expectedKey } of cases) {
      const { unmount } = render(<ErrorCard error={{ type: 'stream_error', category: kind }} />)
      expect(screen.getByText(expectedKey)).toBeTruthy()
      unmount()
    }
  })

  it('falls back to the unknown key when the category is not a backend kind', () => {
    render(<ErrorCard error={{ type: 'stream_error', category: 'my_custom_thing' }} />)
    expect(screen.getByText('chat.error.unknown.title')).toBeTruthy()
  })

  it('prefers category over type when both are present', () => {
    // The store emits errorKind into the chatError.type slot; but the
    // AssistantMessage wiring must surface it via category too so the
    // lookup hits even when the message envelope differs.
    render(<ErrorCard error={{ type: 'provider_error', category: 'timeout' }} />)
    expect(screen.getByText('chat.error.timeout.title')).toBeTruthy()
  })

  it('uses i18n retry label, not the hardcoded literal "Retry"', () => {
    render(<ErrorCard error={{ type: 'stream_error', category: 'internal' }} onRetry={() => {}} />)
    expect(screen.getByText('chat.retry')).toBeTruthy()
    expect(screen.queryByText('Retry')).toBeNull()
  })
})
