/**
 * Entry point: connect, decode, render, and drive the drill-in.
 *
 * No framework, and no graph data in observable state (plan D7). The
 * comparable-repo study found the same failure in every slow graph UI —
 * putting nodes and edges in framework state turns one click into an O(V+E)
 * re-render. Payloads go from the socket into typed arrays and straight to the
 * renderer; only the small scalars in `state.ts` are ever observed.
 *
 * The drill-in is the flagship flow and it reads in one line here: click a
 * meta-graph type node → preview its relationship counts (no fetch) → expand
 * one of them, bounded → collapse back to tombstones.
 */

import './styles.css'

import {
  compileCategoricalColor,
  compileNumericSize,
  fillColors,
  linkWidth,
  typeHue,
  typeRadius,
  UNSET_COLOR,
  type Rgba,
} from './appearance'
import { markRendererMounted, publishBenchHook } from './bench'
import { debugState, probeDeviceFeatures, publishDebugState } from './debug'
import type { Appearance as AppearanceCommand } from './generated/Appearance'
import type { BoundInfo } from './generated/BoundInfo'
import type { Focus } from './generated/Focus'
import type { Highlight } from './generated/Highlight'
import type { ExpansionPreview } from './generated/ExpansionPreview'
import type { MetaGraphMeta } from './generated/MetaGraphMeta'
import type { NodeDetail } from './generated/NodeDetail'
import type { PropertyStat } from './generated/PropertyStat'
import type { Request } from './generated/Request'
import { InteractionState } from './interaction'
import { LabelOverlay } from './labels'
import { Panels } from './panels'
import {
  assertLittleEndian,
  fnv1a,
  ResponseAssembler,
  type Completed,
  type LayoutMessage,
} from './protocol'
import * as store from './queries'
import { SchemaCache } from './schema'
import { validateQuery } from './validate'
import {
  axesFor,
  layoutModeFromSearch,
  mountGraph,
  type Appearance,
  type SeedAuthority,
  type Surface,
} from './render'
import { connectedAtom } from './state'
import { rendersGraph } from './tiers'
import { WebSocketTransport } from './transport'
import { SlotView } from './view'

const mount = document.querySelector<HTMLDivElement>('#app')
if (!mount) throw new Error('#app is missing from index.html')

assertLittleEndian()
debugState.deviceFeatures = probeDeviceFeatures()
/**
 * `force` unless the page was asked for `?deterministic=1`.
 *
 * Read once, at startup: the axes decide how the renderer is *constructed*, so
 * a mid-session change to `simulation` or `drag` would mean tearing the
 * renderer down. `__kglv` reports the mode name because `positionsHash` only
 * means something in the deterministic one.
 */
const layoutMode = layoutModeFromSearch(window.location.search)
const layoutAxes = axesFor(layoutMode)
debugState.layoutMode = layoutMode
publishDebugState()
publishBenchHook()

const root = document.createElement('div')
root.className = 'kglv-root'
mount.appendChild(root)

const canvasHost = document.createElement('div')
canvasHost.className = 'kglv-canvas'
root.appendChild(canvasHost)

const status = document.createElement('div')
status.className = 'kglv-status'
root.appendChild(status)

const labels = new LabelOverlay(root, {
  // A label is the addressable handle on a node: clicking the name selects it
  // and pointing at it emphasises its neighbourhood, both without a single
  // pixel guess. Same code path as a canvas click — `onPointClick` below.
  onSelect: (slot) => {
    interaction.setSelected([slot])
    applyInteraction()
    send({ type: 'preview', slot })
  },
  onHover: (slot) => {
    if (surface === null) return
    if (interaction.hover(surface.graph, slot)) applyInteraction()
  },
})
const view = new SlotView()
const interaction = new InteractionState()
const assembler = new ResponseAssembler()
const transport = new WebSocketTransport('ws')

let surface: Surface | null = null
let lastMeta: MetaGraphMeta | null = null
let lastPreview: ExpansionPreview | null = null
let lastDetail: NodeDetail | null = null
/** The banner the last bounded response produced, or null when nothing was clipped. */
let truncationBanner: string | null = null

