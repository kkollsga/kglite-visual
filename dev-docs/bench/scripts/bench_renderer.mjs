/**
 * P4's client half: does cosmos.gl's GPU layout hold interactive frame times?
 *
 * This is the stop-rule instrument. The plan's rule, dated 2026-08-29 and
 * written before any measurement ran: *if cosmos.gl's GPU layout holds
 * interactive frame times at the largest slice the response bound permits,
 * Rust-side ForceAtlas2 is retired, not implemented.* So the cell that matters
 * is a real slice at the bound, on a real GPU, with the simulation RUNNING.
 *
 * ## What it drives, and why it is the real app
 *
 * The release CLI binary, resolved through `scripts/check_bundle.py
 * --resolve-binary` (never a hard-coded path — a stale bundle in a fresh binary
 * looks exactly like a backend bug), serving the production Vite bundle. The
 * slice is loaded the way a user loads one: click the `Person` type node, set
 * the max-nodes box, click the relationship's expand button. Nothing here
 * constructs a renderer of its own; it reaches the app's own instance through
 * `window.__kglvBench.graph`.
 *
 * ## Simulation ON — the one place fixture mode is the wrong tool
 *
 * The app renders with `enableSimulation: false` because the *tests* need
 * determinism and the server supplies positions (plan D2). The question here is
 * the opposite one: what the GPU layout costs while it is actually laying out.
 * So every frame cell below turns the simulation on through the same
 * `setConfigPartial` the app uses, against a `Graph` constructed with a fixed
 * `randomSeed`. Nothing in this file writes back into the app's view, and the
 * process is discarded after the capture.
 *
 * ## Above the bound is renderer-only, and says so
 *
 * `core::expand::effective_bound` clamps every expansion to
 * `MAX_EXPANSION_NODES`, and there is deliberately no way around it — a slice
 * of 20 000 nodes cannot be produced by the server at all. The above-bound
 * cells therefore push a synthetic array straight at the renderer and are
 * labelled `synthetic`: they answer "where does the renderer fall over relative
 * to the bound", not "what does the product do". The same synthetic path is run
 * at the in-bound sizes too, beside the real-path cells, so the two instruments
 * can be compared and a disagreement is visible rather than assumed away
 * (performance protocol §9).
 *
 * ## Statistics, per cell
 *
 * - frame period under settling and under interaction → **p95 / p99**. A
 *   renderer that is fast on its best frame and stutters on its worst is a
 *   renderer that stutters. Both the 60 fps (16.7 ms) and 30 fps (33.3 ms)
 *   lines are reported against the p95.
 * - time-to-first-paint → **mean of first events**, over `--loads` cold loads.
 *   A once-per-load cost is structurally invisible to `min`.
 * - point/link counts, the WebGL renderer string → **exact**.
 *
 * On a headed run the frame period is quantised to whole refresh intervals, so
 * a p95 of ~16.7 ms is the *pass* condition rather than a ceiling being hit by
 * accident — see `stats()` for where the two lines are drawn and why they sit
 * between buckets. `dropped_fraction` (periods past 1.5 refreshes) is reported
 * beside them so a pass cannot be confused with a stall.
 *
 * ## Controls
 *
 * A fixed integer loop runs in the page immediately before and after every
 * capture. It touches no WebGL and no graph data, so neither cosmos.gl nor
 * kglite can move it; only the machine can. It is sized to tens of
 * milliseconds so its value clears the capture's own scatter with margin.
 *
 * Usage:
 *   node dev-docs/bench/scripts/bench_renderer.mjs \
 *     --graph dev-docs/bench/out/scale-20000.kgl \
 *     --backend gpu|swiftshader --out dev-docs/bench/out/renderer-<tag>.json
 */

