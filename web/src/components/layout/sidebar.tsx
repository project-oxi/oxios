import { Link, useRouterState } from '@tanstack/react-router'
import {
  Activity,
  BookOpen,
  Bot,
  Brain,
  CheckSquare,
  FilePlus,
  Flame,
  FolderKanban,
  FolderOpen,
  FolderPlus,
  GitBranch,
  Images,
  LayoutDashboard,
  LayoutGrid,
  Mail,
  MessageSquare,
  Network,
  PanelLeft,
  PanelLeftClose,
  Settings,
  ShieldCheck,
  Sparkles,
  Theater,
  Timer,
  Trash2,
  Wallet,
  Zap,
} from 'lucide-react'
import React, { useCallback, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { FileTree } from '@/components/knowledge/file-tree'
import { HabitsDialog } from '@/components/knowledge/habits-dialog'
import { KnowledgeSettingsDialog } from '@/components/knowledge/knowledge-settings-dialog'
import { NewFolderDialog } from '@/components/knowledge/new-folder-dialog'
import { Button } from '@/components/ui/button'
import { ConfirmDialog } from '@/components/ui/confirm-dialog'
import { Separator } from '@/components/ui/separator'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import {
  useDeleteFile,
  useJournalToday,
  useKnowledgeRecursiveTree,
  useMoveFile,
  useWriteFile,
} from '@/hooks/use-knowledge'
import { generateUniqueName } from '@/lib/tree-utils'
import { cn } from '@/lib/utils'
import { useKnowledgeStore } from '@/stores/knowledge'
import { deriveSidebarMode, useSidebarStore } from '@/stores/sidebar'
import { ChatSessionNav } from './chat-session-nav'
import { ModeTabs } from './mode-tabs'
import { SidebarFooter } from './sidebar-footer'

// ── Shared sidebar design primitives (consumed by ChatSessionNav) ──

export const itemBase =
  'flex items-center gap-3 rounded-lg px-2.5 py-2 text-sm w-full text-left select-none transition-all focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-sidebar'

export const itemDense =
  'flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-xs w-full text-left select-none transition-all focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring'

export const itemActive = 'bg-sidebar-accent text-sidebar-accent-foreground font-medium'
export const itemInactive =
  'text-sidebar-foreground/70 hover:bg-sidebar-accent/50 hover:text-sidebar-foreground'
export const itemCollapsedBase =
  'flex items-center justify-center rounded-lg p-2 select-none transition-all focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring'

export const sectionHeader =
  'px-2 mb-1 text-xs font-medium text-muted-foreground uppercase tracking-wider select-none'

export const sectionGap = 'mb-3'

export const sectionSeparator = 'border-t border-sidebar-border my-2'

// ── Console mode nav groups ────────────────────────────────────

export interface NavItem {
  labelKey: string
  href: string
  icon: React.ReactNode
  external?: boolean
  badge?: number
}

export const consoleNavGroups: { labelKey: string; items: NavItem[] }[] = [
  {
    labelKey: 'common.main',
    items: [
      { labelKey: 'common.dashboard', href: '/', icon: <LayoutDashboard className="h-4 w-4" /> },
    ],
  },
  {
    labelKey: 'common.agents',
    items: [
      { labelKey: 'common.agents', href: '/agents', icon: <Bot className="h-4 w-4" /> },
      { labelKey: 'common.personas', href: '/personas', icon: <Theater className="h-4 w-4" /> },
      { labelKey: 'common.skills', href: '/skills', icon: <Sparkles className="h-4 w-4" /> },
      { labelKey: 'common.tasks', href: '/tasks', icon: <CheckSquare className="h-4 w-4" /> },
    ],
  },
  {
    labelKey: 'common.projects',
    items: [
      {
        labelKey: 'common.projects',
        href: '/projects',
        icon: <FolderKanban className="h-4 w-4" />,
      },
      { labelKey: 'common.mounts', href: '/mounts', icon: <FolderPlus className="h-4 w-4" /> },
    ],
  },
  {
    labelKey: 'common.storage',
    items: [
      { labelKey: 'common.brain', href: '/brain', icon: <Brain className="h-4 w-4" /> },
      { labelKey: 'common.assets', href: '/assets', icon: <Images className="h-4 w-4" /> },
      {
        labelKey: 'common.workspace',
        href: '/workspace',
        icon: <FolderOpen className="h-4 w-4" />,
      },
    ],
  },
  {
    labelKey: 'common.operations',
    items: [
      { labelKey: 'common.cronJobs', href: '/cron-jobs', icon: <Timer className="h-4 w-4" /> },
      { labelKey: 'common.cost', href: '/budget', icon: <Wallet className="h-4 w-4" /> },
      {
        labelKey: 'common.tokenMaxing',
        href: '/token-maxing',
        icon: <Flame className="h-4 w-4" />,
      },
    ],
  },
  {
    labelKey: 'common.infrastructure',
    items: [
      { labelKey: 'common.mcpServers', href: '/mcp', icon: <Zap className="h-4 w-4" /> },
      { labelKey: 'common.email', href: '/email', icon: <Mail className="h-4 w-4" /> },
      { labelKey: 'common.git', href: '/git', icon: <GitBranch className="h-4 w-4" /> },
    ],
  },
  {
    labelKey: 'common.system',
    items: [
      { labelKey: 'common.resources', href: '/resources', icon: <Activity className="h-4 w-4" /> },
      { labelKey: 'common.security', href: '/security', icon: <ShieldCheck className="h-4 w-4" /> },
      { labelKey: 'common.settings', href: '/settings', icon: <Settings className="h-4 w-4" /> },
    ],
  },
]

// ── Sidebar component ──────────────────────────────────────────

export function Sidebar() {
  const { collapsed, toggle, mode, setMode, mobileOpen } = useSidebarStore()
  const router = useRouterState()
  const currentPath = router.location.pathname

  useEffect(() => {
    const derivedMode = deriveSidebarMode(currentPath)
    setMode(derivedMode)
  }, [currentPath, setMode])

  return (
    <aside
      className={cn(
        'flex h-full w-72 max-w-[85vw] flex-col overflow-hidden border-r bg-sidebar text-sidebar-foreground transition-[width] duration-300 ease-[var(--animate-in-easing)]',
        collapsed ? 'lg:w-16 lg:max-w-none' : 'lg:w-60 lg:max-w-none',
      )}
    >
      <div
        className={cn(
          'flex h-14 items-center px-3',
          collapsed && !mobileOpen ? 'justify-center' : 'justify-between',
        )}
      >
        {!(collapsed && !mobileOpen) && (
          <div className="flex items-center gap-2">
            <img src="/favicon.png" alt="" className="h-6 w-6 rounded-md shrink-0" />
            <span className="font-bold text-lg">Oxios</span>
          </div>
        )}
        <button
          type="button"
          onClick={toggle}
          className="hidden lg:block rounded-md p-1.5 hover:bg-sidebar-accent focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
        >
          {collapsed ? <PanelLeft className="h-4 w-4" /> : <PanelLeftClose className="h-4 w-4" />}
        </button>
      </div>

      <div className="hidden lg:block px-2 pb-2">
        <ModeTabs collapsed={collapsed} />
      </div>

      <Separator />

      <nav className="flex-1 overflow-y-auto p-2">
        {mode === 'console' && <ConsoleNav />}
        {mode === 'knowledge' && <KnowledgeNav />}
        {mode === 'chat' && <ChatSessionNav />}
      </nav>

      <Separator />
      <SidebarFooter collapsed={collapsed && !mobileOpen} />
    </aside>
  )
}

// ── Console Nav ────────────────────────────────────────────────

function ConsoleNav() {
  const { t } = useTranslation()
  const router = useRouterState()
  const currentPath = router.location.pathname
  const { collapsed } = useSidebarStore()

  return (
    <>
      {consoleNavGroups.map((group) => (
        <div key={group.labelKey} className={sectionGap}>
          {!collapsed && <p className={sectionHeader}>{t(group.labelKey)}</p>}
          {group.items.map((item) => (
            <NavItemLink
              key={item.href}
              item={item}
              currentPath={currentPath}
              collapsed={collapsed}
            />
          ))}
        </div>
      ))}
    </>
  )
}

// ── Knowledge Nav ──────────────────────────────────────────────

function KnowledgeNav() {
  const { t } = useTranslation()
  const { collapsed } = useSidebarStore()
  const router = useRouterState()
  const currentPath = router.location.pathname
  const { mode, currentFilePath, openFile, openChat, openHome, markFileCreated, renameCurrent } =
    useKnowledgeStore()
  const moveFile = useMoveFile()
  const { data: tree, isLoading } = useKnowledgeRecursiveTree()
  const writeFile = useWriteFile()
  const deleteFile = useDeleteFile()
  const journalToday = useJournalToday()

  // Phase 4: rename via the atomic move API (no more write+delete).
  // If the renamed file is the one currently open in the editor, swap the
  // open path in place (via renameCurrent) so subsequent saves land on the
  // renamed file and the editor doesn't remount.
  const renameFile = useCallback(
    async (oldPath: string, newName: string) => {
      const parentDir = oldPath.includes('/') ? oldPath.split('/').slice(0, -1).join('/') : ''
      const target = parentDir ? `${parentDir}/${newName}` : newName
      if (target === oldPath) return
      try {
        await moveFile.mutateAsync({ from: oldPath, to: target })
        if (oldPath === currentFilePath) renameCurrent(target)
      } catch (err) {
        console.error('rename failed', err)
      }
    },
    [moveFile, currentFilePath, renameCurrent],
  )

  const [habitsOpen, setHabitsOpen] = useState(false)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [newFolderOpen, setNewFolderOpen] = useState(false)
  const [deleteOpen, setDeleteOpen] = useState(false)

  // Phase 4 (R6): generate a unique filename across the whole tree.
  const handleNewFile = useCallback(
    async (dir?: string) => {
      const basePath = dir ? `${dir}/` : ''
      const name = tree ? generateUniqueName(tree, basePath, 'New file.md') : 'New file.md'
      try {
        await writeFile.mutateAsync({
          path: `${basePath}${name}`,
          content: `# New file\n\n`,
        })
      } catch (err) {
        console.error('create failed', err)
        return
      }
      openFile(`${basePath}${name}`)
      markFileCreated(`${basePath}${name}`)
    },
    [tree, writeFile, openFile, markFileCreated],
  )

  // Phase 4 follow-up: replace window.prompt() with an in-app Dialog.
  const handleNewFolder = useCallback(() => {
    setNewFolderOpen(true)
  }, [])
  const handleDelete = useCallback(() => {
    if (!currentFilePath) return
    setDeleteOpen(true)
  }, [currentFilePath])

  const performDelete = useCallback(async () => {
    if (!currentFilePath) return
    try {
      await deleteFile.mutateAsync(currentFilePath)
    } catch (err) {
      console.error('delete failed', err)
    }
  }, [deleteFile, currentFilePath])

  const handleOpenJournal = useCallback(() => {
    if (journalToday.data?.path) {
      openFile(journalToday.data.path)
    }
  }, [journalToday.data, openFile])

  if (collapsed) {
    return (
      <>
        <div className="flex flex-col items-center gap-1 py-1">
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                onClick={() => openHome()}
                className={cn(itemCollapsedBase, mode === 'home' && itemActive)}
              >
                <LayoutGrid className="h-4 w-4" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="right">{t('knowledge.homeTitle')}</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                onClick={() => openChat()}
                className={cn(itemCollapsedBase, mode === 'chat' && itemActive)}
              >
                <MessageSquare className="h-4 w-4" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="right">{t('knowledge.chatTitle')}</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                onClick={handleOpenJournal}
                className={cn(itemCollapsedBase, itemInactive)}
              >
                <BookOpen className="h-4 w-4" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="right">{t('knowledge.toJournal')}</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Link
                to="/knowledge/graph"
                className={cn(
                  itemCollapsedBase,
                  currentPath === '/knowledge/graph' ? itemActive : itemInactive,
                )}
              >
                <Network className="h-4 w-4" />
              </Link>
            </TooltipTrigger>
            <TooltipContent side="right">{t('knowledge.linkGraphTitle')}</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                onClick={() => handleNewFile()}
                className={cn(itemCollapsedBase, itemInactive)}
              >
                <FilePlus className="h-4 w-4" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="right">{t('knowledge.newFile')}</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                onClick={() => setHabitsOpen(true)}
                className={cn(itemCollapsedBase, itemInactive)}
              >
                <Activity className="h-4 w-4" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="right">{t('knowledge.habitsTitle')}</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                onClick={() => setSettingsOpen(true)}
                className={cn(itemCollapsedBase, itemInactive)}
              >
                <Settings className="h-4 w-4" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="right">{t('knowledge.knowledgeSettings')}</TooltipContent>
          </Tooltip>
        </div>
        <HabitsDialog open={habitsOpen} onOpenChange={setHabitsOpen} />
        <KnowledgeSettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
        <NewFolderDialog
          open={newFolderOpen}
          onOpenChange={setNewFolderOpen}
          onConfirm={async (folderName) => {
            await writeFile.mutateAsync({ path: `${folderName}/.keep`, content: '' })
            toast.success(t('knowledge.folderCreated'))
          }}
        />
        <ConfirmDialog
          open={deleteOpen}
          onOpenChange={setDeleteOpen}
          title={t('knowledge.deleteConfirmTitle')}
          description={t('knowledge.deleteConfirmBody', { path: currentFilePath ?? '' })}
          confirmLabel={t('common.delete')}
          destructive
          onConfirm={performDelete}
        />
      </>
    )
  }

  return (
    <>
      <div className={sectionGap}>
        <button
          type="button"
          onClick={() => openHome()}
          className={cn(itemBase, mode === 'home' ? itemActive : itemInactive)}
        >
          <LayoutGrid className="h-4 w-4" />
          <span>{t('knowledge.homeTitle')}</span>
        </button>
        <button
          type="button"
          onClick={() => openChat()}
          className={cn(itemBase, mode === 'chat' ? itemActive : itemInactive)}
        >
          <MessageSquare className="h-4 w-4" />
          <span>{t('knowledge.chatTitle')}</span>
        </button>
        <button
          type="button"
          onClick={handleOpenJournal}
          disabled={journalToday.isLoading}
          className={cn(itemBase, itemInactive, 'disabled:opacity-50')}
        >
          <BookOpen className="h-4 w-4" />
          <span>{t('knowledge.toJournal')}</span>
        </button>
        <NavItemLink
          item={{
            href: '/knowledge/graph',
            icon: <Network className="h-4 w-4" />,
            labelKey: 'knowledge.linkGraphTitle',
          }}
          currentPath={currentPath}
          collapsed={collapsed}
        />
      </div>

      <div className={sectionSeparator} />

      <p className={sectionHeader}>{t('knowledge.files')}</p>
      <div className="flex-1 overflow-y-auto">
        {isLoading ? (
          <div className="px-4 py-2 text-xs text-sidebar-foreground/50">
            {t('knowledge.loading')}
          </div>
        ) : tree ? (
          <FileTree
            nodes={tree}
            currentPath={currentFilePath}
            onFileSelect={openFile}
            onRename={renameFile}
            onMove={(from, to) => {
              moveFile
                .mutateAsync({ from, to })
                .then(() => openFile(to))
                .catch((err) => console.error('tree move failed', err))
            }}
            onContextMenu={(node) => {
              if (node.is_dir) handleNewFile(node.path)
            }}
          />
        ) : null}
      </div>

      <div className={sectionSeparator} />

      <div className="flex items-center gap-0.5 pt-2 px-1">
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 hover:bg-sidebar-accent/50"
          onClick={() => handleNewFile()}
          title={t('knowledge.newFileShortcut')}
        >
          <FilePlus className="h-4 w-4" />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 hover:bg-sidebar-accent/50"
          onClick={handleNewFolder}
          title={t('knowledge.newFolderShortcut')}
        >
          <FolderPlus className="h-4 w-4" />
        </Button>
        {currentFilePath && (
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7 text-destructive hover:bg-sidebar-accent/50"
            onClick={handleDelete}
            title={t('knowledge.deleteCurrentFile')}
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        )}
        <div className="flex-1" />
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 hover:bg-sidebar-accent/50"
          onClick={() => setHabitsOpen(true)}
          title={t('knowledge.habitsTitle')}
        >
          <Activity className="h-4 w-4" />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 hover:bg-sidebar-accent/50"
          onClick={() => setSettingsOpen(true)}
          title={t('knowledge.knowledgeSettings')}
        >
          <Settings className="h-4 w-4" />
        </Button>
        <span className="text-2xs text-sidebar-foreground/50 font-mono rounded border bg-sidebar/50 px-1.5 py-0.5">
          ⌘K
        </span>
      </div>
      <HabitsDialog open={habitsOpen} onOpenChange={setHabitsOpen} />
      <KnowledgeSettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
      <NewFolderDialog
        open={newFolderOpen}
        onOpenChange={setNewFolderOpen}
        onConfirm={async (folderName) => {
          await writeFile.mutateAsync({ path: `${folderName}/.keep`, content: '' })
          toast.success(t('knowledge.folderCreated'))
        }}
      />
      <ConfirmDialog
        open={deleteOpen}
        onOpenChange={setDeleteOpen}
        title={t('knowledge.deleteConfirmTitle')}
        description={t('knowledge.deleteConfirmBody', { path: currentFilePath ?? '' })}
        confirmLabel={t('common.delete')}
        destructive
        onConfirm={performDelete}
      />
    </>
  )
}

// ── NavItemLink (shared) ───────────────────────────────────────

function NavItemLink({
  item,
  currentPath,
  collapsed,
}: {
  item: NavItem
  currentPath: string
  collapsed: boolean
}) {
  const { t } = useTranslation()
  const isActive =
    currentPath === item.href || (item.href !== '/' && currentPath.startsWith(item.href))
  const showBadge = item.badge != null && item.badge > 0

  const link = (
    <Link
      to={item.href}
      className={cn(itemBase, isActive ? itemActive : itemInactive, collapsed && 'justify-center')}
    >
      {item.icon}
      {!collapsed && <span>{t(item.labelKey)}</span>}
      {!collapsed && showBadge && (
        <span className="ml-auto flex h-4 min-w-4 items-center justify-center rounded-full bg-warning px-1 text-2xs font-bold text-white animate-scale-in">
          {item.badge}
        </span>
      )}
    </Link>
  )

  return collapsed ? (
    <Tooltip key={item.href}>
      <TooltipTrigger asChild>{link}</TooltipTrigger>
      <TooltipContent side="right">
        {`${t(item.labelKey)}${item.badge ? ` (${item.badge})` : ''}`}
      </TooltipContent>
    </Tooltip>
  ) : (
    <React.Fragment key={item.href}>{link}</React.Fragment>
  )
}
