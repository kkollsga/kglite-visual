/** Legend assertions consume the acknowledged canonical domain, including its category bound. */

import { keepSchemaContext, openDrawer, closeDrawer } from './navigation'

import { expect, test, type Page } from '@playwright/test'
import { typedText } from '../../src/cells'

import { appUrl, launch, Listener, type Launched } from './harness'

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__kglv?.ready === true, undefined, {
    timeout: 30_000,
  })
}

test('the legend lists the encoding in force, and changes when it does', async ({ page }) => {
  let server: Launched | null = null
  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)
    await keepSchemaContext(page)

    // ── structural, and collapsed ───────────────────────────────────────
    const body = page.getByTestId('legend-body')
    await expect(body).toBeHidden()
    await page.getByTestId('legend-toggle').click()
    await expect(body).toBeVisible()
    await expect(body).toContainText('colour — structural')
    await expect(body).toContainText('type with capabilities')
    await expect(body).toContainText('grows with its member count')

    // ── expand, so instance nodes are on screen and have properties ─────
    await closeDrawer(page)
    await page.locator('.kglv-label:has-text("Person")').click()
    await page.getByTestId('expand-KNOWS-out').click()
    await page.waitForFunction(() => window.__kglv.lastSliceKind === 'expand', undefined, {
      timeout: 15_000,
    })
    // An instance type now has a row of its own — the hue the canvas is
    // drawing those nodes in.
    await expect(body).toContainText('Person (instance)')

    // ── colour by a categorical property ────────────────────────────────
    const stats = await page.evaluate(async (base) => {
      const response = await fetch(`${base}api/property-stats`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ node_type: 'Person' }),
      })
      return (await response.json()) as {
        categorical_candidates: string[]
        properties: { name: string; values: unknown[] }[]
      }
    }, server.info.url)
    const property = stats.categorical_candidates[0]
    expect(property, 'the fixture must offer a categorical channel').toBeTruthy()
    const values = stats.properties.find((p) => p.name === property)?.values ?? []
    expect(values.length).toBeGreaterThan(1)

    await openDrawer(page, 'appearance')
    await page.getByTestId('color-by').selectOption(property as string)
    await page.waitForFunction((name) => window.__kglv.colorBy === name, property, {
      timeout: 15_000,
    })
    // Choosing a colour opens the card: the one encoding nobody can read off
    // the picture is the one the user just invented.
    await expect(body).toBeVisible()
    await expect(body).toContainText(`colour — ${property}`)
    const listener = new Listener(server.info.url.replace(/^http/, 'ws') + 'ws')
    await listener.open()
    const message = listener.received.find(message => message.kind === 'shared-update')
    listener.close()
    if (message?.kind !== 'shared-update') throw new Error('Missing canonical appearance mapping')
    const mapping = message.value.meta.snapshot.appearance_mapping
    expect(mapping.categories).toHaveLength(Math.min(8, values.length))
    for (const category of mapping.categories) await expect(body).toContainText(`${typedText(category.value)} · ${category.count}`)
    expect(mapping.categories.length + mapping.other_categories).toBe(values.length)
    if (mapping.other_categories > 0) await expect(body).toContainText(`other · ${mapping.other_categories} categories`)
    await expect(body).toContainText('Domain: loaded instances')
    // The structural rows are GONE — the legend describes what is drawn, not
    // everything it knows how to describe. Without this the assertions above
    // would pass on a card that simply lists both encodings forever.
    await expect(body).not.toContainText('type with capabilities')

    const withPalette = await page.evaluate(() => window.__kglv.legendEntries)
    expect(withPalette).toBeGreaterThanOrEqual(mapping.categories.length + (mapping.other_categories > 0 ? 1 : 0))

    // ── back to structural ──────────────────────────────────────────────
    await openDrawer(page, 'appearance')
    await page.getByTestId('color-by').selectOption('')
    await page.waitForFunction(() => window.__kglv.colorBy === null, undefined, {
      timeout: 15_000,
    })
    await expect(body).toContainText('colour — structural')
    await expect(body).not.toContainText(`colour — ${property}`)
  } finally {
    server?.process.kill()
  }
})