import { execFileSync, spawn } from 'node:child_process'
import { createInterface } from 'node:readline'
import { createRequire } from 'node:module'
import { mkdirSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
// Playwright is the frontend's devDependency, and this script lives outside
// that package, so node's own resolution would never find it. Resolving from
// frontend/package.json is what makes "the harness uses the project's pinned
// Playwright" true rather than "whatever is installed globally".
const require = createRequire(path.join(REPO, 'frontend/package.json'))
const { chromium } = require('@playwright/test')

/** Milliseconds of frame capture per phase. */
const SETTLE_MS = 3000
const INTERACT_MS = 4000
/** Iterations of the in-page control loop. */
const CONTROL_ROUNDS = 12_000_000

function parseArgs() {
  const args = {
    graph: 'dev-docs/bench/out/scale-20000.kgl',
    backend: 'gpu',
    sizes: [1000, 5000],
    synthetic: [1000, 5000, 20000],
    loads: 5,
    out: null,
    run: '1',
  }
  const raw = process.argv.slice(2)
  for (let i = 0; i < raw.length; i += 2) {
    const key = raw[i].replace(/^--/, '')
    const value = raw[i + 1]
    if (!(key in args)) throw new Error(`unknown flag --${key}`)
    if (key === 'sizes' || key === 'synthetic') args[key] = value.split(',').map(Number)
    else if (key === 'loads') args.loads = Number(value)
    else args[key] = value
  }
  return args
}

/**
 * The release binary, refused if it is older than the bundle it embeds — or if
 * it is not a release binary at all.
 *
 * `--resolve-binary` resolves *newest of profile*, so a `make e2e` (which
 * builds debug) run after a `cargo build --release` hands this script a debug
 * server, and it recorded the profile without refusing it. Debug-profile
 * timings are not evidence (`R11`), and a debug row sitting beside the release
 * rows in results.csv is a comparison waiting to be made wrongly. `capture.py`
 * has always refused it in `preconditions()`; this script is runnable on its
 * own (its own usage block says so), so it refuses it too.
 */
function resolveBinary() {
  const bin = execFileSync(
    'python3',
    [path.join(REPO, 'scripts/check_bundle.py'), '--resolve-binary', 'kglite-visual'],
    { encoding: 'utf8', cwd: REPO },
  ).trim()
  if (!bin.includes('/release/')) {
    throw new Error(
      `resolved a non-release binary (${bin}). Debug-profile timings are not ` +
        'evidence (R11); run `cargo build --release -p kglite-visual-cli` first ' +
        '(a debug build made after it wins newest-of-profile).',
    )
  }
  return bin
}

async function launchServer(graph) {
  const bin = resolveBinary()
  const child = spawn(bin, [graph, '--no-open', '--port', '0'], { cwd: REPO })
  const stderr = []
  createInterface({ input: child.stderr }).on('line', (l) => stderr.push(l))
  const info = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`no stdout line in 60s: ${stderr.join('\n')}`)), 60_000)
    createInterface({ input: child.stdout }).on('line', (line) => {
      clearTimeout(timer)
      resolve(JSON.parse(line))
    })
    child.on('exit', (code) => reject(new Error(`binary exited ${code}: ${stderr.join('\n')}`)))
  })
  return { child, info, bin }
}

/**
 * Launch options per backend.
 *
 * `gpu` is headed, because that is the only way to reach the machine's real
 * Metal driver — and the performance protocol is explicit that the number a
 * user experiences is the one on real hardware. `swiftshader` reproduces the
 * e2e suite's CI shape exactly (the same three flags), and its numbers are
 * reported as CI context, never as the stop-rule basis.
 */
function launchOptions(backend) {
  if (backend === 'swiftshader') {
    return {
      headless: true,
      args: ['--use-gl=angle', '--use-angle=swiftshader', '--enable-unsafe-swiftshader'],
    }
  }
  if (backend === 'gpu') return { headless: false, args: [] }
  throw new Error(`unknown backend ${backend}`)
}

