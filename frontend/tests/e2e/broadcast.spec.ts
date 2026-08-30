/**
 * L3: one view, every client (plan D14).
 *
 * The gap being closed was already live before this suite existed — the JSON
 * twin's POSTs moved the server-side slot space and the connected WebSocket
 * clients were never told — so these two tests are the regression tests for a
 * shipped defect, not for a new feature's happy path.
 *
 * Neither test touches the UI. That is the assertion: the browser's `__kglv`
 * counts move because *something else* asked the server to move them, with no
 * click, no keystroke and no polling anywhere in the page.
 */

import { expect, test, type Page } from '@playwright/test'

import { Listener, appUrl, launch, type Launched } from './harness'

/** The fixture's Person type: 60 members, largest type, therefore slot 0. */
const PERSON_SLOT = 0
const META_POINTS = 5
const KNOWS_REACHABLE = 60

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__kglv?.ready === true, undefined, { timeout: 30_000 })
}

test('one HTTP expand reaches every connected websocket client', async () => {
  let server: Launched | null = null
  const listeners: Listener[] = []
  try {
    server = await launch()
    const wsUrl = `${server.info.url.replace(/^http/, 'ws')}ws`

    // Two clients, because one client cannot distinguish "the server answers
    // whoever asked" from "the server tells everyone".
    for (let i = 0; i < 2; i += 1) {
      const listener = new Listener(wsUrl)
      await listener.open()
      listeners.push(listener)
    }

    // The initiator is neither of them: a plain HTTP POST, the shape an agent
    // or a `curl` line uses, and the shape that used to mutate the view in
    // silence.
    const response = await fetch(`${server.info.url}api/expand`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ slot: PERSON_SLOT, relationship: 'KNOWS', direction: 'out' }),
    })
    expect(response.status).toBe(200)
    // The wire contract for the initiator is unchanged: it still gets its own
    // answer in its own response body. Broadcast is an addition, not a
    // replacement.
    const body = (await response.json()) as { meta: { kind: string; nodes: unknown[] } }
    expect(body.meta.kind).toBe('expand')
    expect(body.meta.nodes).toHaveLength(KNOWS_REACHABLE)

    for (const [index, listener] of listeners.entries()) {
      const slice = await listener.waitFor(
        (done) => done.kind === 'slice' && done.value.meta.kind === 'expand',
      )
      if (slice.kind !== 'slice') throw new Error('unreachable')
      expect(slice.value.meta.nodes, `client ${index}`).toHaveLength(KNOWS_REACHABLE)
      expect(slice.value.meta.slot_count, `client ${index}`).toBe(META_POINTS + KNOWS_REACHABLE)
      // Positions and links ride with it, not just the metadata — a client
      // that received the description and not the arrays could not draw.
      expect(slice.value.points.length, `client ${index}`).toBe(KNOWS_REACHABLE * 2)
      expect(slice.value.links.length, `client ${index}`).toBeGreaterThan(0)
    }
  } finally {
    for (const listener of listeners) listener.close()
    server?.process.kill()
  }
})

test('the browser follows an agent: curl moves the view with no UI action', async ({ page }) => {
  let server: Launched | null = null
  const consoleErrors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })

  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)

    const entry = await page.evaluate(() => window.__kglv)
    expect(entry.slotCount).toBe(META_POINTS)
    expect(entry.pointCount).toBe(META_POINTS)

    // Not a click, not a keystroke, not a reload — an HTTP request from
    // outside the page entirely. This is the P10 feature in one assertion:
    // the user's screen follows the agent.
    const expand = await fetch(`${server.info.url}api/expand`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ slot: PERSON_SLOT, relationship: 'KNOWS', direction: 'out' }),
    })
    expect(expand.status).toBe(200)

    await page.waitForFunction(
      (expected) => window.__kglv.slotCount === expected,
      META_POINTS + KNOWS_REACHABLE,
      { timeout: 15_000 },
    )
    const expanded = await page.evaluate(() => window.__kglv)
    expect(expanded.lastSliceKind).toBe('expand')
    expect(expanded.pointCount).toBe(META_POINTS + KNOWS_REACHABLE)
    expect(expanded.tombstoneCount).toBe(0)
    // The labels are on screen too, so this is a drawn view and not a counter
    // that moved.
    await expect(page.locator('.kglv-label:has-text("Person_")').first()).toBeVisible()

    // And back: a collapse from outside restores the entry screen, remap
    // included (65 slots is over the compaction minimum).
    const collapse = await fetch(`${server.info.url}api/collapse`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ slot: PERSON_SLOT }),
    })
    expect(collapse.status).toBe(200)

    await page.waitForFunction(() => window.__kglv.compactions === 1, undefined, {
      timeout: 15_000,
    })
    const collapsed = await page.evaluate(() => window.__kglv)
    expect(collapsed.slotCount).toBe(META_POINTS)
    expect(collapsed.pointCount).toBe(META_POINTS)
    expect(collapsed.tombstoneCount).toBe(0)

    expect(consoleErrors, `browser console errors: ${consoleErrors.join(' | ')}`).toEqual([])
  } finally {
    server?.process.kill()
  }
})
