/**
 * Entry point.
 *
 * P1 wires the toolchain end to end and stops: the renderer is imported and
 * the generated protocol types are compiled against, but nothing is drawn and
 * nothing is fetched. The meta-graph render, the WebSocket transport and the
 * `window.__kglv` state hook land in P2.
 */

import { Graph } from '@cosmos.gl/graph'

import type { MetaGraphSummary } from './generated/MetaGraphSummary'
import { viewAtom } from './state'
import { apiUrl } from './urls'

const mount = document.querySelector<HTMLDivElement>('#app')
if (!mount) throw new Error('#app is missing from index.html')

// A value-level reference, not just a type import: it keeps the renderer in
// the bundle, so `npm run build` actually proves the dependency links rather
// than tree-shaking it away and reporting green on an empty chunk.
if (typeof Graph !== 'function') {
  throw new Error('@cosmos.gl/graph did not export a Graph constructor')
}

// The shape the server will answer /api/meta-graph with. Written out here
// rather than left to P2 so a drift between the Rust enum and the generated
// TypeScript fails `tsc` today, not in the first session that renders it.
const placeholder: MetaGraphSummary = {
  node_types: [],
  relationship_types: [],
  truncated: false,
}

mount.textContent =
  `kglite-visual — toolchain ready (view: ${viewAtom.get()}, ` +
  `types: ${placeholder.node_types.length}, api: ${apiUrl('api/meta-graph')})`