/** Install the in-page capture surface on top of `window.__kglvBench.graph`. */
async function installCapture(page) {
  await page.evaluate(() => {
    const hook = window.__kglvBench
    if (!hook?.graph) throw new Error('__kglvBench.graph is not published; is the renderer mounted?')
    const bench = {
      periods: [],
      tickPeriods: [],
      running: false,
      startedAt: 0,
      frames: 0,
      // Which capture is current. A `requestAnimationFrame` callback that is
      // already queued when a capture stops still fires once afterwards, so a
      // `running` flag alone lets the previous capture's loop resume the moment
      // the next one sets the flag back to true — two loops then push into one
      // array and every count doubles. That is not hypothetical: it happened on
      // the first run of this harness and was caught by the frame count
      // disagreeing with the wall-clock span, which is the whole reason the
      // effective-fps cell is computed independently of the period list.
      generation: 0,
    }
    window.__bench = bench

    bench.start = () => {
      const gen = (bench.generation += 1)
      bench.periods = []
      bench.tickPeriods = []
      bench.frames = 0
      bench.running = true
      bench.startedAt = performance.now()
      let lastFrame = performance.now()
      let lastTick = performance.now()
      hook.graph.setConfigPartial({
        onSimulationTick: () => {
          const now = performance.now()
          if (bench.running && gen === bench.generation) bench.tickPeriods.push(now - lastTick)
          lastTick = now
        },
      })
      const step = (now) => {
        if (!bench.running || gen !== bench.generation) return
        bench.periods.push(now - lastFrame)
        lastFrame = now
        bench.frames += 1
        requestAnimationFrame(step)
      }
      requestAnimationFrame(step)
    }

    bench.stop = () => {
      bench.running = false
      const spanMs = performance.now() - bench.startedAt
      hook.graph.setConfigPartial({ onSimulationTick: undefined })
      // The first period spans from `start()` to the first callback, which is
      // a scheduling artefact of the harness rather than a frame the renderer
      // produced. Everything after it is frame-to-frame.
      return { periods: bench.periods.slice(1), tickPeriods: bench.tickPeriods.slice(1), spanMs, frames: bench.frames }
    }

    // Pure integer arithmetic: no WebGL, no graph data, no allocation. Neither
    // cosmos.gl nor kglite can move this, which is what makes it a control
    // rather than a second measurement of the same thing.
    bench.control = (rounds) => {
      const t0 = performance.now()
      let acc = 1
      for (let i = 1; i <= rounds; i += 1) acc = (acc * 31 + i) % 2147483647
      const ms = performance.now() - t0
      return { ms, checksum: acc }
    }

    /**
     * Turn the GPU layout on for the capture.
     *
     * `enableSimulationDuringZoom` goes on with it, and it is not optional
     * here: cosmos.gl pauses the simulation for the duration of an interactive
     * zoom by default, so measuring pan/zoom without this flag measures a
     * *paused* layout and reports a cost the stop rule is not asking about.
     * The first run of this harness showed exactly that — the simulation-tick
     * instrument went quiet during zoom while the frame instrument did not.
     */
    bench.simulation = (on) => {
      hook.graph.setConfigPartial({ enableSimulation: on, enableSimulationDuringZoom: on })
      if (on) hook.graph.start(1)
      else hook.graph.pause()
      return hook.graph.isSimulationRunning
    }

    /**
     * Detach the app's zoom handlers, for one diagnostic cell.
     *
     * The app's zoom handlers reposition the labels already on screen
     * (`main.ts` -> `positionLabels` -> `LabelOverlay.update`), an
     * O(sampled points) pass. Until P4b they also rebuilt the whole spec list
     * per zoom event (`LabelOverlay.setLabels`, O(live slots), tearing down
     * every element), and this cell is what measured that: capture with the
     * handlers, then without, and the difference is the answer. It stays as the
     * floor the label pass is compared against. Nothing in the product does
     * this.
     */
    bench.detachZoomHandlers = () => {
      hook.graph.setConfigPartial({ onZoom: undefined, onZoomEnd: undefined })
      return true
    }

    /**
     * Push a deterministic synthetic view straight at the renderer.
     *
     * Above the response bound the server cannot produce a slice at all, so
     * this is the only way to ask the renderer what it does there. Positions
     * come from a fixed LCG so the layout the simulation starts from is the
     * same on every run and on both backends.
     */
    bench.synthetic = (points, degree) => {
      const SPACE = 4096
      let seed = 0x2f6e2b1
      const rand = () => {
        seed = (seed * 1664525 + 1013904223) >>> 0
        return seed / 4294967296
      }
      const positions = new Float32Array(points * 2)
      for (let i = 0; i < points * 2; i += 1) positions[i] = rand() * SPACE
      const linkCount = points * degree
      const links = new Float32Array(linkCount * 2)
      for (let i = 0; i < linkCount; i += 1) {
        links[i * 2] = Math.floor(rand() * points)
        links[i * 2 + 1] = Math.floor(rand() * points)
      }
      const sizes = new Float32Array(points).fill(4)
      const colors = new Float32Array(points * 4)
      for (let i = 0; i < points; i += 1) {
        colors[i * 4] = 0.4
        colors[i * 4 + 1] = 0.7
        colors[i * 4 + 2] = 0.95
        colors[i * 4 + 3] = 0.9
      }
      const widths = new Float32Array(linkCount).fill(1)
      hook.graph.setPointPositions(positions, true)
      hook.graph.setPointSizes(sizes)
      hook.graph.setPointColors(colors)
      hook.graph.setLinks(links)
      hook.graph.setLinkWidths(widths)
      hook.graph.render(undefined, 0)
      return { points, links: linkCount }
    }
  })
}

