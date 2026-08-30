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
  categoricalLegend,
  compileCategoricalColor,
  compileNumericSize,
  fillColors,
  HIGHLIGHT_COLOR,
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
import type { LayoutKernel } from './generated/LayoutKernel'
import type { ExpansionPreview } from './generated/ExpansionPreview'
import type { MetaGraphMeta } from './generated/MetaGraphMeta'
import type { NodeDetail } from './generated/NodeDetail'
import type { PropertyStat } from './generated/PropertyStat'
import type { PropertyStatsResponse } from './generated/PropertyStatsResponse'
import type { Request } from './generated/Request'
import { InteractionState } from './interaction'
import {
  filterLine,
  matches,
  parseFilter,
  unknownKeys,
  type FilterTerm,
  type SlotFacts,
} from './filter'
import { LabelOverlay } from './labels'
import { Legend, type LegendEntry, type LegendSection } from './legend'
import { formatCell, Panels } from './panels'
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
 * The mode the page *starts* in, and — for `deterministic` — the mode it stays
 * in forever. `force` is the one a user can leave: picking a server-computed
 * kernel moves the live mode to `static` and back (plan E5), which is a runtime
 * change to two axes rather than a reconstruction, because cosmos.gl takes
 * `enableSimulation` and `enableDrag` through `setConfigPartial`.
 *
 * `__kglv` reports the live mode because `positionsHash` only means something
 * where nothing is moving the points.
 */
const startupMode = layoutModeFromSearch(window.location.search)
const layoutAxes = axesFor(startupMode)
/**
 * The kernel the SHARED view is in, as the server last reported it.
 *
 * Never set from a click: the picker asks, the server answers, and the answer
 * is what this holds. A kernel with nothing to work with falls back, and an
 * agent or a second tab can change it without this one asking.
 */
let layoutKernel: LayoutKernel = 'simulation'
/** The kernel a request is in flight for, so a refusal lands on the picker. */
let pendingLayoutKernel: LayoutKernel | null = null
debugState.layoutMode = startupMode
debugState.layoutKernel = layoutKernel
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
/**
 * Over the canvas, bottom-left, opposite the status block.
 *
 * Collapsed to start: on the entry screen the encoding is structural and
 * "bigger circle = more members" is legible without being told. It opens on the
 * first colour-by choice, which is the point at which a colour means something
 * only the person who picked it knows.
 */
const legend = new Legend(root)
const assembler = new ResponseAssembler()
const transport = new WebSocketTransport('ws')

let surface: Surface | null = null
let lastMeta: MetaGraphMeta | null = null
let lastPreview: ExpansionPreview | null = null
let lastDetail: NodeDetail | null = null
/** The banner the last bounded response produced, or null when nothing was clipped. */
let truncationBanner: string | null = null
/**
 * Whether the next settle should reframe the camera.
 *
 * **A fit belongs to a changed node SET, not to every settle.** The
 * `onSimulationEnd` handler used to fit unconditionally, which was right while
 * a settle only ever followed a mount or an expansion. Restarting the
 * simulation after a static layout produces one too — and a fit there is a
 * camera that jumps for no reason the user can connect to what they clicked,
 * which is exactly the "unexplained jump" this flag exists to remove. Set by
 * the mount and by every slice; cleared by the settle that consumes it and by
 * the switch back to the simulation.
 */
let fitOnSettle = true

/** Appearance state: a compiled getter plus the values it reads. */
let colorByStat: PropertyStat | null = null
let sizeByName: string | null = null
const appearanceValues = new Map<number, unknown>()
/** Values for the size channel, keyed by slot. Separate array, separate query. */
const sizeValues = new Map<number, number>()
/**
 * The property each type's nodes are captioned by, or `null` for the title
 * kglite chose (plan E11).
 *
 * Per TYPE, because that is the scope the question has: "what do you call a
 * wellbore" is a different question from "what do you call a field", and the
 * statistics that answer it arrive per type. Seeded from the server's
 * `caption_candidate` the first time a type's statistics land, and overridden
 * from the panel after that — an entry present here is a decision, so a later
 * arrival of the same type's stats does not overwrite it.
 */
