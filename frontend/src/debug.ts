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
   * `force` (the user default) or `deterministic` (`?deterministic=1`).
   *
   * Here because `positionsHash` is only an assertion in the second one: with
   * the simulation running, the positions on the GPU are a product of vendor
   * float behaviour and frame cadence, and a hash of them would be asserting
   * on the scheduler. A test that reads a hash without checking this field is
   * asserting on nothing.
   */
  layoutMode: 'force' | 'deterministic'
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
  hoveredSlot: null,
  emphasizedCount: 0,
  highlightedCount: 0,
  selectedCount: 0,
  previewRows: 0,
  queryRows: 0,
  searchHits: 0,
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
