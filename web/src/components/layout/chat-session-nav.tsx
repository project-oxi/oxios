import { useQuery } from '@tanstack/react-query'
import { ChevronRight, FolderKanban, FolderPlus, Inbox, Plus } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { CreateProjectDialog } from '@/components/project/create-project-dialog'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useMoveSession } from '@/hooks/use-sessions'
import { api } from '@/lib/api-client'
import { cn } from '@/lib/utils'
import { useChatStore } from '@/stores/chat'
import { useSidebarStore } from '@/stores/sidebar'
import type { Project, Session } from '@/types'
import {
  itemActive,
  itemCollapsedBase,
  itemDense,
  itemInactive,
  sectionGap,
  sectionHeader,
  sectionSeparator,
} from './sidebar-primitives'
// ---------------------------------------------------------------------------
// ChatSessionNav — renders inside the sidebar when the Studio surface is active.
// RFC-025: Project-tree layout (Project folders → sessions).
// ---------------------------------------------------------------------------

export function ChatSessionNav() {
  const { collapsed } = useSidebarStore()

  if (collapsed) {
    return <CollapsedChatNav />
  }

  return <ExpandedChatNav />
}

// ---------------------------------------------------------------------------
// Expanded — Project-tree
// ---------------------------------------------------------------------------

