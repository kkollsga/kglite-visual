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
import './workspace.css'
import { Workspace, type GraphScope } from './workspace'
import { SharedState } from './shared'
import { Filters } from './filters'
import { DataWorkspace, handleKey } from './data'
import { FieldDetails } from './field-detail'
import { ExplorationTrail } from './trail'
import { SavedViews } from './views'
import { PresentationControls, DEFAULT_PRESENTATION } from './presentation'
import { AppearanceMappingIndex } from './mapping'
import { typedText } from './cells'
import type { NodeHandle } from './generated/NodeHandle'
import type { QueryRowReferences } from './generated/QueryRowReferences'
import type { SharedWireMeta } from './generated/SharedWireMeta'
import type { SharedSnapshotMeta } from './generated/SharedSnapshotMeta'
import type { ViewReference } from './generated/ViewReference'
import type { RecordTable } from './generated/RecordTable'
import type { RecordCell } from './generated/RecordCell'

import {
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
import { filterLine } from './filter'
import { LabelOverlay } from './labels'
import { ExportCard } from './export'
import { PathBuilder } from './path'
import { Legend, type LegendEntry, type LegendSection } from './legend'
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
import { WebSocketTransport, type TransportHandlers } from './transport'
import { apiUrl } from './urls'
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

let dataWorkspace: DataWorkspace | null = null
let savedViews: SavedViews | null = null
let localSelectionEpoch = 0
const pendingRestores = new Map<string, number>()
const localHandles = new Map<string, NodeHandle>()
let remoteInspectorSlot: number | null = null
let graphScope: GraphScope = 'schema'
let schemaContext = false
const workspace = new Workspace(root, {
  destinationChanged: destination => dataWorkspace?.setPresented(destination === 'data'),
  setScope: (scope, context) => {
    graphScope = scope
    schemaContext = context
    redraw()
  },
  fitVisible: () => fitVisible(),
  zoom: (factor) => {
    if (surface === null) return
    surface.graph.zoom(surface.graph.getZoomLevel() * factor, 0)
  },
  focusSelection: () => focusSelection(),
  showSelectionRows: () => dataWorkspace?.showSelection(),
  clearSelection: () => {
    if ((shared.snapshot?.selected.length ?? 0) > 0) send({type: 'highlight', concept: 'selected', slots: []})
    clearSelection()
  },
  inspectType: (slot) => selectSlot(slot),
})
const canvasHost = workspace.canvasHost
const status = workspace.status

const labels = new LabelOverlay(workspace.graphHost, {
  // A label is the addressable handle on a node: clicking the name selects it
  // and pointing at it emphasises its neighbourhood, both without a single
  // pixel guess. Same code path as a canvas click — `onPointClick` below.
  onSelect: (slot) => {
    selectSlot(slot)
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
const legend = new Legend(workspace.graphHost)
/**
 * Above the legend, in the same corner and for the same reason: both cards are
 * about *this view* rather than about the app. The legend says what the picture
 * means; this says what leaves with it (plan E8).
 */
const exportCard = new ExportCard(workspace.graphHost, root)
const exportButton = document.createElement('button'); exportButton.type = 'button'; exportButton.className = 'kglv-button'
exportButton.textContent = 'Export'; exportButton.dataset['testid'] = 'workspace-export'
exportButton.onclick = () => exportCard.openScoped(exportButton)
workspace.exportHost.append(exportButton)
const assembler = new ResponseAssembler()
const transport = new WebSocketTransport('ws')
const shared = new SharedState()
let sharedSelected: number[] = []
let sharedHiddenEdges: ReadonlySet<number> = new Set()
let incoming = Promise.resolve()
let receivedShared = false
let resyncing = false
let sharedPositionHash: string | null = null
let requestSerial = 0
let pendingGraphFocus: string | null = null
const privateRequests = new Map<string, string>()
const pendingSharedRequests = new Map<string, string>()
const filters = new Filters(workspace.panelHosts.filters, predicates => send({type: 'subset', predicates}))
const readability = new PresentationControls(workspace.panelHosts.appearance, settings => send({type: 'presentation', ...settings}))
const mappedAppearance = new AppearanceMappingIndex()

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

let colorByName: string | null = null
let sizeByName: string | null = null
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

const fieldDetails = new FieldDetails(root, () => shared.generation)
dataWorkspace = new DataWorkspace(workspace.panelHosts.data, {
  select: handles => setTableSelection(handles),
  showGraph: handles => showEntities({nodes: handles, relationships: [], truncated: false}),
  inspectValue: (handle, field) => fieldDetails.open(handle, field),
  reveal: () => workspace.navigate('data'),
})

const panels = new Panels(workspace.panelHosts.inspector, {
  runQuery: (query, asGraph) => {
    if (query.trim() === '') return
    // The note describes a GENERATED table — which columns were capped, which
    // nodes could not be named — so it belongs to that query and to no other.
    // Cleared when the user runs their own, because a note left standing under
    // someone else's rows is a claim about a table that is not there: a
    // `PROFILE` of Fields was rendered under "12 of 18 properties, the ones
    // most FieldReserves nodes carry" before this line existed.
    panels.showTableNote(null)
    send({ type: 'cypher', query, params: {}, limit: null, as_graph: asGraph })
    // The one place a user-typed query is recorded. The app's own queries —
    // generated path queries go through `send` directly and are
    // deliberately not history.
    void refreshQueries(store.recordQuery(query))
  },
  expand: (slot, relationship, direction, limit) =>
    send({ type: 'expand', slot, relationship, direction, limit }),
  collapse: (slot) => send({ type: 'collapse', slot }),
  browseType: (nodeType, limit) => send({ type: 'browse-type', node_type: nodeType, limit }),
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
  loadHits: handles => send({type: 'load-nodes', handles}),
  focusHandle: handle => showEntities({nodes: [handle], relationships: [], truncated: false}),
  selectHandles: handles => setTableSelection(handles),
  showEntities: references => showEntities(references),
  setColorBy: (property) => send({type: 'appearance', color_by: property, size_by: shared.snapshot?.appearance.size_by ?? null}),
  setSizeBy: (property) => send({type: 'appearance', color_by: shared.snapshot?.appearance.color_by ?? null, size_by: property}),
  setLayoutKernel: (kernel) => requestLayout(kernel),
  setCaptionBy: (_nodeType, property) => send({type: 'caption', caption_by: property}),
  saveQuery: (name, query) => void refreshQueries(store.saveQuery(name, query)),
  deleteQuery: (name) => void refreshQueries(store.deleteQuery(name)),
  // Parse-only, over plain HTTP, on the editor's idle timer. It never runs the
  // query and never moves the view.
  validateQuery: (query) => validateQuery(query),
  showTypeTable: (nodeType) => showTypeTable(nodeType),
}, schema, {...workspace.panelHosts, data: dataWorkspace.queryHost, revealData: () => { dataWorkspace?.showQuery(); workspace.navigate('data') }})
const trail = new ExplorationTrail(workspace.panelHosts.inspector)
savedViews = new SavedViews(workspace.savedViewsHost, {
  selection: selectionReferences,
  selectionEpoch: () => localSelectionEpoch,
  restoreRequested: (id, epoch) => pendingRestores.set(id, epoch),
  restoreRefused: id => pendingRestores.delete(id),
})

/**
 * The path builder (plan E9), under its own heading in Query.
 *
 * Its Run goes down the same `cypher` request the Run button sends, with
 * `as_graph` on: a path is a picture, and the nodes and relationships the
 * result names are what land in the slot space. Its count probes go over HTTP
 * instead — the JSON twin answers the same struct with no side effect on the
 * results panel, which is the same reasoning `schema.ts` records for its own
 * lazy fetches. A probe that repainted the query table would overwrite whatever
 * the user was reading.
 */
const pathBuilder = new PathBuilder(document.createElement('div'), schema, {
  runPath: (query, params) => {
    panels.loadGeneratedQuery(query)
    send({ type: 'cypher', query, params, limit: null, as_graph: true })
  },
  copyToEditor: (query) => {
    panels.loadGeneratedQuery(query)
    workspace.navigate('query')
  },
  countRows: async (query, params) => {
    const response = await fetch(apiUrl('api/cypher'), {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ query, params, limit: null, as_graph: false }),
    })
    if (!response.ok) throw new Error(`the count probe was refused (${response.status})`)
    const table = (await response.json()) as { data: unknown[][] }
    const value = table.data[0]?.[0]
    if (typeof value !== 'number') throw new Error('the count probe answered no number')
    return value
  },
})
panels.addSection('Path', pathBuilder.root)

function selectSlot(slot: number): void {
  localSelectionEpoch += 1
  remoteInspectorSlot = null
  localHandles.clear()
  const handle = view.label(slot)?.handle
  if (handle != null) localHandles.set(handleKey(handle), handle)
  interaction.setSelected([slot])
  lastPreview = null
  lastDetail = null
  panels.clearSelection()
  applyInteraction()
  send({ type: 'preview', slot })
  workspace.setInspectedType(view.label(slot)?.isType === true ? slot : null)
  workspace.openInspector()
}

function clearSelection(): void {
  localSelectionEpoch += 1
  remoteInspectorSlot = null
  localHandles.clear()
  workspace.setInspectedType(null)
  interaction.setSelected([])
  lastPreview = null
  lastDetail = null
  panels.clearSelection()
  applyInteraction()
}

function selectionReferences(): ViewReference[] {
  const refs: ViewReference[] = [...localHandles.values()].map(handle => ({kind: 'node', handle}))
  for (const slot of interaction.allSelectedSlots()) {
    const label = view.label(slot)
    if (label?.isType) refs.push({kind: 'type', name: label.text})
  }
  refs.push(...shared.snapshot?.selected ?? [])
  const keyed = new Map(refs.map(ref => [ref.kind === 'type' ? `type:${ref.name}` : handleKey(ref.handle), ref]))
  return [...keyed.values()]
}

function applyRestoredSelection(id: string | null, snapshot: SharedSnapshotMeta): void {
  if (id === null) return
  const epoch = pendingRestores.get(id)
  pendingRestores.delete(id)
  if (epoch === undefined || epoch !== localSelectionEpoch) return
  localHandles.clear()
  for (const ref of snapshot.selected) if (ref.kind === 'node') localHandles.set(handleKey(ref.handle), ref.handle)
  interaction.setSelected(resolveReferences(snapshot.selected))
  remoteInspectorSlot = null; lastPreview = null; lastDetail = null; panels.clearSelection()
  const slots = interaction.allSelectedSlots()
  if (slots.length === 1) { send({type: 'preview', slot: slots[0] as number}); workspace.openInspector() }
}

function setTableSelection(handles: NodeHandle[]): void {
  const requested = new Set(handles.map(handleKey))
  const sharedRefs = shared.snapshot?.selected ?? []
  const remoteNodes = new Set(sharedRefs.flatMap(ref => ref.kind === 'node' ? [handleKey(ref.handle)] : []))
  const remaining = sharedRefs.filter(ref => ref.kind === 'type' || requested.has(handleKey(ref.handle)))
  if (remaining.length !== sharedRefs.length) send({type: 'highlight', concept: 'selected', slots: resolveReferences(remaining)})
  setLocalHandles(handles.filter(handle => !remoteNodes.has(handleKey(handle)) || localHandles.has(handleKey(handle))))
}

function presentationSlots(): number[] {
  return view.liveSlots().filter((slot) => graphScope === 'schema'
    ? view.label(slot)?.isType === true : schemaContext || view.label(slot)?.isType === false)
}

function visibleSlots(): number[] {
  return view.liveSlots().filter((slot) => !hiddenSlots.has(slot))
}

function visibleLinkCount(): number {
  let count = 0
  for (let i = 0; i < view.links.length; i += 2) {
    if (!sharedHiddenEdges.has(i / 2) && !hiddenSlots.has(view.links[i] as number) && !hiddenSlots.has(view.links[i + 1] as number)) count += 1
  }
  return count
}

function fitVisible(): void {
  const slots = visibleSlots()
  if (slots.length > 0) surface?.graph.fitViewByPointIndices(slots, 0)
}

function focusSelection(): void {
  const slots = outlinedSlots()
  if (slots.length === 0) return
  workspace.navigate('explore')
  surface?.graph.fitViewByPointIndices(slots, 0)
}

/** Selected labels participate even when the renderer's density sampler omitted them. */
function samplesWithSelection(current: Surface): { indices: number[]; positions: number[] } {
  const sampled = current.graph.getSampledPoints()
  const indices = [...sampled.indices]
  const positions = [...sampled.positions]
  const included = new Set(indices)
  for (const [slot, position] of current.graph.getTrackedPointPositionsMap()) {
    if (included.has(slot) || hiddenSlots.has(slot)) continue
    indices.push(slot)
    positions.push(position[0], position[1])
  }
  return { indices, positions }
}

function showTypeTable(nodeType: string): void {
  dataWorkspace?.showType(nodeType, [...lastStats.keys()])
}

function setLocalHandles(handles: NodeHandle[]): void {
  localSelectionEpoch += 1
  remoteInspectorSlot = null
  localHandles.clear()
  for (const handle of handles) if (handle.generation === shared.generation) localHandles.set(handleKey(handle), handle)
  interaction.setSelected([...localHandles.values()].flatMap(handle => { const slot = view.slotForHandle(handle); return slot === undefined ? [] : [slot] }))
  lastPreview = null; lastDetail = null; panels.clearSelection()
  const selected = interaction.allSelectedSlots()
  if (selected.length === 1) send({type: 'preview', slot: selected[0] as number})
  applyInteraction()
}

function showEntities(references: QueryRowReferences): void {
  const handles = [...new Map([...references.nodes, ...references.relationships.flatMap(edge => [edge.source, edge.target])].map(handle => [handleKey(handle), handle])).values()]
  if (handles.some(handle => handle.generation !== shared.generation)) { panels.showQueryError('These graph references belong to an expired session. Run the source query again.'); return }
  setLocalHandles(handles)
  graphScope = 'instances'; workspace.showInstances(); workspace.navigate('explore'); workspace.openInspector()
  redraw()
  if (references.relationships.length > 0 || handles.some(handle => view.slotForHandle(handle) === undefined)) {
    pendingGraphFocus = send({type: 'load-entities', nodes: handles, relationships: references.relationships})
  } else focusSelection()
}

function outlinedSlots(): number[] {
  return [...new Set([...interaction.selectedSlots(), ...sharedSelected.filter(slot => !hiddenSlots.has(slot))])]
}

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
  const live = new Set(visibleSlots())
  const targets = command.slots.filter((slot) => live.has(slot))
  // Duration zero: this is a jump to somewhere the agent is about to talk
  // about, not an animation, and an in-flight transition auto-pauses the
  // simulation (see `render.ts`).
  if (targets.length === 0) {
    fitVisible()
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
  surface.framePayload(view)
}

/** Drive both appearance channels from a remote command. */
function applyAppearance(command: AppearanceCommand): void {
  panels.setAppearanceSelection(command.color_by, command.size_by)
  colorByName = command.color_by; sizeByName = command.size_by
  debugState.colorBy = colorByName; debugState.sizeBy = sizeByName
  redraw()
}

/** The property statistics behind the dropdowns, by property name. */
const lastStats = new Map<string, PropertyStat>()

const valueTokens = new Map<string, number>()

const transportHandlers: TransportHandlers = {
  onStatus: (connected) => {
    connectedAtom.set(connected)
    workspace.setConnected(connected)
    if (!connected) readability.disconnected()
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
    incoming = incoming.then(() => handle(completed)).catch(error => fail(String(error)))
  },
}
transport.connect(transportHandlers)

const sharedMutations = new Set(['expand', 'collapse', 'browse-type', 'load-nodes', 'load-entities', 'reset', 'layout', 'appearance', 'presentation', 'caption', 'subset', 'focus', 'highlight'])
function send(request: Request): string {
  const request_id = `browser-${++requestSerial}`
  const mutation = sharedMutations.has(request.type) || (request.type === 'cypher' && request.as_graph)
  if (!mutation) privateRequests.set(request.type, request_id)
  else pendingSharedRequests.set(request_id, request.type)
  trail.request(request_id, request, 'slot' in request ? view.label(request.slot)?.text ?? null : null)
  transport.send(JSON.stringify({...request, request_id, expected: mutation ? shared.stamp : null}))
  return request_id
}

async function handle(completed: Completed): Promise<void> {
  const lane = ({'query-table': 'cypher', 'node-detail': 'node-detail', 'property-stats': 'property-stats', preview: 'preview', search: 'search'} as Record<string, string>)[completed.kind]
  if (lane !== undefined && completed.request_id !== undefined && privateRequests.get(lane) !== completed.request_id) return
  switch (completed.kind) {
    case 'session':
      pendingRestores.clear()
      shared.begin(completed.value.generation)
      trail.begin(completed.value.generation)
      resyncing = false
      debugState.protocolVersion = completed.value.protocol_version
      debugState.tier = completed.value.tier
      // The path builder says what a run of the size it is previewing will
      // actually do, which needs the ceiling the server enforces rather than a
      // number this file remembers.
      pathBuilder.setRowCeiling(completed.value.max_query_rows)
      workspace.setSession(completed.value)
      renderStatus()
      break
    case 'meta-graph':
      await showMetaGraph(completed.value)
      break
    case 'shared-update':
      applySharedUpdate(completed.value)
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
      if (meta.kind === 'query') {
        // A query that drew a graph left no rows, so the results card would
        // otherwise keep describing whatever ran before it.
        panels.showGraphResult(meta.bound.returned, meta.nodes.length)
      }
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
      if (meta.kind !== 'collapse' && instancesOnScreen() > 0) {
        graphScope = 'instances'
        workspace.showInstances()
      }
      redraw(undefined, true)
      if (surface !== null && !surface.axes.simulation) surface.framePayload(view)
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
      if (!interaction.allSelectedSlots().includes(completed.value.slot) && remoteInspectorSlot !== completed.value.slot) break
      lastPreview = completed.value
      if (lastDetail?.slot !== completed.value.slot) lastDetail = null
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
      if (lastPreview?.slot !== completed.value.slot) break
      lastDetail = completed.value
      if (lastPreview !== null) {
        debugState.previewRows = panels.showPreview(lastPreview, lastDetail)
      }
      send({ type: 'property-stats', node_type: completed.value.node_type })
      break
    case 'query-table': {
      const table = completed.value
      if (table.stamp !== null && table.stamp.generation !== shared.generation) break
      debugState.queryRows = panels.showQueryTable(table)
      noteTruncation(table.bound.truncated, table.bound.returned, table.bound.total, 'rows')
      renderStatus()
      break
    }
    case 'search': {
      if (completed.value.stamp !== null && completed.value.stamp.generation !== shared.generation) break
      debugState.searchHits = panels.showSearch({...completed.value, hits: completed.value.hits.map(hit => ({...hit, slot: hit.handle === null ? null : view.slotForHandle(hit.handle) ?? null}))})
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
      if (shared.snapshot !== null) panels.setAppearanceSelection(shared.snapshot.appearance.color_by, shared.snapshot.appearance.size_by)
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
    case 'error': {
      trail.refuse(completed.request_id)
      if (completed.request_id === pendingGraphFocus) pendingGraphFocus = null
      const requestKind = completed.request_id === undefined ? undefined : pendingSharedRequests.get(completed.request_id)
      if (completed.request_id !== undefined) {
        pendingSharedRequests.delete(completed.request_id)
        if (requestKind === undefined && ![...privateRequests.values()].includes(completed.request_id)) break
      }
      if (requestKind === 'presentation') { readability.error(completed.value); break }
      if (requestKind === 'subset') { filters.error(completed.conflict !== undefined ? `Shared view changed: ${completed.value}. Review and apply again.` : completed.value); break }
      if (completed.conflict !== undefined) { filters.error(`Shared view changed: ${completed.value}. Review the current filters and apply again.`); break }
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
}

function referenceForSlot(slot: number): ViewReference | null {
  const label = view.label(slot)
  if (label?.isType === true) return {kind: 'type', name: label.text}
  return label?.handle ? {kind: 'node', handle: label.handle} : null
}
function resolveReference(reference: ViewReference): number | undefined {
  if (reference.kind === 'node') return view.slotForHandle(reference.handle)
  return view.liveSlots().find(slot => view.label(slot)?.isType === true && view.label(slot)?.text === reference.name)
}
function resolveReferences(references: ViewReference[]): number[] {
  return references.flatMap(reference => { const slot = resolveReference(reference); return slot === undefined ? [] : [slot] })
}

/** One complete shared revision replaces membership, subset and encoding before a frame. */
function applySharedUpdate(message: {meta: SharedWireMeta; points: Float32Array; links: Float32Array}): void {
  const previous = shared.snapshot
  const snapshot = message.meta.snapshot
  if (lastMeta === null) return
  if (!shared.accept(snapshot)) {
    if (shared.needsResync && !resyncing) {
      resyncing = true
      transport.close()
      transport.connect(transportHandlers)
    }
    return
  }
  if (message.meta.compacted) debugState.compactions += 1
  const initialShared = !receivedShared
  receivedShared = true
  if (message.meta.request_id !== null) pendingSharedRequests.delete(message.meta.request_id)
  // Clearing or replacing a channel also invalidates reads processed at a newer revision.
  for (const [key, token] of valueTokens) valueTokens.set(key, token + 1)
  root.dataset['sharedRevision'] = snapshot.stamp.revision
  root.dataset['sharedGeneration'] = snapshot.stamp.generation
  root.dataset['subsetRevision'] = snapshot.subset_revision
  const previousHandles = new Set(view.liveSlots().flatMap(slot => { const handle = view.label(slot)?.handle; return handle == null ? [] : [handleKey(handle)] }))
  const nextHandles = new Set(snapshot.slice.nodes.map(node => handleKey(node.handle)))
  const addedNodes = [...nextHandles].filter(handle => !previousHandles.has(handle)).length
  const removedNodes = [...previousHandles].filter(handle => !nextHandles.has(handle)).length
  trail.acknowledge(message.meta, addedNodes, removedNodes)
  const firstAdmission = instancesOnScreen() === 0 && snapshot.slice.nodes.length > 0
  const topology = previous === null || previous.topology_revision !== snapshot.topology_revision
  const positionHash = fnv1a(message.points)
  const layoutChanged = previous !== null && (JSON.stringify(previous.layout) !== JSON.stringify(snapshot.layout) || (!topology && sharedPositionHash !== positionHash))
  sharedPositionHash = positionHash
  const change = {snapshot, previous, topology, layoutChanged}
  applySnapshotMembership(change, message, lastMeta)
  const encodingChanged = applySnapshotEncoding(change)
  applySnapshotLayout(snapshot)
  const emptiedByCollapse = message.meta.mutation_kind === 'collapse' && snapshot.slice.nodes.length === 0
  if (message.meta.restored || emptiedByCollapse) {
    graphScope = snapshot.slice.nodes.length > 0 ? 'instances' : 'schema'
    workspace.showGraphScope(graphScope)
  }
  if (initialShared && instancesOnScreen() > 0) { graphScope = 'instances'; workspace.showInstances() }
  if (message.meta.mutation_kind !== null) {
    debugState.lastSliceKind = message.meta.mutation_kind
    if (message.meta.mutation_kind !== 'collapse' && instancesOnScreen() > 0) { graphScope = 'instances'; workspace.showInstances() }
    if (message.meta.mutation_kind === 'query') panels.showGraphResult(snapshot.last_slice?.bound.returned ?? snapshot.slice.bound.returned, addedNodes)
  }
  const bound = snapshot.last_slice?.bound ?? snapshot.slice.bound
  noteTruncation(bound.truncated, bound.returned, bound.total, 'nodes', snapshot.last_slice?.link_bound ?? snapshot.slice.link_bound)
  filters.update(snapshot.subset)
  readability.update(snapshot.presentation, message.meta.request_id)
  exportCard.update(snapshot)
  legend.setVisible(snapshot.presentation.legend_visible)
  if (previous?.presentation.edge_opacity !== snapshot.presentation.edge_opacity) surface?.graph.setConfigPartial({linkOpacity: snapshot.presentation.edge_opacity})
  dataWorkspace?.update(snapshot)
  savedViews?.update(snapshot)
  redraw(layoutChanged || (topology && layoutKernel !== 'simulation') ? 'server' : undefined, topology)
  if (firstAdmission || emptiedByCollapse || (initialShared && instancesOnScreen() > 0) || (layoutChanged && snapshot.layout !== null)) fitVisible()
  if (message.meta.focus !== null) applyFocus(message.meta.focus)
  if (pendingGraphFocus !== null && message.meta.request_id === pendingGraphFocus) { pendingGraphFocus = null; focusSelection() }
  if (topology && layoutKernel === 'simulation') { fitOnSettle = firstAdmission; surface?.reheat() }
  if (encodingChanged || previous?.stamp.revision !== snapshot.stamp.revision) {
    for (const [type, property] of captionByType) if (property !== null) requestValues(property, 'caption', type)
  }
}

type SnapshotChange = {snapshot: SharedSnapshotMeta; previous: SharedSnapshotMeta | null; topology: boolean; layoutChanged: boolean}

function applySnapshotMembership(change: SnapshotChange, message: {meta: SharedWireMeta; points: Float32Array; links: Float32Array}, meta: MetaGraphMeta): void {
  const {snapshot, previous, topology, layoutChanged} = change
  const localSelection = interaction.allSelectedSlots().flatMap(slot => { const reference = referenceForSlot(slot); return reference === null ? [] : [reference] })
  if (topology) {
    view.replaceSnapshot(meta, snapshot.slice, message.points, message.links)
    interaction.setSelected([...new Set([...resolveReferences(localSelection.filter(reference => reference.kind === 'type')), ...[...localHandles.values()].flatMap(handle => { const slot = view.slotForHandle(handle); return slot === undefined ? [] : [slot] })])])
    panels.refreshSearch(handle => view.slotForHandle(handle) ?? null)
    if (surface !== null) interaction.hover(surface.graph, null)
    lastPreview = null
    lastDetail = null
    panels.clearSelection()
    const selected = interaction.allSelectedSlots()
    if (selected.length === 1) send({type: 'preview', slot: selected[0] as number})
    trackedPointsKey = ''
  } else if (layoutChanged) view.applyLayout(message.points)
  const visible = new Set(snapshot.subset.visible_nodes.map(handle => `${handle.generation}:${handle.node_id}`))
  filterHiddenSlots = new Set(view.liveSlots().filter(slot => {
    const handle = view.label(slot)?.handle
    return handle != null && !visible.has(`${handle.generation}:${handle.node_id}`)
  }))
  const visibleEdges = new Set(snapshot.subset.visible_edge_ids)
  sharedHiddenEdges = new Set(view.edges.flatMap((edge, index) => !edge.meta && (edge.edge_id === null || !visibleEdges.has(edge.edge_id)) ? [index] : []))
  sharedSelected = resolveReferences(snapshot.selected)
  if (message.meta.restored) applyRestoredSelection(message.meta.request_id, snapshot)
  const remoteSelectionChanged = JSON.stringify(previous?.selected) !== JSON.stringify(snapshot.selected)
  if (remoteSelectionChanged && interaction.allSelectedSlots().length === 0 && localHandles.size === 0) {
    remoteInspectorSlot = sharedSelected.length === 1 ? sharedSelected[0] as number : null
    if (remoteInspectorSlot !== null) { send({type: 'preview', slot: remoteInspectorSlot}); workspace.openInspector() }
  }
  interaction.setHighlighted(resolveReferences(snapshot.highlighted))
}

function applySnapshotEncoding(change: SnapshotChange): boolean {
  const {snapshot, previous, topology} = change
  const encodingChanged = topology || JSON.stringify(previous?.appearance) !== JSON.stringify(snapshot.appearance)
  if (encodingChanged) {
    colorByName = snapshot.appearance.color_by
    if (colorByName !== null) legend.open()
    sizeByName = snapshot.appearance.size_by

    debugState.colorBy = snapshot.appearance.color_by; debugState.sizeBy = sizeByName
    panels.setAppearanceSelection(snapshot.appearance.color_by, sizeByName)
  }
  if (encodingChanged || previous?.presentation.node_size_min !== snapshot.presentation.node_size_min || previous?.presentation.node_size_max !== snapshot.presentation.node_size_max) mappedAppearance.set(snapshot.appearance_mapping)
  panels.setCaptionSelection(snapshot.caption_by)
  const captionChanged = topology || previous?.caption_by !== snapshot.caption_by
  if (captionChanged) {
    captionValues.clear()
    if (snapshot.caption_by !== null || previous?.caption_by !== snapshot.caption_by) {
      for (const type of instanceTypesOnScreen()) captionByType.set(type, snapshot.caption_by)
    }
  }
  return encodingChanged
}

function applySnapshotLayout(snapshot: SharedSnapshotMeta): void {
  const previousKernel = layoutKernel
  layoutKernel = snapshot.layout_kernel
  debugState.layoutKernel = layoutKernel
  pendingLayoutKernel = null
  if (surface !== null && previousKernel !== layoutKernel && startupMode !== 'deterministic') {
    debugState.layoutMode = layoutKernel === 'simulation' ? 'force' : 'static'
    fitOnSettle = false
    surface.setAxes(axesFor(debugState.layoutMode))
  }
  panels.showLayoutKernel(layoutKernel, layoutKernel, view.liveCount)
}

async function showMetaGraph(message: {
  meta: MetaGraphMeta
  points: Float32Array
  links: Float32Array
}): Promise<void> {
  lastMeta = message.meta
  if (!receivedShared) view.setMetaGraph(message.meta, message.points, message.links)
  debugState.tier = message.meta.tier
  debugState.protocolVersion = message.meta.protocol_version
  debugState.positionsHash = fnv1a(message.points)
  panels.setNodeTypes(message.meta.nodes.map((node) => node.name))
  workspace.setTypes(message.meta.nodes)
  schema.setMetaGraph(message.meta)
  filters.setSchema(message.meta.nodes.map(node => node.name), [...new Set(message.meta.edges.map(edge => edge.name))])
  // After `schema.setMetaGraph`, which is where the builder's hop lists come
  // from: filling the start picker first would offer types with no hops behind
  // them.
  pathBuilder.setNodeTypes(message.meta.nodes.map((node) => node.name))
  renderStatus()
  if (receivedShared) return

  if (!rendersGraph(message.meta.tier)) {
    // Tier 4: thousands of type nodes is not a picture of anything. The stats
    // panel is the honest view, and search is the way in.
    showSummaryPanel(message.meta)
    syncCounts()
    debugState.ready = true
    return
  }

  try {
    if (surface === null) {
      surface = await mountGraph(canvasHost, view, appearance(), layoutAxes)
      attachHandlers(surface)
    }
    debugState.simRunning = surface.graph.isSimulationRunning
    redraw(undefined, true)
    fitVisible()
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
function redraw(authority?: SeedAuthority, topology = false): void {
  if (surface === null) return
  // Before the arrays are compiled, and on every redraw rather than only on a
  // keystroke: the view moves underneath a filter, and slots an expansion just
  // added have never been matched against the terms.
  recomputeProjection()
  updateTrackedPoints()
  if (topology || authority !== undefined) surface.upload(view, appearance(), authority)
  else surface.updateAppearance(appearance())
  interaction.apply(surface.graph)
  const outlined = outlinedSlots()
  surface.graph.setConfigPartial({outlinedPointIndices: outlined.length > 0 ? outlined : undefined})
  refreshLabelSpecs()
  surface.graph.render(undefined, 0)
  positionLabels(surface)
  legend.update(legendSections())
  debugState.legendEntries = legend.entryCount
  // The export's scope is the instance nodes on screen — the same set
  // `Session::export_view` walks — so the card is refreshed wherever the view
  // is, not on a click that would read a stale count.
  exportCard.setLoaded(instancesOnScreen())
  debugState.exportNodes = exportCard.count
  panels.setInstanceCounts(instanceCountsByType())
  panels.setGeoAvailable(viewHasPlaceableNodes())
  syncCounts()
  renderStatus()
}

/**
 * True while something on screen has somewhere to be.
 *
 * **Instances only, and that is the whole distinction.** A *type* is not
 * anywhere — its nodes are — so the entry screen, which is nothing but type
 * nodes, never offers the map however many of those types carry coordinates.
 * The moment an expansion or a Cypher result puts instances of such a type on
 * screen, it does.
 *
 * The test is kglite's own capability flag, read off the meta-graph the entry
 * screen already loaded: `geo` for a type with a WKT geometry, `loc` for one
 * with a lat/lon pair. They are two spellings of "this type declares where its
 * nodes are", they are independent facts (kglite 0.16.16 stopped suppressing
 * `loc` under `geo`, so a type declaring both now reports both), and testing
 * for either is what catches a type carrying only one of them. The gate is
 * unaffected by that upstream change in either direction: `a || b` does not
 * care whether `b` was being hidden. It is a *type-level* claim, so it can be
 * true for a node whose own
 * fields turn out to be null; the server draws those in its tray and says how
 * many, which is the honest place for that count to appear.
 */
function viewHasPlaceableNodes(): boolean {
  const placeable = new Set(
    (lastMeta?.nodes ?? [])
      .filter((node) => node.capabilities.includes('geo') || node.capabilities.includes('loc'))
      .map((node) => node.name),
  )
  if (placeable.size === 0) return false
  return view.liveSlots().some((slot) => {
    const label = view.label(slot)
    return label !== undefined && !label.isType && label.nodeType !== null
      ? placeable.has(label.nodeType)
      : false
  })
}

function appearance(): Appearance {
  const slots = view.slotCount
  const sizes = new Float32Array(slots)
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
    const mapped = mappedAppearance.get(label.handle)
    if (mapped?.radius != null) {
      sizes[slot] = mapped.radius
    } else if (label.isType) {
      sizes[slot] = typeRadius(label.weight, largestType, label.supporting)
    } else {
      sizes[slot] = 6
    }
  }

  const highlighted = new Set(interaction.highlightedSlots())
  const colors = fillColors(
    slots,
    (slot) => baseColor(slot),
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
    if (sharedHiddenEdges.has(link) || hiddenSlots.has(source) || hiddenSlots.has(target)) {
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
 * `crates/kglite-visual-core/src/render/encoding.rs`; explicit channel
 * overrides come from the shared core mapping used by captured images too.
 *
 * With no colour-by chosen, the two bits a type node carries are whether it
 * declares any capability and whether it is a *supporting* type — a type with
 * a parent in kglite's `is_a` forest, which is the server's own answer to
 * "which of these types is the graph actually about". A supporting type keeps
 * its hue and loses most of its opacity, so the core types it hangs off carry
 * the picture. An instance node is drawn in its own muted hue so the
 * meta-graph stays legible under an expansion.
 */
function baseColor(slot: number): Rgba {
  const label = view.label(slot)
  if (label === undefined) return UNSET_COLOR
  // Size zero already stops the point drawing; alpha zero is the belt to that
  // brace, because a size the renderer clamps to a floor would otherwise leave
  // a coloured speck where the filter said there was nothing.
  if (hiddenSlots.has(slot)) return HIDDEN_COLOR
  const mapped = mappedAppearance.get(label.handle)
  if (mapped?.color != null) return mapped.color
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
  if (shared.snapshot?.caption_by != null) {
    captionByType.set(stats.node_type, shared.snapshot.caption_by)
    requestValues(shared.snapshot.caption_by, 'caption', stats.node_type)
    return
  }
  if (captionByType.has(stats.node_type)) return
  captionByType.set(stats.node_type, stats.caption_candidate)
  if (stats.caption_candidate !== null) {
    requestValues(stats.caption_candidate, 'caption', stats.node_type)
  }
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

let filterHiddenSlots: ReadonlySet<number> = new Set()
let hiddenSlots: ReadonlySet<number> = new Set()

/** Presentation is local and never changes loaded membership or the active filter. */
function recomputeProjection(): void {
  const hidden = new Set(filterHiddenSlots)
  for (const slot of view.liveSlots()) {
    const isType = view.label(slot)?.isType === true
    if (graphScope === 'schema' ? !isType : isType && !schemaContext) hidden.add(slot)
  }
  hiddenSlots = hidden
  interaction.setHidden(hiddenSlots)
}


/** Legend rows use the core mapping and the same structural fallback as the points. */
function legendSections(): LegendSection[] {
  const sections: LegendSection[] = []
  const mapping = shared.snapshot?.appearance_mapping
  if (colorByName !== null && mapping !== undefined) {
    const entries: LegendEntry[] = mapping.categories.map(category => ({color: category.color, label: `${typedText(category.value)} · ${category.count}`}))
    const states = new Map<string, number>()
    for (const node of mapping.nodes) if (node.color_state !== null && node.color_state !== 'value') states.set(node.color_state, (states.get(node.color_state) ?? 0) + 1)
    if (mapping.other_categories > 0) entries.push({color: UNSET_COLOR, label: `other · ${mapping.other_categories} categories`})
    for (const [state, count] of states) entries.push({color: UNSET_COLOR, label: `${state} · ${count}`})
    sections.push({title: `colour — ${colorByName}`, entries, note: 'Domain: loaded instances. Visual filters keep these colors stable.'})
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
        : `node size is ${sizeByName} · loaded instances · ${mapping?.size_min == null ? 'no numeric values' : `${typedText(mapping.size_min)} to ${typedText(mapping.size_max ?? mapping.size_min)}`} · ${shared.snapshot?.presentation.node_size_min ?? 4}–${shared.snapshot?.presentation.node_size_max ?? 22} radius; missing/non-numeric values use the minimum`,
  })
  sections.push({
    title: 'links',
    entries: [
      { color: [0.55, 0.6, 0.68, 0.9], label: 'thicker = more edges between the two types', line: true },
    ],
  })
  return sections
}

/**
 * Instance nodes currently in the slot space — what an export would carry.
 *
 * Not `pointCount`: a filtered-out node is still loaded, and the server's
 * export walks the slot space rather than the client's appearance arrays. A
 * count that tracked the filter would promise a file the server will not write.
 */
function instancesOnScreen(): number {
  let count = 0
  for (const slot of view.liveSlots()) {
    if (view.label(slot)?.isType === false) count += 1
  }
  return count
}

/** Instance nodes on screen per type — what the type panel's table offers. */
function instanceCountsByType(): Map<string, number> {
  const counts = new Map<string, number>()
  for (const slot of view.liveSlots()) {
    const label = view.label(slot)
    if (label?.isType === false && label.nodeType !== null) {
      counts.set(label.nodeType, (counts.get(label.nodeType) ?? 0) + 1)
    }
  }
  return counts
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
      selectSlot(index)
    },
    onClick: (index: number | undefined) => {
      if (index !== undefined) return
      clearSelection()
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
      // zoom no longer frames it. Fitting changes only the camera; seeded
      // coordinates remain deterministic. Membership changes request a fit,
      // while appearance and destination changes preserve the user's camera.
      if (fitOnSettle) fitVisible()
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
  const outlined = outlinedSlots()
  surface.graph.setConfigPartial({outlinedPointIndices: outlined.length > 0 ? outlined : undefined})
  labels.setPinned(priorityLabelSlots())
  updateTrackedPoints()
  surface.graph.render(undefined, 0)
  positionLabels(surface)
  requestAnimationFrame(() => { if (surface !== null) positionLabels(surface) })
  syncCounts()
}

/**
 * Rebuild the candidate list from the view. Slice path only.
 *
 * O(live slots), so it belongs where the *view* changed — `redraw`, which every
 * slice funnels through.
 */
function refreshLabelSpecs(): void {
  const selected = new Set(priorityLabelSlots())
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
        schema: label?.isType === true,
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
  return graphScope === 'schema' || view.liveSlots().every((slot) => view.label(slot)?.isType === true)
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
      sampledPoints: () => (wholeSchema ? everyPoint(current) : samplesWithSelection(current)),
      toScreen: (position: [number, number]) => graph.spaceToScreenPosition(position),
      radius: (index: number) => graph.getPointRadiusByIndex(index) ?? 0,
    },
    wholeSchema,
    shared.snapshot?.presentation.label_density ?? 1,
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
  const indices: number[] = []
  const positions: number[] = []
  for (const [slot, position] of current.graph.getTrackedPointPositionsMap()) {
    if (hiddenSlots.has(slot)) continue
    indices.push(slot)
    positions.push(position[0], position[1])
  }
  return { indices, positions }
}

function priorityLabelSlots(): number[] {
  const settings = shared.snapshot?.presentation ?? DEFAULT_PRESENTATION
  const slots = settings.prioritize_selected_labels ? outlinedSlots() : []
  const hover = interaction.hoveredSlot()
  if (settings.prioritize_hovered_labels && hover !== null && !hiddenSlots.has(hover)) slots.push(hover)
  return [...new Set(slots)]
}

let trackedPointsKey = ''
function updateTrackedPoints(): void {
  if (surface === null) return
  const slots = isMetaGraphOnly() ? visibleSlots() : [...new Set([...outlinedSlots(), ...priorityLabelSlots()])]
  const key = slots.join(',')
  if (key === trackedPointsKey) return
  trackedPointsKey = key
  surface.graph.trackPointPositionsByIndices(slots)
}

function syncCounts(): void {
  // Read back from the renderer, not from whatever this file last asked for:
  // a zoom the GPU did not take is a zoom that did not happen.
  debugState.zoomLevel = surface?.graph.getZoomLevel() ?? null
  // Points that draw something, which a filter changes: a count that kept
  // reporting hidden nodes would be the instrument every agent and every e2e
  // assertion reads, describing a screen nobody is looking at.
  debugState.pointCount = view.liveCount - hiddenSlots.size
  debugState.filteredOut = [...filterHiddenSlots].filter((slot) =>
    graphScope === 'schema' ? view.label(slot)?.isType === true
      : schemaContext || view.label(slot)?.isType === false,
  ).length
  debugState.linkCount = visibleLinkCount()
  debugState.slotCount = view.slotCount
  debugState.tombstoneCount = view.tombstoneCount
  debugState.namedSlots = view.namedCount
  debugState.hoveredSlot = interaction.hoveredSlot()
  debugState.emphasizedCount = interaction.emphasizedSlots().length
  debugState.highlightedCount = interaction.highlightedSlots().length
  debugState.selectedCount = outlinedSlots().length
  const instances = view.liveSlots().filter((slot) => view.label(slot)?.isType === false)
  const references = selectionReferences()
  const selected = references.flatMap(ref => ref.kind === 'node' ? [ref.handle] : [])
  dataWorkspace?.setSelection(selected)
  panels.setRecordSelection(selected)
  workspace.setCounts({
    loaded: instances.length,
    visible: instances.filter((slot) => !filterHiddenSlots.has(slot)).length,
    selected: selected.length,
    hiddenSelected: selected.filter(handle => { const slot = view.slotForHandle(handle); return slot === undefined || hiddenSlots.has(slot) }).length,
    types: view.liveCount - instances.length,
    hasSelection: references.length > 0,
    canFocus: surface !== null && outlinedSlots().length > 0,
    sharedSelection: (shared.snapshot?.selected.length ?? 0) > 0,
  })
  savedViews?.updateSelection()
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
  panel.dataset['testid'] = 'schema-summary'
  panel.style.pointerEvents = 'none'
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
  workspace.graphHost.appendChild(panel)
}

/** Handle-based field reads preserve exact source keys and never use Cypher identity guesses. */
function requestValues(property: string, channel: 'caption', ofType: string | null = null): void {
  const key = `${channel}:${ofType ?? '*'}`
  const token = (valueTokens.get(key) ?? 0) + 1
  valueTokens.set(key, token)
  const handles = view.liveSlots().flatMap(slot => {
    const label = view.label(slot)
    return label?.handle !== null && label?.handle !== undefined && (ofType === null || label.nodeType === ofType) ? [label.handle] : []
  })
  const stamp = shared.stamp
  void (async () => {
    let offset: number | null = 0
    while (offset !== null && handles.length > 0) {
      const response = await fetch(apiUrl('api/records'), {method: 'POST', headers: {'content-type': 'application/json'}, body: JSON.stringify({handles, fields: [property], offset, limit: 500})})
      if (!response.ok) throw new Error(`Field read refused (${response.status})`)
      const table = await response.json() as RecordTable
      if (valueTokens.get(key) !== token || (stamp !== null && !shared.matches(table.stamp))) return
      for (const row of table.rows) {
        const slot = view.slotForHandle(row.handle)
        if (slot === undefined) continue
        const value = cellScalar(row.cells[0])
        if (typeof value === 'string' && value !== '') captionValues.set(slot, value)
      }
      offset = table.next_offset
    }
    if (valueTokens.get(key) === token) redraw()
  })().catch(error => { if (valueTokens.get(key) === token) fail(String(error)) })
}

function cellScalar(cell: RecordCell | undefined): unknown {
  if (cell?.state !== 'value') return null
  const value = cell.value
  if (value.type === 'null') return null
  if (value.type === 'int64' || value.type === 'unique-id') {
    const number = Number(value.value)
    return Number.isSafeInteger(number) ? number : value.value
  }
  return value.value
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
    const drawn = `${view.liveCount.toLocaleString('en-US')} loaded slots`
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
  const projected = presentationSlots()
  const filtered = filterLine(projected.filter((slot) => !filterHiddenSlots.has(slot)).length, projected.length)
  if (filtered !== null) {
    lines.push(
      `<span class="kglv-warn" >${escapeHtml(filtered)}</span>`,
    )
  }
  if (truncationBanner !== null) {
    lines.push(
      `<span class="kglv-warn" >${escapeHtml(truncationBanner)}</span>`,
    )
  }
  if (!debugState.deviceFeatures.webgl2) {
    lines.push('<span class="kglv-error">no WebGL2 on this device</span>')
  }
  if (debugState.error !== null) {
    lines.push(`<span class="kglv-error">${escapeHtml(debugState.error)}</span>`)
  }
  status.innerHTML = lines.join('<br>')
  const notices: { kind: 'filter' | 'truncation' | 'error'; message: string }[] = []
  if (filtered !== null) notices.push({ kind: 'filter', message: filtered })
  if (truncationBanner !== null) notices.push({ kind: 'truncation', message: truncationBanner })
  if (debugState.error !== null) notices.push({ kind: 'error', message: debugState.error })
  workspace.setNotices(notices)
}

function escapeHtml(text: string): string {
  const holder = document.createElement('span')
  holder.textContent = text
  return holder.innerHTML
}

renderStatus()
