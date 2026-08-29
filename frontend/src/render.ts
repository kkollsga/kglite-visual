/**
 * The cosmos.gl renderer, in one of two layout modes.
 *
 * **`force` is what a user gets.** The server's lattice positions are a *seed*
 * — a deterministic, cheap starting point that makes the first paint stable —
 * and the GPU force simulation then runs and turns them into a picture of the
 * graph's structure. A lattice of evenly spaced dots is not that picture: it
 * says nothing about which types are connected to which, which is the entire
 * question the meta-graph entry screen exists to answer.
 *
 * **`deterministic` is what a test gets** (`?deterministic=1`). It is D2's
 * fixture mode, unchanged: the simulation never runs, so the positions on the
 * GPU are exactly the ones the server computed and `positionsHash` describes
 * something. The e2e suite and the bench harness both pass the flag, and
 * `window.__kglv.layoutMode` reports which mode is live — a position hash
 * asserted in force mode would be asserting on GPU float scheduling.
 *
 * The constructor values that are *not* mode-dependent, and why:
 *
 * - `rescalePositions: false` — its default silently rewrites the coordinates
 *   the server just computed, which would make the committed positions
 *   baseline describe nothing. (cosmos.gl's `undefined` default is itself
 *   mode-dependent — it rescales only when the simulation is off — so leaving
 *   it out would make the two modes disagree about the seed.)
 * - `transitionDuration: 0` — animation is opted into per call site, and in
 *   force mode it is load-bearing rather than cosmetic: a position transition
 *   with a positive duration *auto-pauses a running simulation and leaves it
 *   paused* (cosmos.gl's own note on `setPointPositions`). Zero duration means
 *   there is no transition to pause it.
 * - `fitViewOnInit: false` + a fixed `initialZoomLevel` — a fit depends on the
 *   viewport, so it would make the same data render differently at two window
 *   sizes. In force mode the layout stops being a function of the data alone
 *   once it settles, so a settle-time fit is added back there and only there.
 *
 * Uploads go through {@link Surface.upload}: whole typed arrays, one call each,
 * never a per-point callback (plan D7).
 */

import { Graph } from '@cosmos.gl/graph'

import type { SlotView } from './view'

/** Where the positions on the GPU come from. */
export type LayoutMode = 'force' | 'deterministic'

/**
 * Read the layout mode out of the page's query string.
 *
 * Opt-*in* to determinism, not out of it: the default has to be the mode that
 * is useful on a real graph, and a test that forgets the flag fails loudly on
 * `layoutMode` rather than passing by accident.
 */
export function layoutModeFromSearch(search: string): LayoutMode {
  return new URLSearchParams(search).get('deterministic') === '1'
    ? 'deterministic'
    : 'force'
}

/**
 * Screen span, in CSS pixels, the graph is scaled to occupy.
 *
 * cosmos.gl's zoom is a plain linear factor — screen pixels = graph units ×
 * zoom (measured, not assumed) — so a *fixed* zoom cannot serve both a 5-type
 * meta-graph and a 1000-node expansion of it: the spiral layout's extent grows
 * with the slot count, and one number leaves the first invisible or the second
 * off screen. The zoom below is therefore derived from the DATA and from this
 * constant, never from the viewport: same payload, same zoom, at any window
 * size. That is what `fitViewOnInit: false` is protecting, and a viewport fit
 * would break it.
 */
const TARGET_SPAN_PX = 620

/**
 * Point sampling distance for the label layer, in screen pixels.
 *
 * cosmos.gl thins `getSampledPoints()` to one point per cell of this size. The
 * default is tuned for a million instance nodes; at meta-graph scale it drops
 * type nodes that must all be labelled, so it is narrowed to just under a
 * label's own height — below which the overlay's own grid takes over anyway.
 */
const POINT_SAMPLING_DISTANCE_PX = 24

/**
 * cosmos.gl's coordinate space is `[0, spaceSize]` on both axes, not centred
 * on the origin — its default `spaceSize` is 4096, and its GPU sampling pass
 * maps positions through that range before it fills the sampled-points grid.
 * Server positions are centred on 0, so a point at (-30, 130) lands outside the
 * space: it still draws, but `getSampledPoints()` never returns it and it
 * therefore never gets a label. Measured, not assumed — the overlay came back
 * empty until the offset was applied.
 */
const SPACE_SIZE = 4096