function ExpandedChatNav() {
  const { t } = useTranslation()
  const activeSessionId = useChatStore((s) => s.activeSessionId)
  const loadSession = useChatStore((s) => s.loadSession)
  const newSession = useChatStore((s) => s.newSession)
  const moveSession = useMoveSession()

  const [collapsedProjects, setCollapsedProjects] = useState<Set<string>>(new Set())
  const [dragOverProject, setDragOverProject] = useState<string | null>(null)
  const [createProjectOpen, setCreateProjectOpen] = useState(false)

  const { data: projectsData } = useQuery({
    queryKey: ['projects'],
    queryFn: () => api.get<{ items: Project[]; total: number }>('/api/projects'),
    refetchInterval: 30_000,
  })

  const { data: sessionsData } = useQuery({
    queryKey: ['sessions'],
    queryFn: () => api.get<{ items: Session[]; total: number }>('/api/sessions'),
    refetchInterval: 10_000,
  })

  const projects: Project[] = Array.isArray(projectsData?.items) ? projectsData.items : []
  const allSessions: Session[] = Array.isArray(sessionsData?.items) ? sessionsData.items : []

  // Group sessions by project_id.
  const sessionsByProject = new Map<string, Session[]>()
  const unfiledSessions: Session[] = []
  for (const s of allSessions) {
    if (s.project_id) {
      const arr = sessionsByProject.get(s.project_id) ?? []
      arr.push(s)
      sessionsByProject.set(s.project_id, arr)
    } else {
      unfiledSessions.push(s)
    }
  }

  const toggleProject = (id: string) => {
    setCollapsedProjects((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const handleDropToProject = async (projectId: string | null) => {
    const draggedId = window.__draggedSessionId
    setDragOverProject(null)
    if (!draggedId) return
    delete window.__draggedSessionId
    try {
      await moveSession.mutateAsync({ sessionId: draggedId, project_id: projectId })
      // Web-M2: if the moved session is the active session, sync the
      // chat-store's activeProjectId so subsequent messages route to the
      // new project.
      if (useChatStore.getState().activeSessionId === draggedId) {
        useChatStore.setState({ activeProjectId: projectId })
      }
      toast.success(t('chat.sessionMoved'))
    } catch (err) {
      toast.error(err instanceof Error ? err.message : t('chat.sessionMoveFailed'))
    }
  }

  return (
    <>
      {/* New session */}
      <div className={sectionGap}>
        <Button
          variant={activeSessionId ? 'outline' : 'default'}
          size="sm"
          className="w-full"
          onClick={newSession}
        >
          <Plus className="h-3 w-3 mr-1" />
          {t('chat.newConversationButton')}
        </Button>
      </div>

      {/* Sessions tree */}
      <div className="flex-1 overflow-y-auto">
        <div className="flex items-center justify-between px-2 mb-1">
          <span className={sectionHeader.replace('mb-1 ', '')}>{t('chat.sessionsLabel')}</span>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="h-6 w-6"
                onClick={() => setCreateProjectOpen(true)}
              >
                <FolderPlus className="h-3 w-3" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="left">{t('chat.newProjectButton')}</TooltipContent>
          </Tooltip>
        </div>

        {/* ── Project folders ── */}
        {projects.map((p) => {
          const sessions = sessionsByProject.get(p.id) ?? []
          const isCollapsed = collapsedProjects.has(p.id)
          const isDragOver = dragOverProject === p.id
          return (
            <div key={p.id} className="mb-0.5">
              {/* Project header (drop target) */}
              <div className="flex items-center">
                <button
                  type="button"
                  onClick={() => toggleProject(p.id)}
                  className="flex flex-1 items-center gap-1 px-2 py-1 text-sm hover:bg-sidebar-accent rounded-sm"
                >
                  <ChevronRight
                    className={cn(
                      'h-3 w-3 shrink-0 transition-transform',
                      !isCollapsed && 'rotate-90',
                    )}
                  />
                  <FolderKanban className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  <span className="truncate font-medium">{p.name}</span>
                  {sessions.length > 0 && (
                    <span className="ml-auto text-2xs text-muted-foreground/60">
                      {sessions.length}
                    </span>
                  )}
                </button>
              </div>
              {/* Project as drop target */}
              <div
                onDragOver={(e) => {
                  e.preventDefault()
                  setDragOverProject(p.id)
                }}
                onDragLeave={() => setDragOverProject((cur) => (cur === p.id ? null : cur))}
                onDrop={(e) => {
                  e.preventDefault()
                  handleDropToProject(p.id)
                }}
                className={cn(
                  'rounded-sm transition-colors',
                  isDragOver && 'bg-primary/10 ring-1 ring-primary/30',
                )}
              >
                {/* Sessions under this project */}
                {!isCollapsed &&
                  sessions.map((s) => (
                    <SessionItem
                      key={s.id}
                      session={s}
                      active={activeSessionId === s.id}
                      indented
                      onClick={() => loadSession(s.id)}
                    />
                  ))}
                {!isCollapsed && sessions.length === 0 && (
                  <p className="px-7 py-0.5 text-2xs text-muted-foreground/40">
                    {t('chat.noSessionsInProject')}
                  </p>
                )}
              </div>
            </div>
          )
        })}

        {/* ── No project sessions ── */}
        {unfiledSessions.length > 0 && (
          <div className={sectionGap}>
            <div className="flex items-center gap-1 px-2 py-1 text-sm text-muted-foreground">
              <Inbox className="h-3.5 w-3.5" />
              <span className="font-medium">{t('chat.noProject')}</span>
              <span className="ml-auto text-2xs text-muted-foreground/60">
                {unfiledSessions.length}
              </span>
            </div>
            <div
              onDragOver={(e) => {
                e.preventDefault()
                setDragOverProject('__unfiled__')
              }}
              onDragLeave={() => setDragOverProject((cur) => (cur === '__unfiled__' ? null : cur))}
              onDrop={(e) => {
                e.preventDefault()
                handleDropToProject(null)
              }}
              className={cn(
                'rounded-sm transition-colors',
                dragOverProject === '__unfiled__' && 'bg-primary/10 ring-1 ring-primary/30',
              )}
            >
              {unfiledSessions.map((s) => (
                <SessionItem
                  key={s.id}
                  session={s}
                  active={activeSessionId === s.id}
                  indented
                  onClick={() => loadSession(s.id)}
                />
              ))}
            </div>
          </div>
        )}

        {/* Quick project switcher (sets the active project for new sessions) */}
        {projects.length > 0 && <div className={sectionSeparator} />}
      </div>

      <CreateProjectDialog open={createProjectOpen} onOpenChange={setCreateProjectOpen} />
    </>
  )
}

// ---------------------------------------------------------------------------
// Session item (draggable)
// ---------------------------------------------------------------------------

function SessionItem({
  session,
  active,
  indented,
  onClick,
}: {
  session: Session
  active: boolean
  indented?: boolean
  onClick: () => void
}) {
  return (
    <button
      type="button"
      draggable
      onDragStart={(e) => {
        window.__draggedSessionId = session.id
        e.dataTransfer.effectAllowed = 'move'
      }}
      onDragEnd={() => {
        delete window.__draggedSessionId
      }}
      onClick={onClick}
      className={cn(
        itemDense,
        indented && 'pl-7',
        active ? itemActive : itemInactive,
        'cursor-grab active:cursor-grabbing',
      )}
    >
      <span className="block truncate">{session.title ?? `${session.id.slice(0, 8)}...`}</span>
      <span className="block text-2xs text-muted-foreground/60">
        {new Date(session.created_at).toLocaleString(undefined, {
          month: 'short',
          day: 'numeric',
          hour: '2-digit',
          minute: '2-digit',
        })}
      </span>
    </button>
  )
}

// ---------------------------------------------------------------------------
// Collapsed
// ---------------------------------------------------------------------------

function CollapsedChatNav() {
  const { t } = useTranslation()
  const newSession = useChatStore((s) => s.newSession)

  return (
    <div className="flex flex-col items-center gap-1 py-1">
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            onClick={newSession}
            className={cn(itemCollapsedBase, itemInactive)}
          >
            <Plus className="h-4 w-4" />
          </button>
        </TooltipTrigger>
        <TooltipContent side="right">{t('chat.newConversationButton')}</TooltipContent>
      </Tooltip>
    </div>
  )
}
