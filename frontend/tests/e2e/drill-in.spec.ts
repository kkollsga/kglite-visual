/**
 * L3: the drill-in, end to end in a real browser against the real binary.
 *
 * The flagship flow, in the order a user performs it — select a meta-graph type
 * node, read what expanding it would cost *before* fetching anything, expand
 * one relationship under a bound, watch the truncation banner say so, hover to
 * emphasise a neighbourhood, run a query, collapse back to tombstones.
 *
 * Every assertion is on `window.__kglv` or on DOM text; none is on pixels, and
 * there is no `waitForTimeout` anywhere — cosmos.gl v3 initialises
 * asynchronously and renders on demand, so a sleep proves nothing in either
 * direction. Every expected number comes from the committed fixture and is
 * pinned exactly in the core L1 suite too, so a divergence names its own side.
 */

import { expect, test, type Page } from '@playwright/test'

import { appUrl, fillQuery, launch, queryText, type Launched } from './harness'

/** The fixture's Person type: 60 members, largest type, therefore slot 0. */
const PERSON_SLOT = 0
const META_POINTS = 5
const META_LINKS = 7
/** Persons reachable over KNOWS, in either direction. */
const KNOWS_REACHABLE = 60
/** What the `max nodes` box asks for, so the bound fires visibly. */
const EXPAND_LIMIT = 40
/**
 * The KNOWS-out links of those 40, and every KNOWS-out link the walk found.
 *
 * Observed against the fixture through the JSON twin, not derived: a link whose
 * far endpoint the node bound refused cannot be sent (it would be an index into
 * a slot the client was never given), so bounding the nodes cuts links too, and
 * the slice says which.
 */
const LINKS_SENT = 108
const LINKS_FOUND = 180

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__kglv?.ready === true, undefined, {
    timeout: 30_000,
  })
}

function state(page: Page) {
  return page.evaluate(() => window.__kglv)
}