/**
 * Force parameters, chosen by iterating on the real thing: the 98-type /
 * 124-relationship meta-graph of `sodir_graph.kgl` (546 850 nodes, 765 373
 * edges), driven headless at 1280×800 and screenshotted after each settle,
 * until it read as a graph instead of a blob or a scatter.
 *
 * cosmos.gl's defaults are tuned for tens of thousands of instance nodes and
 * are wrong here in both directions at once: a meta-graph is small, its hub
 * types carry twenty-odd relationships each, and its node radii span four
 * orders of magnitude — so the picture has to be sparse enough that one type's
 * circle does not swallow its neighbours' labels.
 *
 * Each departure from the default, and the picture that bought it:
 *
 * - `simulationLinkDistance: 150` (default 10) — at the default every
 *   connected pair sat inside one node's radius and the schema collapsed into
 *   a single unreadable knot roughly 80 px across.
 * - `simulationLinkSpring: 0.15` (default 1) — with a stiff spring the hubs
 *   dragged the whole periphery back into that knot; slackening it is what
 *   lets the core spread while staying visibly connected.
 * - `simulationRepulsion: 4` (default 1) — the separation the labels need.
 *   Tried 5 with gravity 0.25: the picture became an evenly filled disc, which
 *   is a different way of showing no structure.
 * - `simulationGravity: 0.12` (default 0.25) — the counterweight. This graph
 *   has isolated types with no edges at all, and nothing but gravity brings
 *   those back; at 0.05 they drifted off the visible space, at 0.25 the whole
 *   layout compressed back toward uniform.
 * - `simulationDecay: 300` (default 5000) — the parameter is *ticks to
 *   settle*, not a rate (`alphaDecay = 1 - 1e-3^(1/decay)`), so the default is
 *   5 000 frames: over a minute of an entry screen visibly crawling, which a
 *   user reads as broken. 300 ticks settles in 7.6 s under headless
 *   SwiftShader — and, measured against 600, reaches the same picture.
 * - `simulationFriction`, `simulationCenter`, `simulationRepulsionTheta` stay
 *   at their defaults; nothing in the picture asked them to move.
 */
const FORCE_CONFIG = {
  simulationLinkDistance: 150,
  simulationLinkSpring: 0.15,
  simulationRepulsion: 4,
  simulationGravity: 0.12,
  simulationDecay: 300,
} as const

/** One upload's worth of appearance, compiled by the caller. */
export type Appearance = {
  colors: Float32Array
  sizes: Float32Array
  linkWidths: Float32Array
}

/** The renderer, plus the one method that feeds it. */
export class Surface {
  constructor(
    readonly graph: Graph,
    readonly mode: LayoutMode,
  ) {}

  /**
   * Push the whole view to the GPU.
   *
   * Whole arrays every time, deliberately. cosmos.gl's setters replace their
   * buffers, so a partial upload silently drops whatever it omits — and at the
   * sizes the response bound permits (D5: 5 000 points) a full upload is a few
   * hundred kilobytes, which is cheaper than the bookkeeping a diff would need.
   */
  upload(view: SlotView, appearance: Appearance): void {
    // `true` = do not rescale: the second argument is `dontRescale`, and
    // letting cosmos.gl rescale would rewrite the server's coordinates.
    this.graph.setPointPositions(this.positionsFor(view), true)
    this.graph.setPointSizes(appearance.sizes)
    this.graph.setPointColors(appearance.colors)
    this.graph.setLinks(view.links)
    this.graph.setLinkWidths(appearance.linkWidths)
    if (this.mode === 'deterministic') this.zoomToPayload(view)
    // `render(undefined, 0)` — keep the current simulation alpha, no
    // transition. With on-demand rendering a static scene draws exactly one
    // frame, and that frame has to be asked for. Zero duration is what makes a
    // collapse *snap* rather than animating slots into a space they no longer
    // occupy — and in force mode it is what keeps the simulation unpaused.
    this.graph.render(undefined, 0)
  }

  /**
   * Reheat the simulation. Force mode only; a no-op otherwise.
   *
   * Called when the *node set* changed, never when the appearance did: a
   * colour-by choice that re-energised the layout would make the graph jump
   * under the user's cursor for no reason they could name.
   */
  reheat(alpha = 1): void {
    if (this.mode === 'deterministic') return
    this.graph.start(alpha)
  }

  /**
   * Frame the payload at the data-derived zoom.
   *
   * **`setZoomLevel`, not `setConfigPartial({ initialZoomLevel })`.**
   * `initialZoomLevel` is an init-only field, and cosmos.gl explicitly restores
   * it to its pre-update value on every `setConfig` / `setConfigPartial`
   * (`preserveInitOnlyFields`), so setting it after mount is a documented
   * no-op — measured: the zoom read back unchanged at 0.4237 after a
   * `setConfigPartial({ initialZoomLevel: 0.001 })`, and moved to 0.001 through
   * this setter. Before that was found, every expansion after the meta-graph
   * kept the meta-graph's zoom and ran off screen.
   */
  private zoomToPayload(view: SlotView): void {
    this.graph.setZoomLevel(zoomFor(view.positions))
  }

