/**
 * The cosmos.gl renderer, driven by three independent layout axes.
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
 * **The two modes are presets over {@link LayoutAxes}, not a switch.** One
 * boolean used to decide three unrelated questions at once — whether the
 * simulation runs, whether the user may drag a point, and who wins when the
 * server sends a position for a slot the GPU already holds — and they were only
 * ever answered together because the two modes that existed happened to want
 * the same answer to all three. A server-chosen static layout wants the
 * simulation off, dragging off, and the server's positions *authoritative*,
 * which is not a value the old boolean could take: `deterministic` also
 * suppresses the settle-time fit and is the mode the e2e suite asserts on by
 * name. Splitting the axes first means that layout can be added without the
 * mode flag acquiring a third value and a fourth meaning.
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

/**
 * A named combination of {@link LayoutAxes}. Reported as `__kglv.layoutMode`.
 *
 * `force` and `deterministic` are chosen at startup and never move; `static` is
 * entered and left at runtime, when the server sends an arrangement (plan E5).
 * Which *kernel* that arrangement came from is `__kglv.layoutKernel` — one
 * field for "how is the layout driven" and another for "which layout", because
 * a picker with five entries and a mode with three is not the same question
 * asked twice.
 */
export type LayoutMode = 'force' | 'deterministic' | 'static'

/**
 * Who owns a slot's position when the server sends one the GPU already has.
 *
 * `gpu` is the force-layout rule: the server's lattice is a seed, and a seed
 * for a slot that has already been drawn is stale by definition. `server` is
 * what a computed layout needs — the whole point of asking the server for one
 * is that its answer replaces whatever is on screen.
 */
export type SeedAuthority = 'gpu' | 'server'

/**
 * The three questions the renderer's construction and upload path ask.
 *
 * Independent by construction: nothing here reads another field, and the two
 * presets below are the only places a combination is named.
 */
export type LayoutAxes = {
  /** The GPU force simulation runs, and reheats when the node set changes. */
  simulation: boolean
  /** The user may drag a point. */
  drag: boolean
  /** See {@link SeedAuthority}. */
  seedAuthority: SeedAuthority
}

/**
 * The user's mode: a live simulation, draggable, seeded once per slot.
 *
 * Dragging a node is how a user pulls a cluster apart to read it, and it only
 * means anything while a simulation is there to re-settle around the change —
 * which is why `drag` follows `simulation` *in this preset* rather than in the
 * type.
 */
const FORCE_AXES: LayoutAxes = {
  simulation: true,
  drag: true,
  seedAuthority: 'gpu',
}

/**
 * D2's fixture mode: the positions on screen are the server's answer, exactly.
 *
 * Nothing may move them — not the simulation, not the user — because they are
 * what `positionsHash` describes and what the e2e suite asserts on.
 */
const DETERMINISTIC_AXES: LayoutAxes = {
  simulation: false,
  drag: false,
  seedAuthority: 'server',
}

/**
 * A layout the server computed: nothing on the client may move it.
 *
 * Identical to {@link DETERMINISTIC_AXES} in its values and different in its
 * reason, which is why it is a second constant rather than a reuse. D2's mode
 * holds the positions still so a *hash* means something; this one holds them
 * still because a hop ring that a simulation is free to re-heat is a hop ring
 * for about two seconds. The two would diverge the moment either gained a
 * fourth axis, and one shared constant is where that divergence would hide.
 */
const STATIC_AXES: LayoutAxes = {
  simulation: false,
  drag: false,
  seedAuthority: 'server',
}