/** Scripted pan and zoom over the canvas, from Playwright's own input stack. */
async function interact(page, ms) {
  const box = await page.locator('.kglv-canvas').boundingBox()
  const cx = box.x + box.width / 2
  const cy = box.y + box.height / 2
  const deadline = Date.now() + ms
  let i = 0
  while (Date.now() < deadline) {
    // Drag from a point offset far enough from the centre that the press lands
    // on empty space and pans the view rather than grabbing a node.
    const dx = 220 * Math.cos(i / 3)
    const dy = 140 * Math.sin(i / 3)
    await page.mouse.move(cx + dx, cy + dy)
    await page.mouse.down()
    await page.mouse.move(cx + dx * 0.4, cy + dy * 0.4, { steps: 8 })
    await page.mouse.up()
    await page.mouse.wheel(0, i % 2 === 0 ? -240 : 240)
    i += 1
  }
}

/** One 60 Hz refresh interval, in milliseconds. */
const REFRESH_MS = 1000 / 60

/**
 * Summarise a period list, and say which frame-rate line its p95 sits under.
 *
 * **The lines are placed between buckets, not on them.** A vsync-locked
 * presenter cannot return a period between refreshes: every frame costs
 * k x 16.67 ms for an integer k, and the jitter around each bucket is a
 * millisecond or so. Testing `p95 <= 16.7` therefore fails a renderer that
 * hit every single vsync, purely on jitter. What the 60 fps line actually
 * asks is *"did the 95th-percentile frame land in the k = 1 bucket"*, so the
 * threshold goes halfway to k = 2, and the 30 fps line halfway from k = 2 to
 * k = 3. `p95_ms` is reported raw beside them, so a reader who wants a
 * different line can draw it.
 */
