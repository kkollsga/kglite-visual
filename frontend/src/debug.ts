/**
 * `window.__kglv` — the state a driving agent asserts against.
 *
 * The test plan's governing principle: agents build and verify this app, so a
 * UI only a human hand can drive cannot be gated. Playwright waits on
 * `ready === true` and compares counts and `positionsHash`; it never guesses
 * at pixels, and it never sleeps — cosmos.gl v3 initialises asynchronously and
 * renders on demand, so a static scene draws zero frames and a fixed sleep
 * proves nothing either way.
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

export type DebugState = {
  protocolVersion: number
  tier: string | null
  pointCount: number
  linkCount: number
  ready: boolean
  simRunning: boolean
  lastMessageSeq: number
  positionsHash: string | null
  deviceFeatures: DeviceFeatures
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
  pointCount: 0,
  linkCount: 0,
  ready: false,
  simRunning: false,
  lastMessageSeq: -1,
  positionsHash: null,
  deviceFeatures: { float32Renderable: false, textureBlendFloat: false, webgl2: false },
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
