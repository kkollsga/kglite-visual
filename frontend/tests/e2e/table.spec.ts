/**
 * L3: a type's nodes as rows, and the columns sorted by clicking them (E9).
 *
 * The numeric assertion is the one that earns the test. `age` on the fixture's
 * people runs into the seventies, so a column sorted as *text* puts 9 above 71
 * and 10 below 9 — a table that looks sorted and is wrong. Reading the column
 * back as numbers and asserting it is monotonic is what separates the two.
 */

import { expect, test, type Page } from '@playwright/test'

import { appUrl, launch, queryText, type Launched } from './harness'

const META_POINTS = 5
const KNOWS_REACHABLE = 60

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__kglv?.ready === true, undefined, { timeout: 30_000 })
}

/** One column of the results grid, by header name, as it is drawn. */
async function column(page: Page, name: string): Promise<string[]> {
  const headers = await page
    .locator('[data-testid="query-table"] th')
    .allTextContents()
  // The active column's header carries its direction arrow, so the match is on
  // the name the button starts with rather than on equality.
  const index = headers.findIndex((text) => text.trim().split(' ')[0] === name)
  expect(index, `no column named ${name} in ${headers.join(', ')}`).toBeGreaterThanOrEqual(0)
  return page
    .locator(`[data-testid="query-table"] tr:not(:first-child) td:nth-child(${index + 1})`)
    .allTextContents()
}

test('the type panel builds a table of what is on screen, and its columns sort', async ({
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

    // Select the type first — that is what fetches its property statistics,
    // and the statistics are where the table's columns come from — then drill
    // in, so there is something for the table to be *of*.
    await page.locator('.kglv-label:has-text("Person")').click()
    await expect(page.getByTestId('selection-title')).toHaveText('Person (type)')
    // The action is not offered on a type with nothing loaded: the query would
    // be `id(n) IN []`, and an empty grid reads as "this type has no
    // properties" rather than "you have not expanded it".
    const action = page.locator('[data-testid="type-table"]')
    await expect(action).toBeHidden()

    await page.getByTestId('expand-KNOWS-out').click()
    await page.waitForFunction(
      (expected) => window.__kglv.slotCount === expected,
      META_POINTS + KNOWS_REACHABLE,
      { timeout: 15_000 },
    )
    await expect(action).toBeVisible()
    await expect(action).toHaveText(`table of ${KNOWS_REACHABLE} on screen`)

    await action.click()
    await page.waitForFunction(() => window.__kglv.queryRows > 0, undefined, { timeout: 15_000 })

    // The teaching-tool rule: what ran is in the editor, in the user's hands.
    const shown = await queryText(page)
    expect(shown).toContain('MATCH (n:Person)')
    expect(shown).toContain('WHERE id(n) IN $ids')
    // The fixture's Person carries its own `id` property, so the node handle
    // takes the disambiguated alias — the collision that used to make this
    // action a syntax error.
    expect(shown).toContain('RETURN id(n) AS node_id')
    expect(shown).toContain('n.age AS age')

    expect(await page.evaluate(() => window.__kglv.queryRows)).toBe(KNOWS_REACHABLE)

    expect(await column(page, 'node_id')).toHaveLength(KNOWS_REACHABLE)

    // NUMERIC, not lexical — and `node_id` is the column that can tell them
    // apart, because the fixture's people occupy indices 58..117 and a text
    // sort puts "100" before "58". (`age` cannot: every value is two digits, so
    // both comparisons agree and a lexical sort would pass.)
    await page.locator('[data-testid="sort-node_id"]').click()
    const ascending = (await column(page, 'node_id')).map(Number)
    expect(ascending).toHaveLength(KNOWS_REACHABLE)
    expect(ascending.every(Number.isFinite)).toBe(true)
    expect([...ascending].sort((a, b) => a - b)).toEqual(ascending)
    const asText = [...ascending].map(String).sort()
    expect(
      asText.join(','),
      'the column came back in text order, which is what a numeric column must not do',
    ).not.toBe(ascending.map(String).join(','))

    // A second click on the same header flips it rather than re-sorting.
    await page.locator('[data-testid="sort-node_id"]').click()
    const descending = (await column(page, 'node_id')).map(Number)
    expect(descending).toEqual([...ascending].reverse())

    // A string column sorts as a string, and does not throw on the way.
    await page.locator('[data-testid="sort-title"]').click()
    const titles = await column(page, 'title')
    expect([...titles].sort((a, b) => a.localeCompare(b))).toEqual(titles)

    expect(consoleErrors, `browser console errors: ${consoleErrors.join(' | ')}`).toEqual([])
  } finally {
    server?.process.kill()
  }
})
