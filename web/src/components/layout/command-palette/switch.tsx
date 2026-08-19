import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useRouter } from '@tanstack/react-router'
import { Brain, Cpu, LayoutDashboard, MessageSquare, Theater } from 'lucide-react'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { useModels } from '@/hooks/use-engine'
import { api } from '@/lib/api-client'
import { useChatStore } from '@/stores/chat'
import type { CommandProvider, PaletteItem, QueryContext } from './types'

interface Persona {
  id: string
  name: string
}

const MODE_HREF: Record<string, string> = {
  console: '/',
  knowledge: '/brain/knowledge',
  brain: '/brain',
  chat: '/chat',
}

function modeIcon(key: string) {
  if (key === 'knowledge' || key === 'brain') return <Brain className="h-4 w-4" />
  if (key === 'chat') return <MessageSquare className="h-4 w-4" />
  return <LayoutDashboard className="h-4 w-4" />
}

/**
 * Switch provider — verb `switch` (prefix `~`).
 *
 * Owns global/active-state switches: sidebar mode (`~ @mode:`), active persona
 * (`~ @persona:`, the only way to pick a persona in v1), and the per-message
 * model override (`~ @model:`). Mode switches navigate; persona/model switches
 * mutate active state.
 */
export function useSwitchProvider(): CommandProvider {
  const { t } = useTranslation()
  const router = useRouter()
  const setActiveModelId = useChatStore((s) => s.setActiveModelId)
  const qc = useQueryClient()

  const modelsQ = useModels(null)
  const personasQ = useQuery({
    queryKey: ['personas'],
    queryFn: async (): Promise<Persona[]> => {
      const res = await api.get<Persona[]>('/api/personas')
      return Array.isArray(res) ? res : []
    },
    staleTime: 60_000,
  })
  const activateMutation = useMutation({
    // RFC-039: PUT /api/personas/active {id} (was POST /:id/activate — 404)
    mutationFn: (id: string) => api.put('/api/personas/active', { id }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['personas'] })
      toast.success(t('commandPalette.switchedPersona'))
    },
    onError: () => toast.error(t('commandPalette.switchFailed')),
  })

  const models = modelsQ.data ?? []
  const personas = personasQ.data ?? []

  return useMemo(
    () => ({
      id: 'switch',
      verbs: ['switch'],
      resolve(ctx: QueryContext): PaletteItem[] {
        if (ctx.verb !== 'switch') return []
        const ent = ctx.entity
        if (!ent) return []
        const name = ent.name.toLowerCase()

        // @mode — explicit only (bare name rarely a mode). Capture into a const
        // so the `string | undefined` from indexed access narrows after the guard.
        const modeHref = ent.type === 'mode' ? MODE_HREF[name] : undefined
        if (modeHref) {
          return [
            {
              id: `switch-mode-${name}`,
              verb: 'switch',
              icon: modeIcon(name),
              title: t('commandPalette.switchMode', { mode: name }),
              score: 100,
              onSelect: () => router.history.push(modeHref),
            },
          ]
        }
        if (ent.type === 'mode') return [] // explicit @mode:… with no match

        // @persona — explicit type, or bare name matching a persona.
        if (ent.type === 'persona' || !ent.type) {
          const persona = personas.find((p) => p.name.toLowerCase().includes(name))
          if (persona) {
            return [
              {
                id: `switch-persona-${persona.id}`,
                verb: 'switch',
                icon: <Theater className="h-4 w-4" />,
                title: t('commandPalette.switchPersona', { name: persona.name }),
                score: 100,
                onSelect: () => activateMutation.mutate(persona.id),
              },
            ]
          }
          if (ent.type === 'persona') return []
        }

        // @model — explicit type, or bare name matching a model id/name.
        if (ent.type === 'model' || !ent.type) {
          const model = models.find(
            (m) => m.id.toLowerCase().includes(name) || m.name.toLowerCase().includes(name),
          )
          if (model) {
            return [
              {
                id: `switch-model-${model.id}`,
                verb: 'switch',
                icon: <Cpu className="h-4 w-4" />,
                title: t('commandPalette.switchModel', { name: model.name }),
                subtitle: model.id,
                score: 100,
                onSelect: () => {
                  setActiveModelId(model.id)
                  toast.success(t('commandPalette.switchedModel', { name: model.name }))
                },
              },
            ]
          }
          if (ent.type === 'model') return []
        }

        return []
      },
    }),
    [t, router, models, personas, activateMutation, setActiveModelId],
  )
}
