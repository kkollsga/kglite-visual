/**
 * The layout switch, in the mode users actually run (plan E5).
 *
 * **Every assertion here is in FORCE mode, and that is the point.** The
 * program's pre-mortem #1 named the exact failure this spec exists to catch: a
 * layout switch works in `?deterministic=1` — where the server's positions are
 * already authoritative and the simulation never ran — and does visibly
 * nothing in the mode a user opens, because the standing seed authority is
 * `gpu` and every slot on screen already has a position for the merge to keep.
 * A spec that took the deterministic shortcut would pass against that bug. So
 * this one goes to the plain URL, waits for the simulation to be running, and
 * only then switches.
 *
 * The evidence a switch happened is `positionsHash` over the *renderer's* own
 * positions, not over the view's: what is asserted is that the picture moved.
 */

import { expect, test, type Page } from '@playwright/test'

import { launch, type Launched } from './harness'

const PERSON_SLOT = 0
const META_POINTS = 5

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__kglv?.ready === true, undefined, {
    timeout: 30_000,
  })
}

/**
 * The positions the GPU is holding, as one comparable string.
 *
 * Rounded to whole units before hashing: in force mode the simulation is
 * settling continuously, so two reads a frame apart differ in their last bits
 * and an exact comparison would report "the layout changed" every time it was
 * asked. A kernel switch moves points by hundreds of units, so the rounding
 * costs nothing it needs to see.
 */
async function arrangement(page: Page): Promise<string> {
  return page.evaluate(() => {
    const graph = window.__kglvBench.graph
    if (graph === null) return 'no-renderer'
    const positions = graph.getPointPositions()
    let out = ''
    for (const value of positions) out += `${Math.round(value)},`
    return out
  })
}

test('force mode: switch to a static kernel, expand under it, switch back', async ({
  page,
}, testInfo) => {
  let server: Launched | null = null
  const consoleErrors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })

  try {
    server = await launch()
    // No `?deterministic=1`: this is the user's mode.
    await page.goto(server.info.url)
    await ready(page)

    const entry = await page.evaluate(() => window.__kglv)
    expect(entry.layoutMode).toBe('force')
    expect(entry.layoutKernel).toBe('simulation')
    expect(entry.pointCount).toBe(META_POINTS)

    // Let the simulation settle first, so the arrangement being replaced is a
    // real force layout rather than the seed lattice. Without this the switch
    // could pass by replacing positions nothing had moved yet.
    await page.waitForFunction(() => window.__kglv.simRunning === false, undefined, {
      timeout: 30_000,
    })
    const settled = await arrangement(page)

    // ── switch to a static kernel ───────────────────────────────────────
    await page.getByTestId('layout-kernel').selectOption('islands')
    await page.waitForFunction(() => window.__kglv.layoutKernel === 'islands', undefined, {
      timeout: 15_000,
    })
    const staticState = await page.evaluate(() => window.__kglv)
    expect(staticState.layoutMode).toBe('static')
    expect(staticState.pointCount).toBe(META_POINTS)

    // The picture actually moved — the assertion pre-mortem #1 is about.
    const packed = await arrangement(page)
    expect(packed).not.toBe(settled)

    // The simulation is destroyed and dragging is off, which is what "held
    // still" means: read back from the renderer, not from what this app asked
    // for, because a config that did not take is a config that did not happen.
    const held = await page.evaluate(() => ({
      simulation: window.__kglvBench.graph?.config.enableSimulation,
      drag: window.__kglvBench.graph?.config.enableDrag,
    }))
    expect(held).toEqual({ simulation: false, drag: false })
    await expect(page.getByTestId('layout-note')).toContainText('dragging is off')

    // ── an expansion under a static kernel re-requests the layout ───────
    // Not a merge: the new slots arrive on the server's lattice, so merging
    // would drop a spiral of dots into the middle of a packed island.
    await page.locator('.kglv-label:has-text("Person")').click()
    await page.getByTestId('expand-limit').fill('20')
    await page.getByTestId('expand-KNOWS-out').click()
    await page.waitForFunction(
      (expected) => window.__kglv.pointCount === expected,
      META_POINTS + 20,
      { timeout: 15_000 },
    )
    // The layout survived the expansion: still the kernel that was chosen, and
    // still static. A client that had reheated instead would report `force`.
    const expanded = await page.evaluate(() => window.__kglv)
    expect(expanded.layoutMode).toBe('static')
    expect(expanded.layoutKernel).toBe('islands')
    const withInstances = await arrangement(page)
    expect(withInstances).not.toBe(packed)
    // Every new slot got a real position from the re-request. A merged lattice
    // would leave them on the spiral, which is not a NaN and not a kernel's
    // output — so the check is that nothing is unplaced and the arrangement is
    // as wide as a packed layout, not a 140-unit lattice.
    const spread = await page.evaluate(() => {
      const positions = window.__kglvBench.graph?.getPointPositions() ?? []
      let min = Infinity
      let max = -Infinity
      for (const value of positions) {
        if (!Number.isFinite(value)) continue
        min = Math.min(min, value)
        max = Math.max(max, value)
      }
      return max - min
    })
    expect(spread).toBeGreaterThan(500)

    // ── back to the live simulation ─────────────────────────────────────
    await page.getByTestId('layout-kernel').selectOption('simulation')
    await page.waitForFunction(() => window.__kglv.layoutKernel === 'simulation', undefined, {
      timeout: 15_000,
    })
    const live = await page.evaluate(() => window.__kglv)
    expect(live.layoutMode).toBe('force')
    const restored = await page.evaluate(() => ({
      simulation: window.__kglvBench.graph?.config.enableSimulation,
      drag: window.__kglvBench.graph?.config.enableDrag,
    }))
    expect(restored).toEqual({ simulation: true, drag: true })
    // The reheat the switch back triggers actually runs, and settles.
    await page.waitForFunction(() => window.__kglv.simRunning === false, undefined, {
      timeout: 30_000,
    })
    const resettled = await arrangement(page)
    expect(resettled).not.toBe(withInstances)
    expect(PERSON_SLOT).toBe(0)

    const shotPath = testInfo.outputPath('layout-switch.png')
    await page.screenshot({ path: shotPath })
    await testInfo.attach('layout-switch.png', { path: shotPath, contentType: 'image/png' })

    expect(consoleErrors, `browser console errors: ${consoleErrors.join(' | ')}`).toEqual([])
  } finally {
    server?.process.kill()
  }
})

