/**
 * Chat brain binding resolution order: the explicit session picker choice
 * wins; otherwise the project default, then the global default. An empty
 * global default means "no default" → null (unconnected turn).
 *
 * Pure helper so the store payload (`sendMessage`) and the picker display
 * resolve identically without importing component modules into the store.
 */
export function resolveBrainSpace(
  explicit: string | null,
  projectDefault: string | null | undefined,
  globalDefault: string,
): string | null {
  return explicit ?? projectDefault ?? (globalDefault || null)
}