/** Appearance state: a compiled getter plus the values it reads. */
let colorByStat: PropertyStat | null = null
let sizeByName: string | null = null
const appearanceValues = new Map<number, unknown>()
/** Values for the size channel, keyed by slot. Separate array, separate query. */
const sizeValues = new Map<number, number>()

/**
 * The schema behind the editor's completions.
 *
 * Filled from the meta-graph the moment it arrives — the same payload that
 * draws the entry screen — and topped up per type, lazily, when the editor asks
 * for a label's properties (plan E2).
 */
const schema = new SchemaCache()

const panels = new Panels(root, {
  runQuery: (query, asGraph) => {
    if (query.trim() === '') return
    send({ type: 'cypher', query, params: {}, limit: null, as_graph: asGraph })
    // The one place a user-typed query is recorded. The app's own queries —
    // appearance values, "load into view" — go through `send` directly and are
    // deliberately not history.
    void refreshQueries(store.recordQuery(query))
  },
  expand: (slot, relationship, direction, limit) =>
    send({ type: 'expand', slot, relationship, direction, limit }),
  collapse: (slot) => send({ type: 'collapse', slot }),
  search: (query, nodeType) => {
    if (query.trim() === '') return
    send({
      type: 'search',
      query,
      node_type: nodeType,
      // The fixture's instance nodes carry kglite's canonical `title`, which is
      // also what the label overlay shows — so the box searches what the user
      // can see. A different default would find things the screen cannot name.
      property: 'title',
      mode: 'contains',
      limit: null,
    })
  },
  loadHits: (nodeIds, nodeType) => {
    // Bounded by construction: the id list is whatever the search returned,
    // and the search was itself bounded in core (D5). The query's own row
    // bound clamps it again on the way back.
    const label = nodeType === null ? '' : `:${nodeType}`
    send({
      type: 'cypher',
      query: `MATCH (n${label}) WHERE id(n) IN $ids RETURN n`,
      params: { ids: nodeIds },
      limit: null,
      as_graph: true,
    })
  },
  focusSlot: (slot) => {
    interaction.setSelected([slot])
    applyInteraction()
    send({ type: 'preview', slot })
  },
  setColorBy: (property) => applyColorBy(property),
  setSizeBy: (property) => applySizeBy(property),
  saveQuery: (name, query) => void refreshQueries(store.saveQuery(name, query)),
  deleteQuery: (name) => void refreshQueries(store.deleteQuery(name)),
  // Parse-only, over plain HTTP, on the editor's idle timer. It never runs the
  // query and never moves the view.
  validateQuery: (query) => validateQuery(query),
}, schema)

/**
 * Run a store mutation, then re-read the store and redraw the list.
 *
 * Always a re-read, never a local edit of what the panel is holding: the store
 * is a file on the server, and a `curl`, a second tab or an agent's
 * `run_saved_query` may have moved it since this page last looked. A panel that
 * patched its own copy would drift from the file it claims to show.
 *
 * A refusal — a ceiling, a name that is not there — is the store's own sentence
 * and goes to the panel verbatim; the number it names is the whole point.
 */
async function refreshQueries(mutation?: Promise<unknown>): Promise<void> {
  try {
    if (mutation !== undefined) await mutation
    panels.showSavedQueries(await store.listQueries())
  } catch (err) {
    panels.showQueriesError(err instanceof Error ? err.message : String(err))
  }
}

void refreshQueries()

/**
 * The colour channel, from either driver.
 *
 * Extracted when the `appearance` command landed (plan D14): the menu and a
 * remote agent must move the same channel through the same code, or the two
 * drivers would drift into two behaviours for one control.
 */
function applyColorBy(property: string | null): void {
  colorByStat = property === null ? null : (lastStats.get(property) ?? null)
  appearanceValues.clear()
  debugState.colorBy = property
  if (property === null) {
    redraw()
    return
  }
  requestAppearanceValues(property, 'color')
}

/** The size channel, from either driver. See {@link applyColorBy}. */
function applySizeBy(property: string | null): void {
  sizeByName = property
  sizeValues.clear()
  debugState.sizeBy = property
  if (property === null) {
    redraw()
    return
  }
  requestAppearanceValues(property, 'size')
}

/**
 * Frame the named slots, or the whole view when the list is empty.
 *
 * Slots ARE renderer point indices — that is the D4 identity contract, and it
 * is why an agent can name what the user should look at without either side
 * translating. Absent slots are dropped rather than passed through: a
 * tombstoned index has a NaN position, and cosmos.gl's fit would take it as an
 * extent and zoom to nothing.
 */
