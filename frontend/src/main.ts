/**
 * Entry point: connect, decode, render the meta-graph.
 *
 * No framework, and no graph data in observable state (plan D7). The
 * comparable-repo study found the same failure in every slow graph UI —
 * putting nodes and edges in framework state turns one click into an O(V+E)
 * re-render. Payloads go from the socket into typed arrays and straight to the
 * renderer; only the small scalars in `state.ts` are ever observed.
 */

import './styles.css'

import type { Graph } from '@cosmos.gl/graph'

import { debugState, probeDeviceFeatures, publishDebugState } from './debug'
import type { MetaGraphMeta } from './generated/MetaGraphMeta'
import { LabelOverlay } from './labels'
import { assertLittleEndian, fnv1a, ResponseAssembler, type MetaGraphMessage } from './protocol'
import { mountMetaGraph } from './render'
import { connectedAtom } from './state'
import { rendersGraph } from './tiers'
import { WebSocketTransport } from './transport'

const mount = document.querySelector<HTMLDivElement>('#app')
if (!mount) throw new Error('#app is missing from index.html')

assertLittleEndian()
debugState.deviceFeatures = probeDeviceFeatures()
publishDebugState()

const root = document.createElement('div')
root.className = 'kglv-root'
mount.appendChild(root)

const canvasHost = document.createElement('div')
canvasHost.className = 'kglv-canvas'
root.appendChild(canvasHost)

const status = document.createElement('div')
status.className = 'kglv-status'
root.appendChild(status)

const labels = new LabelOverlay(root)

const assembler = new ResponseAssembler()
const transport = new WebSocketTransport('ws')

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

    switch (completed.kind) {
      case 'session':
        debugState.protocolVersion = completed.value.protocol_version
        debugState.tier = completed.value.tier
        renderStatus()
        break
      case 'meta-graph':
        void showMetaGraph(completed.value)
        break
      case 'error':
        fail(completed.value)
        break
    }
  },
})

let lastMeta: MetaGraphMeta | null = null

async function showMetaGraph(message: MetaGraphMessage): Promise<void> {
  lastMeta = message.meta
  debugState.tier = message.meta.tier
  debugState.protocolVersion = message.meta.protocol_version
  debugState.pointCount = message.meta.nodes.length
  debugState.linkCount = message.meta.edges.length
  debugState.positionsHash = fnv1a(message.points)
  renderStatus()

  if (!rendersGraph(message.meta.tier)) {
    // Tier 4: thousands of type nodes is not a picture of anything. The stats
    // panel is the honest view, and search (P3) is the way in.
    showSummaryPanel(message.meta)
    debugState.ready = true
    return
  }

  try {
    const view = await mountMetaGraph(canvasHost, message)
    attachLabels(view.graph, message)
    debugState.simRunning = view.graph.isSimulationRunning
    debugState.ready = true
  } catch (err) {
    // A device with no WebGL2 lands here. D10: an honest error, never a
    // second renderer.
    fail(
      `the renderer could not start: ${err instanceof Error ? err.message : String(err)}`,
    )
  }
}

function attachLabels(graph: Graph, message: MetaGraphMessage): void {
  labels.setLabels(
    message.meta.nodes.map((node) => ({
      slot: node.slot,
      text: node.name,
      badges: node.capabilities,
      weight: node.count,
    })),
  )
  const source = {
    sampledPoints: () => graph.getSampledPoints(),
    toScreen: (position: [number, number]) => graph.spaceToScreenPosition(position),
    radius: (index: number) => graph.getPointRadiusByIndex(index) ?? 0,
  }
  labels.update(source)
  // Labels are screen-space, so they move on every zoom and pan. `onZoom`
  // fires per frame of a zoom gesture; `onZoomEnd` catches the final resting
  // transform, which is the one a screenshot sees.
  graph.setConfigPartial({
    onZoom: () => labels.update(source),
    onZoomEnd: () => labels.update(source),
  })
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
