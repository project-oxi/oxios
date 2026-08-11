/**
 * OAuth device-code modal (RFC-041 Phase 3).
 *
 * Flow: open → start → show `user_code` + verification URL → poll until
 * terminal → close + refresh. The `device_code` never reaches the browser
 * (H1) — the daemon polls the provider using an opaque `handle`.
 */

import { useQueryClient } from '@tanstack/react-query'
import { CheckCircle2, Copy, ExternalLink, Loader2, XCircle } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import type { IntegrationRow } from '@/hooks/use-integrations'
import { useOAuthPoll, useOAuthStart } from '@/hooks/use-integrations'

interface OAuthModalProps {
  row: IntegrationRow | null
  onOpenChange: (open: boolean) => void
}

type Phase = 'starting' | 'pending' | 'success' | 'expired' | 'denied' | 'error'

export function OAuthModal({ row, onOpenChange }: OAuthModalProps) {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const startMut = useOAuthStart()
  const pollMut = useOAuthPoll()
  const [phase, setPhase] = useState<Phase>('starting')
  const [userCode, setUserCode] = useState('')
  const [verifyUrl, setVerifyUrl] = useState('')
  const handleRef = useRef<string | null>(null)
  const stoppedRef = useRef(false)

  // Start the flow when a row is opened.
  useEffect(() => {
    if (!row) return
    stoppedRef.current = false
    setPhase('starting')
    setUserCode('')
    setVerifyUrl('')
    handleRef.current = null

    let cancelled = false
    void (async () => {
      try {
        const res = await startMut.mutateAsync(row.id)
        if (cancelled || stoppedRef.current) return
        handleRef.current = res.handle
        setUserCode(res.userCode)
        setVerifyUrl(res.verificationUrl)
        setPhase('pending')
      } catch {
        if (!cancelled && !stoppedRef.current) setPhase('error')
      }
    })()
    return () => {
      cancelled = true
    }
  }, [row]) // eslint-disable-line react-hooks/exhaustive-deps

  // Poll while pending.
  useEffect(() => {
    if (phase !== 'pending' || !row || !handleRef.current) return
    let cancelled = false
    const id = setInterval(async () => {
      if (cancelled || stoppedRef.current || !handleRef.current) return
      try {
        const res = await pollMut.mutateAsync({ id: row.id, handle: handleRef.current })
        if (cancelled || stoppedRef.current) return
        if (res.status === 'success') {
          stoppedRef.current = true
          setPhase('success')
          void qc.invalidateQueries({ queryKey: ['integrations'] })
          toast.success(t('settings.integrationsOAuthSuccess'))
          setTimeout(() => onOpenChange(false), 1500)
        } else if (res.status === 'expired') {
          stoppedRef.current = true
          setPhase('expired')
        } else if (res.status === 'denied') {
          stoppedRef.current = true
          setPhase('denied')
        }
        // 'pending' → keep polling
      } catch {
        // transient poll error — keep polling
      }
    }, 3000)
    return () => {
      cancelled = true
      clearInterval(id)
    }
  }, [phase, row]) // eslint-disable-line react-hooks/exhaustive-deps

  // Stop polling when the modal closes.
  useEffect(() => {
    if (row === null) stoppedRef.current = true
  }, [row])

  const open = row !== null
  const busy = phase === 'starting' || phase === 'pending'

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>
            {t('settings.integrationsOAuthTitle', { name: row?.label ?? '' })}
          </DialogTitle>
          <DialogDescription>{t('settings.integrationsOAuthInstructions')}</DialogDescription>
        </DialogHeader>

        {phase === 'starting' && (
          <div className="flex items-center gap-2 py-6 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            {t('settings.integrationsOAuthStarting')}
          </div>
        )}

        {(phase === 'pending' || phase === 'success') && (
          <div className="space-y-4 py-2">
            <p className="text-sm text-muted-foreground">
              {t('settings.integrationsOAuthInstructions')}
            </p>
            <div className="flex items-center gap-2">
              <Input
                readOnly
                value={userCode}
                className="font-mono text-center text-lg tracking-widest"
              />
              <Button
                variant="outline"
                size="icon"
                aria-label={t('settings.integrationsOAuthCopy')}
                onClick={() => {
                  navigator.clipboard.writeText(userCode)
                  toast.success(t('settings.integrationsOAuthCopied'))
                }}
              >
                <Copy className="h-4 w-4" />
              </Button>
            </div>
            <Button className="w-full" onClick={() => window.open(verifyUrl, '_blank')}>
              <ExternalLink className="mr-2 h-4 w-4" />
              {t('settings.integrationsOAuthOpen')}
            </Button>
            {phase === 'pending' && (
              <div className="flex items-center justify-center gap-2 text-xs text-muted-foreground">
                <Loader2 className="h-3 w-3 animate-spin" />
                {t('settings.integrationsOAuthWaiting')}
              </div>
            )}
            {phase === 'success' && (
              <div className="flex items-center justify-center gap-2 text-sm text-status-success-on-surface">
                <CheckCircle2 className="h-4 w-4" />
                {t('settings.integrationsOAuthSuccess')}
              </div>
            )}
          </div>
        )}

        {(phase === 'expired' || phase === 'denied' || phase === 'error') && (
          <div className="space-y-4 py-2">
            <div className="flex items-center gap-2 text-sm text-destructive">
              <XCircle className="h-4 w-4" />
              {phase === 'expired' && t('settings.integrationsOAuthExpired')}
              {phase === 'denied' && t('settings.integrationsOAuthDenied')}
              {phase === 'error' && t('settings.integrationsOAuthError')}
            </div>
            <Button
              variant="outline"
              className="w-full"
              onClick={() => {
                // Restart: re-trigger the start effect by toggling.
                stoppedRef.current = false
                setPhase('starting')
                setUserCode('')
                handleRef.current = null
                if (row) {
                  const retryRow = row
                  void startMut
                    .mutateAsync(retryRow.id)
                    .then((res) => {
                      if (stoppedRef.current || row?.id !== retryRow.id) return
                      handleRef.current = res.handle
                      setUserCode(res.userCode)
                      setVerifyUrl(res.verificationUrl)
                      setPhase('pending')
                    })
                    .catch(() => {
                      if (!stoppedRef.current && row?.id === retryRow.id) setPhase('error')
                    })
                }
              }}
            >
              {t('settings.integrationsOAuthRetry')}
            </Button>
          </div>
        )}

        {busy && (
          <p className="text-center text-xs text-muted-foreground">
            {t('settings.integrationsOAuthCloseNote')}
          </p>
        )}
      </DialogContent>
    </Dialog>
  )
}
