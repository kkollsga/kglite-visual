import { expect, test, type Page } from '@playwright/test'
import { appUrl, fillQuery, launch, openDestination } from './harness'
import { openDrawer } from './navigation'

const IDENTITY = 'crates/kglite-visual-core/tests/fixtures/viewer-identity.kgl'
async function ready(page: Page): Promise<void> { await page.waitForFunction(() => window.__kglv?.ready === true) }
async function addField(page: Page, field: string): Promise<void> {
  await page.getByTestId('records-field').fill(field)
  await page.getByTestId('records-add-field').click()
  await expect(page.getByTestId(`records-sort-${field}`)).toBeVisible()
}
async function run(page: Page, query: string): Promise<void> {
  await fillQuery(page, query); await page.getByTestId('query-as-graph').uncheck(); await page.getByTestId('query-run').click()
  await expect(page.getByTestId('query-table')).toBeVisible()
}

test('Records retain exact duplicate and null keys, typed values and selection after sorting/filtering', async ({ page }) => {
  const server = await launch(IDENTITY)
  try {
    await page.goto(appUrl(server.info)); await ready(page)
    expect((await page.request.post(`${server.info.url}api/browse-type`, {data: {node_type: 'Person', limit: 100}})).ok()).toBe(true)
    await expect(page.getByTestId('count-loaded')).toHaveText('3')
    await openDestination(page, 'data')
    await expect(page.getByTestId('records-table').locator('tr')).toHaveCount(4)
    await expect(page.getByTestId('records-table')).toContainText('9007199254740993')
    await expect(page.getByTestId('records-table').locator('[data-cell-state=null]')).toHaveCount(1)
    await addField(page, 'score'); await addField(page, 'active'); await addField(page, 'category')
    const row = page.getByTestId('records-table').locator('tr').filter({hasText: 'Null key'})
    const handle = await row.getAttribute('data-handle')
    await row.getByRole('checkbox').check()
    await page.getByTestId('records-sort-score').click()
    await expect(page.getByTestId('records-table').locator('tr').filter({hasText: 'Null key'}).getByRole('checkbox')).toBeChecked()
    expect(await page.getByTestId('records-table').locator('tr').filter({hasText: 'Null key'}).getAttribute('data-handle')).toBe(handle)
    await expect(page.getByTestId('records-table')).toContainText('false')
    await expect(row).toContainText('missing')
    await expect(row).toContainText('""')
    await page.getByTestId('records-scope').selectOption('loaded')
    await expect(page.getByTestId('records-table').locator('tr')).toHaveCount(4)
    await openDrawer(page, 'filters')
    await page.getByTestId('subset-kind').selectOption('numeric-range')
    await page.getByTestId('subset-field').fill('score'); await page.getByTestId('subset-max').fill('0')
    await page.getByTestId('subset-apply').click()
    await expect(page.getByTestId('count-visible')).toHaveText('1')
    await page.keyboard.press('Escape')
    await expect(page.getByTestId('records-selection')).toContainText('1 hidden or unloaded')
    await expect(row.getByRole('checkbox')).toBeChecked()
    await row.getByTestId('record-show-graph').click()
    await expect(page.getByTestId('selection-title')).toContainText('Null key')
    await expect(page.getByTestId('count-visible')).toHaveText('1')
    await expect(page.getByTestId('count-selected')).toHaveText('1')
    await openDrawer(page, 'filters'); await page.getByTestId('subset-clear').click()
    await expect(page.getByTestId('count-visible')).toHaveText('3')
  } finally { server.process.kill() }
})

test('Records pages remain local across peer focus/style and never prefetch while Explore is shown', async ({ page }) => {
  const server = await launch()
  const reads: string[] = []
  page.on('request', request => {
    if (request.url().endsWith('/api/records') && String((request.postDataJSON() as {request_id?: string}).request_id ?? '').startsWith('records-')) reads.push(request.postData() ?? '')
  })
  try {
    await page.goto(appUrl(server.info)); await ready(page)
    expect((await page.request.post(`${server.info.url}api/cypher`, {data: {query: 'MATCH (n) RETURN n', params: {}, limit: 500, as_graph: true}})).ok()).toBe(true)
    await expect(page.getByTestId('count-loaded')).toHaveText('118')
    expect(reads).toHaveLength(0)
    await openDestination(page, 'data')
    await expect(page.getByTestId('records-page')).toHaveText('1–100 of 118')
    await page.getByTestId('records-sort-id').click(); await page.getByTestId('records-next').click()
    await expect(page.getByTestId('records-page')).toHaveText('101–118 of 118')
    const before = await page.getByTestId('records-table').innerText(); const count = reads.length
    await page.getByTestId('records-previous').focus()
    expect((await page.request.post(`${server.info.url}api/focus`, {data: {slots: [5]}})).ok()).toBe(true)
    expect((await page.request.post(`${server.info.url}api/appearance`, {data: {color_by: null, size_by: null}})).ok()).toBe(true)
    await expect(page.locator('.kglv-root')).toHaveAttribute('data-shared-revision', '3')
    await expect(page.getByTestId('records-page')).toHaveText('101–118 of 118')
    expect(await page.getByTestId('records-table').innerText()).toBe(before)
    expect(reads).toHaveLength(count)
    await expect(page.getByTestId('records-previous')).toBeFocused()
    await page.getByTestId('records-page-size').selectOption('500')
    await expect(page.getByTestId('records-table').locator('tr')).toHaveCount(119)
  } finally { server.process.kill() }
})