const captionByType = new Map<string, string | null>()
/** The caption value of each slot whose type has one, keyed by slot. */
const captionValues = new Map<number, string>()

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
  setLayoutKernel: (kernel) => requestLayout(kernel),
  setFilter: (query) => applyFilter(query),
  setCaptionBy: (nodeType, property) => applyCaptionBy(nodeType, property),
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

// The fixture mode has no picker: its positions are the server's lattice and
// `positionsHash` is asserted against them, so a control that replaced them
// would be a button for breaking the suite.
if (startupMode === 'deterministic') panels.hideLayoutPicker()

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
  // A colour the user chose is the one encoding nobody can read off the
  // picture, so this is the moment the card earns its space.
  if (property !== null) legend.open()
  if (property === null) {
    redraw()
    return
  }
  requestValues(property, 'color')
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
  requestValues(property, 'size')
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
 * Ask for an arrangement. The answer arrives as a broadcast, like every other
 * change to the shared view.
 *
 * A single selected slot rides along as the radial seed: "centre the rings on
 * the thing I have selected" is what a user picking hop rings means, and the
 * server ignores the hint for every other kernel.
 */
function requestLayout(kernel: LayoutKernel): void {
  const selected = interaction.selectedSlots()
  pendingLayoutKernel = kernel
  send({
    type: 'layout',
    kernel,
    seed_slot: selected.length === 1 ? (selected[0] as number) : null,
  })
}

/**
 * Take a server-computed arrangement, whoever asked for it (plan E5).
 *
 * **`'server'` authority for this one upload.** The standing authority in force
 * mode is `gpu`, which is right for a slice — the server's lattice is a seed,
 * and a seed for a slot already on screen is stale. It is exactly wrong here:
 * every slot the layout covers is already drawn, so the merge would discard the
 * whole answer and the picture would not move. That is pre-mortem #1. The mode
 * switch below also sets that authority, and the e2e shows either alone is
 * enough (see `Surface.positionsFor`); the argument stays because it is the
 * half that holds whatever order the two land in.
 *
 * **The simulation is stopped before the positions land, and restarted after
 * nothing else is pending.** `Surface.setAxes` owns that ordering; what is
 * owned here is not fitting the camera on the settle that a restart produces —
 * see {@link fitOnSettle}.
 */
function applyLayout(message: LayoutMessage): void {
  pendingLayoutKernel = null
  const chosen = message.meta.kernel_chosen
  const previous = layoutKernel
  layoutKernel = chosen
  debugState.layoutKernel = chosen
  panels.showLayoutKernel(message.meta.kernel_requested, chosen, message.meta.live_count)
  if (surface === null) return

  if (chosen === 'simulation') {
    if (previous === 'simulation') return
    // Back to the GPU. The static layout on screen is the seed the simulation
    // starts from, so nothing is uploaded — and the settle this produces must
    // not move the camera, because the user asked for a different layout, not
    // for a different view of it.
    fitOnSettle = false
    if (startupMode !== 'deterministic') {
      debugState.layoutMode = 'force'
      surface.setAxes(axesFor('force'))
    }
    positionLabels(surface)
    syncCounts()
    return
  }

  // `deterministic` keeps its name: it is the mode the e2e suite asserts by,
  // its axes are already what a static layout wants, and a broadcast that
  // arrived from an agent must move the points without moving the mode.
  if (startupMode !== 'deterministic') {
    debugState.layoutMode = 'static'
    surface.setAxes(axesFor('static'))
  }
  view.applyLayout(message.points)
  redraw('server')
}

/** Drive both appearance channels from a remote command. */
function applyAppearance(command: AppearanceCommand): void {
  panels.setAppearanceSelection(command.color_by, command.size_by)
  applyColorBy(command.color_by)
  applySizeBy(command.size_by)
}

/** The property statistics behind the dropdowns, by property name. */
const lastStats = new Map<string, PropertyStat>()

