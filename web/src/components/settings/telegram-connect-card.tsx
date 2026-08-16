/**
 * Telegram connection card — instant connect/disconnect from Settings.
 *
 * One-call connect flow: optional token input → POST /api/channels/telegram/connect.
 * The backend stores the token, validates it via getMe, starts the channel,
 * and persists `channels.enabled` — no daemon restart.
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link2, Link2Off, Loader2, Send } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { ApiError, api } from '@/lib/api-client'

/** One row from GET /api/channels. */
export interface ChannelInfo {
  name: string
  available: boolean
  enabled: boolean
  running: boolean
  token_source: string | null
}

/** Connect response: { status, info? }. */
interface ConnectResponse {
  status: string
  info?: { bot_username?: string } | null
}

/**
 * Derive the card's state machine from channel info:
 * - `loading`   — channels query not resolved yet
 * - `no-token`  — nothing to connect with; token input is the primary action
 * - `ready`     — token present, channel stopped
 * - `connected` — channel registered with the gateway
 */
export function deriveTelegramState(
  channel: ChannelInfo | undefined,
): 'loading' | 'no-token' | 'ready' | 'connected' {
  if (!channel) return 'loading'
  if (channel.running) return 'connected'
  if (channel.token_source) return 'ready'
  return 'no-token'
}

/** Pull the backend's `{"error": msg}` body out of an ApiError for toasts. */
export function extractErrorMessage(err: unknown): string {
  if (err instanceof ApiError && err.body) {
    try {
      const parsed = JSON.parse(err.body) as { error?: string }
      if (parsed.error) return parsed.error
    } catch {
      // body wasn't JSON — fall through to the raw text
      return err.body
    }
  }
  return err instanceof Error ? err.message : String(err)
}

/** Fetch helper shared with the test suite. */
export async function fetchChannels(): Promise<ChannelInfo[]> {
  const res = await api.get<{ channels: ChannelInfo[] }>('/api/channels')
  return res.channels
}

export function TelegramConnectCard() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [token, setToken] = useState('')
  const [showTokenInput, setShowTokenInput] = useState(false)

  const { data: channels, isLoading } = useQuery({
    queryKey: ['channels'],
    queryFn: fetchChannels,
  })
  const telegram = channels?.find((c) => c.name === 'telegram')
  const state = deriveTelegramState(telegram)

  const connectMutation = useMutation({
    mutationFn: (body: { token?: string }) =>
      api.post<ConnectResponse>('/api/channels/telegram/connect', body),
    onSuccess: (res) => {
      queryClient.invalidateQueries({ queryKey: ['channels'] })
      queryClient.invalidateQueries({ queryKey: ['secrets'] })
      setToken('')
      setShowTokenInput(false)
      const bot = res.info?.bot_username
      toast.success(
        bot ? t('settings.telegramConnectedAs', { bot }) : t('settings.telegramConnected'),
      )
    },
    onError: (err) =>
      toast.error(`${t('settings.telegramConnectFailed')}: ${extractErrorMessage(err)}`),
  })

  const disconnectMutation = useMutation({
    mutationFn: () => api.post('/api/channels/telegram/disconnect', {}),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['channels'] })
      toast.success(t('settings.telegramDisconnected'))
    },
    onError: (err) =>
      toast.error(`${t('settings.telegramDisconnectFailed')}: ${extractErrorMessage(err)}`),
  })

  if (telegram && !telegram.available) {
    return <p className="text-sm text-muted-foreground">{t('settings.telegramUnavailable')}</p>
  }

  const statusBadge = () => {
    if (isLoading || state === 'loading')
      return (
        <Badge variant="outline" className="text-2xs">
          <Loader2 className="h-3 w-3 animate-spin" />
        </Badge>
      )
    if (state === 'connected')
      return (
        <Badge variant="outline" className="border-success/40 text-success text-2xs">
          {t('settings.telegramConnected')}
        </Badge>
      )
    if (state === 'ready')
      return (
        <Badge variant="outline" className="text-2xs">
          {t('settings.telegramDisconnected')}
        </Badge>
      )
    return (
      <Badge variant="outline" className="border-warning/40 text-warning text-2xs">
        {t('settings.telegramNoToken')}
      </Badge>
    )
  }

  const tokenSourceBadge =
    telegram?.token_source === 'env' ? (
      <Badge variant="outline" className="border-info/30 text-info text-2xs">
        {t('settings.telegramTokenSourceEnv')}
      </Badge>
    ) : telegram?.token_source === 'auth_store' ? (
      <Badge variant="outline" className="border-success/30 text-success text-2xs">
        {t('settings.telegramTokenSourceStore')}
      </Badge>
    ) : null

  return (
    <div className="space-y-3 rounded-md border border-border bg-muted/30 p-4">
      <div className="flex items-center gap-2">
        <Send className="h-3.5 w-3.5" />
        {statusBadge()}
        {tokenSourceBadge}
        <span className="ml-auto text-xs text-muted-foreground">
          {t('settings.telegramReconnectHint')}
        </span>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        {(state === 'no-token' || showTokenInput) && (
          <Input
            type="password"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder={t('settings.telegramTokenPlaceholder')}
            className="min-w-56 flex-1"
            autoComplete="off"
          />
        )}
        {state !== 'connected' && (
          <>
            <Button
              size="sm"
              onClick={() => connectMutation.mutate(token.trim() ? { token: token.trim() } : {})}
              disabled={connectMutation.isPending || (state === 'no-token' && !token.trim())}
            >
              {connectMutation.isPending ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Link2 className="h-3.5 w-3.5" />
              )}
              {t('settings.telegramConnect')}
            </Button>
            {state === 'ready' && !showTokenInput && (
              <Button variant="ghost" size="sm" onClick={() => setShowTokenInput(true)}>
                {t('settings.telegramChangeToken')}
              </Button>
            )}
          </>
        )}
        {state === 'connected' && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => disconnectMutation.mutate()}
            disabled={disconnectMutation.isPending}
          >
            {disconnectMutation.isPending ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Link2Off className="h-3.5 w-3.5" />
            )}
            {t('settings.telegramDisconnect')}
          </Button>
        )}
      </div>
    </div>
  )
}
