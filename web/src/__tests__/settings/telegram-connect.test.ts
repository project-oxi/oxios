import { describe, expect, it } from 'vitest'
import {
  type ChannelInfo,
  deriveTelegramState,
  extractErrorMessage,
} from '@/components/settings/telegram-connect-card'
import { ApiError } from '@/lib/api-client'

const base: ChannelInfo = {
  name: 'telegram',
  available: true,
  enabled: false,
  running: false,
  token_source: null,
}

describe('deriveTelegramState', () => {
  it('loading while the channels query is unresolved', () => {
    expect(deriveTelegramState(undefined)).toBe('loading')
  })

  it('no-token when neither store nor env has a token', () => {
    expect(deriveTelegramState({ ...base, token_source: null })).toBe('no-token')
  })

  it('ready when a token exists but the channel is stopped', () => {
    expect(deriveTelegramState({ ...base, token_source: 'auth_store' })).toBe('ready')
  })

  it('connected wins over token state (running channel)', () => {
    expect(deriveTelegramState({ ...base, running: true, token_source: null })).toBe('connected')
  })
})

describe('extractErrorMessage', () => {
  it('parses the {"error": msg} body from ApiError', () => {
    const err = new ApiError(
      400,
      'Bad Request',
      '{"error":"Telegram rejected the bot token: Unauthorized"}',
    )
    expect(extractErrorMessage(err)).toBe('Telegram rejected the bot token: Unauthorized')
  })

  it('falls back to the raw body when it is not JSON', () => {
    const err = new ApiError(500, 'Internal Server Error', 'plain text')
    expect(extractErrorMessage(err)).toBe('plain text')
  })

  it('uses Error.message for non-ApiError failures', () => {
    expect(extractErrorMessage(new Error('network down'))).toBe('network down')
  })
})
