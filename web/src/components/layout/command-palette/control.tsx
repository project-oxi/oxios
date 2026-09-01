import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Ban, Flame, Play, Power } from 'lucide-react'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { api } from '@/lib/api-client'
import type { Skill } from '@/types'
import type { AgentListResponse } from '@/types/agent'
import type { CommandProvider, PaletteItem, QueryContext } from './types'

/** Minimal Automation fields the palette needs (WT-1 `/api/automations`). */
interface AutomationRef {
  id: string
  name: string
}

/**
 * Control provider — verb `control` (prefix `!`).
 *
 * Direct REST actions (Tier 1, design §4). Conventions (§5 "inline action
 * args"): destructive targets (agent kill, automation run) need no action
 * token — the entity alone implies the verb's default; toggle-able targets
 * (skill, token-maxing) require an explicit `enable|disable|start|stop` action.
 *
 * Note: `@maxing` has no colon, so the lexer yields `{ name: 'maxing' }` with no
 * `type` — match it by name too.
 */
export function useControlProvider(): CommandProvider {
  const { t } = useTranslation()
  const qc = useQueryClient()

  const agentsQ = useQuery({
    queryKey: ['agents', 'running', 'palette'],
    queryFn: () =>
      api.get<AgentListResponse>(
        '/api/agents?status=running&per_page=100&sort_by=created_at&sort_dir=desc',
      ),
    refetchInterval: 10_000,
  })
  const automationsQ = useQuery({
    queryKey: ['automations'],
    queryFn: async (): Promise<AutomationRef[]> => {
      const res = await api.get<{ automations: AutomationRef[] }>('/api/automations')
      return Array.isArray(res?.automations) ? res.automations : []
    },
    refetchInterval: 30_000,
  })
  const skillsQ = useQuery({
    queryKey: ['skills'],
    queryFn: () => api.get<{ skills: Skill[] }>('/api/skills'),
    staleTime: 60_000,
  })

  const killMutation = useMutation({
    mutationFn: (id: string) => api.post(`/api/agents/${id}/kill`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['agents'] })
      toast.success(t('commandPalette.killedAgent'))
    },
    onError: () => toast.error(t('commandPalette.controlFailed')),
  })
  const runMutation = useMutation({
    mutationFn: (id: string) => api.post(`/api/automations/${id}/run`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['automations'] })
      toast.success(t('commandPalette.triggeredAutomation'))
    },
    onError: () => toast.error(t('commandPalette.controlFailed')),
  })
  const skillToggle = useMutation({
    mutationFn: ({ name, enable }: { name: string; enable: boolean }) =>
      api.post(`/api/skills/${encodeURIComponent(name)}/${enable ? 'enable' : 'disable'}`),
    onSuccess: (_d, v) => {
      qc.invalidateQueries({ queryKey: ['skills'] })
      toast.success(t(v.enable ? 'commandPalette.skillEnabled' : 'commandPalette.skillDisabled'))
    },
    onError: () => toast.error(t('commandPalette.controlFailed')),
  })
  const maxingMutation = useMutation({
    mutationFn: (start: boolean) => api.post(`/api/token-maxing/${start ? 'start' : 'stop'}`),
    onSuccess: (_d, start) =>
      toast.success(t(start ? 'commandPalette.maxingStarted' : 'commandPalette.maxingStopped')),
    onError: () => toast.error(t('commandPalette.controlFailed')),
  })

  const agents = agentsQ.data?.items ?? []
  const automations = automationsQ.data ?? []
  const skills = skillsQ.data?.skills ?? []

  return useMemo(
    () => ({
      id: 'control',
      verbs: ['control'],
      resolve(ctx: QueryContext): PaletteItem[] {
        if (ctx.verb !== 'control') return []
        const ent = ctx.entity
        if (!ent) return []
        const name = ent.name.toLowerCase()
        const action = ctx.action

        // @maxing start|stop — matched by type OR bare name (no colon typed).
        if (ent.type === 'maxing' || name === 'maxing') {
          if (action !== 'start' && action !== 'stop') {
            return [
              {
                id: 'control-maxing-hint',
                verb: 'control',
                icon: <Flame className="h-4 w-4" />,
                title: t('commandPalette.maxingHint'),
                score: 100,
                onSelect: () => {},
              },
            ]
          }
          return [
            {
              id: `control-maxing-${action}`,
              verb: 'control',
              icon: <Flame className="h-4 w-4" />,
              title: t(
                action === 'start' ? 'commandPalette.maxingStart' : 'commandPalette.maxingStop',
              ),
              score: 100,
              onSelect: () => maxingMutation.mutate(action === 'start'),
            },
          ]
        }

        // @agent → kill (entity implies default; no action token)
        if (ent.type === 'agent' || !ent.type) {
          const agent = agents.find(
            (a) =>
              a.id === ent.name ||
              a.name.toLowerCase().includes(name) ||
              a.id.toLowerCase() === name,
          )
          if (agent) {
            return [
              {
                id: `control-agent-${agent.id}`,
                verb: 'control',
                icon: <Ban className="h-4 w-4" />,
                title: t('commandPalette.killAgent', { name: agent.name }),
                subtitle: agent.id,
                score: 100,
                onSelect: () => killMutation.mutate(agent.id),
              },
            ]
          }
          if (ent.type === 'agent') return []
        }

        // @automation → run
        if (ent.type === 'automation' || !ent.type) {
          const automation = automations.find(
            (a) =>
              a.id === ent.name ||
              a.name.toLowerCase().includes(name) ||
              a.id.toLowerCase() === name,
          )
          if (automation) {
            return [
              {
                id: `control-automation-${automation.id}`,
                verb: 'control',
                icon: <Play className="h-4 w-4" />,
                title: t('commandPalette.triggerAutomation', { name: automation.name }),
                score: 100,
                onSelect: () => runMutation.mutate(automation.id),
              },
            ]
          }
          if (ent.type === 'automation') return []
        }

        // @skill enable|disable (requires action token)
        if (ent.type === 'skill' || (!ent.type && action)) {
          const skill = skills.find(
            (s) => s.name.toLowerCase() === name || s.name.toLowerCase().includes(name),
          )
          if (skill) {
            if (action !== 'enable' && action !== 'disable') {
              return [
                {
                  id: `control-skill-hint-${skill.name}`,
                  verb: 'control',
                  icon: <Power className="h-4 w-4" />,
                  title: t('commandPalette.skillToggleHint', { name: skill.name }),
                  score: 100,
                  onSelect: () => {},
                },
              ]
            }
            return [
              {
                id: `control-skill-${skill.name}-${action}`,
                verb: 'control',
                icon: <Power className="h-4 w-4" />,
                title: t(
                  action === 'enable'
                    ? 'commandPalette.enableSkill'
                    : 'commandPalette.disableSkill',
                  { name: skill.name },
                ),
                score: 100,
                onSelect: () =>
                  skillToggle.mutate({ name: skill.name, enable: action === 'enable' }),
              },
            ]
          }
        }

        return []
      },
    }),
    [t, agents, automations, skills, killMutation, runMutation, skillToggle, maxingMutation],
  )
}
