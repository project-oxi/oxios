// Bash render — terminal output with command display
import { Terminal } from 'lucide-react'
import type { ToolRenderComponent } from './registry'

export const BashRender: ToolRenderComponent = ({ args, result, isRunning, durationMs }) => {
  const command = (args?.command ?? args?.cmd ?? '') as string

  return (
    <div className="space-y-2 text-sm">
      {/* Command display */}
      <div className="flex items-center gap-2 text-xs">
        <Terminal className="w-3.5 h-3.5 text-muted-foreground" />
        <code className="font-mono bg-muted px-1.5 py-0.5 rounded text-muted-foreground">
          {command.length > 100 ? `${command.slice(0, 100)}...` : command}
        </code>
        {durationMs != null && (
          <span className="text-muted-foreground/60 ml-auto">{formatDuration(durationMs)}</span>
        )}
      </div>

      {/* Output */}
      {isRunning ? (
        <div className="flex items-center gap-2 text-muted-foreground">
          <span className="inline-block w-2 h-2 rounded-full bg-status-warning animate-pulse" />
          Running...
        </div>
      ) : result != null ? (
        <pre className="p-3 rounded bg-surface-sunken text-status-success-on-surface text-xs overflow-x-auto max-h-96 whitespace-pre-wrap font-mono leading-relaxed">
          {typeof result === 'string' ? result.slice(0, 10000) : JSON.stringify(result, null, 2)}
        </pre>
      ) : null}
    </div>
  )
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  return `${(ms / 1000).toFixed(1)}s`
}
