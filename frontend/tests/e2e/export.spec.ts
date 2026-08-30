/**
 * L3: the view leaves the viewer (plan E8).
 *
 * Two questions, and the HTTP one is the load-bearing half. What a user clicks
 * is an `<a href download>`, which Playwright cannot follow into the operating
 * system's save dialog — so the button is asserted as *what it points at* and
 * the file itself is fetched over the same URL the anchor holds.
 *
 * The scope assertion is the one that matters most: the fixture holds 118
 * nodes and the expansion loads 60, so a handler that had reached kglite's
 * whole-graph mode produces a file with 118 in it and this spec says so.
 */

import { expect, test, type Page } from '@playwright/test'

import { appUrl, launch, type Launched } from './harness'

const PERSON_SLOT = 0
const META_POINTS = 5
const KNOWS_REACHABLE = 60
/** Every node in the fixture — what a whole-graph dump would have written. */
const FIXTURE_NODES = 118

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__kglv?.ready === true, undefined, { timeout: 30_000 })
}

test('the export endpoint writes the view, names the file and states its caveats', async () => {
  let server: Launched | null = null
  try {
    server = await launch()
    const url = (query: string): string => `${server?.info.url}api/export?${query}`

    // An empty view is refused by name rather than answered with an empty
    // file: the whole-graph dump this endpoint must never produce would be
    // the *easiest* thing to answer here.
    const empty = await fetch(url('format=csv&source=live-view'))
    expect(empty.status).toBe(400)
    expect((await empty.json()).error).toContain('nothing to export')

    const expand = await fetch(`${server.info.url}api/expand`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ slot: PERSON_SLOT, relationship: 'KNOWS', direction: 'out' }),
    })
    expect(expand.status).toBe(200)

    const csv = await fetch(url('format=csv&source=live-view'))
    expect(csv.status).toBe(200)
    expect(csv.headers.get('content-type')).toBe('text/csv; charset=utf-8')
    expect(csv.headers.get('x-kglv-nodes')).toBe(String(KNOWS_REACHABLE))
    expect(csv.headers.get('x-kglv-note')).toContain('MORE than you saw')
    // Named twice: an ASCII fallback and the RFC 5987 form that carries the
    // real bytes. The fixture's own name is ASCII, so what is asserted here is
    // that both parameters are present and agree.
    const disposition = csv.headers.get('content-disposition') ?? ''
    expect(disposition).toContain('attachment;')
    expect(disposition).toContain('filename="meta-view.csv"')
    expect(disposition).toContain("filename*=UTF-8''meta-view.csv")

    const rows = (await csv.text()).trim().split('\n')
    expect(rows[0]).toBe('id,type,title')
    expect(
      rows.length - 1,
      `the export wrote ${rows.length - 1} nodes; the view holds ${KNOWS_REACHABLE} and the ` +
        `graph holds ${FIXTURE_NODES}`,
    ).toBe(KNOWS_REACHABLE)
    expect(rows.length - 1).toBeLessThan(FIXTURE_NODES)

    // GraphML carries the upstream caveat as well as the edge one, because it
    // is the format whose node names import wrong.
    const graphml = await fetch(url('format=graphml&source=live-view'))
    expect(graphml.headers.get('x-kglv-note')).toContain('Gephi reads attr.name="label"')
    expect(await graphml.text()).toContain('<graphml')

    // A format nobody offers names the ones that exist, rather than answering
    // with a default the caller did not ask for.
    const wrong = await fetch(url('format=xlsx&source=live-view'))
    expect(wrong.status).toBe(400)
    expect((await wrong.json()).error).toContain('graphml')

    // …and so does a source. `meta` is a real render source, which is exactly
    // why an export must refuse it instead of quietly exporting the live view.
    const source = await fetch(url('format=csv&source=meta'))
    expect(source.status).toBe(400)
    expect((await source.json()).error).toContain('live-view')
  } finally {
    server?.process.kill()
  }
})

test('the export card offers the formats and refuses the click on an empty view', async ({
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

    await page.locator('[data-testid="export-toggle"]').click()
    await expect(page.locator('[data-testid="export-body"]')).toBeVisible()

    // The entry screen has no instances, so every link is inert and the note
    // says what to do about it — rather than the user downloading a JSON error.
    const graphml = page.locator('[data-testid="export-graphml"]')
    await expect(graphml).toHaveAttribute('aria-disabled', 'true')
    expect(await graphml.getAttribute('href')).toBeNull()
    await expect(page.locator('[data-testid="export-note"]')).toContainText('nothing to export')
    expect(await page.evaluate(() => window.__kglv.exportNodes)).toBe(0)

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

    expect(await page.evaluate(() => window.__kglv.exportNodes)).toBe(KNOWS_REACHABLE)
    await expect(graphml).toHaveAttribute('aria-disabled', 'false')
    const href = await graphml.getAttribute('href')
    expect(href).toContain('api/export?format=graphml&source=live-view')
    await expect(page.locator('[data-testid="export-note"]')).toContainText('60 nodes')

    // The anchor is not decoration: what it points at answers with the file.
    const response = await fetch(href as string)
    expect(response.status).toBe(200)
    expect(response.headers.get('x-kglv-nodes')).toBe(String(KNOWS_REACHABLE))

    expect(consoleErrors, `browser console errors: ${consoleErrors.join(' | ')}`).toEqual([])
  } finally {
    server?.process.kill()
  }
})
