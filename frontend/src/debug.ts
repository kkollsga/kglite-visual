/**
 * `window.__kglv` — the state a driving agent asserts against.
 *
 * The test plan's governing principle: agents build and verify this app, so a
 * UI only a human hand can drive cannot be gated. Playwright waits on
 * `ready === true` and compares counts and `positionsHash`; it never guesses
 * at pixels, and it never sleeps — cosmos.gl v3 initialises asynchronously and
 * renders on demand, so a static scene draws zero frames and a fixed sleep
 * proves nothing either way.
 *
 * **Every field here exists because an assertion needs it.** A debug object
 * that reports what was convenient to expose rather than what a test must check
 * grows without bound and still fails to answer the question in front of it.
 */

/** What the WebGL device actually supports, probed at startup. */
export type DeviceFeatures = {
  /** `EXT_color_buffer_float` — rendering to a float texture. */
  float32Renderable: boolean
  /**
   * `EXT_float_blend` — blending into a 32-bit float target. Phase 0 left this
   * as the open question about headless SwiftShader; the probe answers it on
   * whatever device is actually running.
   */
  textureBlendFloat: boolean
  /** False when the browser gave us no WebGL2 context at all (plan D10). */
  webgl2: boolean
}

/** The `{returned, total, truncated}` a bounded response carried (D5). */
export type Truncation = {
  returned: number
  total: number
  truncated: boolean
  /** The banner text the UI is actually showing, asserted verbatim. */
  banner: string | null
}

export type DebugState = {
  protocolVersion: number
  tier: string | null
  /**
   * `force` (the user default), `deterministic` (`?deterministic=1`), or
   * `static` (a server-computed layout is in force — plan E5).
   *
   * Here because `positionsHash` is only an assertion when the simulation is
   * off: with it running, the positions on the GPU are a product of vendor
   * float behaviour and frame cadence, and a hash of them would be asserting
   * on the scheduler. A test that reads a hash without checking this field is
   * asserting on nothing.
   *
   * `deterministic` never changes once set — it is the mode the e2e suite
   * asserts by name, and a broadcast layout arriving in it moves the points
   * without moving the mode.
   */
  layoutMode: 'force' | 'deterministic' | 'static'
  /**
   * Which kernel the shared view's arrangement came from: `simulation` (the
   * viewer's own GPU, the default), or `radial` / `islands` / `force` for one
   * the server computed.
   *
   * Separate from `layoutMode` because they answer different questions — *how*
   * the layout is driven, and *which* layout — and an agent switching kernels
   * needs to read back the second. It is also the field that makes a switch
   * observable at all: two force layouts and two island packings differ in
   * their coordinates, which is not something a test can assert on.
   */
  layoutKernel: string
  /** Points that currently draw something — tombstones excluded. */
  pointCount: number
  linkCount: number
  /** Slots allocated, tombstones included. */
  slotCount: number
  tombstoneCount: number
  ready: boolean
  simRunning: boolean
  lastMessageSeq: number
  positionsHash: string | null
  deviceFeatures: DeviceFeatures
  /** `expand` / `collapse` / `query` / `search` — what last changed the view. */
  lastSliceKind: string | null
  /** Compaction remaps applied. A collapse that reclaimed nothing leaves it. */
  compactions: number
  /** The last bounded response's truncation metadata, banner included. */
  truncation: Truncation | null
  /**
   * The camera's zoom factor, as the renderer reports it.
   *
   * Here because `focus` (plan D14) is otherwise unobservable: an agent that
   * asked the user's view to frame three slots has no other evidence the
   * camera moved, and a screenshot is not an assertion. Read back from
   * cosmos.gl rather than remembered, so it describes the renderer's state and
   * not this file's intention.
   */
  zoomLevel: number | null
  /** Slots the last `focus` command named. `[]` means "the whole view". */
  focusedSlots: number[]
  /** The property the colour channel is driven by, or null for structural. */
  colorBy: string | null
  /** The property the size channel is driven by, or null for structural. */
  sizeBy: string | null
  /** The four interaction concepts, as sizes (plan D7). */
  hoveredSlot: number | null
  emphasizedCount: number
  highlightedCount: number
  selectedCount: number
  /** Rows in the expansion-preview panel. */
  previewRows: number
  /** Rows in the query results table. */
  queryRows: number
  /** Hits in the search panel. */
  searchHits: number
  /**
   * Rows the legend is drawing (plan E11).
   *
   * The assertion that the legend tracks the *active* encoding rather than
   * being decorative: with a categorical colour-by chosen it is the value count
   * plus whatever else the card lists, and it moves when the encoding does.
   */
  legendEntries: number
  /**
   * Slots the client-side filter is hiding (plan E7).
   *
   * Beside `pointCount`, which already excludes them, because the two together
   * are the honest pair: "12 drawn" alone cannot be distinguished from a view
   * that only ever held twelve. It is also what an e2e asserts a filter
   * actually did, and what proves clearing one gave everything back.
   */
  filteredOut: number
  /** Property-stat rows offered as appearance channels, and how many are approximate. */
  appearanceCandidates: number
  approximateStats: number
  /**
   * The reason the app is not ready, when it is not. Beyond the fields a
   * driving agent asserts on, because "ready is false" without a reason turns
   * every headless failure into a bisect.
   */
  error: string | null
}

declare global {
  interface Window {
    __kglv: DebugState
  }
}

export const debugState: DebugState = {
  protocolVersion: 0,
  tier: null,
  layoutMode: 'force',
  layoutKernel: 'simulation',
  pointCount: 0,
  linkCount: 0,
  slotCount: 0,
  tombstoneCount: 0,
  ready: false,
  simRunning: false,
  lastMessageSeq: -1,
  positionsHash: null,
  deviceFeatures: { float32Renderable: false, textureBlendFloat: false, webgl2: false },
  lastSliceKind: null,
  compactions: 0,
  truncation: null,
  zoomLevel: null,
  focusedSlots: [],
  colorBy: null,
  sizeBy: null,
  hoveredSlot: null,
  emphasizedCount: 0,
  highlightedCount: 0,
  selectedCount: 0,
  previewRows: 0,
  queryRows: 0,
  searchHits: 0,
  legendEntries: 0,
  filteredOut: 0,
  appearanceCandidates: 0,
  approximateStats: 0,
  error: null,
}

export function publishDebugState(): void {
  window.__kglv = debugState
}

/**
 * Probe WebGL feature support on a throwaway context.
 *
 * Asking the extensions directly rather than through luma.gl's `device.
 * features`: cosmos.gl keeps its device private, and the two extension names
 * below are exactly what luma's `float32-renderable-webgl` and
 * `texture-blend-float-webgl` resolve to — so this answers the same question
 * without adding a dependency the app does not otherwise have.
 */
export function probeDeviceFeatures(): DeviceFeatures {
  const canvas = document.createElement('canvas')
  const gl = canvas.getContext('webgl2')
  if (!gl) {
    return { float32Renderable: false, textureBlendFloat: false, webgl2: false }
  }
  const features: DeviceFeatures = {
    webgl2: true,
    float32Renderable: gl.getExtension('EXT_color_buffer_float') !== null,
    textureBlendFloat: gl.getExtension('EXT_float_blend') !== null,
  }
  // Release the context immediately: browsers cap simultaneous WebGL contexts
  // (16 on Chrome), and the oldest is dropped when the cap is hit — which
  // would be the renderer's.
  gl.getExtension('WEBGL_lose_context')?.loseContext()
  return features
}
