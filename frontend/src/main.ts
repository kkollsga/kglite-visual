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
  UNSET_COLOR,
  type Rgba,
} from './appearance'
import { markRendererMounted, publishBenchHook } from './bench'
import { debugState, probeDeviceFeatures, publishDebugState } from './debug'
import type { ExpansionPreview } from './generated/ExpansionPreview'
import type { MetaGraphMeta } from './generated/MetaGraphMeta'
import type { NodeDetail } from './generated/NodeDetail'
import type { PropertyStat } from './generated/PropertyStat'
import type { Request } from './generated/Request'
import { InteractionState } from './interaction'
import { LabelOverlay } from './labels'
import { Panels } from './panels'
import { assertLittleEndian, fnv1a, ResponseAssembler, type Completed } from './protocol'
import { mountGraph, type Appearance, type Surface } from './render'
import { connectedAtom } from './state'
import { rendersGraph } from './tiers'
import { WebSocketTransport } from './transport'
import { SlotView } from './view'

const mount = document.querySelector<HTMLDivElement>('#app')
if (!mount) throw new Error('#app is missing from index.html')

assertLittleEndian()
debugState.deviceFeatures = probeDeviceFeatures()
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

const panels = new Panels(root, {
  runQuery: (query, asGraph) => {
    if (query.trim() === '') return
    send({ type: 'cypher', query, params: {}, limit: null, as_graph: asGraph })
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
  setColorBy: (property) => {
    colorByStat = property === null ? null : (lastStats.get(property) ?? null)
    appearanceValues.clear()
    if (property === null) {
      redraw()
      return
    }
    requestAppearanceValues(property, 'color')
  },
  setSizeBy: (property) => {
    sizeByName = property
    sizeValues.clear()
    if (property === null) {
      redraw()
      return
    }
    requestAppearanceValues(property, 'size')
  },
})

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
      noteTruncation(
        meta.bound.truncated,
        meta.bound.returned,
        meta.bound.total,
        meta.kind === 'collapse' ? 'collapsed' : 'nodes',
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
    surface = await mountGraph(canvasHost, view, appearance())
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
function redraw(): void {
  if (surface === null) return
  surface.upload(view, appearance())
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
  for (let slot = 0; slot < slots; slot += 1) {
    const label = view.label(slot)
    if (label === undefined) {
      sizes[slot] = 0
      continue
    }
    if (sizeByName !== null && sizeValues.has(slot)) {
      sizes[slot] = sizeOf(sizeValues.get(slot))
    } else if (label.isType) {
      // Fourth root, not linear or square-root: a meta-graph routinely spans
      // four orders of magnitude between its largest and smallest type, and
      // both of the usual scales make the small ones invisible at that spread.
      sizes[slot] = 8 + 26 * Math.pow(label.weight, 0.25) / Math.pow(maxTypeCount(), 0.25)
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

  const widths = new Float32Array(view.linkCount)
  widths.fill(1)
  return { colors, sizes, linkWidths: widths }
}

/**
 * A slot's colour before highlighting.
 *
 * With no colour-by chosen, the one bit a type node carries is whether it
 * declares any capability, and an instance node is drawn in its own muted hue
 * so the meta-graph stays legible under an expansion.
 */
function baseColor(slot: number, colorOf: ((value: unknown) => Rgba) | null): Rgba {
  const label = view.label(slot)
  if (label === undefined) return UNSET_COLOR
  if (colorOf !== null && appearanceValues.has(slot)) return colorOf(appearanceValues.get(slot))
  if (!label.isType) return [0.55, 0.70, 0.90, 0.85]
  const plain = label.badges.length === 0
  // cosmos.gl takes 0..1 channels, not 0..255.
  return plain ? [0.35, 0.65, 0.98, 0.92] : [0.98, 0.75, 0.32, 0.92]
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
  labels.setLabels(
    view.liveSlots().map((slot) => {
      const label = view.label(slot)
      return {
        slot,
        text: label?.text ?? '',
        badges: label?.badges ?? [],
        weight: label?.weight ?? 0,
      }
    }),
  )
}

/** Re-place the already-built candidates against the current camera. */
function positionLabels(current: Surface): void {
  const graph = current.graph
  labels.update({
    sampledPoints: () => graph.getSampledPoints(),
    toScreen: (position: [number, number]) => graph.spaceToScreenPosition(position),
    radius: (index: number) => graph.getPointRadiusByIndex(index) ?? 0,
  })
}

function syncCounts(): void {
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
): void {
  truncation = { returned, total, truncated }
  truncationBanner = truncated
    ? `showing ${returned.toLocaleString('en-US')} of ${total.toLocaleString('en-US')} ${unit}`
    : null
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
