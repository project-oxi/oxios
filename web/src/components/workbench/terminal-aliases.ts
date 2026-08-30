// terminal-aliases — the tool-name matcher shared by the terminal stage and
// the §7.2 emergence observer. Case-insensitive substring match: a tool whose
// name contains any alias (bash, exec, terminal, command, shell) is a
// terminal-class call.

export const TERMINAL_TOOL_ALIASES = ['bash', 'exec', 'terminal', 'command', 'shell'] as const

export function isTerminalToolName(name: string | undefined): boolean {
  if (!name) return false
  const lower = name.toLowerCase()
  return TERMINAL_TOOL_ALIASES.some((alias) => lower.includes(alias))
}