function applyFocus(command: Focus): void {
  debugState.focusedSlots = [...command.slots]
  if (surface === null) return
  const live = new Set(view.liveSlots())
  const targets = command.slots.filter((slot) => live.has(slot))
  // Duration zero: this is a jump to somewhere the agent is about to talk
  // about, not an animation, and an in-flight transition auto-pauses the
  // simulation (see `render.ts`).
  if (targets.length === 0) {
    surface.graph.fitView(0)
  } else {
    surface.graph.fitViewByPointIndices(targets, 0)
  }
  syncCounts()
}

/** Set one interaction concept's index array from a remote command. */
function applyHighlight(command: Highlight): void {
  const live = new Set(view.liveSlots())
  const slots = command.slots.filter((slot) => live.has(slot))
  if (command.concept === 'selected') {
    interaction.setSelected(slots)
    // A selection the user did not make still has to say what it selected, or
    // the outline ring names a node with no panel behind it.
    if (slots.length === 1) send({ type: 'preview', slot: slots[0] as number })
  } else {
    interaction.setHighlighted(slots)
  }
  // `highlighted` rides the colour array, so it needs a full redraw; `selected`
  // is a ring and would be satisfied by `applyInteraction`. One path for both,
  // because two would be one more place to get the distinction wrong.
  redraw()
}

/**
 * Take a server-computed arrangement, whoever asked for it (plan E5).
 *
 * **`'server'` authority for this one upload, and only this one.** The
 * standing authority in force mode is `gpu`, which is right for a slice — the
 * server's lattice is a seed and a seed for a slot already on screen is stale.
 * It is exactly wrong here: every slot the layout covers is already drawn, so
 * the merge would discard the whole answer and the picture would not move.
 * That is pre-mortem #1, and it is why the axis is resolved per upload rather
 * than switched.
 */
function applyLayout(message: LayoutMessage): void {
  if (message.meta.kernel_chosen === 'simulation') return
  view.applyLayout(message.points)
  redraw('server')
}

/** Drive both appearance channels from a remote command. */
function applyAppearance(command: AppearanceCommand): void {
  panels.setAppearanceSelection(command.color_by, command.size_by)
  applyColorBy(command.color_by)
  applySizeBy(command.size_by)
}

/** The property statistics behind the two dropdowns, by property name. */
const lastStats = new Map<string, PropertyStat>()
/** Which channel the in-flight appearance query is filling. */
let appearanceChannel: 'color' | 'size' | null = null

transport.connect({
  onStatus: (connected) => {
    connectedAtom.set(connected)
    renderStatus()
  },
  onError: (message) => fail(message),
  onFrame: (frame) => {
    let completed
    try {
      completed = assembler.push(frame)
    } catch (err) {
      fail(err instanceof Error ? err.message : String(err))
      return
    }
    debugState.lastMessageSeq = assembler.lastSeq
    if (completed === null) return
    void handle(completed)
  },
})

function send(request: Request): void {
  transport.send(JSON.stringify(request))
}

