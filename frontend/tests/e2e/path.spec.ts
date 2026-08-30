/**
 * L3: a two-hop path built from dropdowns, previewed, and run (plan E9).
 *
 * The three things this spec exists to hold, in order: the picker offers only
 * hops the graph has, the count probes answer before anything is drawn, and
 * **the Cypher shown is the Cypher that ran** — asserted by comparing the strip
 * against the editor after Run, not by trusting that they were built from the
 * same string.
 */

import { expect, test, type Page } from '@playwright/test'

import { appUrl, launch, queryText, type Launched } from './harness'

const META_POINTS = 5
/** Every Person works at a Company, and every Company is in exactly one City. */
const WORKS_AT_ROWS = 60

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__kglv?.ready === true, undefined, { timeout: 30_000 })
}

test('the path builder offers real hops, counts them, and runs what it shows', async ({
  page,
}) => {
  let server: Launched | null = null
  const consoleErrors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })

  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)

    const start = page.locator('[data-testid="path-start"]')
    await expect(start).toBeVisible()
    await start.selectOption('Person')

    // The hop picker is filled from the meta-graph, so it offers exactly the
    // relationships Person actually has — with the direction in the label and
    // the graph-wide edge count beside it.
    await page.locator('[data-testid="path-add"]').click()
    const hop1 = page.locator('[data-testid="path-hop-1"]')
    const offered = await hop1.locator('option').allTextContents()
    expect(offered).toEqual([
      'HAS_SKILL → Skill (180)',
      // One entry, not two: KNOWS is a self-loop, and offering it as "out" and
      // "in" separately would double its count as well as its rows.
      'KNOWS ↔ Person (180)',
      'CONTRIBUTES_TO → Project (93)',
      'WORKS_AT → Company (60)',
    ])
    await hop1.selectOption('WORKS_AT|out|Company')

    // Hop two's options come from where hop one landed — Company, not Person.
    await page.locator('[data-testid="path-add"]').click()
    const hop2 = page.locator('[data-testid="path-hop-2"]')
    expect(await hop2.locator('option').allTextContents()).toEqual([
      'WORKS_AT ← Person (60)',
      'OWNS → Project (16)',
      'LOCATED_IN → City (8)',
    ])
    await hop2.selectOption('LOCATED_IN|out|City')

    // The generated query is on screen the whole time, not behind a toggle.
    const strip = page.locator('[data-testid="path-query"]')
    await expect(strip).toHaveText(
      'MATCH (n0:Person)-[r1:WORKS_AT]->(n1:Company)-[r2:LOCATED_IN]->(n2:City)\n' +
        'RETURN n0, r1, n1, r2, n2',
    )

    // The card says what Run will do. On this fixture the answer is well under
    // the server's row ceiling, so it is the plain sentence; the warning that
    // replaces it above the ceiling is `showNote`'s other branch, and it was
    // observed firing on sodir at 1 941 015 rows.
    await expect(page.locator('[data-testid="path-note"]')).toContainText(
      'exactly what Run sends',
    )

    // …and the count probes answer, per hop, before anything is drawn.
    await expect(page.locator('[data-testid="path-count-1"]')).toHaveText(
      `${WORKS_AT_ROWS} rows`,
      { timeout: 15_000 },
    )
    await expect(page.locator('[data-testid="path-count-2"]')).toHaveText(
      `${WORKS_AT_ROWS} rows`,
      { timeout: 15_000 },
    )
    // Nothing has been loaded yet: a preview that quietly fetched would be the
    // opposite of what a preview is for.
    expect(await page.evaluate(() => window.__kglv.slotCount)).toBe(META_POINTS)

    const shown = await strip.textContent()
    await page.locator('[data-testid="path-run"]').click()
    await page.waitForFunction(() => window.__kglv.lastSliceKind === 'query', undefined, {
      timeout: 15_000,
    })

    // The path landed as nodes: 60 people, 8 companies, and the cities they
    // are in — all in the same slot space as the type nodes (D4).
    const after = await page.evaluate(() => window.__kglv)
    expect(after.slotCount).toBeGreaterThan(META_POINTS + WORKS_AT_ROWS)
    expect(after.linkCount).toBeGreaterThan(0)

    // A query that answered with a graph says so, rather than leaving whatever
    // row count was last on screen to describe it.
    await expect(page.locator('[data-testid="query-status"]')).toContainText(
      'nodes in the result —',
    )

    // The assertion the whole card rests on: what ran is what was shown.
    expect(await queryText(page)).toBe(shown)

    expect(consoleErrors, `browser console errors: ${consoleErrors.join(' | ')}`).toEqual([])
  } finally {
    server?.process.kill()
  }
})

test('a filter is bound, not written, and narrows the count it is beside', async ({ page }) => {
  let server: Launched | null = null
  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)

    await page.locator('[data-testid="path-start"]').selectOption('Person')
    await page.locator('[data-testid="path-add"]').click()
    await page.locator('[data-testid="path-hop-1"]').selectOption('WORKS_AT|out|Company')
    await expect(page.locator('[data-testid="path-count-1"]')).toHaveText('60 rows', {
      timeout: 15_000,
    })

    // The start node's filter. Its property list is the same `property-stats`
    // the editor's completions read, fetched lazily — so the option has to be
    // waited for rather than assumed present.
    const filter = page.locator('[data-testid="path-filter-0"]')
    await expect(filter.locator('option[value="title"]')).toHaveCount(1, { timeout: 15_000 })
    await filter.selectOption('title')
    await page.locator('[data-testid="path-op-0"]').selectOption('contains')
    await page.locator('[data-testid="path-value-0"]').fill('Person_1')

    // `contains` folds case on both sides, exactly as the server's own search
    // does, and the needle is a PARAMETER: the strip shows `$p0`, never the
    // text that was typed.
    const strip = page.locator('[data-testid="path-query"]')
    await expect(strip).toContainText('WHERE toLower(toString(n0.title)) CONTAINS $p0')
    expect(await strip.textContent()).not.toContain('Person_1')

    // Person_1 and Person_10..Person_19 — eleven of the sixty.
    await expect(page.locator('[data-testid="path-count-1"]')).toHaveText('11 rows', {
      timeout: 15_000,
    })
  } finally {
    server?.process.kill()
  }
})