test('the drill-in: preview, bounded expand, hover, query, collapse', async ({
  page,
}, testInfo) => {
  let server: Launched | null = null
  const consoleErrors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })

  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)

    // ── the entry screen ────────────────────────────────────────────────
    const entry = await state(page)
    expect(entry.protocolVersion).toBe(3)
    expect(entry.tier).toBe('full')
    expect(entry.pointCount).toBe(META_POINTS)
    expect(entry.linkCount).toBe(META_LINKS)
    expect(entry.slotCount).toBe(META_POINTS)
    expect(entry.tombstoneCount).toBe(0)
    expect(entry.error).toBeNull()

    // ── select the Person type node, by its label ───────────────────────
    await page.locator('.kglv-label:has-text("Person")').click()
    await expect(page.getByTestId('selection-title')).toHaveText('Person (type)')

    // The D12 preview: counts BEFORE a single node is fetched. Asserted
    // against the fixture's own truth, which the core suite pins identically.
    await expect(page.getByTestId('preview-summary')).toContainText('693 edges')
    await expect(page.getByTestId('preview-summary')).toContainText('5 relationships')
    const rows = page.getByTestId('preview-rows').locator('.kglv-preview-row')
    await expect(rows).toHaveCount(5)
    await expect(rows.filter({ hasText: 'HAS_SKILL → Skill' })).toContainText('180')
    await expect(rows.filter({ hasText: 'KNOWS → Person' })).toContainText('180')
    await expect(rows.filter({ hasText: 'WORKS_AT → Company' })).toContainText('60')

    const previewed = await state(page)
    expect(previewed.previewRows).toBe(5)
    expect(previewed.selectedCount).toBe(1)
    expect(previewed.slotCount).toBe(META_POINTS)
    // The whole point of previewing: it allocated nothing and fetched nothing.
    expect(previewed.pointCount).toBe(META_POINTS)

    // ── hover emphasises a neighbourhood, client-side ───────────────────
    // The seq counter is the proof: cosmos.gl already holds the adjacency it
    // drew, so emphasis is computed from `getNeighboringPointIndices` with no
    // server round trip (plan D7). If a hover ever started asking the server,
    // the last message sequence would move.
    const beforeHover = await state(page)
    await page.locator('.kglv-label:has-text("Person")').hover()
    await page.waitForFunction(() => window.__kglv.hoveredSlot !== null)
    const hovered = await state(page)
    expect(hovered.hoveredSlot).toBe(PERSON_SLOT)
    // Person links to Project, Skill and Company in the meta-graph, plus its
    // own KNOWS self-loop.
    expect(hovered.emphasizedCount).toBeGreaterThanOrEqual(4)
    expect(hovered.lastMessageSeq).toBe(beforeHover.lastMessageSeq)

    // A smaller neighbourhood, to prove emphasis is a neighbourhood and not
    // "everything": City is reached only by Company's LOCATED_IN.
    await page.locator('.kglv-label:has-text("City")').hover()
    await page.waitForFunction(() => window.__kglv.hoveredSlot === 3)
    const city = await state(page)
    expect(city.emphasizedCount).toBeLessThan(hovered.emphasizedCount)
    expect(city.lastMessageSeq).toBe(beforeHover.lastMessageSeq)

    // ── expand KNOWS, bounded ───────────────────────────────────────────
    await page.locator('.kglv-label:has-text("Person")').click()
    await expect(page.getByTestId('selection-title')).toHaveText('Person (type)')
    await page.getByTestId('expand-limit').fill(String(EXPAND_LIMIT))
    await page.getByTestId('expand-KNOWS-out').click()

    await page.waitForFunction(
      (expected) => window.__kglv.lastSliceKind === 'expand' && window.__kglv.slotCount === expected,
      META_POINTS + EXPAND_LIMIT,
      { timeout: 15_000 },
    )
    const expanded = await state(page)
    // The instance nodes are in the SAME slot space as the type nodes (D4).
    expect(expanded.slotCount).toBe(META_POINTS + EXPAND_LIMIT)
    expect(expanded.pointCount).toBe(META_POINTS + EXPAND_LIMIT)
    expect(expanded.tombstoneCount).toBe(0)
    expect(expanded.linkCount).toBeGreaterThan(META_LINKS)
    // D5: the bound fired and the UI says so, in the words the user reads —
    // for the links as well as the nodes. A slice that reported only its node
    // bound would let 40 nodes carrying 108 of their 180 edges read as a
    // complete neighbourhood.
    const banner =
      `showing ${EXPAND_LIMIT} of ${KNOWS_REACHABLE} nodes` +
      ` and ${LINKS_SENT} of up to ${LINKS_FOUND} links`
    expect(expanded.truncation).toEqual({
      returned: EXPAND_LIMIT,
      total: KNOWS_REACHABLE,
      truncated: true,
      banner,
    })
    await expect(page.getByTestId('truncation-banner')).toHaveText(banner)
    // Instance nodes are labelled with their titles, not with slot numbers.
    await expect(page.locator('.kglv-label:has-text("Person_")').first()).toBeVisible()

    // Property statistics arrived with the selection and marked what is not
    // exact — the D12 rule that a sampled or capped distinct-count is never
    // presented as an exact one.
    expect(expanded.appearanceCandidates).toBeGreaterThan(0)
    await expect(page.getByTestId('appearance-note')).toContainText('Person: 60 nodes')

    // ── a Cypher query, through the panel ───────────────────────────────
    await fillQuery(
      page,
      'MATCH (p:Person)-[:WORKS_AT]->(c:Company) ' +
        'RETURN c.title AS company, count(p) AS staff ORDER BY staff DESC LIMIT 3',
    )
    await page.getByTestId('query-run').click()
    const table = page.getByTestId('query-table')
    await expect(table.locator('tr')).toHaveCount(4) // header + 3 rows
    await expect(table.locator('th').first()).toHaveText('company')
    await expect(table.locator('tr').nth(1)).toContainText('Company_3')
    await expect(table.locator('tr').nth(1)).toContainText('12')
    await expect(page.getByTestId('query-status')).toContainText('3 rows')
    expect((await state(page)).queryRows).toBe(3)
    // A clean query says nothing extra. Asserted so the advisory check below
    // cannot pass by the panel simply always showing a line.
    await expect(page.getByTestId('query-warning')).toHaveCount(0)

    // ── a mistyped label is an advisory, not an empty graph ─────────────
    // kglite raises this one and it used to reach only the SERVER's stderr, so
    // the panel showed "0 rows" and the user read it as "no such nodes".
    await fillQuery(page, 'MATCH (n:Persn) RETURN n LIMIT 3')
    await page.getByTestId('query-run').click()
    await expect(page.getByTestId('query-status')).toContainText('0 rows')
    await expect(page.getByTestId('query-warning').first()).toContainText('Persn')
    // ── EXPLAIN gets a plan, not a three-column grid ────────────────────
    await fillQuery(page, 'EXPLAIN MATCH (p:Person) RETURN p.title')
    await page.getByTestId('query-run').click()
    const plan = page.getByTestId('query-plan')
    await expect(plan).toBeVisible()
    await expect(plan.locator('.kglv-plan-op').first()).toContainText('Match')
    // The generic table is what this treatment replaces, so its absence is
    // half of the assertion.
    await expect(page.getByTestId('query-table')).toHaveCount(0)
    await expect(page.getByTestId('query-status')).toContainText('not executed')

    // Back to a query that works, so the collapse below acts on the same view
    // the assertions above described.
    await fillQuery(page, 'MATCH (p:Person) RETURN p.title LIMIT 3')
    await page.getByTestId('query-run').click()
    await expect(page.getByTestId('query-warning')).toHaveCount(0)
    await expect(page.getByTestId('query-plan')).toHaveCount(0)

    // ── saved queries and recent, through the store ─────────────────────
    // The run above is the newest thing in the recent list, and clicking it
    // puts it back in the editor. History is a list to pick from, so the
    // assertion is that picking works, not that a row exists.
    await expect(page.getByTestId('query-history').locator('button').first()).toContainText(
      'MATCH (p:Person) RETURN p.title LIMIT 3',
    )
    await fillQuery(page, '')
    await page.getByTestId('query-history').locator('button').first().click()
    expect(await queryText(page)).toBe('MATCH (p:Person) RETURN p.title LIMIT 3')

    // Save it, reload the editor from somewhere else, and load it back.
    page.once('dialog', (dialog) => void dialog.accept('people'))
    await page.getByTestId('query-save').click()
    await expect(page.getByTestId('saved-note')).toContainText('1 of 64 saved')
    await fillQuery(page, 'RETURN 1')
    await page.getByTestId('saved-list').selectOption('people')
    expect(await queryText(page)).toBe('MATCH (p:Person) RETURN p.title LIMIT 3')
    await page.getByTestId('query-delete').click()
    await expect(page.getByTestId('saved-note')).toContainText('0 of 64 saved')

    // ── collapse back to the meta-graph ─────────────────────────────────
    await page.getByTestId('collapse').click()
    await page.waitForFunction(() => window.__kglv.lastSliceKind === 'collapse', undefined, {
      timeout: 15_000,
    })
    const collapsed = await state(page)
    // Tombstones, not a re-index: the slots stay allocated and the type nodes
    // keep the indices the client has been holding all along.
    expect(collapsed.tombstoneCount).toBe(EXPAND_LIMIT)
    expect(collapsed.slotCount).toBe(META_POINTS + EXPAND_LIMIT)
    expect(collapsed.pointCount).toBe(META_POINTS)
    expect(collapsed.linkCount).toBe(META_LINKS)
    // 45 slots is under the compaction minimum, so nothing was reclaimed —
    // the remap costs more than the waste at this size.
    expect(collapsed.compactions).toBe(0)
    await expect(page.locator('.kglv-label:has-text("Person")')).toHaveCount(1)

    const shotPath = testInfo.outputPath('drill-in.png')
    await page.screenshot({ path: shotPath })
    await testInfo.attach('drill-in.png', { path: shotPath, contentType: 'image/png' })

    expect(consoleErrors, `browser console errors: ${consoleErrors.join(' | ')}`).toEqual([])
  } finally {
    server?.process.kill()
  }
})