function stats(periods) {
  if (periods.length === 0) return { n: 0 }
  const sorted = [...periods].sort((a, b) => a - b)
  const q = (p) => sorted[Math.min(sorted.length - 1, Math.round((sorted.length - 1) * p))]
  const p95 = q(0.95)
  return {
    statistic: 'p95/p99 of frame period',
    n: sorted.length,
    p50: +q(0.5).toFixed(3),
    p95: +p95.toFixed(3),
    p99: +q(0.99).toFixed(3),
    max: +sorted[sorted.length - 1].toFixed(3),
    mean: +(sorted.reduce((a, b) => a + b, 0) / sorted.length).toFixed(3),
    budget_60fps_ms: +REFRESH_MS.toFixed(2),
    budget_30fps_ms: +(REFRESH_MS * 2).toFixed(2),
    holds_60fps: p95 < REFRESH_MS * 1.5,
    holds_30fps: p95 < REFRESH_MS * 2.5,
    // A period past 1.5 refreshes is a frame that missed its deadline.
    dropped_fraction: +(
      sorted.filter((v) => v > REFRESH_MS * 1.5).length / sorted.length
    ).toFixed(4),
  }
}

/** Median of three control runs — one sample is a coin flip under real load. */
async function control(page) {
  const runs = []
  for (let i = 0; i < 3; i += 1) {
    runs.push((await page.evaluate((r) => window.__bench.control(r), CONTROL_ROUNDS)).ms)
  }
  runs.sort((a, b) => a - b)
  return { median: runs[1], runs: runs.map((v) => +v.toFixed(2)) }
}

async function capture(page, label, out) {
  const before = await control(page)
  await page.evaluate(() => window.__bench.start())
  await page.waitForTimeout(SETTLE_MS)
  const settle = await page.evaluate(() => window.__bench.stop())

  await page.evaluate(() => window.__bench.start())
  await interact(page, INTERACT_MS)
  const interactive = await page.evaluate(() => window.__bench.stop())
  const after = await control(page)

  out[`${label}_settle_frame_ms`] = stats(settle.periods)
  out[`${label}_interact_frame_ms`] = stats(interactive.periods)
  out[`${label}_settle_simtick_ms`] = stats(settle.tickPeriods)
  out[`${label}_interact_simtick_ms`] = stats(interactive.tickPeriods)
  out[`${label}_effective_fps`] = {
    statistic: 'exact',
    settle: +((settle.frames / settle.spanMs) * 1000).toFixed(2),
    interact: +((interactive.frames / interactive.spanMs) * 1000).toFixed(2),
  }
  out[`${label}_control_ms`] = {
    statistic: 'median of 3, before and after each capture',
    before: +before.median.toFixed(2),
    after: +after.median.toFixed(2),
    before_runs: before.runs,
    after_runs: after.runs,
    drift_ratio: +(after.median / before.median).toFixed(3),
  }
}

/**
 * The two client costs that set a bound number of their own.
 *
 * `MAX_QUERY_ROWS` exists because the results table is HTML and a browser
 * building a hundred thousand rows of DOM is the freeze the whole design avoids
 * — so the number that sets it is *how long a full table takes to build*, not a
 * frame rate. `COMPACTION_TOMBSTONE_RATIO` exists because a compaction
 * invalidates every index the client holds, so the number that sets it is what
 * one compaction round trip costs against the waste it reclaims.
 *
 * Both are once-per-event costs, so both are reported as the **mean of first
 * events** over fresh pages: a repeat on a warmed page is a different question.
 */