/** One outstanding per-node value fetch. */
type ValueRequest = { channel: 'color' | 'size' | 'caption'; property: string; ids: number[] }
/**
 * The value fetch currently on the wire, and the ones waiting behind it.
 *
 * A queue rather than a single slot, because the caption channel made
 * concurrency real: selecting a type fetches its statistics, which can start a
 * caption fetch while a colour-by fetch from the previous selection is still
 * out — and the results are told apart only by which request was in flight.
 * Two at once would absorb one answer into the wrong channel, which is a
 * mis-coloured graph with nothing on screen saying so.
 */
let inFlightValues: ValueRequest | null = null
const pendingValues: ValueRequest[] = []

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
      // The node set changed, so the layout has new work to do — and *which*
      // work depends on who owns the layout.
      //
      // Under the simulation it is a reheat, on a slice and nowhere else: an
      // appearance change that re-energised the layout would make the graph
      // jump under the user's cursor for no reason they could name.
      //
      // Under a static kernel there is nothing to reheat, and merging is not
      // an option either: the new slots arrived on the server's *lattice*, so
      // an expansion would drop a spiral of dots into the middle of a hop ring
      // and the picture would stop being the arrangement the user chose. So
      // the whole layout is re-requested. That is also the compaction answer —
      // a remap renumbers every slot, which makes the cached arrangement name
      // the wrong nodes — and a compaction always arrives on a slice, so one
      // path covers both.
      if (layoutKernel === 'simulation') {
        fitOnSettle = true
        surface?.reheat()
      } else {
        requestLayout(layoutKernel)
      }
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
      if (inFlightValues !== null) {
        const request = inFlightValues
        inFlightValues = null
        absorbValues(table.columns, table.data, request)
        drainValueRequests()
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
      adoptCaption(completed.value)
      const [candidates, approximate] = panels.showPropertyStats(
        completed.value,
        captionByType.get(completed.value.node_type) ?? null,
      )
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
      // A failed value fetch must not wedge the queue: the next channel's
      // answer would otherwise be absorbed as this one's.
      if (inFlightValues !== null) {
        inFlightValues = null
        drainValueRequests()
      }
      // …and a *layout* refusal belongs under the layout picker, not in the
      // Cypher card. The wire carries no request id, so the in-flight kernel is
      // the correlation — the same trick the value queue uses above, and it
      // is enough because a refusal ends the one request that was outstanding.
      if (pendingLayoutKernel !== null) {
        pendingLayoutKernel = null
        // Put the picker back on the kernel that is actually in force first —
        // a select left showing a layout the server refused is the same lie
        // `showLayoutKernel` exists to prevent — and then say why.
        panels.showLayoutKernel(layoutKernel, layoutKernel, view.liveCount)
        panels.showLayoutError(completed.value)
        break
      }
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
  // Before the arrays are compiled, and on every redraw rather than only on a
  // keystroke: the view moves underneath a filter, and slots an expansion just
  // added have never been matched against the terms.
  recomputeFilter()
  surface.upload(view, appearance(), authority)
  interaction.apply(surface.graph)
  refreshLabelSpecs()
  positionLabels(surface)
  legend.update(legendSections())
  debugState.legendEntries = legend.entryCount
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
    // A tombstone and a filtered-out node draw the same nothing. The filter's
    // half is reversible and the tombstone's is not, but the appearance arrays
    // cannot tell them apart and do not need to.
    if (label === undefined || hiddenSlots.has(slot)) {
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
    // A link to a node that is not drawn is a line into empty space. cosmos.gl
    // fades a link whose endpoint is NaN, but a filtered node's position is
    // perfectly valid — it is only invisible — so the width has to say so.
    if (hiddenSlots.has(source) || hiddenSlots.has(target)) {
      counts[link] = -1
      continue
    }
    const count = view.typeLinkWeight(source, target)
    counts[link] = count
    heaviest = Math.max(heaviest, count)
  }
  for (let link = 0; link < widths.length; link += 1) {
    const count = counts[link] ?? 0
    widths[link] = count < 0 ? 0 : linkWidth(count, heaviest)
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
  // Size zero already stops the point drawing; alpha zero is the belt to that
  // brace, because a size the renderer clamps to a floor would otherwise leave
  // a coloured speck where the filter said there was nothing.
  if (hiddenSlots.has(slot)) return HIDDEN_COLOR
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

/**
 * Take the server's caption suggestion the FIRST time a type is described.
 *
 * First time only, because an entry in the map is a decision — the server's or
 * the user's — and a second arrival of the same type's statistics (every
 * re-selection sends one) would otherwise walk a manual override back to the
 * heuristic while the user watched.
 */
function adoptCaption(stats: PropertyStatsResponse): void {
  if (captionByType.has(stats.node_type)) return
  captionByType.set(stats.node_type, stats.caption_candidate)
  if (stats.caption_candidate !== null) {
    requestValues(stats.caption_candidate, 'caption', stats.node_type)
  }
}

/**
 * Caption this type's nodes by a different property, or by the title again.
 *
 * **No slice is re-sent.** The nodes are already on screen with the identity
 * the server gave them; what changes is the string this client draws over each
 * one, so the fetch is one column of values through the ordinary Cypher path —
 * the same route the colour and size channels take.
 */
function applyCaptionBy(nodeType: string, property: string | null): void {
  captionByType.set(nodeType, property)
  // Drop this type's stored captions before the new ones land, or the labels
  // would keep the previous property's values until the round trip returns.
  for (const slot of view.liveSlots()) {
    if (view.label(slot)?.nodeType === nodeType) captionValues.delete(slot)
  }
  if (property === null) {
    redraw()
    return
  }
  requestValues(property, 'caption', nodeType)
}

/**
 * The text a slot's label carries.
 *
 * The caption when this client has fetched one for that node, and the stored
 * title otherwise — so a type with no caption chosen, and a node whose caption
 * property is empty, both keep the name the graph actually holds.
 */
function slotCaption(slot: number): string {
  return captionValues.get(slot) ?? view.label(slot)?.text ?? ''
}

/**
 * The filter box's terms, and the slots they are hiding (plan E7).
 *
 * Hidden, not removed: the nodes are still loaded, still in the slot space,
 * still what the server believes is on screen. What changes is what this client
 * DRAWS — sizes and colours to nothing, link widths to zero, no label, and the
 * interaction sets projected without them. Tombstoning instead would be a
 * protocol-level operation that destroys edges, triggers compaction, and makes
 * "clear the filter" a re-fetch.
 */
let filterTerms: FilterTerm[] = []
let hiddenSlots: ReadonlySet<number> = new Set()

/**
 * Property values this client has actually fetched, by lower-cased name.
 *
 * The whole of what a filter can match on beyond titles and types — and the
 * reason a term naming anything else is refused rather than served: answering
 * it would mean a query, and a filter that fetches is a search wearing the
 * wrong label (plan E7).
 */
function loadedPropertyKeys(): Set<string> {
  const keys = new Set<string>()
  if (colorByStat !== null) keys.add(colorByStat.name.toLowerCase())
  if (sizeByName !== null) keys.add(sizeByName.toLowerCase())
  return keys
}

/** What one slot offers the matcher, from what this client already holds. */
function slotFacts(slot: number): SlotFacts {
  const label = view.label(slot)
  const values = new Map<string, unknown>()
  if (colorByStat !== null && appearanceValues.has(slot)) {
    values.set(colorByStat.name.toLowerCase(), appearanceValues.get(slot))
  }
  if (sizeByName !== null && sizeValues.has(slot)) {
    values.set(sizeByName.toLowerCase(), sizeValues.get(slot))
  }
  // The caption, not the stored title: the box hides what does not match, and
  // what a user matches against is the name they can read on the screen.
  return { text: slotCaption(slot), nodeType: label?.nodeType ?? null, values }
}

/**
 * Recompute what the filter hides, and say so.
 *
 * Runs on every keystroke and again on every redraw, because the *view* moves
 * underneath a filter: an expansion brings slots the terms have never been
 * applied to, and leaving them visible would make the box quietly stop meaning
 * what it says.
 */
function applyFilter(query: string): void {
  filterTerms = parseFilter(query)
  recomputeFilter()
  redraw()
}

function recomputeFilter(): void {
  const refused = unknownKeys(filterTerms, loadedPropertyKeys())
  if (filterTerms.length === 0 || refused.length > 0) {
    // A refused term hides NOTHING. Applying the terms it could answer would
    // be filtering on less than the user typed while looking like it worked —
    // the failure the refusal exists to prevent, arriving one term later.
    hiddenSlots = new Set()
    interaction.setHidden(hiddenSlots)
    panels.showFilterState(null, refused)
    return
  }
  const hidden = new Set<number>()
  for (const slot of view.liveSlots()) {
    if (!matches(filterTerms, slotFacts(slot))) hidden.add(slot)
  }
  hiddenSlots = hidden
  interaction.setHidden(hiddenSlots)
  panels.showFilterState(filterLine(view.liveCount - hidden.size, view.liveCount), [])
}

/**
 * Describe the encoding currently on screen (plan E11).
 *
 * Built from the same state `appearance()` fills its arrays from, one function
 * below it, so the two cannot drift: the categorical swatches come from
 * `categoricalLegend` — the palette's own assignment — and the structural ones
 * are the literals `baseColor` returns, cited there.
 *
 * The colour section is the one that earns the card. Size and links are listed
 * because a reader who has just learned that colour means something reasonably
 * asks what the other two channels mean, and "nothing you chose" is an answer.
 */
function legendSections(): LegendSection[] {
  const sections: LegendSection[] = []
  const palette = colorByStat === null ? null : categoricalLegend(colorByStat)

  if (colorByStat !== null && palette !== null) {
    const entries: LegendEntry[] = palette.map(({ value, color }) => ({
      color,
      label: formatCell(value) === '' ? '(no value)' : formatCell(value),
    }))
    // Only when something on screen actually landed there. An "other" row on a
    // view where every node matched a value is a swatch for the empty set.
    const covered = new Set(palette.map(({ value }) => JSON.stringify(value ?? null)))
    const hasOther = [...appearanceValues.values()].some(
      (value) => !covered.has(JSON.stringify(value ?? null)),
    )
    const unset = view.liveSlots().some((slot) => !appearanceValues.has(slot))
    if (hasOther || unset) entries.push({ color: UNSET_COLOR, label: 'not set / other' })
    sections.push({
      title: `colour — ${colorByStat.name}`,
      entries,
      note: colorByStat.approx ? 'these values are approximate (kglite sampled)' : undefined,
    })
  } else if (colorByStat !== null) {
    // The compiler refused to build a palette, so nothing IS coloured by it.
    // Saying so is the whole job: a legend that listed the values anyway would
    // be captioning colours the canvas does not carry.
    sections.push({
      title: `colour — ${colorByStat.name}`,
      entries: [{ color: UNSET_COLOR, label: 'not coloured by this property' }],
      note: 'its distinct values are a lower bound, so some nodes would silently get no colour',
    })
  } else {
    // The structural encoding. These four literals mirror `baseColor` above.
    const entries: LegendEntry[] = [
      { color: [0.35, 0.65, 0.98, 0.92], label: 'type' },
      { color: [0.98, 0.75, 0.32, 0.92], label: 'type with capabilities (ts/geo/loc/vec)' },
      { color: [0.35, 0.65, 0.98, 0.92 * 0.45], label: 'supporting type — quieter, and smaller' },
    ]
    for (const type of instanceTypesOnScreen()) {
      entries.push({ color: typeHue(type), label: `${type} (instance)` })
    }
    sections.push({ title: 'colour — structural', entries })
  }

  if (interaction.highlightedSlots().length > 0) {
    sections.push({
      title: 'found',
      entries: [{ color: HIGHLIGHT_COLOR, label: 'search or query hit' }],
    })
  }

  sections.push({
    title: 'size',
    entries: [],
    note:
      sizeByName === null
        ? 'a type circle grows with its member count (log); an instance node is fixed'
        : `node size is ${sizeByName}`,
  })
  sections.push({
    title: 'links',
    entries: [
      { color: [0.55, 0.6, 0.68, 0.9], label: 'thicker = more edges between the two types', line: true },
    ],
  })
  return sections
}

/** Instance types currently drawn, ascending — one legend row each. */
function instanceTypesOnScreen(): string[] {
  const types = new Set<string>()
  for (const slot of view.liveSlots()) {
    const label = view.label(slot)
    if (label !== undefined && !label.isType && label.nodeType !== null) types.add(label.nodeType)
  }
  return [...types].sort()
}

/** A slot the filter is hiding: no size, and no colour either. */
const HIDDEN_COLOR: Rgba = [0, 0, 0, 0]

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
      if (fitOnSettle) current.graph.fitView(0)
      fitOnSettle = false
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
    view.liveSlots().filter((slot) => !hiddenSlots.has(slot)).map((slot) => {
      const label = view.label(slot)
      return {
        slot,
        text: slotCaption(slot),
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
    if (hiddenSlots.has(slot)) continue
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
  // Points that draw something, which a filter changes: a count that kept
  // reporting hidden nodes would be the instrument every agent and every e2e
  // assertion reads, describing a screen nobody is looking at.
  debugState.pointCount = view.liveCount - hiddenSlots.size
  debugState.filteredOut = hiddenSlots.size
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
 * Fetch the per-node values a display channel needs.
 *
 * Through the ordinary Cypher path, over the ids currently on screen — the
 * statistics say what a property *looks like* across the type, and colouring,
 * sizing or captioning needs each node's own value. Bounded because the id list
 * is bounded: nothing on screen got there except through a bounded response.
 *
 * `ofType` narrows it to one type's nodes, which is what the caption channel
 * wants: a caption is a per-type decision, and asking every node on screen for
 * `n.wlbWellboreName` would spend the round trip on the nodes that have no such
 * property.
 */
function requestValues(
  property: string,
  channel: 'color' | 'size' | 'caption',
  ofType: string | null = null,
): void {
  const ids: number[] = []
  for (const slot of view.liveSlots()) {
    const label = view.label(slot)
    if (label?.nodeId == null) continue
    if (ofType !== null && label.nodeType !== ofType) continue
    ids.push(label.nodeId)
  }
  if (ids.length === 0) {
    redraw()
    return
  }
  pendingValues.push({ channel, property, ids })
  drainValueRequests()
}

/** Send the next fetch, if nothing is already out. */
function drainValueRequests(): void {
  if (inFlightValues !== null) return
  const next = pendingValues.shift()
  if (next === undefined) return
  inFlightValues = next
  send({
    type: 'cypher',
    query: `MATCH (n) WHERE id(n) IN $ids RETURN id(n) AS id, n.${next.property} AS value`,
    params: { ids: next.ids },
    limit: null,
    as_graph: false,
  })
}

function absorbValues(columns: string[], data: unknown[][], request: ValueRequest): void {
  const idColumn = data[columns.indexOf('id')] ?? []
  const valueColumn = data[columns.indexOf('value')] ?? []
  for (const [row, rawId] of idColumn.entries()) {
    const slot = view.slotForNode(Number(rawId))
    if (slot === undefined) continue
    const value = valueColumn[row]
    if (request.channel === 'color') {
      appearanceValues.set(slot, value)
    } else if (request.channel === 'size') {
      const numeric = Number(value)
      if (Number.isFinite(numeric)) sizeValues.set(slot, numeric)
    } else if (typeof value === 'string' && value !== '') {
      // An empty or absent caption is left out rather than stored: the label
      // then falls back to the title, which is a real name, where a blank chip
      // would be a node the user cannot address at all.
      captionValues.set(slot, value)
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
  // The filter's own honesty line, in the truncation banner's voice and beside
  // it, because they say the same kind of thing: you are not looking at all of
  // it. Its own testid, because the two have different causes and a test that
  // could not tell them apart would pass on either.
  const filtered = filterLine(view.liveCount - hiddenSlots.size, view.liveCount)
  if (filtered !== null) {
    lines.push(
      `<span class="kglv-warn" data-testid="filter-banner">${escapeHtml(filtered)}</span>`,
    )
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
