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
