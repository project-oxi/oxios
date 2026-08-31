/**
 * Chat slash command registry (2026-08-31 design).
 *
 * Single source for the input menu, the send-path parser, and `/help`.
 * Execution classes:
 * - `client` — executed locally in the chat page (no LLM turn, no WS traffic)
 * - `server`  — sent verbatim; the WS layer intercepts `/compact` / `/persona`,
 *   executes them inline and replies with a `command_result` frame
 * - `turn`    — sent verbatim; the WS layer strips the prefix, attaches a
 *   `TurnCommand` directive to the turn's system prompt, and the agent runs
 *   with it (`/search` forces grounded web search; `/skill` inlines SKILL.md)
 */

export interface SlashCommandDef {
  /** Registry id; also the i18n key suffix (`slash.<id>.description`). */
  id: string
  /** Canonical label including the leading slash. */
  label: string
  icon: string
  /** Usage placeholder shown in the menu. Omitted for arg-less commands. */
  argHint?: string
  kind: 'client' | 'server' | 'turn'
}

export const SLASH_COMMANDS: SlashCommandDef[] = [
  { id: 'compact', label: '/compact', icon: '📝', kind: 'server' },
  { id: 'clear', label: '/clear', icon: '🆕', kind: 'client' },
  { id: 'search', label: '/search', icon: '🌐', argHint: '<query>', kind: 'turn' },
  { id: 'skill', label: '/skill', icon: '⚡', argHint: '<name> [args]', kind: 'turn' },
  { id: 'persona', label: '/persona', icon: '🎭', argHint: '[id]', kind: 'server' },
  { id: 'model', label: '/model', icon: '🧠', argHint: '[provider/model]', kind: 'client' },
  { id: 'export', label: '/export', icon: '📤', kind: 'client' },
  { id: 'help', label: '/help', icon: '❓', kind: 'client' },
]

/** A leading `/command` parse of the composer text. */
export interface ParsedSlashInput {
  cmd: SlashCommandDef
  /** For `skill`: the skill name (first word after the command). */
  name?: string
  /** Text after the command word (and the skill name, for `skill`), trimmed.
   *  Empty for arg-less invocations. */
  args: string
}

/**
 * Parse a leading `/cmd` prefix against the registry. Returns `null` for
 * plain messages and unknown commands. For `skill`, the first word names
 * the skill and is lifted into `name` — mirroring the server-side
 * `SlashCommand::parse` split — so `args` is only the skill arguments.
 */
export function parseSlashInput(text: string): ParsedSlashInput | null {
  const match = text.match(/^\/(\w+)(?:\s+([\s\S]*))?$/)
  if (!match) return null
  const cmd = SLASH_COMMANDS.find((c) => c.id === match[1])
  if (!cmd) return null
  let rest = (match[2] ?? '').trim()
  if (cmd.id === 'skill' && rest) {
    const split = rest.search(/\s/)
    const name = split < 0 ? rest : rest.slice(0, split)
    rest = split < 0 ? '' : rest.slice(split).trim()
    return { cmd, name, args: rest }
  }
  return { cmd, args: rest }
}