async function handle(completed: Completed): Promise<void> {
  switch (completed.kind) {
    case 'session':
      debugState.protocolVersion = completed.value.protocol_version
      debugState.tier = completed.value.tier
      renderStatus()
      break
    case 'meta-graph':
      await showMetaGraph(completed.value)
      break
    case 'slice': {
      const { meta, compaction, points, links } = completed.value
      view.applySlice(meta, compaction, points, links)
      if (compaction !== null) {
        // The remap is inside `applySlice`, which needs it before it rebuilds
        // its maps; this only counts it for the debug hook.
        debugState.compactions += 1
      }
      debugState.lastSliceKind = meta.kind
      // The slots this slice killed cannot stay in an interaction set: they
      // draw nothing, and the counts would keep describing them.
      interaction.dropSlots(meta.tombstones)
      noteTruncation(
        meta.bound.truncated,
        meta.bound.returned,
        meta.bound.total,
        meta.kind === 'collapse' ? 'collapsed' : 'nodes',
        meta.link_bound,
      )
      // A compaction renumbers every slot, so anything the user had selected or
      // hovered now names something else. Dropping the sets is the only honest
      // response — carrying them across would highlight arbitrary nodes.
      if (compaction !== null) {
        interaction.setSelected([])
        interaction.setHighlighted([])
        if (surface !== null) interaction.hover(surface.graph, null)
        lastPreview = null
        lastDetail = null
        panels.clearSelection()
      }
      redraw()
      // The node set changed, so the layout has new work to do. Reheating on a
      // slice and nowhere else is the rule: an appearance change that
      // re-energised the simulation would make the graph jump under the
      // user's cursor for no reason they could name.
      surface?.reheat()
      break
    }
    case 'preview':
      lastPreview = completed.value
      // A type node has no stored properties; an instance does, and the panel
      // shows both in one place, so the detail is fetched alongside.
      if (completed.value.scope === 'node') {
        send({ type: 'node-detail', slot: completed.value.slot })
      } else {
        lastDetail = null
        send({ type: 'property-stats', node_type: completed.value.node_type })
      }
      debugState.previewRows = panels.showPreview(completed.value, lastDetail)
      break
    case 'node-detail':
      lastDetail = completed.value
      if (lastPreview !== null) {
        debugState.previewRows = panels.showPreview(lastPreview, lastDetail)
      }
      send({ type: 'property-stats', node_type: completed.value.node_type })
      break
    case 'query-table': {
      const table = completed.value
      if (appearanceChannel !== null) {
        absorbAppearanceValues(table.columns, table.data, appearanceChannel)
        appearanceChannel = null
        break
      }
      debugState.queryRows = panels.showQueryTable(table)
      noteTruncation(table.bound.truncated, table.bound.returned, table.bound.total, 'rows')
      renderStatus()
      break
    }
    case 'search': {
      debugState.searchHits = panels.showSearch(completed.value)
      interaction.setHighlighted(panels.loadedHitSlots())
      redraw()
      break
    }
    case 'property-stats': {
      lastStats.clear()
      for (const stat of completed.value.properties) lastStats.set(stat.name, stat)
      const [candidates, approximate] = panels.showPropertyStats(completed.value)
      debugState.appearanceCandidates = candidates
      debugState.approximateStats = approximate
      break
    }
    // The three steering commands. Unsolicited by construction — they arrive
    // because an agent, or another tab, asked this view to move — so they are
    // handled exactly like every other message: by kind, never by matching a
    // request this client remembers making.
    case 'focus':
      applyFocus(completed.value)
      break
    case 'highlight':
      applyHighlight(completed.value)
      break
    case 'appearance':
      applyAppearance(completed.value)
      break
    case 'layout':
      applyLayout(completed.value)
      break
    case 'error':
      // A query failure is the panel's business, not the whole app's: the graph
      // on screen is still valid and blanking it would lose the user's place.
      if (appearanceChannel !== null) appearanceChannel = null
      panels.showQueryError(completed.value)
      break
  }
}

async function showMetaGraph(message: {
  meta: MetaGraphMeta
  points: Float32Array
  links: Float32Array
}): Promise<void> {
  lastMeta = message.meta
  view.setMetaGraph(message.meta, message.points, message.links)
  debugState.tier = message.meta.tier
  debugState.protocolVersion = message.meta.protocol_version
  debugState.positionsHash = fnv1a(message.points)
  panels.setNodeTypes(message.meta.nodes.map((node) => node.name))
  schema.setMetaGraph(message.meta)
  renderStatus()

  if (!rendersGraph(message.meta.tier)) {
    // Tier 4: thousands of type nodes is not a picture of anything. The stats
    // panel is the honest view, and search is the way in.
    showSummaryPanel(message.meta)
    syncCounts()
    debugState.ready = true
    return
  }

  try {
    surface = await mountGraph(canvasHost, view, appearance(), layoutAxes)
    attachHandlers(surface)
    debugState.simRunning = surface.graph.isSimulationRunning
    redraw()
    markRendererMounted(surface.graph)
    debugState.ready = true
  } catch (err) {
    // A device with no WebGL2 lands here. D10: an honest error, never a
    // second renderer.
    fail(`the renderer could not start: ${err instanceof Error ? err.message : String(err)}`)
  }
}

