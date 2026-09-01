import { useQuery } from '@tanstack/react-query'
import { useRouter } from '@tanstack/react-router'
import { FolderKanban, MessageSquare, Zap } from 'lucide-react'
import { useCallback, useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import { useProjects } from '@/hooks/use-projects'
import { api } from '@/lib/api-client'
import { useChatStore } from '@/stores/chat'
import type { Skill } from '@/types'
import { surfacePrimaryVerb } from './ranker'
import type { CommandProvider, PaletteItem, QueryContext } from './types'

/**
 * Run provider — verb `run` (prefix `>`).
 *
 * v1 = frontend message synthesis (design §8). Composes an intent over the
 * existing per-message slot (`project_id`) and sends via the chat
 * WS. `@persona` is deferred to v2 (no per-message slot yet); persona switching
 * is owned by `~ @persona` (SwitchProvider).
 *
 * Also owns Studio bare-text (surface-primary `run`): typing plain text in
 * Studio sends it as a message.
 */
export function useRunProvider(): CommandProvider {
  const { t } = useTranslation()
  const router = useRouter()

  const setActiveProject = useChatStore((s) => s.setActiveProject)
  const sendMessage = useChatStore((s) => s.sendMessage)

  const projectsQ = useProjects()
  const skillsQ = useQuery({
    queryKey: ['skills'],
    queryFn: () => api.get<{ skills: Skill[] }>('/api/skills'),
    staleTime: 60_000,
  })

  const projects = projectsQ.data?.items ?? []
  const skills = skillsQ.data?.skills ?? []

  const startChat = useCallback(
    (content: string) => {
      router.history.push('/studio')
      sendMessage(content)
    },
    [router, sendMessage],
  )

  return useMemo(
    () => ({
      id: 'run',
      verbs: ['run'],
      resolve(ctx: QueryContext): PaletteItem[] {
        const isRun = ctx.verb === 'run'
        const isStudioBare = ctx.verb === null && surfacePrimaryVerb(ctx.surface) === 'run'
        if (!isRun && !isStudioBare) return []

        const text = ctx.text.trim()
        const ent = ctx.entity

        // Entity-targeted run.
        if (ent) {
          const name = ent.name.toLowerCase()
          const wantsSkill = ent.type === 'skill'
          const wantsProject = ent.type === 'project'
          const skill = !wantsProject
            ? skills.find(
                (s) => s.name.toLowerCase() === name || s.name.toLowerCase().includes(name),
              )
            : undefined
          const project = !wantsSkill
            ? projects.find(
                (p) => p.name.toLowerCase() === name || p.name.toLowerCase().includes(name),
              )
            : undefined

          // Explicit @skill:… with no match → nothing (avoid a dead action).
          if (wantsSkill && !skill) return []
          if (wantsProject && !project) return []

          if (skill && (wantsSkill || !project)) {
            const intent = text
            const composed = intent ? `[skill: ${skill.name}] ${intent}` : `[skill: ${skill.name}]`
            return [
              {
                id: `run-skill-${skill.name}`,
                verb: 'run',
                icon: <Zap className="h-4 w-4" />,
                title: t('commandPalette.runSkill', { name: skill.name }),
                subtitle: intent || t('commandPalette.composed'),
                score: 100,
                onSelect: () => startChat(composed),
              },
            ]
          }

          if (project) {
            const intent = text
            // Undefined intent → set project context, navigate, let the user type.
            if (!intent) {
              return [
                {
                  id: `run-project-${project.id}`,
                  verb: 'run',
                  icon: <FolderKanban className="h-4 w-4" />,
                  title: t('commandPalette.runProject', { name: project.name }),
                  subtitle: t('commandPalette.typeIntent'),
                  score: 100,
                  onSelect: () => {
                    setActiveProject(project.id)
                    router.history.push('/studio')
                  },
                },
              ]
            }
            return [
              {
                id: `run-project-${project.id}`,
                verb: 'run',
                icon: <FolderKanban className="h-4 w-4" />,
                title: t('commandPalette.runProject', { name: project.name }),
                subtitle: intent,
                score: 100,
                onSelect: () => {
                  setActiveProject(project.id)
                  startChat(intent)
                },
              },
            ]
          }

          // Bare @name matched neither skill nor project.
          return []
        }

        // No entity: run the text as a chat intent.
        if (text) {
          return [
            {
              id: 'run-intent',
              verb: 'run',
              icon: <MessageSquare className="h-4 w-4" />,
              title: t('commandPalette.runIntent'),
              subtitle: text,
              hint: <kbd className="text-[10px] text-muted-foreground">⏎</kbd>,
              score: 100,
              onSelect: () => startChat(text),
            },
          ]
        }

        // `>` with nothing yet → hint.
        if (isRun) {
          return [
            {
              id: 'run-hint',
              verb: 'run',
              icon: <MessageSquare className="h-4 w-4" />,
              title: t('commandPalette.runHint'),
              score: 0,
              onSelect: () => {},
            },
          ]
        }

        return []
      },
    }),
    [t, skills, projects, startChat, setActiveProject, router],
  )
}