test('query row provenance loads genuine parallel-edge identity and scalar results have no graph actions', async ({ page }) => {
  const server = await launch(IDENTITY)
  try {
    await page.goto(appUrl(server.info)); await ready(page)
    await run(page, 'RETURN 0 AS id, \'Person\' AS node_type')
    await expect(page.getByTestId('query-show-graph')).toHaveCount(0)
    await expect(page.getByTestId('count-loaded')).toHaveText('0')
    await run(page, 'MATCH (a)-[r:KNOWS]->(b) RETURN r LIMIT 1')
    await expect(page.getByTestId('query-show-graph')).toHaveCount(1)
    await page.getByTestId('query-show-graph').click()
    await expect(page.getByTestId('count-loaded')).toHaveText('2')
    await expect.poll(() => page.evaluate(() => window.__kglv.linkCount)).toBe(1)
    await run(page, "UNWIND [2,9007199254740993,'9007199254740993'] AS v RETURN v")
    await page.getByTestId('sort-v').click()
    expect(await page.getByTestId('query-table').locator('tr:not(:first-child) td:first-child').allTextContents()).toEqual(['2', '9007199254740993', '"9007199254740993"'])
    await expect(page.getByTestId('query-show-graph')).toHaveCount(0)
    await expect(page.getByTestId('count-loaded')).toHaveText('2')
  } finally { server.process.kill() }
})

test('source search loads duplicate unsafe keys and a null key by handles', async ({ page }) => {
  const server = await launch(IDENTITY)
  const requests: string[] = []
  page.on('websocket', socket => socket.on('framesent', event => requests.push(String(event.payload))))
  try {
    await page.goto(appUrl(server.info)); await ready(page)
    await page.getByTestId('browse-type-picker').selectOption({label: 'Person'})
    await page.getByTestId('search-input').fill('Ada'); await page.getByTestId('search-run').click()
    await expect(page.getByTestId('search-status')).toContainText('2 hits')
    await page.getByTestId('search-load').click(); await expect(page.getByTestId('count-loaded')).toHaveText('2')
    await page.getByTestId('search-input').fill('Null key'); await page.getByTestId('search-run').click()
    await expect(page.getByTestId('search-status')).toContainText('1 hit')
    await page.getByTestId('search-load').click(); await expect(page.getByTestId('count-loaded')).toHaveText('3')
    expect(requests.some(request => request.includes('"type":"load-nodes"'))).toBe(true)
    expect(requests.some(request => request.includes('id(n)'))).toBe(false)
  } finally { server.process.kill() }
})

test('long source values have bounded next pages and narrow keyboard focus recovery', async ({ page }) => {
  const server = await launch('crates/kglite-visual-core/tests/fixtures/spill.kgl')
  await page.setViewportSize({width: 640, height: 720})
  try {
    await page.goto(appUrl(server.info)); await ready(page)
    expect((await page.request.post(`${server.info.url}api/browse-type`, {data: {node_type: 'Blob', limit: 1}})).ok()).toBe(true)
    await expect(page.getByTestId('count-loaded')).toHaveText('1')
    await openDestination(page, 'data'); await addField(page, 'payload')
    const inspect = page.getByTestId('records-table').getByRole('button', {name: 'Inspect value'})
    await inspect.click()
    await expect(page.getByTestId('field-detail-status')).toContainText('partial page')
    await expect(page.getByTestId('field-detail-copy')).toHaveText('Copy current page')
    const before = await page.getByTestId('field-detail-status').textContent()
    await page.getByTestId('field-detail-next').click()
    await expect(page.getByTestId('field-detail-status')).not.toHaveText(before ?? '')
    await expect(page.getByTestId('field-detail-previous')).toBeEnabled()
    await page.keyboard.press('Escape'); await expect(inspect).toBeFocused()
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  } finally { server.process.kill() }
})

