/**
 * L3 smoke: launch the real binary, drive the real browser, assert on state.
 *
 * The asserts are on `window.__kglv`, never on pixels. The screenshot is an
 * artifact — it is saved and sanity-checked for non-blankness, but a
 * screenshot cannot say *why* a canvas is empty and a state object can.
 *
 * There is no `waitForTimeout` anywhere in this file, and there must never be:
 * cosmos.gl v3 initialises asynchronously and renders on demand, so a static
 * scene draws zero frames after the first. A sleep would prove nothing in
 * either direction.
 */

import { expect, test } from '@playwright/test'

import { appUrl, FIXTURE, launch } from './harness'

/** The meta-graph of `meta.kgl`, asserted exactly (see the core L1 tests). */
const EXPECTED = {
  protocolVersion: 3,
  tier: 'full',
  pointCount: 5,
  linkCount: 7,
  // FNV-1a over the server's positions buffer. This is the D2 determinism
  // assert: it fails if the layout, the slot order or the type ordering moves.
  // A red here after a deliberate change is regenerated with a reason, in the
  // same commit — never to silence a diff (CLAUDE.md → "Gate honesty").
  positionsHash: 'ab2ea15b',
}

test('the meta-graph renders and reports the fixture back through __kglv', async ({
  page,
}, testInfo) => {
  const server = await launch()
  const consoleErrors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })

  try {
    expect(server.info.port).toBeGreaterThan(0)
    expect(server.info.graph).toBe(FIXTURE)

    await page.goto(appUrl(server.info))

    // The whole point of the debug hook: wait for a *state*, not a duration.
    await page.waitForFunction(() => window.__kglv?.ready === true, undefined, {
      timeout: 30_000,
    })

    const state = await page.evaluate(() => window.__kglv)
    testInfo.annotations.push({
      type: 'deviceFeatures',
      description: JSON.stringify(state.deviceFeatures),
    })
    console.log('__kglv:', JSON.stringify(state, null, 2))

    expect(state.error).toBeNull()
    // Asserted BEFORE the position hash, because it is what makes the hash an
    // assertion: in the force mode a user gets, the positions on the GPU are
    // the simulation's and hashing them would prove nothing.
    expect(state.layoutMode).toBe('deterministic')
    expect(state.protocolVersion).toBe(EXPECTED.protocolVersion)
    expect(state.tier).toBe(EXPECTED.tier)
    expect(state.pointCount).toBe(EXPECTED.pointCount)
    expect(state.linkCount).toBe(EXPECTED.linkCount)
    expect(state.positionsHash).toBe(EXPECTED.positionsHash)
    // D2 fixture mode: the simulation must be off. If it ever runs, positions
    // stop being the server's and every determinism assert above is luck.
    expect(state.simRunning).toBe(false)
    expect(state.lastMessageSeq).toBeGreaterThanOrEqual(0)

    // The label overlay is the P2 deliverable a state assert can still see:
    // one label per type node, each carrying its count, City carrying `loc`.
    const labels = page.locator('.kglv-label')
    await expect(labels).toHaveCount(EXPECTED.pointCount)
    await expect(page.locator('.kglv-label:has-text("Person")')).toContainText('60')
    await expect(page.locator('.kglv-label:has-text("City") .kglv-badge-loc')).toHaveText(
      'loc',
    )

    // Written to a path, not just attached: an artifact a human can open
    // afterwards is the whole point of the screenshot tier.
    const shotPath = testInfo.outputPath('meta-graph.png')
    const screenshot = await page.screenshot({ path: shotPath })
    await testInfo.attach('meta-graph.png', { path: shotPath, contentType: 'image/png' })
    // A byte-entropy floor, not a pixel comparison: a blank canvas compresses
    // to almost nothing, and PNG is already compressed. This is a smoke check
    // on the artifact; the assertions that matter are the state ones above.
    expect(screenshot.byteLength).toBeGreaterThan(20_000)

    expect(consoleErrors, `browser console errors: ${consoleErrors.join(' | ')}`).toEqual([])
  } finally {
    server.process.kill()
  }
})

test('a path with an extension 404s instead of silently serving index.html', async ({
  request,
}) => {
  const server = await launch()
  try {
    const missing = await request.get(`${server.info.url}assets/nope.js`)
    expect(missing.status()).toBe(404)

    const route = await request.get(`${server.info.url}graph/Person`)
    expect(route.status()).toBe(200)
    expect(route.headers()['content-type']).toContain('text/html')
  } finally {
    server.process.kill()
  }
})

test('the JSON twin agrees with what the renderer was given', async ({ request }) => {
  const server = await launch()
  try {
    const response = await request.get(`${server.info.url}api/meta-graph`)
    expect(response.ok()).toBeTruthy()
    const body = (await response.json()) as {
      meta: { tier: string; nodes: unknown[]; edges: unknown[] }
      points: number[]
    }
    expect(body.meta.tier).toBe(EXPECTED.tier)
    expect(body.meta.nodes).toHaveLength(EXPECTED.pointCount)
    expect(body.meta.edges).toHaveLength(EXPECTED.linkCount)
    expect(body.points).toHaveLength(EXPECTED.pointCount * 2)

    const session = await request.get(`${server.info.url}api/session`)
    expect((await session.json()).protocol_version).toBe(EXPECTED.protocolVersion)

    const describe = await request.get(`${server.info.url}api/describe`)
    expect((await describe.json()).schema.node_types).toHaveLength(EXPECTED.pointCount)
  } finally {
    server.process.kill()
  }
})
