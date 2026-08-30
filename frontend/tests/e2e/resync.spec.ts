/**
 * L3: a client that connects mid-session sees the session, not the entry screen.
 *
 * The regression test for a shipped defect, found in G4 by opening a second
 * browser onto a view an agent had already drilled into. Every client used to
 * be greeted with the session info and the *meta-graph* — slots 0..n, the entry
 * screen — and nothing else. A session that had already expanded was therefore
 * describing a slot space the newcomer had never been given: the next broadcast
 * arrived with a `first_slot` beyond the end of its positions array, the splice
 * grew the array over the gap, and every slot in between drew as a point with
 * no label, no id and nothing to click. G4 measured 144 of them on sodir.
 *
 * The fix is a resync frame on connect (see `Session::sync_slice`), so both
 * assertions below are about the *newcomer*: what it is handed before it asks
 * for anything, and what its renderer holds afterwards.
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

test('a socket that connects after an expansion is handed the whole view', async () => {
  let server: Launched | null = null
  const listeners: Listener[] = []
  try {
    server = await launch()
    const wsUrl = `${server.info.url.replace(/^http/, 'ws')}ws`

    // Somebody drills in before the newcomer exists. Over HTTP so the
    // expansion is nobody's socket: this is the agent-moved-the-view shape.
    const expand = await fetch(`${server.info.url}api/expand`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ slot: PERSON_SLOT, relationship: 'KNOWS', direction: 'out' }),
    })
    expect(expand.status).toBe(200)

    const late = new Listener(wsUrl)
    await late.open()
    listeners.push(late)

    // The greeting's third message is the whole view: from slot zero, every
    // live instance named, positions for the entire space.
    const sync = await late.waitFor(
      (done) => done.kind === 'slice' && done.value.meta.kind === 'sync',
    )
    if (sync.kind !== 'slice') throw new Error('unreachable')
    expect(sync.value.meta.kind).toBe('sync')
    expect(sync.value.meta.first_slot).toBe(0)
    expect(sync.value.meta.slot_count).toBe(META_POINTS + KNOWS_REACHABLE)
    expect(sync.value.meta.nodes).toHaveLength(KNOWS_REACHABLE)
    // Positions for the *whole* space, meta-graph slots included — a range
    // starting at `first_slot` is what the newcomer cannot splice into an
    // array it does not have.
    expect(sync.value.points.length).toBe((META_POINTS + KNOWS_REACHABLE) * 2)
    expect(sync.value.links.length).toBeGreaterThan(0)
    // Every instance the session holds is named, so nothing draws anonymously.
    for (const node of sync.value.meta.nodes) {
      expect(node.title, `slot ${node.slot} arrived unnamed`).not.toBe('')
    }

    // And the sync is not a view mutation: it changed nothing, so the session's
    // report of what the bound last did must still be the expansion's.
    const state = (await (await fetch(`${server.info.url}api/view-state`)).json()) as {
      last_slice: { kind: string } | null
      slot_count: number
    }
    expect(state.last_slice?.kind).toBe('expand')
    expect(state.slot_count).toBe(META_POINTS + KNOWS_REACHABLE)
  } finally {
    for (const listener of listeners) listener.close()
    server?.process.kill()
  }
})

test('a second browser opened mid-session draws the same named nodes as the first', async ({
  browser,
}) => {
  let server: Launched | null = null
  const first = await browser.newPage()
  const second = await browser.newPage()
  const consoleErrors: string[] = []
  second.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })

  try {
    server = await launch()
    await first.goto(appUrl(server.info))
    await ready(first)

    const expand = await fetch(`${server.info.url}api/expand`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ slot: PERSON_SLOT, relationship: 'KNOWS', direction: 'out' }),
    })
    expect(expand.status).toBe(200)
    await first.waitForFunction(
      (expected) => window.__kglv.slotCount === expected,
      META_POINTS + KNOWS_REACHABLE,
      { timeout: 15_000 },
    )
    const a = await first.evaluate(() => window.__kglv)

    // The newcomer. It missed the expansion entirely and must still land on
    // the same view — the same slot count, and the same number of slots it can
    // put a name to. `namedSlots` is the field the defect moved: the points
    // arrived, the identities did not.
    await second.goto(appUrl(server.info))
    await ready(second)
    await second.waitForFunction(
      (expected) => window.__kglv.slotCount === expected,
      META_POINTS + KNOWS_REACHABLE,
      { timeout: 15_000 },
    )
    const b = await second.evaluate(() => window.__kglv)

    expect(b.slotCount).toBe(a.slotCount)
    expect(b.pointCount).toBe(a.pointCount)
    expect(b.namedSlots).toBe(a.namedSlots)
    expect(b.namedSlots).toBe(META_POINTS + KNOWS_REACHABLE)
    await expect(second.locator('.kglv-label:has-text("Person_")').first()).toBeVisible()

    expect(consoleErrors, `browser console errors: ${consoleErrors.join(' | ')}`).toEqual([])
  } finally {
    await first.close()
    await second.close()
    server?.process.kill()
  }
})
