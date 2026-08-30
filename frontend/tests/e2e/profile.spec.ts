/**
 * L3: `PROFILE` puts what each clause cost beside the answer it produced (E10).
 *
 * Two halves, and the negative one is what makes the positive one mean
 * something: the same query without the prefix must draw *no* profile panel. A
 * wiring that attached an empty profile to every result, or a panel that
 * rendered its own header unconditionally, would satisfy the positive half
 * alone and be wrong in the way a user would notice — a "profile" appearing for
 * a query they never asked to profile.
 *
 * The bar widths are read as computed styles rather than asserted against the
 * microsecond numbers, because the numbers are a real machine's timings and
 * nothing here should depend on which clause happens to be slowest. What is
 * asserted is the invariant the bar claims: the slowest clause fills its track,
 * and no clause is drawn as nothing.
 */

import { expect, test, type Page } from '@playwright/test'

import { appUrl, fillQuery, launch, type Launched } from './harness'

const QUERY = 'MATCH (p:Person)-[:WORKS_AT]->(c:Company) RETURN c.title AS company, count(p) AS staff'

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__kglv?.ready === true, undefined, { timeout: 30_000 })
}

async function run(page: Page, query: string): Promise<void> {
  await fillQuery(page, query)
  await page.getByTestId('query-run').click()
  await page.waitForFunction(() => window.__kglv.queryRows > 0, undefined, { timeout: 15_000 })
}

test('PROFILE reports each clause, and an unprofiled query reports none', async ({ page }) => {
  let server: Launched | null = null
  const consoleErrors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })

  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)

    const panel = page.getByTestId('query-profile')

    // ── the negative half: no prefix, no panel ──────────────────────────
    await run(page, QUERY)
    await expect(panel).toBeHidden()
    // Both, because an empty `<div>` has no size and Playwright already calls
    // that hidden — so `toBeHidden` alone would pass for a panel that renders
    // its header and rows for every query and merely happens to be empty here.
    await expect(panel.locator('.kglv-profile-row')).toHaveCount(0)

    // ── the positive half ───────────────────────────────────────────────
    await run(page, `PROFILE ${QUERY}`)
    await expect(panel).toBeVisible()

    // The rows are still data: PROFILE executes, unlike EXPLAIN, so the grid
    // is there beside the profile rather than replaced by it.
    await expect(page.getByTestId('query-table')).toBeVisible()

    const rows = panel.locator('.kglv-profile-row')
    const clauses = await rows.count()
    expect(clauses, 'a profiled query has at least one clause').toBeGreaterThan(0)

    // The header counts the rows it drew, which is the check that the summary
    // and the list cannot disagree.
    await expect(panel.locator('.kglv-hint')).toHaveText(
      new RegExp(`^profile — ${clauses} clause`),
    )

    // Every clause names itself and carries a rows-in → rows-out pair.
    const names = await rows.locator('.kglv-profile-clause').allTextContents()
    expect(names.every((name) => name.trim().length > 0)).toBe(true)
    expect(names.join(' ').toUpperCase()).toContain('MATCH')
    for (const flow of await rows.locator('.kglv-profile-rows').allTextContents()) {
      expect(flow).toMatch(/^[\d,]+ → [\d,]+$/)
    }

    // The bar: the widest clause fills its track, and none is drawn as nothing.
    const widths = await rows
      .locator('.kglv-profile-fill')
      .evaluateAll((nodes) =>
        nodes.map((node) => {
          const fill = node as HTMLElement
          const track = fill.parentElement as HTMLElement
          return track.clientWidth > 0 ? fill.clientWidth / track.clientWidth : 0
        }),
      )
    expect(widths).toHaveLength(clauses)
    expect(Math.max(...widths), 'the slowest clause fills its track').toBeGreaterThan(0.95)
    expect(Math.min(...widths), 'no clause is drawn as a zero-width bar').toBeGreaterThan(0)

    // ── and it clears when the next query is not profiled ───────────────
    await run(page, QUERY)
    await expect(panel).toBeHidden()
    // Both, because an empty `<div>` has no size and Playwright already calls
    // that hidden — so `toBeHidden` alone would pass for a panel that renders
    // its header and rows for every query and merely happens to be empty here.
    await expect(panel.locator('.kglv-profile-row')).toHaveCount(0)

    expect(consoleErrors, `browser console errors: ${consoleErrors.join(' | ')}`).toEqual([])
  } finally {
    server?.process.kill()
  }
})
