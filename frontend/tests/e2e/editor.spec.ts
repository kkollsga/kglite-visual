/**
 * L3: the query editor upgrades itself, and the upgrade is the thing asserted.
 *
 * The panel paints a `<textarea>` and then dynamically imports CodeMirror. Both
 * halves are behaviour a user gets, so both are checked: that the swap actually
 * happens (the textarea is gone, no fallback note is showing), that the
 * tokenizer runs (a keyword and an identifier are not the same colour), that
 * the caret is not the only thing that survived (Ctrl/Cmd+Enter still runs, the
 * chord the textarea bound), and that undo works — CodeMirror takes the
 * document away from the browser, so the textarea's free native undo is a thing
 * this editor has to put back.
 *
 * Colours are read as computed styles rather than as class names on purpose:
 * CodeMirror mints its own obfuscated class names (`ͼ8`), so a class assertion
 * would pin a build detail and say nothing about what the user sees.
 */

import { expect, test, type Page } from '@playwright/test'

import { appUrl, fillQuery, launch, queryText, type Launched } from './harness'

const HOST = '[data-testid="query-editor"]'

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__kglv?.ready === true, undefined, {
    timeout: 30_000,
  })
}

/** The computed colour of the first token whose text is exactly `text`. */
async function tokenColor(page: Page, text: string): Promise<string> {
  return page
    .locator(`${HOST} .cm-line span`)
    .filter({ hasText: new RegExp(`^${text.replace(/[.$]/g, '\\$&')}$`) })
    .first()
    .evaluate((node) => getComputedStyle(node).color)
}

test('the query editor: CodeMirror replaces the textarea and keeps its contract', async ({
  page,
}) => {
  let server: Launched | null = null
  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)

    // ── the swap happened ───────────────────────────────────────────────
    await expect(page.locator(`${HOST} .cm-content`)).toBeVisible()
    // Both halves: the textarea is gone AND the note that would have explained
    // its survival is gone. Asserting only the first would pass on an app that
    // lost its editor entirely.
    await expect(page.locator(`${HOST} textarea`)).toHaveCount(0)
    await expect(page.getByTestId('editor-note')).toHaveCount(0)

    // ── the tokenizer runs ──────────────────────────────────────────────
    await fillQuery(page, 'MATCH (w:Wellbore)-[:KNOWS]->(f) WHERE w.title > 1 RETURN $p')
    const keyword = await tokenColor(page, 'MATCH')
    const identifier = await tokenColor(page, 'w')
    const label = await tokenColor(page, ':Wellbore')
    const relationship = await tokenColor(page, ':KNOWS')
    const property = await tokenColor(page, '.title')
    const parameter = await tokenColor(page, '$p')
    // Five distinct colours. A tokenizer that fell back to one style for
    // everything — the failure a "highlighting works" screenshot would miss —
    // collapses this set.
    expect(new Set([keyword, identifier, label, relationship, property, parameter]).size).toBe(6)
    // Named separately because these two are the halves of the meta-graph, and
    // telling a node label from a relationship type is the editor's job.
    expect(label).not.toBe(relationship)

    // ── Ctrl/Cmd+Enter still runs ───────────────────────────────────────
    await fillQuery(page, 'MATCH (p:Person) RETURN p.title AS who LIMIT 3')
    await page.locator(`${HOST} .cm-content`).click()
    await page.keyboard.press('ControlOrMeta+Enter')
    await expect(page.getByTestId('query-status')).toContainText('3 rows')
    await expect(page.getByTestId('query-table').locator('th').first()).toHaveText('who')

    // ── undo, which a textarea got from the browser for free ────────────
    await page.keyboard.press('End')
    await page.keyboard.type(' // scratch')
    expect(await queryText(page)).toContain('// scratch')
    await page.keyboard.press('ControlOrMeta+z')
    await expect
      .poll(() => queryText(page))
      .toBe('MATCH (p:Person) RETURN p.title AS who LIMIT 3')
  } finally {
    server?.process.kill()
  }
})

/** Type at the caret and read back whatever the completion list is offering. */
async function offered(page: Page, typed: string): Promise<string[]> {
  await fillQuery(page, '')
  await page.locator(`${HOST} .cm-content`).click()
  await page.keyboard.type(typed, { delay: 20 })
  await page.locator('.cm-tooltip-autocomplete').waitFor({ timeout: 10_000 })
  // The label span, not the whole row: a property row also carries the type it
  // belongs to as detail text, so `li.textContent` reads "titlePerson".
  return page
    .locator('.cm-tooltip-autocomplete li .cm-completionLabel')
    .evaluateAll((items) => items.map((item) => item.textContent ?? ''))
}