/**
 * One upload, one label pass, one interaction projection.
 *
 * Everything that changes the view funnels through here so there is exactly one
 * place that talks to the GPU — and so the debug counts can never describe a
 * frame that was not drawn.
 */
function redraw(authority?: SeedAuthority): void {
  if (surface === null) return
  surface.upload(view, appearance(), authority)
  interaction.apply(surface.graph)
  refreshLabelSpecs()
  positionLabels(surface)
  syncCounts()
  renderStatus()
}

function appearance(): Appearance {
  const slots = view.slotCount
  const sizes = new Float32Array(slots)
  const sizeOf = compileNumericSize([...sizeValues.values()])
  const largestType = maxTypeCount()
  for (let slot = 0; slot < slots; slot += 1) {
    const label = view.label(slot)
    if (label === undefined) {
      sizes[slot] = 0
      continue
    }
    if (sizeByName !== null && sizeValues.has(slot)) {
      sizes[slot] = sizeOf(sizeValues.get(slot))
    } else if (label.isType) {
      sizes[slot] = typeRadius(label.weight, largestType, label.supporting)
    } else {
      sizes[slot] = 6
    }
  }

  const colorOf = colorByStat === null ? null : compileCategoricalColor(colorByStat)
  const highlighted = new Set(interaction.highlightedSlots())
  const colors = fillColors(
    slots,
    (slot) => baseColor(slot, colorOf),
    highlighted,
  )

  return { colors, sizes, linkWidths: linkWidths() }
}

/**
 * One width per link, from the edge count the meta-graph carried.
 *
 * **Ported to Rust** in `render::meta_scene`
 * (`crates/kglite-visual-core/src/render/mod.rs`), pair-summing included.
 *
 * A link between two type nodes is a summary of hundreds of thousands of real
 * edges, and drawing every one of them at the same 1 px says the schema is
 * uniform when it is not. Links that are not meta links — anything an
 * expansion or a query added — get the floor: they are single edges, and there
 * is no count to encode.
 */
function linkWidths(): Float32Array {
  const widths = new Float32Array(view.linkCount)
  let heaviest = 1
  const counts = new Float32Array(view.linkCount)
  for (let link = 0; link < view.linkCount; link += 1) {
    const source = view.links[link * 2]
    const target = view.links[link * 2 + 1]
    if (source === undefined || target === undefined) continue
    const count = view.typeLinkWeight(source, target)
    counts[link] = count
    heaviest = Math.max(heaviest, count)
  }
  for (let link = 0; link < widths.length; link += 1) {
    widths[link] = linkWidth(counts[link] ?? 0, heaviest)
  }
  return widths
}

/**
 * A slot's colour before highlighting.
 *
 * **Ported to Rust** as `base_color` in
 * `crates/kglite-visual-core/src/render/encoding.rs` (minus the colour-by
 * branch, which a render request has no equivalent of).
 *
 * With no colour-by chosen, the two bits a type node carries are whether it
 * declares any capability and whether it is a *supporting* type — a type with
 * a parent in kglite's `is_a` forest, which is the server's own answer to
 * "which of these types is the graph actually about". A supporting type keeps
 * its hue and loses most of its opacity, so the core types it hangs off carry
 * the picture. An instance node is drawn in its own muted hue so the
 * meta-graph stays legible under an expansion.
 */
function baseColor(slot: number, colorOf: ((value: unknown) => Rgba) | null): Rgba {
  const label = view.label(slot)
  if (label === undefined) return UNSET_COLOR
  if (colorOf !== null && appearanceValues.has(slot)) return colorOf(appearanceValues.get(slot))
  // An instance node takes its type's hue; a type node keeps the
  // capability/supporting encoding, which is a different fact and would be
  // destroyed by overwriting it with a hue.
  if (!label.isType) return typeHue(label.nodeType)
  const plain = label.badges.length === 0
  // cosmos.gl takes 0..1 channels, not 0..255.
  const [r, g, b, a]: Rgba = plain
    ? [0.35, 0.65, 0.98, 0.92]
    : [0.98, 0.75, 0.32, 0.92]
  return label.supporting ? [r, g, b, a * 0.45] : [r, g, b, a]
}

function maxTypeCount(): number {
  let max = 1
  for (const slot of view.liveSlots()) {
    const label = view.label(slot)
    if (label?.isType === true) max = Math.max(max, label.weight)
  }
  return max
}