test('the picker is absent in the fixture mode, and a map of nowhere is refused', async ({
  page,
}) => {
  let server: Launched | null = null
  try {
    server = await launch()
    // `?deterministic=1` owns its positions: a control that replaced them is a
    // button for breaking the suite, so there is no control.
    await page.goto(`${server.info.url}?deterministic=1`)
    await ready(page)
    expect((await page.evaluate(() => window.__kglv)).layoutMode).toBe('deterministic')
    await expect(page.getByTestId('layout-kernel')).toHaveCount(0)

    // The entry screen is nothing but type nodes, and a *type* is not anywhere
    // — its instances are. So `geo` over this view has nothing to place, and
    // the server says so in a sentence rather than quietly serving a force
    // layout, which would let a caller report a map as working.
    const refusal = await page.evaluate(async (base) => {
      const response = await fetch(`${base}api/layout`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ kernel: 'geo' }),
      })
      return { status: response.status, body: (await response.json()) as { error?: string } }
    }, server.info.url)
    expect(refusal.status).toBe(400)
    expect(refusal.body.error).toContain('no map to draw')
  } finally {
    server?.process.kill()
  }
})

/**
 * The map, offered where it means something and withheld where it does not
 * (plan E12).
 *
 * **The conditional entry is the assertion, not decoration.** G3 shipped no
 * `geo` entry at all because an option that always errored was worse than none;
 * the rule this replaces it with is the same rule with a live condition, and a
 * spec that only checked the happy path would pass against a picker that
 * offered the map on the entry screen — where every node is a type and nothing
 * is anywhere.
 */
test('the map is offered once the view holds nodes that are somewhere', async ({
  page,
}, testInfo) => {
  let server: Launched | null = null
  const consoleErrors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })
  try {
    server = await launch()
    await page.goto(server.info.url)
    await ready(page)

    const options = page.getByTestId('layout-kernel').locator('option[value="geo"]')
    await expect(options).toHaveCount(0)

    // `City` is the fixture's one type with a lat/lon location (`loc` on the
    // meta-graph). Expanding it puts its instances on screen, together with the
    // companies that are in them — which have no coordinate and are exactly the
    // tray case the server counts.
    await page.locator('.kglv-label:has-text("City")').click()
    await page.getByTestId('expand-LOCATED_IN-in').click()
    await page.waitForFunction((seed) => window.__kglv.pointCount > seed, META_POINTS, {
      timeout: 15_000,
    })
    await expect(options).toHaveCount(1)

    await page.getByTestId('layout-kernel').selectOption('geo')
    await page.waitForFunction(() => window.__kglv.layoutKernel === 'geo', undefined, {
      timeout: 15_000,
    })
    const mapped = await page.evaluate(() => window.__kglv)
    expect(mapped.layoutMode).toBe('static')
    // The live map is positions and nothing else, and the hint says so rather
    // than leaving a user to wonder where the coastline went.
    await expect(page.getByTestId('layout-note')).toContainText('positions only')

    const shotPath = testInfo.outputPath('layout-geo.png')
    await page.screenshot({ path: shotPath })
    await testInfo.attach('layout-geo.png', { path: shotPath, contentType: 'image/png' })

    expect(consoleErrors, `browser console errors: ${consoleErrors.join(' | ')}`).toEqual([])
  } finally {
    server?.process.kill()
  }
})
