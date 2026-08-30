// ActionTool render — generic renderer for Oxios action-based kernel tools.
//
// Many kernel tools (knowledge, persona, cron, budget, security, project,
// resource, marketplace, skill_forge) use an `action` parameter to
// dispatch operations. This render shows the action name + key args inline,
// then the result in a compact formatted block.

import type { ToolRenderComponent } from './registry'

export const ActionToolRender: ToolRenderComponent = ({ args, result, isRunning, toolName }) => {
  const action = (args?.action ?? args?.op ?? '') as string
  const resultStr = typeof result === 'string' ? result : JSON.stringify(result, null, 2)

  return (
    <div className="space-y-2 text-sm">
      <div className="flex items-center gap-2 text-xs">
        <span className="font-mono bg-muted px-1.5 py-0.5 rounded">{toolName}</span>
        {action && <span className="text-primary font-medium">{action}</span>}
        {isRunning && (
          <span className="inline-block w-2 h-2 rounded-full bg-status-warning animate-pulse" />
        )}
      </div>
      {resultStr && !isRunning && (
        <pre className="p-2 rounded bg-muted text-xs overflow-x-auto max-h-64 whitespace-pre-wrap">
          {resultStr.length > 2000 ? `${resultStr.slice(0, 2000)}\n...` : resultStr}
        </pre>
      )}
    </div>
  )
}