function attachHandlers(current: Surface): void {
  current.graph.setConfigPartial({
    // Index-addressed callbacks from the renderer, and the only ones: the four
    // interaction concepts are index ARRAYS pushed back through
    // `setConfigPartial`, never a per-item reducer (plan D7).
    onPointMouseOver: (index: number) => {
      if (interaction.hover(current.graph, index)) applyInteraction()
    },
    onPointMouseOut: () => {
      if (interaction.hover(current.graph, null)) applyInteraction()
    },
    onPointClick: (index: number) => {
      interaction.setSelected([index])
      applyInteraction()
      send({ type: 'preview', slot: index })
    },
    onClick: (index: number | undefined) => {
      if (index !== undefined) return
      interaction.setSelected([])
      applyInteraction()
      lastPreview = null
      lastDetail = null
      panels.clearSelection()
    },
    // Camera events reposition the labels already on screen; they never rebuild
    // the spec table. The spec list is a function of the *view*, and the camera
    // is not part of it — rebuilding here cost 15 ms of p95 frame period at the
    // 5 000-node response bound (measured 2026-08-29: 33.3 ms with the rebuild,
    // 18.0 ms without, same slice and same simulation).
    onZoom: () => positionLabels(current),
    onZoomEnd: () => positionLabels(current),
    // A moving layout moves the points a label names, so the overlay has to
    // follow it. This is the same cheap sampled-points pass a zoom already runs
    // once per frame — never `setLabels`, which walks the whole view.
    onSimulationTick: () => {
      debugState.simRunning = true
      positionLabels(current)
    },
    onSimulationEnd: () => {
      debugState.simRunning = false
      // The settled layout's extent is not the seed's, so the data-derived
      // zoom no longer frames it. A fit is a viewport-dependent operation and
      // therefore banned wherever the positions are an assertion (D2). This
      // callback fires only with the simulation axis on, where the layout was
      // already viewport-independent up to the simulation and nothing is left
      // to protect — `Surface.upload` reads the same axis to decide whether to
      // apply the data-derived zoom instead.
      current.graph.fitView(0)
      positionLabels(current)
      syncCounts()
    },
  })
}

/** One `setConfigPartial` for all four concepts, then one frame. */
function applyInteraction(): void {
  if (surface === null) return
  interaction.apply(surface.graph)
  surface.graph.render(undefined, 0)
  syncCounts()
}

/**
 * Rebuild the candidate list from the view. Slice path only.
 *
 * O(live slots), so it belongs where the *view* changed — `redraw`, which every
 * slice funnels through.
 */
function refreshLabelSpecs(): void {
  const selected = new Set(interaction.selectedSlots())
  labels.setLabels(
    view.liveSlots().map((slot) => {
      const label = view.label(slot)
      return {
        slot,
        text: label?.text ?? '',
        badges: label?.badges ?? [],
        weight: label?.weight ?? 0,
        // A count chip where there is a count. A type node's count is the whole
        // reason it is that size; an instance node's is always 1, and a chip
        // that says so on every node is width spent on nothing.
        showCount: label?.isType === true || (label?.weight ?? 0) > 1,
        pinned: selected.has(slot),
        dimmed: label?.supporting === true,
      }
    }),
  )
}

/**
 * True while the view is nothing but the type-level meta-graph.
 *
 * **Ported to Rust** as `Scene::place_all_labels` in
 * `crates/kglite-visual-core/src/render/mod.rs`, which the render sets from the
 * request's source rather than from a live view.
 *
 * The meta-graph is a picture *of its labels* — a hundred type names is the
 * schema, and a hundred unlabelled dots is the "cloud" this screen was
 * reported as. So on this view every candidate is offered (the renderer's own
 * point sampler exists to thin a million instance nodes and drops type nodes
 * that must all be named) and none is dropped for want of a cell. The moment
 * an expansion adds instance nodes the view stops being that, and both
 * thinnings come back.
 */
function isMetaGraphOnly(): boolean {
  return view.liveSlots().every((slot) => view.label(slot)?.isType === true)
}

