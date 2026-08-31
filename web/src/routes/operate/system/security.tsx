import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import {
  FileWarning,
  Filter,
  Hand,
  KeyRound,
  ListChecks,
  Loader2,
  Search,
  Shield,
  Trash2,
  Zap,
} from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { ApprovalsQueue } from '@/components/dashboard/approvals-queue'
import { EmptyState } from '@/components/shared/empty-state'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { PageHeader } from '@/components/shared/page-header'
import { RefreshButton } from '@/components/shared/refresh-button'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { useApprovalConfig, useRemoveGrant, useSetApprovalMode } from '@/hooks/use-approval-config'
import { api } from '@/lib/api-client'
import { cn } from '@/lib/utils'
import type { ApprovalMode } from '@/types/approval'

export const Route = createFileRoute('/operate/system/security')({ component: SecurityPage })

// Inline labels — same i18n-conflict-avoidance pattern as Tasks 11/12.
// Do NOT touch i18n files; user has uncommitted i18n work.
const MODE_LABELS: Record<ApprovalMode, { en: string; ko: string; icon: typeof Hand }> = {
  manual: { en: 'Manual approval', ko: '수동 승인', icon: Hand },
  'allow-list': { en: 'Allow list', ko: '허용 목록', icon: ListChecks },
  'auto-run': { en: 'Auto-run', ko: '자동 실행', icon: Zap },
}

const MODE_ORDER: ApprovalMode[] = ['manual', 'allow-list', 'auto-run']

/**
 * ApprovalConfigPanel — power-user surface for the approval config.
 *
 * Sits below ApprovalsQueue on the security page. Lets the user:
 *   - switch the approval mode (Manual / Allow list / Auto-run)
 *   - view & remove entries from the allow_list
 *
 * Tool overrides are intentionally NOT exposed here — see the
 * "Advanced" hint. Editing `config.toml` keeps that surface stable.
 */
function ApprovalConfigPanel({ isKo }: { isKo: boolean }) {
  const { data, isLoading, isError, refetch } = useApprovalConfig()
  const setMode = useSetApprovalMode()
  const removeGrant = useRemoveGrant()

  const mode: ApprovalMode = data?.mode ?? 'manual'
  const allowList = Array.isArray(data?.allow_list) ? data.allow_list : []
  const toolOverrides = data?.tool_overrides ?? {}
  const isPending = setMode.isPending || removeGrant.isPending

  const handleModeChange = (next: ApprovalMode) => {
    if (next === mode) return
    setMode.mutate(next, {
      onSuccess: () => {
        toast.success(isKo ? '승인 모드가 변경되었습니다' : 'Approval mode updated')
      },
      onError: (err) => {
        toast.error(isKo ? '승인 모드 변경 실패' : 'Failed to update approval mode', {
          description: String(err instanceof Error ? err.message : err),
        })
      },
    })
  }

  const handleRemove = (key: string) => {
    removeGrant.mutate(key, {
      onSuccess: () => {
        toast.success(isKo ? '허용 목록에서 제거되었습니다' : 'Removed from allow list')
      },
      onError: (err) => {
        toast.error(isKo ? '제거 실패' : 'Failed to remove entry', {
          description: String(err instanceof Error ? err.message : err),
        })
      },
    })
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Shield className="h-4 w-4" />
          {isKo ? '승인 설정' : 'Approval settings'}
        </CardTitle>
        <CardDescription>
          {isKo
            ? '도구 승인 모드와 허용 목록을 관리합니다.'
            : 'Manage tool approval mode and the allow list.'}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        {isLoading ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>{isKo ? '설정 불러오는 중…' : 'Loading settings…'}</span>
          </div>
        ) : isError ? (
          <ErrorState onRetry={() => refetch()} />
        ) : (
          <>
            {/* Mode selector */}
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <p className="text-sm font-medium">{isKo ? '승인 모드' : 'Approval mode'}</p>
                {isPending && (
                  <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                )}
              </div>
              <div className="flex flex-wrap gap-2">
                {MODE_ORDER.map((m) => {
                  const entry = MODE_LABELS[m]
                  const Icon = entry.icon
                  const isCurrent = m === mode
                  return (
                    <Button
                      key={m}
                      type="button"
                      variant={isCurrent ? 'default' : 'outline'}
                      size="sm"
                      disabled={isPending}
                      onClick={() => handleModeChange(m)}
                      aria-pressed={isCurrent}
                      className={cn('gap-1.5', !isCurrent && 'text-muted-foreground')}
                    >
                      <Icon className="h-3.5 w-3.5" />
                      <span>{isKo ? entry.ko : entry.en}</span>
                    </Button>
                  )
                })}
              </div>
              <p className="text-xs text-muted-foreground">
                {mode === 'manual'
                  ? isKo
                    ? '모든 도구 사용 전 사용자 승인이 필요합니다.'
                    : 'Require approval before any tool runs.'
                  : mode === 'allow-list'
                    ? isKo
                      ? '허용 목록에 있는 도구만 자동 실행됩니다.'
                      : 'Auto-run only tools in the allow list.'
                    : isKo
                      ? '모든 도구를 자동으로 실행합니다. 주의해서 사용하세요.'
                      : 'Auto-run every tool. Use with caution.'}
              </p>
            </div>

            {/* Allow list */}
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <p className="text-sm font-medium">{isKo ? '허용 목록' : 'Allow list'}</p>
                <Badge variant="outline" className="text-2xs">
                  {allowList.length}
                </Badge>
              </div>
              {allowList.length === 0 ? (
                <p className="text-xs text-muted-foreground">
                  {isKo
                    ? '허용 목록이 비어 있습니다. 승인 카드에서 “다시 묻지 않기”를 눌러 도구를 추가하세요.'
                    : "No tools in allow-list yet. Approve a tool with 'don't ask again' to add one."}
                </p>
              ) : (
                <ul className="space-y-1.5">
                  {allowList.map((key) => (
                    <li
                      key={key}
                      className="flex items-center justify-between rounded-md border bg-muted/30 px-3 py-1.5"
                    >
                      <span className="font-mono text-xs break-all">{key}</span>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="h-6 w-6 text-muted-foreground hover:text-destructive"
                        disabled={isPending}
                        onClick={() => handleRemove(key)}
                        aria-label={isKo ? `${key} 제거` : `Remove ${key}`}
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    </li>
                  ))}
                </ul>
              )}
            </div>

            {/* Tool overrides — advanced, edit config.toml */}
            <div className="space-y-1 border-t pt-4">
              <p className="text-sm font-medium">{isKo ? '도구 정책 재정의' : 'Tool overrides'}</p>
              <p className="text-xs text-muted-foreground">
                {isKo
                  ? `고급 설정입니다. config.toml 에서 직접 편집하세요. (현재 ${Object.keys(toolOverrides).length}개)`
                  : `Advanced — edit config.toml directly. (${Object.keys(toolOverrides).length} entries)`}
              </p>
            </div>
          </>
        )}
      </CardContent>
    </Card>
  )
}

