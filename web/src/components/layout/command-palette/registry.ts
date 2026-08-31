import type { CommandProvider, PaletteItem, QueryContext } from './types'

/**
 * The federated command registry.
 *
 * Providers register once at module load. On each keystroke the host builds a
 * `QueryContext` and calls `resolve`, which asks every surface-active provider
 * for its matching items. The host then ranks (see `ranker.ts`) and renders.
 *
 * cmdk filtering is OFF (design §6 prerequisite / D7 fix), so the items
 * returned here — in ranked order — are exactly what the user sees and
 * navigates with the keyboard.
 */
export class CommandRegistry {
  private providers: CommandProvider[] = []

  register(p: CommandProvider): void {
    if (!this.providers.some((x) => x.id === p.id)) this.providers.push(p)
  }

  /** All registered providers (host may use this to build empty-state lists). */
  list(): CommandProvider[] {
    return [...this.providers]
  }

  resolve(ctx: QueryContext): PaletteItem[] {
    const out: PaletteItem[] = []
    for (const p of this.providers) {
      if (p.surfaces && p.surfaces.length > 0 && !p.surfaces.includes(ctx.surface)) continue
      out.push(...p.resolve(ctx))
    }
    return out
  }
}
