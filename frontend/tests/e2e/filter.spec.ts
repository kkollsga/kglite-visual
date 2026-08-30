/**
 * The filter hides, counts truthfully, and gives everything back (plan E7).
 *
 * The unit suite owns the grammar; this owns the consequences — what the
 * counts say, what the selection count says about a node the filter hid, and
 * that clearing the box restores the view rather than needing a re-fetch.
 * Those are the assertions a wrong implementation passes on paper: hiding by
 * tombstoning would look identical for one screenshot and then be unable to
 * come back.
 */

import { expect, test, type Page } from '@playwright/test'

import { appUrl, launch, type Launched } from './harness'

const META_POINTS = 5

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__kglv?.ready === true, undefined, {
    timeout: 30_000,
  })
}

test('filtering hides without unloading, counts honestly, and clears', async ({ page }) => {
  let server: Launched | null = null
  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)

    // Load instance nodes, so the view holds two kinds of thing.
    await page.locator('.kglv-label:has-text("Person")').click()
    await page.getByTestId('expand-KNOWS-out').click()
    await page.waitForFunction(() => window.__kglv.lastSliceKind === 'expand', undefined, {
      timeout: 15_000,
    })
    const loaded = await page.evaluate(() => window.__kglv)
    expect(loaded.filteredOut).toBe(0)
    const everything = loaded.pointCount
    expect(everything).toBeGreaterThan(META_POINTS)

    // The click that opened the preview left the Person type node selected, so
    // there is a selection for the filter to hide out from under.
    expect(loaded.selectedCount).toBe(1)

    // ── hide all but one type ───────────────────────────────────────────
    await page.getByTestId('filter-input').fill('type:Company')
    await page.waitForFunction(() => window.__kglv.filteredOut > 0, undefined, {
      timeout: 15_000,
    })
    const filtered = await page.evaluate(() => window.__kglv)
    expect(filtered.pointCount).toBeLessThan(everything)
    expect(filtered.pointCount + filtered.filteredOut).toBe(everything)
    // The slot space did NOT move: filtering is a client decision about
    // drawing, so nothing was tombstoned and nothing was fetched.
    expect(filtered.slotCount).toBe(loaded.slotCount)
    expect(filtered.tombstoneCount).toBe(loaded.tombstoneCount)
    expect(filtered.lastMessageSeq).toBe(loaded.lastMessageSeq)
    // A hidden node is not a selected node. The count is the instrument every
    // agent reads, and one that kept describing a node nobody can see is the
    // same lie a tombstone left in a set would be.
    expect(filtered.selectedCount).toBe(0)

    const banner = page.getByTestId('filter-banner')
    await expect(banner).toHaveText(
      `filter: showing ${filtered.pointCount} of ${everything} drawn`,
    )
    // Distinct from the truncation banner beside it — same voice, different
    // cause, so a test can tell which one fired.
    await expect(page.getByTestId('truncation-banner')).toHaveCount(0)
    // The labels went with the nodes.
    await expect(page.locator('.kglv-label:has-text("Person")')).toHaveCount(0)

    // ── a term nothing loaded can answer is refused, and hides nothing ──
    await page.getByTestId('filter-input').fill('depth:2000')
    await expect(page.getByTestId('filter-note')).toContainText('nothing loaded carries "depth"')
    await expect(page.getByTestId('filter-note')).toContainText('Search above')
    const refused = await page.evaluate(() => window.__kglv)
    expect(refused.filteredOut).toBe(0)
    expect(refused.pointCount).toBe(everything)
    expect(refused.lastMessageSeq).toBe(loaded.lastMessageSeq)

    // ── clear gives everything back ─────────────────────────────────────
    await page.getByTestId('filter-input').fill('type:Company')
    await page.waitForFunction(() => window.__kglv.filteredOut > 0, undefined, {
      timeout: 15_000,
    })
    await page.getByTestId('filter-clear').click()
    await page.waitForFunction(() => window.__kglv.filteredOut === 0, undefined, {
      timeout: 15_000,
    })
    const cleared = await page.evaluate(() => window.__kglv)
    expect(cleared.pointCount).toBe(everything)
    expect(cleared.slotCount).toBe(loaded.slotCount)
    await expect(page.getByTestId('filter-banner')).toHaveCount(0)
    // The selection came back too: a filter hides, it does not forget.
    expect(cleared.selectedCount).toBe(1)
  } finally {
    server?.process.kill()
  }
})