function SecurityPage() {
  const { t, i18n } = useTranslation()
  const {
    data: audits,
    isLoading: auditLoading,
    isError: auditError,
    refetch,
    isFetching,
  } = useQuery<{
    items: {
      timestamp: string
      agent_name: string
      action: string
      resource: string
      allowed: boolean
      reason: string | null
    }[]
  }>({
    queryKey: ['audit'],
    queryFn: async () => {
      const res = await api.get<{
        items: {
          timestamp: string
          agent_name: string
          action: string
          resource: string
          allowed: boolean
          reason: string | null
        }[]
      }>('/api/audit')
      return res
    },
    refetchInterval: 15000,
  })

  const {
    data: permissions,
    isError: permissionsError,
    refetch: refetchPermissions,
  } = useQuery({
    queryKey: ['permissions'],
    queryFn: () =>
      api.get<{
        roles: string[]
        policies: { name: string; effect: string; resources: string[] }[]
      }>('/api/security/permissions'),
    refetchInterval: 15000,
  })

  const [auditPage, setAuditPage] = useState(1)
  const [auditQuery, setAuditQuery] = useState('')
  const [auditOnlyDenied, setAuditOnlyDenied] = useState(false)
  const AUDIT_PAGE_SIZE = 20

  if (auditLoading) return <LoadingCards count={4} />
  if (auditError) return <ErrorState onRetry={() => refetch()} />

  const allEntries = (Array.isArray(audits?.items) ? audits.items : []).map((e) => ({
    ...e,
    id: `${e.timestamp}-${e.agent_name}`,
    agent_id: e.agent_name,
  }))
  const q = auditQuery.trim().toLowerCase()
  const entries = allEntries.filter((e) => {
    if (auditOnlyDenied && e.allowed) return false
    if (!q) return true
    return (
      e.action.toLowerCase().includes(q) ||
      (e.resource ?? '').toLowerCase().includes(q) ||
      (e.agent_name ?? '').toLowerCase().includes(q) ||
      (e.reason ?? '').toLowerCase().includes(q)
    )
  })
  const totalPages = Math.max(1, Math.ceil(entries.length / AUDIT_PAGE_SIZE))
  const safePage = Math.min(auditPage, totalPages)
  const pagedEntries = entries.slice((safePage - 1) * AUDIT_PAGE_SIZE, safePage * AUDIT_PAGE_SIZE)
  const isKo = i18n.language?.startsWith('ko') ?? false

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('security.title')}
        subtitle={t('security.subtitle')}
        actions={<RefreshButton onClick={() => refetch()} isFetching={isFetching} />}
      />
      <ApprovalsQueue />

      <ApprovalConfigPanel isKo={isKo} />

      {/* Permissions */}

      {permissionsError ? (
        <ErrorState onRetry={() => refetchPermissions()} />
      ) : permissions ? (
        <div className="space-y-3">
          <div className="flex items-center gap-2 px-1">
            <Shield className="h-4 w-4 text-muted-foreground" />
            <h2 className="text-sm font-semibold">{t('security.permissions')}</h2>
            <Badge variant="outline" className="text-2xs">
              {t('security.readOnly')}
            </Badge>
          </div>
          {permissions.policies
            .slice()
            .sort((a, b) => b.resources.length - a.resources.length)
            .map((policy) => {
              const roleName = policy.name.replace('-default', '')
              return (
                <Card key={policy.name}>
                  <CardContent className="pt-4">
                    <div className="flex items-center justify-between mb-2">
                      <div className="flex items-center gap-2">
                        <KeyRound className="h-4 w-4 text-muted-foreground" />
                        <span className="font-medium">{roleName}</span>
                      </div>
                      <span className="text-xs text-muted-foreground">
                        {policy.resources.length}{' '}
                        {policy.resources.length !== 1
                          ? t('security.permissions')
                          : t('security.permission')}
                      </span>
                    </div>
                    {policy.resources.length > 0 ? (
                      <div className="flex gap-1.5 flex-wrap">
                        {policy.resources.map((resource) => (
                          <Badge key={resource} variant="secondary" className="text-xs">
                            {resource}
                          </Badge>
                        ))}
                      </div>
                    ) : (
                      <p className="text-xs text-muted-foreground">{t('security.noPermissions')}</p>
                    )}
                  </CardContent>
                </Card>
              )
            })}
        </div>
      ) : null}

      {/* Audit Trail */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between gap-2 flex-wrap">
            <CardTitle className="flex items-center gap-2">
              <FileWarning className="h-4 w-4" /> {t('security.auditTrail')}
            </CardTitle>
            <div className="flex items-center gap-2">
              <div className="relative">
                <Search className="absolute left-2 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
                <Input
                  value={auditQuery}
                  onChange={(e) => {
                    setAuditQuery(e.target.value)
                    setAuditPage(1)
                  }}
                  placeholder={t('security.searchAudit')}
                  className="pl-7 h-8 w-56"
                />
              </div>
              <Button
                size="sm"
                variant={auditOnlyDenied ? 'default' : 'outline'}
                onClick={() => {
                  setAuditOnlyDenied((v) => !v)
                  setAuditPage(1)
                }}
                className="gap-1.5"
              >
                <Filter className="h-3.5 w-3.5" />
                {t('security.onlyDenied')}
              </Button>
            </div>
          </div>
        </CardHeader>
        <CardContent>
          {entries.length === 0 ? (
            <EmptyState
              icon={<Shield className="h-8 w-8" />}
              title={
                q || auditOnlyDenied
                  ? t('security.noMatchingEntries')
                  : t('security.noAuditEntries')
              }
              description={t('security.noAuditEntriesDescription')}
              className="py-6"
            />
          ) : (
            <div className="space-y-2">
              {pagedEntries.map((entry) => (
                <div
                  key={entry.id}
                  className="flex items-center justify-between rounded-lg border p-3"
                >
                  <div className="flex items-center gap-3">
                    <Badge variant={entry.allowed ? 'success' : 'destructive'} className="shrink-0">
                      {entry.allowed ? t('security.allow') : t('security.deny')}
                    </Badge>
                    <div>
                      <p className="font-medium text-sm">{entry.action}</p>
                      {entry.resource && (
                        <p className="text-xs text-muted-foreground">{entry.resource}</p>
                      )}
                      {entry.agent_id && (
                        <p className="text-xs text-muted-foreground">
                          {t('security.agent')}: {entry.agent_id.slice(0, 8)}...
                        </p>
                      )}
                      {entry.reason && <p className="text-xs text-warning">{entry.reason}</p>}
                    </div>
                  </div>
                  <div className="text-right">
                    <p className="text-xs text-muted-foreground">
                      {new Date(entry.timestamp).toLocaleString()}
                    </p>
                  </div>
                </div>
              ))}
            </div>
          )}
          {/* Pagination */}
          {totalPages > 1 && (
            <div className="flex items-center justify-between pt-3 border-t mt-3">
              <p className="text-xs text-muted-foreground">
                {t('security.showingEntries', {
                  start: (auditPage - 1) * AUDIT_PAGE_SIZE + 1,
                  end: Math.min(auditPage * AUDIT_PAGE_SIZE, entries.length),
                  total: entries.length,
                })}
              </p>
              <div className="flex gap-1">
                <Button
                  variant="outline"
                  size="sm"
                  disabled={auditPage <= 1}
                  onClick={() => setAuditPage((p) => Math.max(1, p - 1))}
                >
                  {t('common.previous')}
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={auditPage >= totalPages}
                  onClick={() => setAuditPage((p) => Math.min(totalPages, p + 1))}
                >
                  {t('common.next')}
                </Button>
              </div>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
