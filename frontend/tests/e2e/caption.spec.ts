/**
 * Captions: a type's nodes drawn under a property instead of their title
 * (plan E11).
 *
 * **The committed fixture cannot exercise the *automatic* half, and that is a
 * fact about the fixture rather than a gap.** Auto-caption fires only where a
 * type's title is inadequate — badly covered, or a handful of distinct values
 * across thousands of nodes — and every type in `meta.kgl` is titled
 * `Person_0`, `City_3`, `Company_5`: well covered and near-unique, so the
 * server correctly suggests nothing. The suggestion's ranking is pinned against
 * sodir's real shapes in `core::stats`'s unit tests, where a synthetic stat can
 * be given a useless title on purpose.
 *
 * What this spec owns is the half a unit test cannot reach: the values are
 * fetched, the labels on the canvas change, no slice is re-sent, and an
 * override survives the property statistics arriving again.
 */

import { expect, test, type Page } from '@playwright/test'

import { appUrl, launch, type Launched } from './harness'

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__kglv?.ready === true, undefined, {
    timeout: 30_000,
  })
}

test('a caption redraws the labels, and an override sticks', async ({ page }) => {
  let server: Launched | null = null
  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)

    // The server suggests nothing for this fixture, because its titles are
    // already names. A suggestion here would be the failure the title gate
    // exists to prevent, so its absence is the assertion.
    const suggested = await page.evaluate(async (base) => {
      const response = await fetch(`${base}api/property-stats`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ node_type: 'Person' }),
      })
      return ((await response.json()) as { caption_candidate: string | null }).caption_candidate
    }, server.info.url)
    expect(suggested).toBeNull()

    // Load Person instances and open their statistics.
    await page.locator('.kglv-label:has-text("Person")').click()
    await page.getByTestId('expand-KNOWS-out').click()
    await page.waitForFunction(() => window.__kglv.lastSliceKind === 'expand', undefined, {
      timeout: 15_000,
    })
    await expect(page.getByTestId('appearance-note')).toContainText('Person: 60 nodes')
    await expect(page.getByTestId('caption-by')).toHaveValue('')
    await expect(page.locator('.kglv-label:has-text("Person_")').first()).toBeVisible()

    // ── caption by a property that is not the title ─────────────────────
    const before = await page.evaluate(() => window.__kglv)
    await page.getByTestId('caption-by').selectOption('city')
    await page.waitForSelector('.kglv-label:has-text("City_")', { timeout: 15_000 })
    // The chips now carry city names where they carried person names.
    await expect(page.locator('.kglv-label:has-text("Person_")')).toHaveCount(0)

    const after = await page.evaluate(() => window.__kglv)
    // No slice was re-sent: the nodes are already on screen with the identity
    // the server gave them, and a caption changes only the string this client
    // draws over each one.
    expect(after.lastSliceKind).toBe(before.lastSliceKind)
    expect(after.pointCount).toBe(before.pointCount)
    expect(after.slotCount).toBe(before.slotCount)

    // ── the override survives the statistics arriving again ─────────────
    // Re-selecting the type re-sends `property-stats`, and the server's own
    // suggestion rides in with it. A handler that re-seeded from the response
    // would walk the user's choice back while they watched.
    await page.locator('.kglv-label:has-text("City_")').first().click()
    await page.locator('.kglv-label[data-slot="0"]').click()
    await expect(page.getByTestId('appearance-note')).toContainText('Person: 60 nodes')
    await expect(page.getByTestId('caption-by')).toHaveValue('city')
    await expect(page.locator('.kglv-label:has-text("City_")').first()).toBeVisible()

    // ── back to the stored title ────────────────────────────────────────
    await page.getByTestId('caption-by').selectOption('')
    await page.waitForSelector('.kglv-label:has-text("Person_")', { timeout: 15_000 })
    await expect(page.getByTestId('caption-by')).toHaveValue('')
  } finally {
    server?.process.kill()
  }
})