  /**
   * The positions to upload: the server's, except where the simulation has
   * already moved a slot somewhere better.
   *
   * In force mode the server's lattice is a *seed*, and it is only a seed for
   * slots that have never been drawn. Re-pushing it wholesale on every slice
   * would yank the settled layout back to a grid each time a user expanded
   * anything — the picture would rebuild itself from scratch on every click.
   * So: keep whatever the GPU has for a slot it already holds, and take the
   * server's value for a slot it does not. A NaN from the server wins outright,
   * because that is a tombstone and absence is the server's call (D4).
   */
  private positionsFor(view: SlotView): Float32Array {
    const seeded = toRendererSpace(view.positions)
    if (this.mode === 'deterministic') return seeded
    const live = this.graph.getPointPositions()
    const shared = Math.min(live.length, seeded.length)
    for (let i = 0; i < shared; i += 2) {
      const x = live[i]
      const y = live[i + 1]
      if (x === undefined || y === undefined) continue
      if (Number.isNaN(seeded[i]) || !Number.isFinite(x) || !Number.isFinite(y)) continue
      seeded[i] = x
      seeded[i + 1] = y
    }
    return seeded
  }
}

export async function mountGraph(
  container: HTMLDivElement,
  view: SlotView,
  appearance: Appearance,
  mode: LayoutMode,
): Promise<Surface> {
  const force = mode === 'force'
  const graph = new Graph(container, {
    enableSimulation: force,
    ...(force ? FORCE_CONFIG : {}),
    rescalePositions: false,
    transitionDuration: 0,
    fitViewOnInit: false,
    spaceSize: SPACE_SIZE,
    initialZoomLevel: zoomFor(view.positions),
    randomSeed: 'kglite-visual',
    backgroundColor: '#0d1117',
    scalePointsOnZoom: true,
    renderLinks: true,
    linkWidthScale: 1,
    pointSizeScale: 1,
    // Dragging a node is how a user pulls a cluster apart to read it, and it
    // only means anything while a simulation is there to re-settle around the
    // change. In deterministic mode the positions on screen are an assertion,
    // so nothing may move them.
    enableDrag: force,
    // The hover affordances the four interaction concepts drive. Rings rather
    // than a colour change, so hovering composes with a colour-by choice
    // instead of overwriting it.
    renderHoveredPointRing: true,
    hoveredPointCursor: 'pointer',
    hoveredPointRingColor: '#f0f6fc',
    focusedPointRingColor: '#f0f6fc',
    outlinedPointRingColor: '#f0883e',
    // The greyout `highlightedPointIndices` applies to everything else while a
    // hover is emphasising its neighbourhood.
    pointGreyoutOpacity: 0.18,
    linkGreyoutOpacity: 0.08,
    // Every meta-graph node carries a label, so the sampler must return them
    // all: the default distance thins points that are close together, which is
    // right for a million instance nodes and wrong for a dozen type nodes.
    pointSamplingDistance: POINT_SAMPLING_DISTANCE_PX,
    attribution: 'cosmos.gl',
  })

  const surface = new Surface(graph, mode)
  surface.upload(view, appearance)
  await graph.ready
  graph.render(undefined, 0)
  surface.reheat()
  return surface
}

/**
 * Server positions (origin-centred) into cosmos.gl's `[0, spaceSize]` space.
 *
 * A copy, deliberately: `view.positions` is what `window.__kglv.positionsHash`
 * hashes, and shifting it in place would make the determinism assert describe
 * the renderer's convention instead of the server's answer. A NaN stays NaN —
 * that is the tombstone, and cosmos.gl reads it as absence.
 */
function toRendererSpace(points: Float32Array): Float32Array {
  const shifted = new Float32Array(points.length)
  for (const [i, coordinate] of points.entries()) shifted[i] = coordinate + SPACE_SIZE / 2
  return shifted
}

/**
 * Zoom that puts the payload's own extent inside {@link TARGET_SPAN_PX}.
 *
 * A pure function of the positions: no viewport, no canvas, no measurement of
 * anything the browser owns. NaN coordinates are skipped, so collapsing a
 * thousand nodes does not leave the view zoomed for a graph that is gone.
 */
function zoomFor(points: Float32Array): number {
  let extent = 0
  for (const coordinate of points) {
    if (Number.isFinite(coordinate)) extent = Math.max(extent, Math.abs(coordinate))
  }
  // A single point has no extent; anything else would divide by zero.
  if (extent === 0) return 1
  return TARGET_SPAN_PX / (2 * extent)
}