async function interactionCosts(context, url, out, repeats) {
  const queryMs = []
  const compactionMs = []
  let queryRows = 0
  let reclaimed = 0

  for (let i = 0; i < repeats; i += 1) {
    const { page } = await readyPage(context, url)
    // A table at the row bound: the graph has 20 000 Persons and the bound
    // clamps to MAX_QUERY_ROWS, so this is the largest table the panel can be
    // asked to build.
    await page.getByTestId('query-input').fill('MATCH (n:Person) RETURN id(n) AS id, n.title AS title')
    const t0 = Date.now()
    await page.getByTestId('query-run').click()
    await page.waitForFunction(() => window.__kglv.queryRows > 0, undefined, { timeout: 120_000 })
    queryMs.push(Date.now() - t0)
    queryRows = await page.evaluate(() => window.__kglv.queryRows)

    // Expand to the node bound, then collapse it: 5 005 slots with 5 000
    // tombstoned is far past the 30% ratio and past the 64-slot floor, so the
    // collapse is answered with a compaction and the client applies the remap.
    await page.locator('.kglv-label:has-text("Person")').click()
    await page.getByTestId('expand-limit').fill('5000')
    await page.getByTestId('expand-KNOWS-out').click()
    await page.waitForFunction(
      () => window.__kglv.lastSliceKind === 'expand' && window.__kglv.pointCount >= 5000,
      undefined,
      { timeout: 180_000 },
    )
    const t1 = Date.now()
    await page.getByTestId('collapse').click()
    await page.waitForFunction(() => window.__kglv.compactions === 1, undefined, { timeout: 180_000 })
    compactionMs.push(Date.now() - t1)
    reclaimed = await page.evaluate(() => window.__kglv.slotCount)
    await page.close()
  }

  const mean = (xs) => +(xs.reduce((a, b) => a + b, 0) / xs.length).toFixed(2)
  out.query_table_build_ms = {
    statistic: 'mean of first events',
    value: mean(queryMs),
    events: queryMs,
    n: queryMs.length,
  }
  out.query_table_rows = { statistic: 'exact', value: queryRows }
  out.collapse_with_compaction_ms = {
    statistic: 'mean of first events',
    value: mean(compactionMs),
    events: compactionMs,
    n: compactionMs.length,
  }
  out.compaction_slots_after = { statistic: 'exact', value: reclaimed }
}

/**
 * The app URL in D2's deterministic layout mode.
 *
 * Every cell in this harness starts from a known layout: the frontend's user
 * default is the GPU force simulation, which moves points continuously until
 * it settles, and a frame-period capture taken over a settling layout measures
 * the settle rather than the thing under test. The synthetic cells toggle the
 * simulation on themselves (`__bench.simulation(true)`), which is what makes
 * "simulation on" a *variable* here rather than an ambient condition.
 */
function appUrl(info) {
  return `${info.url}?deterministic=1`
}

async function readyPage(context, url) {
  const page = await context.newPage()
  const errors = []
  page.on('console', (m) => {
    if (m.type() === 'error') errors.push(m.text())
  })
  await page.goto(url)
  await page.waitForFunction(() => window.__kglv?.ready === true, undefined, { timeout: 60_000 })
  return { page, errors }
}

