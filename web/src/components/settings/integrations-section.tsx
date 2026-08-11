/**
 * Integrations section (RFC-041 Phase 2).
 *
 * Lists registry integrations with live detect + credential status. `secret`
 * resolvers get an inline set/remove control; `oauth` shows a placeholder
 * "Connect" button (Phase 3 wires the device-code flow); `none` shows no
 * credential UI. This supersedes the host-tools-only view for integrations,
 * while host-tools remains the raw inventory surface.
 */

import { CheckCircle2, Eye, EyeOff, Loader2, Trash2, XCircle } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { OAuthModal } from '@/components/settings/oauth-modal'
import { SectionCard } from '@/components/settings/section-card'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { IntegrationRow } from '@/hooks/use-integrations'
import {
  useDeleteIntegrationCredential,
  useInstallIntegration,
  useInstallJobStatus,
  useIntegrations,
  useSetIntegrationCredential,
} from '@/hooks/use-integrations'

function DetectedBadge({ detected }: { detected: IntegrationRow['detected'] }) {
  const { t } = useTranslation()
  if (!detected) return null
  if (!detected.installed) {
    return (
      <Badge variant="outline" className="border-muted-foreground/30 text-muted-foreground">
        {t('settings.integrationsNotInstalled')}
      </Badge>
    )
  }
  return (
    <div className="flex items-center gap-2">
      <CheckCircle2 className="h-4 w-4 shrink-0 text-status-success" />
      {detected.version && (
        <span className="text-xs text-muted-foreground">{detected.version}</span>
      )}
      <Badge variant="outline" className="border-info/30 text-info">
        {detected.source}
      </Badge>
    </div>
  )
}

function CredentialBadge({ configured, source }: { configured: boolean; source: string }) {
  const { t } = useTranslation()
  return (
    <Badge
      variant="outline"
      className={
        configured
          ? 'border-status-success/30 text-status-success-on-surface'
          : 'border-muted-foreground/30 text-muted-foreground'
      }
    >
      {configured ? t('settings.integrationsConfigured') : t('settings.integrationsNotConfigured')}
      {configured && source !== 'none' ? ` · ${source}` : ''}
    </Badge>
  )
}

