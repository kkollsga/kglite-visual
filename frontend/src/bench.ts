/**
 * `window.__kglvBench` — the two things a bench harness cannot get from outside.
 *
 * The performance protocol says numbers come from driving the real app in a
 * production build, so the harness loads *this* bundle rather than a bench page
 * that imports the renderer separately. Two facts are then unreachable from a
 * Playwright script, and only two:
 *
 * 1. **The live renderer.** cosmos.gl's `Graph` is a local in `main.ts`; a
 *    harness that constructed its own would be measuring a second app.
 * 2. **The first data frame.** Time-to-first-paint is a once-per-load event
 *    that has already happened by the time any external script can attach, so
 *    it has to be stamped from inside, at the moment it occurs.
 *
 * Everything else a bench needs — frame-delta capture, simulation toggling,
 * synthetic uploads, control loops — is expressible from `page.evaluate` on top
 * of `graph`, and therefore lives in the harness where it can change without a
 * rebuild. Keeping this hook at two fields is deliberate: a debug surface that
 * exposes what was convenient grows until it is a second API.
 *
 * This ships in the production bundle on purpose. A hook compiled out of the
 * build that is measured is a hook that measures a different binary — the same
 * trap as a dev-server timing, one layer down.
 */

import type { Graph } from '@cosmos.gl/graph'

export type BenchHook = {
  /** The live cosmos.gl instance, or null before the renderer mounts. */
  graph: Graph | null
  /**
   * Milliseconds from navigation start to the first frame composited with
   * graph data on it, or null if that has not happened yet.
   *
   * `performance.now()` is measured from `timeOrigin`, which is navigation
   * start, so this is the whole cold path: document, bundle, WebSocket connect,
   * meta-graph request, decode, upload, draw.
   */
  firstDataFrameMs: number | null
}

declare global {
  interface Window {
    __kglvBench: BenchHook
  }
}

const hook: BenchHook = { graph: null, firstDataFrameMs: null }

export function publishBenchHook(): void {
  window.__kglvBench = hook
}

/**
 * Record the renderer and stamp the first data frame.
 *
 * **Double `requestAnimationFrame`, not one.** A rAF callback runs *before* the
 * frame it belongs to is composited, so a single one stamps the moment the
 * browser decided to draw rather than the moment anything appeared. The second
 * callback cannot run until the first frame has gone out, which makes the
 * stamp describe a frame the user could have seen.
 */
export function markRendererMounted(graph: Graph): void {
  hook.graph = graph
  if (hook.firstDataFrameMs !== null) return
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      hook.firstDataFrameMs = performance.now()
    })
  })
}
