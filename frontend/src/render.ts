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
 */

import { Graph } from '@cosmos.gl/graph'

import type { MetaGraphMessage } from './protocol'

/**
 * Screen span, in CSS pixels, the meta-graph is scaled to occupy.
 *
 * cosmos.gl's zoom is a plain linear factor — screen pixels = graph units ×
 * zoom (measured, not assumed) — so a *fixed* zoom cannot serve both a 5-type
 * meta-graph and a 200-type one: the spiral layout's extent grows with the
 * type count, and one number leaves the first invisible or the second off
 * screen. The zoom below is therefore derived from the DATA and from this
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

export type MetaGraphView = {
  graph: Graph
  /** Slot ids in the order their points were uploaded. */
  slots: number[]
}

export async function mountMetaGraph(
  container: HTMLDivElement,
  message: MetaGraphMessage,
): Promise<MetaGraphView> {
  const graph = new Graph(container, {
    enableSimulation: false,
    rescalePositions: false,
    transitionDuration: 0,
    fitViewOnInit: false,
    spaceSize: SPACE_SIZE,
    initialZoomLevel: zoomFor(message.points),
    randomSeed: 'kglite-visual',
    backgroundColor: '#0d1117',
    scalePointsOnZoom: true,
    renderLinks: true,
    linkWidthScale: 1,
    pointSizeScale: 1,
    // Every meta-graph node carries a label, so the sampler must return them
    // all: the default distance thins points that are close together, which is
    // right for a million instance nodes and wrong for a dozen type nodes.
    pointSamplingDistance: POINT_SAMPLING_DISTANCE_PX,
    attribution: 'cosmos.gl',
  })

  graph.setPointPositions(toRendererSpace(message.points), true)
  graph.setPointSizes(pointSizes(message))
  graph.setPointColors(pointColors(message))
  graph.setLinks(message.links)
  graph.setLinkWidths(linkWidths(message))

  await graph.ready
  // `render(undefined, 0)` — no simulation alpha, no transition. With
  // on-demand rendering a static scene draws exactly one frame, and that frame
  // has to be asked for.
  graph.render(undefined, 0)

  return { graph, slots: message.meta.nodes.map((node) => node.slot) }
}

/**
 * Server positions (origin-centred) into cosmos.gl's `[0, spaceSize]` space.
 *
 * A copy, deliberately: `message.points` is what `window.__kglv.positionsHash`
 * hashes, and shifting it in place would make the determinism assert describe
 * the renderer's convention instead of the server's answer.
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
 * anything the browser owns.
 */
function zoomFor(points: Float32Array): number {
  let extent = 0
  for (const coordinate of points) extent = Math.max(extent, Math.abs(coordinate))
  // A single point has no extent; anything else would divide by zero.
  if (extent === 0) return 1
  return TARGET_SPAN_PX / (2 * extent)
}

/**
 * Point radius from member count.
 *
 * Fourth root, not linear or square-root: a meta-graph routinely spans four
 * orders of magnitude between its largest and smallest type, and both of the
 * usual scales make the small ones invisible at that spread.
 */
function pointSizes(message: MetaGraphMessage): Float32Array {
  const sizes = new Float32Array(message.meta.nodes.length)
  for (const [i, node] of message.meta.nodes.entries()) {
    sizes[i] = 8 + 26 * Math.pow(node.count, 0.25) / Math.pow(maxCount(message), 0.25)
  }
  return sizes
}

function maxCount(message: MetaGraphMessage): number {
  return Math.max(1, ...message.meta.nodes.map((node) => node.count))
}

/**
 * Colour carries one bit today: whether the type declares any capability.
 *
 * The four interaction concepts (hovered / emphasized / highlighted /
 * selected) are index arrays pushed through `setConfigPartial`, and they land
 * with P3's selection work; keeping the base colour a single compiled array
 * fill now is what makes that cheap then.
 */
function pointColors(message: MetaGraphMessage): Float32Array {
  const colors = new Float32Array(message.meta.nodes.length * 4)
  for (const [i, node] of message.meta.nodes.entries()) {
    const plain = node.capabilities.length === 0
    // cosmos.gl takes 0..1 channels, not 0..255.
    colors[i * 4] = plain ? 0.35 : 0.98
    colors[i * 4 + 1] = plain ? 0.65 : 0.75
    colors[i * 4 + 2] = plain ? 0.98 : 0.32
    colors[i * 4 + 3] = 0.92
  }
  return colors
}

function linkWidths(message: MetaGraphMessage): Float32Array {
  const widths = new Float32Array(message.meta.edges.length)
  const max = Math.max(1, ...message.meta.edges.map((edge) => edge.count))
  for (const [i, edge] of message.meta.edges.entries()) {
    widths[i] = 0.5 + 4 * Math.sqrt(edge.count / max)
  }
  return widths
}