test('completions come from this graph, not from a word list', async ({ page }) => {
  let server: Launched | null = null
  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)
    await expect(page.locator(`${HOST} .cm-content`)).toBeVisible()

    // ── ':' in a node pattern offers the fixture's node labels ───────────
    const labels = await offered(page, 'MATCH (p:Pe')
    expect(labels).toContain('Person')
    // Never the relationship vocabulary: the two are different halves of the
    // schema and offering both would make the list useless in both positions.
    expect(labels).not.toContain('KNOWS')

    // ── ':' inside brackets offers relationship types ────────────────────
    const relationships = await offered(page, 'MATCH (p)-[:KN')
    expect(relationships).toContain('KNOWS')
    expect(relationships).not.toContain('Person')

    // ── '.' after an alias offers that type's properties ─────────────────
    // These are not in hand when the editor opens: the type's `property-stats`
    // are fetched on the first ask, so this also proves the re-query that lets
    // a late answer reach an already-open list.
    await expect
      .poll(() => offered(page, 'MATCH (p:Person) RETURN p.ti'), { timeout: 15_000 })
      .toContain('title')

    // ── an alias the scan cannot bind offers nothing ─────────────────────
    // Rather than every property of every type, which is a list you can only
    // filter by already knowing the answer.
    await fillQuery(page, '')
    await page.locator(`${HOST} .cm-content`).click()
    await page.keyboard.type('MATCH (p:Person) RETURN q.ti', { delay: 20 })
    await expect(page.locator('.cm-tooltip-autocomplete')).toHaveCount(0)
  } finally {
    server?.process.kill()
  }
})

test('diagnostics come from the engine, on an idle timer', async ({ page }) => {
  let server: Launched | null = null
  // Counted from the browser's side, so this measures the requests that were
  // actually sent rather than the calls the editor believes it debounced.
  let requests = 0
  page.on('request', (request) => {
    if (request.url().includes('/api/validate')) requests += 1
  })
  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)
    await expect(page.locator(`${HOST} .cm-content`)).toBeVisible()

    // ── a syntax error underlines, and is listed ────────────────────────
    await fillQuery(page, 'MATCH (p:Person RETURN p')
    const error = page.getByTestId('editor-error')
    await expect(error).toHaveCount(1)
    // kglite's own diagnostic, not a summary of it.
    await expect(error).toContainText('Cypher syntax error')
    // The underline, not just the list: the caret is the reason this endpoint
    // reports a position at all.
    await expect(page.locator(`${HOST} .cm-lintRange-error`)).toHaveCount(1)

    // ── a valid query shows nothing ─────────────────────────────────────
    await fillQuery(page, 'MATCH (p:Person) RETURN p.title LIMIT 3')
    await expect(page.getByTestId('editor-diagnostics')).toBeEmpty()
    await expect(page.locator(`${HOST} .cm-lintRange-error`)).toHaveCount(0)

    // ── a mistyped label is a warning, not an error ─────────────────────
    // The query is legal Cypher and will run; it just answers nothing.
    await fillQuery(page, 'MATCH (p:Persn) RETURN p')
    await expect(page.getByTestId('editor-warning')).toContainText('Persn')
    await expect(page.getByTestId('editor-error')).toHaveCount(0)

    // ── typing does not spam the endpoint ───────────────────────────────
    const before = requests
    await page.locator(`${HOST} .cm-content`).click()
    await page.keyboard.press('ControlOrMeta+a')
    // Sixty characters over ~1.5 s — long enough that a linter with no idle
    // delay gets to finish a request and start another, repeatedly.
    //
    // The bound is calibrated rather than guessed, because a request bound is
    // exactly the shape that passes vacuously. Measured 2026-08-30 on this
    // burst: **1** request with the 750 ms delay, **42** with the delay set to
    // zero. Three leaves room for scheduling jitter and is nowhere near 42.
    await page.keyboard.type(
      'MATCH (p:Person)-[:KNOWS]->(q:Person) RETURN p.title, q.title',
      { delay: 25 },
    )
    await expect(page.getByTestId('editor-diagnostics')).toBeEmpty()
    const burst = requests - before
    expect(burst, `the burst sent ${burst} validate requests`).toBeLessThanOrEqual(3)
    // And it did send at least one, or the bound above would pass vacuously.
    expect(burst).toBeGreaterThan(0)
  } finally {
    server?.process.kill()
  }
})
