// tool-renders/index.ts — registers all builtin tool renders + inspectors.
//
// Aliases: a single render often covers multiple tool names. We register
// under both Oxios kernel names and oxicode-sdk builtins so lookups hit
// regardless of which layer emitted the call.

import { A2aDelegateRender, A2aQueryRender, A2aSendRender } from './A2a'
import { ActionToolRender } from './ActionTool'
import { BashRender } from './Bash'
import { BrowseRender } from './Browse'
import { CalendarRender } from './Calendar'
import { FileEditRender } from './FileEdit'
import { FileReadRender } from './FileRead'
import { GlobRender } from './Glob'
import { GrepRender } from './Grep'
import { ImageGenerationRender } from './ImageGeneration'
import { IssueRender } from './Issue'
import { ListFilesRender } from './ListFiles'
import { registerToolRender } from './registry'
import { SendEmailRender } from './SendEmail'
import { TodoRender } from './Todo'
import { WebFetchRender } from './WebFetch'
import { WebSearchRender } from './WebSearch'

// ── File operations ──
registerToolRender('read_file', FileReadRender)
registerToolRender('readFile', FileReadRender)
registerToolRender('read', FileReadRender)

registerToolRender('write_file', FileEditRender)
registerToolRender('writeFile', FileEditRender)
registerToolRender('write', FileEditRender)

registerToolRender('edit_file', FileEditRender)
registerToolRender('editFile', FileEditRender)
registerToolRender('edit', FileEditRender)

// ── Shell ──
registerToolRender('exec', BashRender)
registerToolRender('bash', BashRender)
registerToolRender('run_command', BashRender)
registerToolRender('shell', BashRender)

// ── Search/listing ──
registerToolRender('glob', GlobRender)
registerToolRender('find_files', GlobRender)
registerToolRender('grep', GrepRender)
registerToolRender('search_files', GrepRender)
registerToolRender('list_files', ListFilesRender)
registerToolRender('listFiles', ListFilesRender)
registerToolRender('ls', ListFilesRender)

// ── Web ──
registerToolRender('web_search', WebSearchRender)
registerToolRender('webSearch', WebSearchRender)
registerToolRender('get_search_results', WebSearchRender)
registerToolRender('web_fetch', WebFetchRender)
registerToolRender('webFetch', WebFetchRender)
registerToolRender('fetch', WebFetchRender)

// Browse (headless browser page reading)
registerToolRender('browse', BrowseRender)
registerToolRender('browse_extract', BrowseRender)
registerToolRender('browse_session', BrowseRender)
registerToolRender('browse_script', BrowseRender)

// ── Communication ──
registerToolRender('send_email', SendEmailRender)
registerToolRender('sendEmail', SendEmailRender)

// ── Image generation ──
registerToolRender('image_generation', ImageGenerationRender)
registerToolRender('imageGeneration', ImageGenerationRender)

// ── A2A (agent-to-agent) ──
registerToolRender('a2a_delegate', A2aDelegateRender)
registerToolRender('a2aDelegate', A2aDelegateRender)
registerToolRender('a2a_send', A2aSendRender)
registerToolRender('a2aSend', A2aSendRender)
registerToolRender('a2a_query', A2aQueryRender)
registerToolRender('a2aQuery', A2aQueryRender)

// ── Calendar ──
registerToolRender('calendar', CalendarRender)

// ── Planning + issue tracking (structured payloads on tool_end.results) ──
registerToolRender('todo', TodoRender)
registerToolRender('issue', IssueRender)

// ── Action-based kernel tools (generic renderer) ──
registerToolRender('knowledge', ActionToolRender)
registerToolRender('persona', ActionToolRender)
registerToolRender('cron', ActionToolRender)
registerToolRender('budget', ActionToolRender)
registerToolRender('security', ActionToolRender)
registerToolRender('project', ActionToolRender)
registerToolRender('resource', ActionToolRender)
registerToolRender('marketplace', ActionToolRender)
registerToolRender('skill_forge', ActionToolRender)
registerToolRender('kernel_agent', ActionToolRender)

export type {
  ToolInspectorComponent,
  ToolInspectorProps,
  ToolInterventionComponent,
  ToolInterventionProps,
  ToolRenderComponent,
  ToolRenderProps,
  ToolStreamingComponent,
  ToolStreamingProps,
} from './registry'
export {
  DefaultToolRender,
  getToolInspector,
  getToolIntervention,
  getToolRender,
  getToolStreaming,
  registerToolInspector,
  registerToolIntervention,
  registerToolRender,
  registerToolStreaming,
} from './registry'
