import {
  Check,
  CircleAlert,
  CircleCheck,
  CircleX,
  ExternalLink,
  Pencil,
  X,
  Zap,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { SkillContent } from '@/components/skills/skill-content'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { cn } from '@/lib/utils'
import type { Skill, SkillFormat, SkillStatus } from '@/types'

const FORMAT_META: Record<
  SkillFormat,
  { label: string; variant: 'default' | 'secondary' | 'outline'; description: string }
> = {
  oxios: { label: 'Oxios', variant: 'default', description: 'Oxios native skill' },
  openclaw: { label: 'OpenClaw', variant: 'secondary', description: 'ClawHub marketplace skill' },
  claude_code: { label: 'Claude', variant: 'outline', description: 'Claude Code skill' },
  agent_skills: { label: 'Standard', variant: 'outline', description: 'Agent Skills standard' },
}

const STATUS_DISPLAY: Record<
  SkillStatus,
  { icon: React.ReactNode; label: string; variant: 'success' | 'warning' | 'destructive' }
> = {
  ready: { icon: <CircleCheck className="h-3 w-3" />, label: 'ready', variant: 'success' },
  needs_setup: {
    icon: <CircleAlert className="h-3 w-3" />,
    label: 'needs-setup',
    variant: 'warning',
  },
  disabled: { icon: <CircleX className="h-3 w-3" />, label: 'disabled', variant: 'destructive' },
}

// ─── Skill Detail side panel ────────────────────────────────

export function SkillDetail({
  skill,
  onClose,
  onEdit,
}: {
  skill: Skill
  onClose: () => void
  onEdit?: () => void
}) {
  const { t } = useTranslation()
  const sd = STATUS_DISPLAY[skill.status]
  const fm = FORMAT_META[skill.format]
  const hasMissing =
    skill.missing.bins.length > 0 ||
    skill.missing.anyBins.length > 0 ||
    skill.missing.env.length > 0 ||
    skill.missing.config.length > 0 ||
    (skill.missing.integrations?.length ?? 0) > 0 ||
    (skill.missing.anyIntegrations?.length ?? 0) > 0

  return (
    <div className="space-y-4">
      {/* Header */}
      <div className="flex items-start justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <Zap className="h-5 w-5 shrink-0" />
          <div className="min-w-0">
            <h2 className="font-semibold text-lg leading-tight truncate">{skill.name}</h2>
            <Badge variant={fm.variant} className="text-xs mt-1">
              {fm.label}
            </Badge>
          </div>
        </div>
        {onEdit && (
          <Button
            variant="ghost"
            size="icon"
            className="shrink-0 h-7 w-7"
            onClick={onEdit}
            title={t('skills.edit')}
          >
            <Pencil className="h-3.5 w-3.5" />
          </Button>
        )}
        <Button variant="ghost" size="icon" className="shrink-0 h-7 w-7" onClick={onClose}>
          <X className="h-4 w-4" />
        </Button>
      </div>
      <Tabs defaultValue="overview" className="w-full">
        <TabsList className="w-full">
          <TabsTrigger value="overview">{t('skills.overview')}</TabsTrigger>
          <TabsTrigger value="content">{t('skills.content')}</TabsTrigger>
        </TabsList>
        <TabsContent value="overview" className="space-y-4 mt-4">
          {/* Status */}
          <div className="flex items-center gap-2">
            <Badge variant={sd.variant} className="text-xs gap-1">
              sd.icon {sd.label}
            </Badge>
            {skill.version && (
              <span className="text-xs font-mono text-muted-foreground">v{skill.version}</span>
            )}
            <Badge variant="outline" className="text-xs">
              {skill.source}
            </Badge>
          </div>

          {/* Description */}
          {skill.description && (
            <p className="text-sm text-muted-foreground">{skill.description}</p>
          )}

          {skill.author && (
            <p className="text-xs text-muted-foreground">
              {t('skills.by')} {skill.author}
            </p>
          )}

          <Separator />

          {/* Requirements */}
          {(skill.requirements.bins.length > 0 ||
            skill.requirements.anyBins.length > 0 ||
            skill.requirements.env.length > 0 ||
            skill.requirements.config.length > 0) && (
            <div className="space-y-2">
              <p className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
                {t('skills.requires')}
              </p>
              <div className="space-y-1.5 pl-1">
                {skill.requirements.bins.length > 0 && (
                  <ReqList items={skill.requirements.bins} missing={skill.missing.bins} />
                )}
                {skill.requirements.anyBins.length > 0 && (
                  <ReqList items={skill.requirements.anyBins} missing={skill.missing.anyBins} />
                )}
                {skill.requirements.env.length > 0 && (
                  <ReqList items={skill.requirements.env} missing={skill.missing.env} />
                )}
                {skill.requirements.config.length > 0 && (
                  <ReqList items={skill.requirements.config} missing={skill.missing.config} />
                )}
                {(skill.requirements.integrations?.length ?? 0) > 0 && (
                  <div className="space-y-1">
                    <span className="text-xs text-muted-foreground">
                      {t('skills.requiresIntegrations')}:
                    </span>
                    <ul className="flex flex-col gap-1">
                      {(skill.requirements.integrations ?? []).map((id) => {
                        const status = skill.integration_status?.find((s) => s.id === id)
                        const ok = status?.satisfied ?? false
                        return (
                          <li
                            key={id}
                            className="flex items-center justify-between gap-2 rounded-md px-2 py-1 hover:bg-muted/40"
                          >
                            <div className="flex items-center gap-2">
                              {ok ? (
                                <CircleCheck className="h-3 w-3 text-success" />
                              ) : (
                                <CircleX className="h-3 w-3 text-destructive" />
                              )}
                              <span className="font-mono text-xs">{id}</span>
                              {status && !status.installed && (
                                <Badge variant="outline" className="text-[10px] text-warning">
                                  {t('skills.integrationNotInstalled')}
                                </Badge>
                              )}
                              {status?.installed && !status.configured && (
                                <Badge variant="outline" className="text-[10px] text-warning">
                                  {t('skills.integrationNotConfigured')}
                                </Badge>
                              )}
                            </div>
                            {!ok && (
                              <a
                                className="text-[10px] text-info underline hover:opacity-80"
                                href="/settings?section=host-tools"
                              >
                                {t('skills.configureInSettings')}
                              </a>
                            )}
                          </li>
                        )
                      })}
                    </ul>
                  </div>
                )}
              </div>
            </div>
          )}

          {/* Missing warning */}
          {hasMissing && skill.status === 'needs_setup' && (
            <div className="rounded-md bg-warning/10 border border-warning/20 px-3 py-2">
              <p className="text-xs text-warning">
                {t('skills.missingWarning', {
                  missing: [
                    ...skill.missing.bins.map((b) => `bin:${b}`),
                    ...skill.missing.env.map((e) => `env:${e}`),
                    ...skill.missing.config.map((c) => `config:${c}`),
                    ...skill.missing.anyBins.map((b) => `any_bin:${b}`),
                    ...(skill.missing.integrations ?? []).map((i) => `integration:${i}`),
                    ...(skill.missing.anyIntegrations ?? []).map((i) => `any_integration:${i}`),
                  ].join(', '),
                })}
              </p>
            </div>
          )}

          {/* Install specs */}
          {skill.install.length > 0 && (
            <div className="space-y-2">
              <p className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
                {t('skills.install')}
              </p>
              <div className="space-y-1 pl-1">
                {skill.install.map((sp, i) => (
                  <div
                    key={`${sp.kind}-${i}`}
                    className="flex items-center gap-2 text-sm text-muted-foreground"
                  >
                    <span className="text-xs font-mono bg-muted px-1.5 py-0.5 rounded">
                      {sp.kind}
                    </span>
                    <span>{sp.label ?? sp.bins.join(', ')}</span>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Config checks */}
          {skill.config_checks.length > 0 && (
            <div className="space-y-2">
              <p className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
                Config
              </p>
              <div className="space-y-1 pl-1">
                {skill.config_checks.map((cc, i) => (
                  <div key={i} className="flex items-center gap-2 text-xs">
                    {cc.satisfied ? (
                      <Check className="h-3 w-3 text-success" />
                    ) : (
                      <X className="h-3 w-3 text-error" />
                    )}
                    <span className={cn('font-mono', !cc.satisfied && 'text-error')}>
                      {cc.path}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* OS / Flags */}
          <div className="flex flex-wrap gap-1.5 text-xs">
            {skill.os.length > 0 &&
              skill.os.map((o) => (
                <Badge key={o} variant="outline" className="text-xs">
                  {o}
                </Badge>
              ))}
            {skill.always && (
              <Badge variant="secondary" className="text-xs">
                {t('skills.always')}
              </Badge>
            )}
            {skill.bundled && (
              <Badge variant="outline" className="text-xs">
                bundled
              </Badge>
            )}
          </div>

          {/* Homepage link */}
          {skill.homepage && (
            <a
              href={skill.homepage}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1 text-xs text-primary hover:underline"
            >
              <ExternalLink className="h-3 w-3" /> {skill.homepage}
            </a>
          )}

          {/* File path */}
          <p
            className="text-xs text-muted-foreground/60 font-mono truncate"
            title={skill.file_path}
          >
            {skill.file_path}
          </p>
        </TabsContent>
        <TabsContent value="content" className="mt-4">
          <SkillContent skill={skill} onEdit={onEdit} />
        </TabsContent>
      </Tabs>
    </div>
  )
}

// ─── Helpers ─────────────────────────────────────────────────

function ReqList({ items, missing }: { items: string[]; missing: string[] }) {
  return (
    <div className="flex flex-wrap gap-x-3 gap-y-0.5">
      {items.map((item) => {
        const m = missing.includes(item)
        return (
          <span
            key={item}
            className={cn('flex items-center gap-1 text-xs', m ? 'text-error' : 'text-success')}
          >
            {m ? <X className="h-3 w-3" /> : <Check className="h-3 w-3" />}
            {item}
          </span>
        )
      })}
    </div>
  )
}