test('delayed Records reads cannot overwrite a newer field list, sort or selected handle', async ({ page }) => {
  const server = await launch(IDENTITY)
  let held = false; let release: (() => void) | undefined
  await page.route('**/api/records', async route => {
    const fields = (route.request().postDataJSON() as {fields: string[]}).fields
    if (!held && fields.includes('category') && !fields.includes('score')) {
      held = true; await new Promise<void>(resolve => { release = resolve })
      try { await route.continue() } catch { /* The newer request may have aborted this fetch. */ }
    } else await route.continue()
  })
  try {
    await page.goto(appUrl(server.info)); await ready(page)
    expect((await page.request.post(`${server.info.url}api/browse-type`, {data: {node_type: 'Person', limit: 100}})).ok()).toBe(true)
    await expect(page.getByTestId('count-loaded')).toHaveText('3'); await openDestination(page, 'data')
    await expect(page.getByTestId('records-table').locator('tr')).toHaveCount(4)
    const selected = page.getByTestId('records-table').locator('tr').filter({hasText: 'Null key'})
    await selected.getByRole('checkbox').check(); const handle = await selected.getAttribute('data-handle')
    await page.getByTestId('records-field').fill('category'); await page.getByTestId('records-add-field').click()
    await expect.poll(() => held).toBe(true)
    await addField(page, 'score')
    await page.getByTestId('records-sort-score').click(); await page.getByTestId('records-sort-score').click()
    release?.()
    await expect(selected.getByRole('checkbox')).toBeChecked()
    expect(await selected.getAttribute('data-handle')).toBe(handle)
    expect(await page.getByTestId('records-table').locator('tr:not(:first-child) td:last-child').allTextContents()).toEqual(['10', '5', '0'])
    await run(page, 'RETURN 7 AS source_answer')
    await page.getByTestId('data-records').click()
    await expect(selected.getByRole('checkbox')).toBeChecked()
    await page.getByTestId('data-query').click()
    await expect(page.getByTestId('query-table')).toContainText('source_answer')
    await expect(page.getByTestId('query-table')).toContainText('7')
  } finally { release?.(); server.process.kill() }
})

test('removed instance selection remains a source handle through compaction and Show rows can reload it', async ({ page }) => {
  const server = await launch()
  try {
    await page.goto(appUrl(server.info)); await ready(page)
    expect((await page.request.post(`${server.info.url}api/browse-type`, {data: {node_type: 'Person', limit: 100}})).ok()).toBe(true)
    await expect(page.getByTestId('count-loaded')).toHaveText('60')
    await openDestination(page, 'data')
    const record = page.getByTestId('records-table').locator('tr').filter({hasText: /^.*Person_0$/})
    await expect(record).toHaveCount(1)
    const handle = await record.getAttribute('data-handle')
    await record.getByRole('checkbox').check()
    await record.getByTestId('record-show-graph').click()
    await expect(page.getByTestId('selection-title')).toHaveText('Person_0 — Person')
    expect((await page.request.post(`${server.info.url}api/collapse`, {data: {slot: 0}})).ok()).toBe(true)
    await expect.poll(() => page.evaluate(() => window.__kglv.compactions)).toBe(1)
    await expect(page.getByTestId('count-loaded')).toHaveText('0')
    await expect.poll(() => page.evaluate(() => window.__kglv.selectedCount)).toBe(0)
    await expect(page.getByTestId('selection-hidden')).toContainText('1 selected hidden or unloaded')
    await page.getByTestId('show-selection-rows').click()
    await expect(page.getByTestId('records-scope')).toHaveValue('selected')
    await expect(page.getByTestId('records-table').locator('tr')).toHaveCount(2)
    const saved = page.getByTestId('records-table').locator('tr').filter({hasText: 'Person_0'})
    expect(await saved.getAttribute('data-handle')).toBe(handle)
    await expect(saved.getByRole('checkbox')).toBeChecked()
    await expect(saved.getByTestId('record-show-graph')).toContainText('Unloaded')
    await saved.getByTestId('record-show-graph').click()
    await expect(page.getByTestId('count-loaded')).toHaveText('1')
    await expect(page.getByTestId('selection-title')).toHaveText('Person_0 — Person')
    await page.getByTestId('clear-selection').click()
    await expect(page.getByTestId('count-selected')).toHaveText('0')
  } finally { server.process.kill() }
})

test('this browser’s trail records bounded expansion acknowledgements and excludes refused or remote actions', async ({ page }) => {
  const server = await launch()
  try {
    await page.goto(appUrl(server.info)); await ready(page)
    await page.locator('.kglv-label:has-text("Person")').click()
    await expect(page.getByTestId('selection-title')).toHaveText('Person (type)')
    await page.getByTestId('expand-limit').fill('40')
    await page.getByTestId('expand-KNOWS-out').click()
    await expect(page.getByTestId('count-loaded')).toHaveText('40')
    const trail = page.getByTestId('exploration-trail')
    await trail.locator('summary').click()
    await expect(trail.locator('li')).toHaveCount(1)
    await expect(trail).toContainText('Expand Person · out · KNOWS · 40 admitted · 0 removed · at least 20 nodes omitted')
    await fillQuery(page, 'THIS IS INVALID CYPHER')
    await page.getByTestId('query-as-graph').check(); await page.getByTestId('query-run').click()
    await expect(page.getByTestId('query-status')).toContainText('query failed: Cypher syntax error')
    await openDestination(page, 'explore')
    await expect(trail.locator('li')).toHaveCount(1)
    expect((await page.request.post(`${server.info.url}api/browse-type`, {data: {node_type: 'City', limit: 1}})).ok()).toBe(true)
    await expect(page.getByTestId('count-loaded')).toHaveText('41')
    await expect(trail.locator('li')).toHaveCount(1)
  } finally { server.process.kill() }
})
