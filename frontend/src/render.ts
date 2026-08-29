/**
 * The cosmos.gl renderer, in D2's fixture mode.
 *
 * Every constructor value below is a determinism decision, not a preference:
 *
 * - `enableSimulation: false` — positions come from the server. cosmos.gl's
 *   `randomSeed` is init-only and the simulation leaks nondeterminism three
 *   ways (GPU float math differs by vendor, tick count rides
 *   requestAnimationFrame cadence, and v3 defaults to an 800 ms transition), so
 *   a seed is not a determinism switch and server-supplied positions are.
 * - `rescalePositions: false` — its default silently rewrites the coordinates
 *   the server just computed, which would make the committed positions
 *   baseline describe nothing.
 * - `transitionDuration: 0` — animation is opted into per call site.
 * - `fitViewOnInit: false` + a fixed `initialZoomLevel` — a fit depends on the
 *   viewport, so it would make the same data render differently at two window
 *   sizes.
 *
 * Uploads go through {@link Surface.upload}: whole typed arrays, one call each,
 * never a per-point callback (plan D7).
 */

import { Graph } from '@cosmos.gl/graph'

import type { SlotView } from './view'

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

/** One upload's worth of appearance, compiled by the caller. */
export type Appearance = {
  colors: Float32Array
  sizes: Float32Array
  linkWidths: Float32Array
}

/** The renderer, plus the one method that feeds it. */
export class Surface {
  constructor(readonly graph: Graph) {}

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
    this.graph.setPointPositions(toRendererSpace(view.positions), true)
    this.graph.setPointSizes(appearance.sizes)
    this.graph.setPointColors(appearance.colors)
    this.graph.setLinks(view.links)
    this.graph.setLinkWidths(appearance.linkWidths)
    this.graph.setConfigPartial({ initialZoomLevel: zoomFor(view.positions) })
    // `render(undefined, 0)` — no simulation alpha, no transition. With
    // on-demand rendering a static scene draws exactly one frame, and that
    // frame has to be asked for. Zero duration is what makes a collapse *snap*
    // rather than animating slots into a space they no longer occupy.
    this.graph.render(undefined, 0)
  }
}

export async function mountGraph(
  container: HTMLDivElement,
  view: SlotView,
  appearance: Appearance,
): Promise<Surface> {
  const graph = new Graph(container, {
    enableSimulation: false,
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

  const surface = new Surface(graph)
  surface.upload(view, appearance)
  await graph.ready
  graph.render(undefined, 0)
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