/** Re-place the already-built candidates against the current camera. */
function positionLabels(current: Surface): void {
  const graph = current.graph
  // The camera moved (a zoom, a settle, a `focus` command), so the one number
  // an agent can assert a camera move on is re-read here. cosmos.gl runs the
  // fit through a d3 transition, so it is NOT settled when `fitView*` returns
  // and a read at the call site reports the old zoom — measured, not assumed.
  debugState.zoomLevel = graph.getZoomLevel()
  const wholeSchema = isMetaGraphOnly()
  labels.update(
    {
      sampledPoints: () => (wholeSchema ? everyPoint(current) : graph.getSampledPoints()),
      toScreen: (position: [number, number]) => graph.spaceToScreenPosition(position),
      radius: (index: number) => graph.getPointRadiusByIndex(index) ?? 0,
    },
    wholeSchema,
  )
}

/**
 * Every live point, in the shape `getSampledPoints()` returns.
 *
 * O(slots) and therefore only ever called on the meta-graph, where the slot
 * count is the type count. Positions come from the renderer rather than from
 * the view because in force mode the two disagree by design: the view holds
 * the server's seed, and the simulation holds where the point actually is.
 */
function everyPoint(current: Surface): { indices: number[]; positions: number[] } {
  const live = current.graph.getPointPositions()
  const indices: number[] = []
  const positions: number[] = []
  for (const slot of view.liveSlots()) {
    const x = live[slot * 2]
    const y = live[slot * 2 + 1]
    if (x === undefined || y === undefined || !Number.isFinite(x) || !Number.isFinite(y)) {
      continue
    }
    indices.push(slot)
    positions.push(x, y)
  }
  return { indices, positions }
}

function syncCounts(): void {
  // Read back from the renderer, not from whatever this file last asked for:
  // a zoom the GPU did not take is a zoom that did not happen.
  debugState.zoomLevel = surface?.graph.getZoomLevel() ?? null
  debugState.pointCount = view.liveCount
  debugState.linkCount = view.linkCount
  debugState.slotCount = view.slotCount
  debugState.tombstoneCount = view.tombstoneCount
  debugState.hoveredSlot = interaction.hoveredSlot()
  debugState.emphasizedCount = interaction.emphasizedSlots().length
  debugState.highlightedCount = interaction.highlightedSlots().length
  debugState.selectedCount = interaction.selectedSlots().length
  debugState.truncation =
    truncation === null
      ? null
      : { ...truncation, banner: truncationBanner }
}

let truncation: { returned: number; total: number; truncated: boolean } | null = null

/**
 * Record what a bounded response did, and phrase the banner.
 *
 * **Ported to Rust** as `truncation_banner` in
 * `crates/kglite-visual-core/src/render/encoding.rs`, wording included — the
 * headless render draws these exact words INTO the image, because an image
 * travels without its response (plan D13).
 *
 * D5: a truncated answer that does not say so reads as a complete one, so this
 * runs on every bounded response — not only the truncated ones — and the banner
 * text is what `window.__kglv` reports, so a test asserts the words the user
 * actually sees rather than a boolean beside them.
 */
function noteTruncation(
  truncated: boolean,
  returned: number,
  total: number,
  unit: string,
  links: BoundInfo | null = null,
): void {
  truncation = { returned, total, truncated }
  const count = (n: number): string => n.toLocaleString('en-US')
  const clauses: string[] = []
  if (truncated) clauses.push(`${count(returned)} of ${count(total)} ${unit}`)
  // The node list can be complete while the link list is not: nodes and links
  // share one byte budget in core, and a dense relationship spends it on links.
  // A slice that says nothing here reads as "these nodes have no other edges".
  // "up to" because the server's link total counts every edge its walk found
  // and refused, and a both-directions walk can find one edge from either end.
  if (links !== null && links.truncated) {
    clauses.push(`${count(links.returned)} of up to ${count(links.total)} links`)
  }
  truncationBanner = clauses.length === 0 ? null : `showing ${clauses.join(' and ')}`
  syncCounts()
}