test('a full expansion compacts on collapse and the client applies the remap', async ({
  page,
}) => {
  let server: Launched | null = null
  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)

    // No limit this time: all 60 Persons, which puts the view at 65 slots —
    // over the compaction minimum, so collapsing it reclaims them.
    await page.locator('.kglv-label:has-text("Person")').click()
    await page.getByTestId('expand-KNOWS-out').click()
    await page.waitForFunction(
      (expected) => window.__kglv.slotCount === expected,
      5 + KNOWS_REACHABLE,
      { timeout: 15_000 },
    )
    const full = await state(page)
    expect(full.truncation?.truncated).toBe(false)
    expect(full.pointCount).toBe(5 + KNOWS_REACHABLE)

    await page.getByTestId('collapse').click()
    await page.waitForFunction(() => window.__kglv.compactions === 1, undefined, {
      timeout: 15_000,
    })
    const compacted = await state(page)
    expect(compacted.slotCount).toBe(META_POINTS)
    expect(compacted.pointCount).toBe(META_POINTS)
    expect(compacted.tombstoneCount).toBe(0)
    expect(compacted.linkCount).toBe(META_LINKS)
    // A compaction renumbers everything, so the selection it invalidated is
    // dropped rather than carried across onto whatever moved into those slots.
    expect(compacted.selectedCount).toBe(0)
    // The five type nodes kept slots 0..4, so their labels are still theirs.
    await expect(page.locator('.kglv-label:has-text("Person")')).toHaveCount(1)
    await expect(page.locator('.kglv-label:has-text("Company")')).toHaveCount(1)
  } finally {
    server?.process.kill()
  }
})