async function main() {
  const args = parseArgs()
  // The production-bundle precondition, read as an exit code rather than
  // assumed: a dev-server number is measuring the dev server (R11).
  execFileSync('python3', [path.join(REPO, 'scripts/check_bundle.py'), '--freshness'], {
    cwd: REPO,
    stdio: 'inherit',
  })

  const out = {
    backend: { statistic: 'exact', value: args.backend },
    graph: { statistic: 'exact', value: args.graph },
    run: { statistic: 'exact', value: args.run },
    captured_at: { statistic: 'exact', value: new Date().toISOString() },
    loadavg: { statistic: 'exact', value: (await import('node:os')).loadavg().map((v) => +v.toFixed(2)) },
  }

  const server = await launchServer(args.graph)
  out.binary = { statistic: 'exact', value: server.bin }
  out.profile = { statistic: 'exact', value: server.bin.includes('/release/') ? 'release' : 'debug' }

  const browser = await chromium.launch(launchOptions(args.backend))
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } })

  try {
    // ── time to first paint: mean of first events, cold loads ──────────
    const ttfp = []
    for (let i = 0; i < args.loads; i += 1) {
      // A fresh context per load, not a fresh page: a page reused inside one
      // context finds the bundle in the HTTP cache, so every load after the
      // first would be measuring a warm open. "Cold" is the number a user
      // opening the tool experiences.
      const coldContext = await browser.newContext({ viewport: { width: 1280, height: 800 } })
      const cold = await coldContext.newPage()
      await cold.goto(appUrl(server.info))
      await cold.waitForFunction(() => window.__kglvBench?.firstDataFrameMs !== null, undefined, {
        timeout: 60_000,
      })
      ttfp.push(await cold.evaluate(() => window.__kglvBench.firstDataFrameMs))
      if (i === 0) {
        out.device_features = {
          statistic: 'exact',
          value: await cold.evaluate(() => window.__kglv.deviceFeatures),
        }
      }
      await coldContext.close()
    }
    out.time_to_first_data_frame_ms = {
      statistic: 'mean of first events',
      value: +(ttfp.reduce((a, b) => a + b, 0) / ttfp.length).toFixed(2),
      events: ttfp.map((v) => +v.toFixed(2)),
      n: ttfp.length,
    }

    // ── the real path: a slice the server actually produced ────────────
    for (const size of args.sizes) {
      const { page, errors } = await readyPage(context, appUrl(server.info))
      await page.locator('.kglv-label:has-text("Person")').click()
      await page.getByTestId('expand-limit').fill(String(size))
      await page.getByTestId('expand-KNOWS-out').click()
      // Both conditions: `lastSliceKind` alone is set before the upload, so
      // waiting on it can start a capture against a view that is still five
      // meta-graph nodes.
      await page.waitForFunction(
        (want) => window.__kglv.lastSliceKind === 'expand' && window.__kglv.pointCount >= want,
        Math.min(size, 5000),
        { timeout: 180_000 },
      )
      const state = await page.evaluate(() => window.__kglv)
      const label = `real_${size}`
      out[`${label}_points`] = { statistic: 'exact', value: state.pointCount }
      out[`${label}_links`] = { statistic: 'exact', value: state.linkCount }
      out[`${label}_truncated`] = { statistic: 'exact', value: state.truncation }
      await installCapture(page)
      out[`${label}_webgl_renderer`] = {
        statistic: 'exact',
        value: await page.evaluate(() => {
          const gl = document.createElement('canvas').getContext('webgl2')
          const ext = gl?.getExtension('WEBGL_debug_renderer_info')
          const name = ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : 'unavailable'
          gl?.getExtension('WEBGL_lose_context')?.loseContext()
          return name
        }),
      }
      out[`${label}_sim_running`] = {
        statistic: 'exact',
        value: await page.evaluate(() => window.__bench.simulation(true)),
      }
      await capture(page, label, out)

      // Same page, same slice, same simulation — only the app's per-zoom label
      // pass removed. Two cells that differ in exactly one thing is what makes
      // the difference between them attributable.
      await page.evaluate(() => window.__bench.detachZoomHandlers())
      await capture(page, `${label}_nozoomlabels`, out)

      out[`${label}_console_errors`] = { statistic: 'exact', value: errors }
      await page.close()
    }

    // ── the synthetic path: in-bound cross-check, plus above the bound ──
    for (const size of args.synthetic) {
      const { page, errors } = await readyPage(context, appUrl(server.info))
      await installCapture(page)
      const loaded = await page.evaluate(([n, d]) => window.__bench.synthetic(n, d), [size, 3])
      const label = `synthetic_${size}`
      out[`${label}_points`] = { statistic: 'exact', value: loaded.points }
      out[`${label}_links`] = { statistic: 'exact', value: loaded.links }
      out[`${label}_above_bound`] = { statistic: 'exact', value: size > 5000 }
      out[`${label}_sim_running`] = {
        statistic: 'exact',
        value: await page.evaluate(() => window.__bench.simulation(true)),
      }
      await capture(page, label, out)
      out[`${label}_console_errors`] = { statistic: 'exact', value: errors }
      await page.close()
    }
    await interactionCosts(context, appUrl(server.info), out, 3)
  } finally {
    await browser.close()
    server.child.kill()
  }

  const text = `${JSON.stringify(out, null, 2)}\n`
  if (args.out) {
    mkdirSync(path.dirname(path.resolve(REPO, args.out)), { recursive: true })
    writeFileSync(path.resolve(REPO, args.out), text)
    console.error(`wrote ${args.out}`)
  }
  process.stdout.write(text)
}

await main()