function showSummaryPanel(meta: MetaGraphMeta): void {
  const panel = document.createElement('div')
  panel.className = 'kglv-panel'
  const inner = document.createElement('div')
  inner.className = 'kglv-panel-inner'
  const heading = document.createElement('h1')
  heading.textContent = `${meta.stats.core_type_count.toLocaleString('en-US')} node types — too many to draw`
  inner.appendChild(heading)

  const list = document.createElement('dl')
  for (const [label, value] of [
    ['nodes', meta.stats.node_count],
    ['edges', meta.stats.edge_count],
    ['node types', meta.stats.node_type_count],
    ['relationship types', meta.stats.relationship_type_count],
  ] as const) {
    const dt = document.createElement('dt')
    dt.textContent = label
    const dd = document.createElement('dd')
    dd.textContent = value.toLocaleString('en-US')
    list.append(dt, dd)
  }
  inner.appendChild(list)
  panel.appendChild(inner)
  root.appendChild(panel)
}

/**
 * Fetch the per-node values an appearance channel needs.
 *
 * Through the ordinary Cypher path, over the ids currently on screen — the
 * statistics say what a property *looks like* across the type, and colouring
 * needs each node's own value. Bounded because the id list is bounded: nothing
 * on screen got there except through a bounded response.
 */
function requestAppearanceValues(property: string, channel: 'color' | 'size'): void {
  const ids: number[] = []
  for (const slot of view.liveSlots()) {
    const label = view.label(slot)
    if (label?.nodeId != null) ids.push(label.nodeId)
  }
  if (ids.length === 0) {
    redraw()
    return
  }
  appearanceChannel = channel
  send({
    type: 'cypher',
    query: `MATCH (n) WHERE id(n) IN $ids RETURN id(n) AS id, n.${property} AS value`,
    params: { ids },
    limit: null,
    as_graph: false,
  })
}

function absorbAppearanceValues(
  columns: string[],
  data: unknown[][],
  channel: 'color' | 'size',
): void {
  const idColumn = data[columns.indexOf('id')] ?? []
  const valueColumn = data[columns.indexOf('value')] ?? []
  for (const [row, rawId] of idColumn.entries()) {
    const slot = view.slotForNode(Number(rawId))
    if (slot === undefined) continue
    const value = valueColumn[row]
    if (channel === 'color') {
      appearanceValues.set(slot, value)
    } else {
      const numeric = Number(value)
      if (Number.isFinite(numeric)) sizeValues.set(slot, numeric)
    }
  }
  redraw()
}

function fail(message: string): void {
  debugState.error = message
  renderStatus()
  console.error(`kglite-visual: ${message}`)
}

function renderStatus(): void {
  const lines: string[] = []
  lines.push(connectedAtom.get() ? 'connected' : 'disconnected')
  if (debugState.tier !== null) lines.push(`tier ${debugState.tier}`)
  if (lastMeta !== null) {
    lines.push(
      `${lastMeta.stats.node_count.toLocaleString('en-US')} nodes / ` +
        `${lastMeta.stats.edge_count.toLocaleString('en-US')} edges`,
    )
    // D5: a truncated answer that does not say so reads as a complete one.
    if (lastMeta.node_bound.truncated) {
      lines.push(
        `<span class="kglv-warn">showing ${lastMeta.node_bound.returned} of ` +
          `${lastMeta.node_bound.total} types</span>`,
      )
    }
    if (lastMeta.edge_bound.truncated) {
      lines.push(
        `<span class="kglv-warn">showing ${lastMeta.edge_bound.returned} of ` +
          `${lastMeta.edge_bound.total} relationships</span>`,
      )
    }
  }
  if (view.slotCount > 0) {
    const drawn = `${view.liveCount.toLocaleString('en-US')} drawn`
    const dead =
      view.tombstoneCount > 0
        ? ` / ${view.tombstoneCount.toLocaleString('en-US')} collapsed`
        : ''
    lines.push(`${drawn}${dead}`)
  }
  if (truncationBanner !== null) {
    lines.push(
      `<span class="kglv-warn" data-testid="truncation-banner">${escapeHtml(truncationBanner)}</span>`,
    )
  }
  if (!debugState.deviceFeatures.webgl2) {
    lines.push('<span class="kglv-error">no WebGL2 on this device</span>')
  }
  if (debugState.error !== null) {
    lines.push(`<span class="kglv-error">${escapeHtml(debugState.error)}</span>`)
  }
  status.innerHTML = lines.join('<br>')
}

function escapeHtml(text: string): string {
  const holder = document.createElement('span')
  holder.textContent = text
  return holder.innerHTML
}

renderStatus()