test('server-side search highlights what is loaded and offers to load the rest', async ({
  page,
}) => {
  let server: Launched | null = null
  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)

    // Nothing is loaded yet, so every hit is cold: the answer is a list plus a
    // bounded "load into view", never a client-side index over data the
    // browser does not have (plan D7).
    await page.getByTestId('search-type').selectOption('Person')
    await page.getByTestId('search-input').fill('person_1')
    await page.getByTestId('search-run').click()
    await page.waitForFunction(() => window.__kglv.searchHits > 0, undefined, { timeout: 15_000 })

    await expect(page.getByTestId('search-status')).toContainText('hits on title')
    const hits = page.getByTestId('search-hits').locator('.kglv-hit')
    await expect(hits.first()).toContainText('not loaded')
    const cold = await hits.count()
    expect(cold).toBeGreaterThan(0)

    await page.getByTestId('search-load').click()
    await page.waitForFunction(
      (expected) => window.__kglv.pointCount === expected,
      META_POINTS + cold,
      { timeout: 15_000 },
    )
    const loaded = await state(page)
    expect(loaded.lastSliceKind).toBe('query')
    expect(loaded.pointCount).toBe(META_POINTS + cold)

    // Search again: the same hits are now on screen, so they come back with
    // slots and become the `highlighted` interaction concept.
    await page.getByTestId('search-run').click()
    await page.waitForFunction(() => window.__kglv.highlightedCount > 0, undefined, {
      timeout: 15_000,
    })
    const warm = await state(page)
    expect(warm.highlightedCount).toBe(cold)
    await expect(hits.first()).not.toContainText('not loaded')
  } finally {
    server?.process.kill()
  }
})