/** The axes a named mode stands for. */
export function axesFor(mode: LayoutMode): LayoutAxes {
  if (mode === 'deterministic') return { ...DETERMINISTIC_AXES }
  if (mode === 'static') return { ...STATIC_AXES }
  return { ...FORCE_AXES }
}

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
/*
 * These five numbers are also the reference for the headless export layout's
 * force balance — `GRAVITY`, `REPULSION_SCALE` and `ATTRACTION_SCALE` in
 * `crates/kglite-visual-core/src/render/layout.rs` (plan D13). Different
 * algorithm (a seeded Fruchterman–Reingold, not a GPU simulation), same graph
 * and the same failure to avoid, so the ratios were taken from here rather than
 * re-derived.
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
    private currentAxes: LayoutAxes,
  ) {}

  /** The axes in force right now. */
  get axes(): Readonly<LayoutAxes> {
    return this.currentAxes
  }

  /**
   * Move to a new combination of axes, in the order cosmos.gl requires.
   *
   * **The order is the whole method, and it is asymmetric because cosmos.gl
   * is.** Turning `enableSimulation` from false to true does not just switch a
   * flag: it rebuilds the simulation modules, reheats at alpha 1 and kills any
   * in-flight position transition. So —
   *
   * - **Going static**, the config goes first and the caller's upload second:
   *   the simulation has to be gone before the server's positions land, or the
   *   next tick moves them and the arrangement the user asked for is visibly
   *   wrong within a frame.
   * - **Going live**, the config goes last and nothing is uploaded after it:
   *   the reheat is the point (the static layout becomes the simulation's
   *   seed), and an upload afterwards would be a second start from a picture
   *   the GPU has already begun pulling apart.
   *
   * The force parameters are re-applied on the way back in rather than assumed
   * to have survived the round trip, because they were set at construction and
   * the modules reading them have been destroyed since.
   */
  setAxes(axes: LayoutAxes): void {
    const wasRunning = this.currentAxes.simulation
    this.currentAxes = { ...axes }
    if (!axes.simulation) {
      this.graph.setConfigPartial({ enableSimulation: false, enableDrag: axes.drag })
      return
    }
    if (!wasRunning) {
      this.graph.setConfigPartial({
        enableSimulation: true,
        enableDrag: axes.drag,
        ...FORCE_CONFIG,
      })
      return
    }
    this.graph.setConfigPartial({ enableDrag: axes.drag })
  }

  /**
   * Push the whole view to the GPU.
   *
   * Whole arrays every time, deliberately. cosmos.gl's setters replace their
   * buffers, so a partial upload silently drops whatever it omits — and at the
   * sizes the response bound permits (D5: 5 000 points) a full upload is a few
   * hundred kilobytes, which is cheaper than the bookkeeping a diff would need.
   */
  upload(view: SlotView, appearance: Appearance, authority?: SeedAuthority): void {
    // `true` = do not rescale: the second argument is `dontRescale`, and
    // letting cosmos.gl rescale would rewrite the server's coordinates.
    this.graph.setPointPositions(this.positionsFor(view, authority), true)
    this.graph.setPointSizes(appearance.sizes)
    this.graph.setPointColors(appearance.colors)
    this.graph.setLinks(view.links)
    this.graph.setLinkWidths(appearance.linkWidths)
    // The data-derived zoom, re-applied per upload — but only where nothing is
    // going to move afterwards. A running simulation reframes at
    // `onSimulationEnd` instead (`main.ts`), and framing the *seed* first would
    // be a zoom to an arrangement the user never sees.
    if (!this.axes.simulation) this.zoomToPayload(view)
    // `render(undefined, 0)` — keep the current simulation alpha, no
    // transition. With on-demand rendering a static scene draws exactly one
    // frame, and that frame has to be asked for. Zero duration is what makes a
    // collapse *snap* rather than animating slots into a space they no longer
    // occupy — and in force mode it is what keeps the simulation unpaused.
    this.graph.render(undefined, 0)
  }

  /**
   * Reheat the simulation. A no-op where there is no simulation.
   *
   * Called when the *node set* changed, never when the appearance did: a
   * colour-by choice that re-energised the layout would make the graph jump
   * under the user's cursor for no reason they could name.
   */
  reheat(alpha = 1): void {
    if (!this.axes.simulation) return
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
   * The positions to upload, resolved by {@link LayoutAxes.seedAuthority} —
   * unless this one upload says otherwise.
   *
   * **The authority is what pre-mortem #1 is about** (plan E5). A
   * server-computed layout arriving while the force simulation is live has to
   * win: under `gpu` it is merged away point by point, because every slot on
   * screen already has a GPU position — so a kernel switch works in
   * deterministic mode and does visibly nothing in the mode users run.
   *
   * **Two things provide it, and the e2e proves each is sufficient alone**
   * (measured 2026-08-30, `layout.spec.ts` driven three ways): the mode switch
   * to {@link STATIC_AXES}, which lands before the upload, and this argument.
   * With `gpu` standing authority and no argument the spec fails at "the
   * picture moved"; with either one it passes. The argument is kept because it
   * is the half that does not depend on ordering — a future caller that applies
   * a layout without having switched modes first still gets the answer it
   * asked for, rather than a silent no-op nobody would think to test for.
   */
  private positionsFor(view: SlotView, authority?: SeedAuthority): Float32Array {
    const resolved = authority ?? this.axes.seedAuthority
    return mergePositions(
      toRendererSpace(view.positions),
      resolved === 'gpu' ? this.graph.getPointPositions() : null,
      resolved,
    )
  }
}

/**
 * Resolve the server's positions against the ones already on the GPU.
 *
 * Under `server` authority the server's array wins outright — that is what a
 * computed layout means, and re-deriving it from anything on screen would make
 * the answer a function of what was there before.
 *
 * Under `gpu` authority the server's lattice is a *seed*, and it is only a seed
 * for slots that have never been drawn. Re-pushing it wholesale on every slice
 * would yank the settled layout back to a grid each time a user expanded
 * anything — the picture would rebuild itself from scratch on every click. So:
 * keep whatever the GPU has for a slot it already holds, and take the server's
 * value for a slot it does not. A NaN from the server still wins, because that
 * is a tombstone and absence is the server's call (D4).
 *
 * `seeded` is mutated and returned; it is already a private copy
 * ({@link toRendererSpace}).
 */
export function mergePositions(
  seeded: Float32Array,
  live: ArrayLike<number> | null,
  authority: SeedAuthority,
): Float32Array {
  if (authority === 'server' || live === null) return seeded
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

export async function mountGraph(
  container: HTMLDivElement,
  view: SlotView,
  appearance: Appearance,
  axes: LayoutAxes,
): Promise<Surface> {
  const graph = new Graph(container, {
    enableSimulation: axes.simulation,
    ...(axes.simulation ? FORCE_CONFIG : {}),
    rescalePositions: false,
    transitionDuration: 0,
    fitViewOnInit: false,
    spaceSize: SPACE_SIZE,
    initialZoomLevel: zoomFor(view.positions),
    randomSeed: 'kglite-visual',
    // Also `Palette::background` for the dark theme in
    // `crates/kglite-visual-core/src/render/encoding.rs`.
    backgroundColor: '#0d1117',
    scalePointsOnZoom: true,
    renderLinks: true,
    linkWidthScale: 1,
    pointSizeScale: 1,
    enableDrag: axes.drag,
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

  const surface = new Surface(graph, axes)
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
