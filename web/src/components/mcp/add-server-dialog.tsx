import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import { useMcpRegisterServer } from '@/hooks/use-mcp'

// Parse "KEY=VALUE" lines (one per line) into an env record. Blank lines
// and lines without `=` are ignored.
function parseEnv(text: string): Record<string, string> {
  const env: Record<string, string> = {}
  for (const line of text.split('\n')) {
    const trimmed = line.trim()
    if (!trimmed) continue
    const eq = trimmed.indexOf('=')
    if (eq <= 0) continue
    const key = trimmed.slice(0, eq).trim()
    const value = trimmed.slice(eq + 1).trim()
    if (key) env[key] = value
  }
  return env
}

interface AddServerDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function AddServerDialog({ open, onOpenChange }: AddServerDialogProps) {
  const { t } = useTranslation()
  const [name, setName] = useState('')
  const [command, setCommand] = useState('')
  const [args, setArgs] = useState('')
  const [env, setEnv] = useState('')

  const registerServer = useMcpRegisterServer()

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    if (!name.trim() || !command.trim()) return

    const parsedArgs = args.trim()
      ? args
          .split(',')
          .map((a) => a.trim())
          .filter(Boolean)
      : []

    registerServer.mutate(
      { name: name.trim(), command: command.trim(), args: parsedArgs, env: parseEnv(env) },
      {
        onSuccess: () => {
          setName('')
          setCommand('')
          setArgs('')
          setEnv('')
          onOpenChange(false)
        },
      },
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t('mcp.addServer')}</DialogTitle>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="mcp-name">{t('mcp.serverName')}</Label>
            <Input
              id="mcp-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t('mcp.serverNamePlaceholder')}
              required
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="mcp-command">{t('mcp.command')}</Label>
            <Input
              id="mcp-command"
              value={command}
              onChange={(e) => setCommand(e.target.value)}
              placeholder={t('mcp.commandPlaceholder')}
              required
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="mcp-args">{t('mcp.args')}</Label>
            <Input
              id="mcp-args"
              value={args}
              onChange={(e) => setArgs(e.target.value)}
              placeholder={t('mcp.argsPlaceholder')}
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="mcp-env">{t('mcp.env')}</Label>
            <Textarea
              id="mcp-env"
              value={env}
              onChange={(e) => setEnv(e.target.value)}
              placeholder={t('mcp.envPlaceholder')}
              rows={3}
            />
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {t('common.cancel')}
            </Button>
            <Button
              type="submit"
              disabled={!name.trim() || !command.trim() || registerServer.isPending}
            >
              {registerServer.isPending ? t('common.loading') : t('mcp.register')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
