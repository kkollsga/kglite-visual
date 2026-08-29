/**
 * Compiled appearance getters (plan D7 + D12).
 *
 * **One typed-array fill per change, never a callback per point.** The
 * comparable-repo study found the same failure in every slow graph UI: a
 * `getColor(node)` reducer runs V times per frame and turns a dropdown into a
 * freeze. Here a colour-by or size-by choice is *compiled* once into a function
 * of one value, and then a single loop fills one `Float32Array` that goes
 * straight to `setPointColors` / `setPointSizes`.
 *
 * The candidates come from the server's property statistics, and a stat kglite
 * marked `approx` is labelled as such wherever it is offered — a distinct-value
 * count that is really a lower bound is a wrong number, not a smaller one.
 */

import type { PropertyStat } from './generated/PropertyStat'

/** RGBA in 0..1, which is what cosmos.gl takes — not 0..255. */
export type Rgba = [number, number, number, number]

/**
 * Categorical palette. Eight hues, deliberately: past that a legend stops being
 * readable and the honest answer is a different channel, not more colours.
 */
const CATEGORY_COLORS: Rgba[] = [
  [0.38, 0.65, 0.98, 0.95],
  [0.98, 0.55, 0.28, 0.95],
  [0.35, 0.82, 0.53, 0.95],
  [0.85, 0.45, 0.85, 0.95],
  [0.95, 0.82, 0.35, 0.95],
  [0.45, 0.85, 0.85, 0.95],
  [0.92, 0.44, 0.52, 0.95],
  [0.65, 0.60, 0.92, 0.95],
]

/** Anything the chosen property does not cover. */
export const UNSET_COLOR: Rgba = [0.42, 0.47, 0.55, 0.75]

/** A search or query hit. Overrides whatever the colour-by chose. */
export const HIGHLIGHT_COLOR: Rgba = [1.0, 0.86, 0.2, 1.0]

/**
 * Compile a colour function for a categorical property.
 *
 * Returns `null` when the stat cannot drive a palette — sampled or capped
 * distinct values, or more of them than the palette has. Colouring by a value
 * *set* that is a lower bound leaves some nodes silently uncoloured, which
 * reads as missing data rather than as a missing colour.
 */
export function compileCategoricalColor(
  stat: PropertyStat,
): ((value: unknown) => Rgba) | null {
  if (stat.approx || stat.values.length === 0) return null
  const index = new Map<string, number>()
  for (const [i, value] of stat.values.entries()) index.set(JSON.stringify(value), i)
  return (value: unknown) => {
    const at = index.get(JSON.stringify(value ?? null))
    if (at === undefined) return UNSET_COLOR
    return CATEGORY_COLORS[at % CATEGORY_COLORS.length] ?? UNSET_COLOR
  }
}

/**
 * Compile a radius function for a numeric property.
 *
 * Fourth root, matching the meta-graph's own size ramp: real graph properties
 * routinely span four orders of magnitude, and both linear and square-root
 * scales make the small end invisible at that spread.
 */
export function compileNumericSize(
  values: number[],
  minPx = 4,
  maxPx = 22,
): (value: unknown) => number {
  const finite = values.filter((v) => Number.isFinite(v))
  const low = finite.length > 0 ? Math.min(...finite) : 0
  const high = finite.length > 0 ? Math.max(...finite) : 1
  const span = high - low
  return (value: unknown) => {
    const n = typeof value === 'number' ? value : Number(value)
    if (!Number.isFinite(n) || span <= 0) return minPx
    return minPx + (maxPx - minPx) * Math.pow((n - low) / span, 0.25)
  }
}

/**
 * Fill a whole colour array in one pass.
 *
 * `base` is what a slot would be without any highlight; `highlighted` wins,
 * because a search hit the user just asked for must be findable whatever the
 * colour-by says.
 */
export function fillColors(
  slotCount: number,
  base: (slot: number) => Rgba,
  highlighted: ReadonlySet<number>,
): Float32Array {
  const colors = new Float32Array(slotCount * 4)
  for (let slot = 0; slot < slotCount; slot += 1) {
    const rgba = highlighted.has(slot) ? HIGHLIGHT_COLOR : base(slot)
    colors[slot * 4] = rgba[0]
    colors[slot * 4 + 1] = rgba[1]
    colors[slot * 4 + 2] = rgba[2]
    colors[slot * 4 + 3] = rgba[3]
  }
  return colors
}

/**
 * The label a stat gets in a dropdown.
 *
 * The word "approximate" is verbatim and not negotiable (Phase 0 finding):
 * kglite marks a stat `approx` when it sampled the population or capped the
 * distinct-value set, and presenting either as exact is the failure the flag
 * exists to prevent.
 */
export function statLabel(stat: PropertyStat): string {
  const unique = stat.approx ? `${stat.unique}+ (approximate)` : `${stat.unique}`
  return `${stat.name} — ${unique} distinct, ${stat.non_null} set`
}
