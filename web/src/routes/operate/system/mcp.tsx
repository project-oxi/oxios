import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { Plus } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { AddServerDialog } from '@/components/mcp/add-server-dialog'
import { ServerList } from '@/components/mcp/server-list'
import { ToolList } from '@/components/mcp/tool-list'
import { ToolTester } from '@/components/mcp/tool-tester'
import { PageHeader } from '@/components/shared/page-header'
import { Button } from '@/components/ui/button'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'

export const Route = createFileRoute('/operate/system/mcp')({
  component: McpPage,
  validateSearch: (search: Record<string, unknown>) => ({
    tab: (search.tab as string) || undefined,
  }),
})

function McpPage() {
  const { t } = useTranslation()
  const [addDialogOpen, setAddDialogOpen] = useState(false)
  const navigate = useNavigate({ from: Route.id })
  const { tab: tabParam } = Route.useSearch()
  const tab = tabParam === 'tools' || tabParam === 'test' ? tabParam : 'servers'

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('mcp.title')}
        subtitle={t('mcp.subtitle')}
        actions={
          tab === 'servers' ? (
            <Button onClick={() => setAddDialogOpen(true)}>
              <Plus className="h-4 w-4" /> {t('mcp.addServer')}
            </Button>
          ) : undefined
        }
      />

      <Tabs
        value={tab}
        onValueChange={(v) =>
          navigate({
            search: (prev) => ({ ...prev, tab: v === 'servers' ? undefined : v }),
            replace: true,
          })
        }
      >
        <TabsList>
          <TabsTrigger value="servers">{t('mcp.servers')}</TabsTrigger>
          <TabsTrigger value="tools">{t('mcp.tools')}</TabsTrigger>
          <TabsTrigger value="test">{t('mcp.test')}</TabsTrigger>
        </TabsList>

        <TabsContent value="servers">
          <ServerList />
        </TabsContent>

        <TabsContent value="tools">
          <ToolList />
        </TabsContent>

        <TabsContent value="test">
          <ToolTester />
        </TabsContent>
      </Tabs>

      <AddServerDialog open={addDialogOpen} onOpenChange={setAddDialogOpen} />
    </div>
  )
}