function SecretControl({ row }: { row: IntegrationRow }) {
  const { t } = useTranslation()
  const [value, setValue] = useState('')
  const [show, setShow] = useState(false)
  const setMut = useSetIntegrationCredential()
  const delMut = useDeleteIntegrationCredential()

  const onSave = async () => {
    if (!value) return
    try {
      await setMut.mutateAsync({ id: row.id, value })
      setValue('')
      toast.success(t('settings.integrationsCredentialSaved'))
    } catch {
      toast.error(t('settings.integrationsCredentialError'))
    }
  }

  const onRemove = async () => {
    try {
      await delMut.mutateAsync(row.id)
      toast.success(t('settings.integrationsCredentialRemoved'))
    } catch {
      toast.error(t('settings.integrationsCredentialError'))
    }
  }

  return (
    <div className="flex items-center gap-2">
      <div className="relative">
        <Input
          type={show ? 'text' : 'password'}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          placeholder={t('settings.integrationsEnterValue')}
          className="h-8 w-48 pr-8 font-mono text-xs"
        />
        <button
          type="button"
          aria-label={
            show
              ? t('settings.integrationsHideCredential')
              : t('settings.integrationsShowCredential')
          }
          onClick={() => setShow((s) => !s)}
          className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
        >
          {show ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
        </button>
      </div>
      <Button size="sm" variant="outline" onClick={onSave} disabled={!value || setMut.isPending}>
        {setMut.isPending && <Loader2 className="mr-1 h-3 w-3 animate-spin" />}
        {t('common.save')}
      </Button>
      {row.credential.configured && (
        <Button
          size="sm"
          variant="ghost"
          aria-label={t('settings.integrationsRemoveCredential')}
          onClick={onRemove}
          disabled={delMut.isPending}
        >
          <Trash2 className="h-3.5 w-3.5" />
        </Button>
      )}
    </div>
  )
}

function InstallButton({ row }: { row: IntegrationRow }) {
  const { t } = useTranslation()
  const installMut = useInstallIntegration()
  const [jobId, setJobId] = useState<string | null>(null)
  const status = useInstallJobStatus(jobId)
  // Only show Install when the integration has a CLI and it is NOT installed.
  // Credential-only integrations (no cli) and already-installed ones hide it.
  const notInstalled = row.cli && row.detected && !row.detected.installed

  // Surface terminal outcomes as a toast exactly once. The mutation itself
  // only resolves with `{ jobId }` — the real success/failure signal rides
  // SSE via `useInstallJobStatus`.
  useEffect(() => {
    if (status.state === 'completed') {
      toast.success(t('settings.integrationsInstallDone'))
      setJobId(null)
    } else if (status.state === 'failed') {
      toast.error(`${t('settings.integrationsInstallFailed')} (${status.error})`)
      setJobId(null)
    }
  }, [status, t])

  if (!notInstalled && !jobId) return null

  const onInstall = async () => {
    if (!window.confirm(t('settings.integrationsInstallConfirm', { name: row.label }))) return
    try {
      const res = await installMut.mutateAsync(row.id)
      setJobId(res.jobId)
    } catch (e) {
      toast.error(`${t('settings.integrationsInstallFailed')} (${String(e)})`)
    }
  }

  const running = jobId !== null && (status.state === 'running' || status.state === 'idle')
  const label = running
    ? status.state === 'running' && status.line
      ? status.line
      : t('settings.integrationsInstalling')
    : t('settings.integrationsInstall')

  return (
    <Button size="sm" variant="outline" onClick={onInstall} disabled={running}>
      {running && <Loader2 className="mr-1 h-3 w-3 animate-spin" />}
      <span className="max-w-[16rem] truncate">{label}</span>
    </Button>
  )
}

export function IntegrationsSectionCard() {
  const { t } = useTranslation()
  const { data: rows, isLoading, isError } = useIntegrations()
  const [oauthRow, setOauthRow] = useState<IntegrationRow | null>(null)

  return (
    <SectionCard
      title={t('settings.sectionHostTools')}
      description={t('settings.integrationsDescription')}
      sectionId="host-tools"
    >
      {isLoading && (
        <div className="flex items-center gap-2 py-8 text-sm text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
          {t('settings.hostToolsScanning')}
        </div>
      )}
      {isError && (
        <div className="flex items-center gap-2 py-8 text-sm text-destructive">
          <XCircle className="h-4 w-4" />
          {t('settings.hostToolsError')}
        </div>
      )}
      {rows && rows.length === 0 && (
        <div className="py-8 text-center text-sm text-muted-foreground">
          {t('settings.integrationsNone')}
        </div>
      )}
      {rows && rows.length > 0 && (
        <div className="space-y-4">
          {(
            [
              ['package_manager', t('settings.integrationsKindPackageManagers')],
              ['cli_tool', t('settings.integrationsKindCliTools')],
              ['credential_only', t('settings.integrationsKindCredentials')],
            ] as const
          ).map(([kind, heading]) => {
            const groupRows = rows.filter((r) => r.kind === kind)
            if (groupRows.length === 0) return null
            return (
              <div key={kind} className="space-y-2">
                <p className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
                  {heading}
                </p>
                <ul className="space-y-2">
                  {groupRows.map((row) => (
                    <li
                      key={row.id}
                      className="flex flex-wrap items-center justify-between gap-3 rounded-md px-2 py-2 hover:bg-muted/40"
                    >
                      <div className="flex min-w-0 flex-col gap-1">
                        <div className="flex items-center gap-2">
                          <span className="font-mono text-sm font-medium">{row.label}</span>
                          {row.cli && (
                            <span className="text-xs text-muted-foreground">{row.cli}</span>
                          )}
                        </div>
                        <div className="flex items-center gap-3">
                          <DetectedBadge detected={row.detected} />
                          {row.resolverKind !== 'none' && (
                            <CredentialBadge
                              configured={row.credential.configured}
                              source={row.credential.source}
                            />
                          )}
                        </div>
                      </div>
                      <InstallButton row={row} />
                      {row.resolverKind === 'secret' && <SecretControl row={row} />}
                      {row.resolverKind === 'oauth' && (
                        <Button size="sm" variant="outline" onClick={() => setOauthRow(row)}>
                          {row.credential.configured
                            ? t('settings.integrationsConnected')
                            : t('settings.integrationsConnect')}
                        </Button>
                      )}
                    </li>
                  ))}
                </ul>
              </div>
            )
          })}
        </div>
      )}
      <OAuthModal row={oauthRow} onOpenChange={(o) => !o && setOauthRow(null)} />
    </SectionCard>
  )
}
