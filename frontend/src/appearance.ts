/** Structural encoding and one typed-array fill per appearance change. Explicit mappings come from core. */

import type { PropertyStat } from './generated/PropertyStat'

/** RGBA in 0..1, which is what cosmos.gl takes — not 0..255. */
export type Rgba = [number, number, number, number]

/** Anything the chosen property does not cover. */
export const UNSET_COLOR: Rgba = [0.42, 0.47, 0.55, 0.75]

/**
 * One hue per node type, for instance nodes with no colour-by chosen.
 *
 * **Ported to Rust** as `TYPE_HUES` / `type_hue` in
 * `crates/kglite-visual-core/src/render/encoding.rs`, entry for entry.
 *
 * Every instance node used to be drawn in one blue, which on a mixed
 * neighbourhood — a wellbore with its licences, its cores and its logs — said
 * "these are all the same kind of thing" about a view whose whole content is
 * that they are not. Separable at small size against both the dark and the
 * light ground, and ordered so adjacent indices are far apart in hue, because
 * the assignment is by hash and neighbours in the table are what a two-type
 * view is most likely to draw.
 */
const TYPE_HUES: Rgba[] = [
  [0.36, 0.68, 0.98, 0.9],
  [0.98, 0.62, 0.3, 0.9],
  [0.42, 0.82, 0.52, 0.9],
  [0.85, 0.5, 0.92, 0.9],
  [0.98, 0.8, 0.34, 0.9],
  [0.4, 0.83, 0.85, 0.9],
  [0.96, 0.51, 0.6, 0.9],
  [0.62, 0.74, 0.42, 0.9],
  [0.68, 0.62, 0.96, 0.9],
  [0.92, 0.68, 0.52, 0.9],
]

/** An instance node whose type is unknown. */
export const INSTANCE_COLOR: Rgba = [0.55, 0.7, 0.9, 0.85]

/**
 * The hue a type name maps to.
 *
 * FNV-1a over the name, not the type's position in any list: the same type
 * must get the same colour in every view forever, and a position-derived index
 * would repaint everything the day a query returned its rows in a different
 * order.
 */
export function typeHue(nodeType: string | null): Rgba {
  if (nodeType === null || nodeType === '') return INSTANCE_COLOR
  let hash = 0x811c9dc5
  for (let i = 0; i < nodeType.length; i += 1) {
    hash = (hash ^ nodeType.charCodeAt(i)) >>> 0
    hash = Math.imul(hash, 0x01000193) >>> 0
  }
  return TYPE_HUES[hash % TYPE_HUES.length] as Rgba
}

/**
 * Radius range for a meta-graph type node, in graph units before zoom.
 *
 * **Ported to Rust** as `TYPE_MIN_PX` / `TYPE_MAX_PX` in
 * `crates/kglite-visual-core/src/render/encoding.rs`, which the headless render
 * draws from (plan D13). Moving a number here without moving it there makes the
 * exported image and the app show two different graphs; the golden SVG baseline
 * (`make check-render-baseline`) goes red on the Rust half of that, and nothing
 * catches the TypeScript half but this comment.
 *
 * The floor is the smallest circle that still reads as a disc rather than a
 * speck and still has room for the hover ring the interaction layer draws
 * around it. The ceiling is set by the label chips: a label sits *below* its
 * circle, so a radius much past this puts the biggest type's name a third of
 * the way down the screen from the thing it names, and its circle covers its
 * neighbours' labels.
 */
const TYPE_MIN_PX = 6
const TYPE_MAX_PX = 36

/**
 * Scale applied to a supporting type's radius.
 *
 * **Ported to Rust** as `SUPPORTING_SCALE` in
 * `crates/kglite-visual-core/src/render/encoding.rs`.
 *
 * Not a separate ramp — the same ramp, one step quieter — so a large
 * supporting type still reads as larger than a small one. On the graph that
 * motivated this, 63 of 98 types are supporting, and drawing all 98 at equal
 * weight is most of why the entry screen read as a cloud of dots.
 */
const SUPPORTING_SCALE = 0.6

/**
 * Radius for a type node with `count` members, on a graph whose largest type
 * has `max`.
 *
 * **Ported to Rust** as `type_radius` in
 * `crates/kglite-visual-core/src/render/encoding.rs`.
 *
 * **Log, not a root.** Type populations are log-uniform in practice — on the
 * graph this was tuned against the deciles run 3, 23, 118, 1 051, 4 249,
 * 11 000, 102 420 — so a log ramp spreads them evenly across the pixel range
 * and every decile is a visibly different size. The fourth-root ramp this
 * replaces put the bottom three quartiles of that graph inside 10–16 px of a
 * 8–34 px range: the small types were not merely small, they were
 * indistinguishable from each other, which is the "lot of dots" the entry
 * screen was reported as. Linear is worse again: at a 34 000× spread it puts
 * 97 of 98 types on the floor.
 */
export function typeRadius(count: number, max: number, supporting: boolean): number {
  const ceiling = Math.max(max, 1)
  const ramp = Math.log1p(Math.max(count, 0)) / Math.log1p(ceiling)
  const radius = TYPE_MIN_PX + (TYPE_MAX_PX - TYPE_MIN_PX) * Math.min(ramp, 1)
  return supporting ? radius * SUPPORTING_SCALE : radius
}

/**
 * Width range for a meta-graph link, in the same units.
 *
 * **Ported to Rust** as `LINK_MIN_PX` / `LINK_MAX_PX` in
 * `crates/kglite-visual-core/src/render/encoding.rs`.
 */
const LINK_MIN_PX = 0.5
const LINK_MAX_PX = 5

/**
 * Width for a meta-graph link carrying `count` edges.
 *
 * **Ported to Rust** as `link_width` in
 * `crates/kglite-visual-core/src/render/encoding.rs`.
 *
 * Same argument as {@link typeRadius}, over the same spread: the relationship
 * counts on the tuning graph run from 1 to 102 420. A count of 0 means the
 * server had no number to give — which after the load-time connectivity repair
 * only happens for an edge whose endpoint type does not resolve — and gets the
 * floor rather than a fabricated width.
 */
export function linkWidth(count: number, max: number): number {
  if (count <= 0) return LINK_MIN_PX
  const ceiling = Math.max(max, 1)
  const ramp = Math.log1p(count) / Math.log1p(ceiling)
  return LINK_MIN_PX + (LINK_MAX_PX - LINK_MIN_PX) * Math.min(ramp, 1)
}

/** A search or query hit. Overrides whatever the colour-by chose. */
export const HIGHLIGHT_COLOR: Rgba = [1.0, 0.86, 0.2, 1.0]

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
