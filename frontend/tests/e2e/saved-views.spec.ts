import { expect, test, type Page } from '@playwright/test'
import { appUrl, launch, openDestination, type Launched } from './harness'

async function ready(page: Page): Promise<void> { await page.waitForFunction(() => window.__kglv?.ready === true) }
async function browse(page: Page, server: Launched): Promise<void> {
  expect((await page.request.post(`${server.info.url}api/browse-type`, {data: {node_type: 'Person', limit: 100}})).ok()).toBe(true)
  await expect(page.getByTestId('count-loaded')).toHaveText('60')
}
async function rows(page: Page): Promise<void> { await openDestination(page, 'data'); await page.getByTestId('data-records').click(); await expect(page.getByTestId('records-table')).toBeVisible() }
function person(page: Page, name: string) { return page.getByTestId('records-table').locator('tr').filter({hasText: new RegExp(`${name}$`)}) }
async function select(page: Page, name: string): Promise<void> { await rows(page); await person(page, name).getByRole('checkbox').check() }
async function open(page: Page): Promise<void> { await page.getByTestId('views-open').click(); await expect(page.getByTestId('view-storage-note')).not.toContainText('Checking') }
async function save(page: Page, name: string): Promise<void> {
  await page.getByTestId('view-name').fill(name); await page.getByTestId('view-save').click()
  await expect(page.getByTestId('views-status')).toContainText(`Saved “${name}”`)
}
function named(page: Page, name: string) { return page.getByTestId('views-catalog').locator('[data-name]').filter({has: page.locator(`span:text-is("${name} · durable")`)}) }
async function clear(page: Page): Promise<void> {
  await openDestination(page, 'explore'); await page.getByTestId('clear-selection').click()
  await expect(page.getByTestId('count-selected')).toHaveText('0')
}

test('saved views reopen exact membership and selection; Clear then new selection saves a clean new baseline', async ({ page }) => {
  let server = await launch()
  const first = `saved-first-${Date.now()}`; const second = `saved-second-${Date.now()}`
  try {
    await page.goto(appUrl(server.info)); await ready(page); await browse(page, server)
    await select(page, 'Person_0'); await open(page)
    await expect(page.getByTestId('view-geometry-note')).toContainText('recomputes geometry')
    await page.getByTestId('view-focus').selectOption('fit'); await save(page, first)
    await expect(page.getByTestId('saved-view-marker')).toHaveAttribute('data-dirty', 'false')
    await page.getByTestId('views-close').click(); await clear(page)
    await expect(page.getByTestId('saved-view-marker')).toHaveAttribute('data-dirty', 'true')
    await select(page, 'Person_1'); await open(page); await save(page, second)
    await expect(page.getByTestId('saved-view-marker')).toHaveAttribute('data-dirty', 'false')
    await named(page, first).getByTestId('view-restore').click()
    await expect(page.getByTestId('views-status')).toContainText(`Restored “${first}”`)
    await page.getByTestId('views-close').click(); await rows(page)
    await expect(person(page, 'Person_0').getByRole('checkbox')).toBeChecked()
    await expect(person(page, 'Person_1').getByRole('checkbox')).not.toBeChecked()
    await expect(page.getByTestId('saved-view-marker')).toHaveAttribute('data-dirty', 'false')
    server.process.kill(); server = await launch()
    await page.goto(appUrl(server.info)); await ready(page); await open(page)
    await named(page, first).getByTestId('view-restore').click()
    await expect(page.getByTestId('count-loaded')).toHaveText('60')
    await expect(page.getByTestId('views-status')).toContainText(`Restored “${first}”`)
    await expect(page.getByTestId('scope-instances')).toHaveAttribute('aria-pressed', 'true')
    await page.getByTestId('views-close').click(); await rows(page)
    await expect(person(page, 'Person_0').getByRole('checkbox')).toBeChecked()
  } finally { server.process.kill() }
})

test('peer restored selection joins local Data selection and shared Clear removes only peer ownership', async ({ page, context }) => {
  const server = await launch(); const peer = await context.newPage(); const name = `peer-${Date.now()}`
  try {
    await page.goto(appUrl(server.info)); await ready(page); await browse(page, server)
    await peer.goto(appUrl(server.info)); await ready(peer)
    await select(page, 'Person_1'); await select(peer, 'Person_0')
    await open(peer); await save(peer, name); await peer.getByTestId('views-close').click(); await clear(peer)
    await expect(person(page, 'Person_1').getByRole('checkbox')).toBeChecked()
    await open(peer); await named(peer, name).getByTestId('view-restore').click()
    await expect(peer.getByTestId('views-status')).toContainText('Restored')
    await expect(page.getByTestId('scope-instances')).toHaveAttribute('aria-pressed', 'true')
    await rows(page)
    await expect(person(page, 'Person_0').getByRole('checkbox')).toBeChecked()
    await expect(person(page, 'Person_1').getByRole('checkbox')).toBeChecked()
    await expect(page.getByTestId('count-selected')).toHaveText('2')
    await openDestination(page, 'explore'); await page.getByTestId('scope-schema').click()
    expect((await peer.request.post(`${server.info.url}api/focus`, {data: {slots: [5]}})).ok()).toBe(true)
    await expect(page.getByTestId('scope-schema')).toHaveAttribute('aria-pressed', 'true')
    await rows(page)
    await peer.getByTestId('views-close').click(); await clear(peer)
    await expect(person(page, 'Person_0').getByRole('checkbox')).not.toBeChecked()
    await expect(person(page, 'Person_1').getByRole('checkbox')).toBeChecked()
    await expect(page.getByTestId('count-selected')).toHaveText('1')
  } finally { await peer.close(); server.process.kill() }
})

test('a delayed own restore preserves newer local selection and marks it unsaved', async ({ page }) => {
  const server = await launch(); const name = `delayed-${Date.now()}`
  let release: (() => void) | undefined; let held = false
  try {
    await page.goto(appUrl(server.info)); await ready(page); await browse(page, server)
    await select(page, 'Person_0'); await open(page); await save(page, name)
    await page.getByTestId('views-close').click(); await clear(page); await open(page)
    await page.route('**/api/views/restore', async route => { held = true; await new Promise<void>(resolve => {release = resolve}); await route.continue() })
    await named(page, name).getByTestId('view-restore').click(); await expect.poll(() => held).toBe(true)
    await page.keyboard.press('Escape'); await select(page, 'Person_2')
    release?.()
    await expect(page.getByTestId('saved-view-marker')).toContainText(name)
    await expect(page.getByTestId('count-selected')).toHaveText('2')
    await rows(page)
    await expect(person(page, 'Person_2').getByRole('checkbox')).toBeChecked()
    await expect(person(page, 'Person_0').getByRole('checkbox')).toBeChecked()
    await expect(page.getByTestId('saved-view-marker')).toHaveAttribute('data-dirty', 'true')
  } finally { release?.(); server.process.kill() }
})

test('stale saves are refused visibly and shared history restores a checkpoint as a new action', async ({ page }) => {
  const server = await launch(); const name = `conflict-${Date.now()}`
  let release: (() => void) | undefined; let held = false
  try {
    await page.goto(appUrl(server.info)); await ready(page); await browse(page, server); await open(page)
    await page.route('**/api/views/save', async route => { held = true; await new Promise<void>(resolve => {release = resolve}); await route.continue() })
    await page.getByTestId('view-name').fill(name); await page.getByTestId('view-save').click(); await expect.poll(() => held).toBe(true)
    expect((await page.request.post(`${server.info.url}api/focus`, {data: {slots: [5]}})).ok()).toBe(true)
    release?.()
    await expect(page.getByTestId('views-status')).toContainText('Shared view changed')
    await expect(page.getByTestId('views-catalog')).not.toContainText(name)
    await expect(page.getByTestId('count-loaded')).toHaveText('60')
    const history = page.getByTestId('views-history').locator('li').filter({hasText: 'browse'})
    await expect(history).toHaveCount(1)
    await history.getByTestId('history-restore').click()
    await expect(page.getByTestId('views-status')).toContainText('Restored the checkpoint as a new shared change')
    await expect(page.getByTestId('count-loaded')).toHaveText('0')
    await expect(page.getByTestId('scope-schema')).toHaveAttribute('aria-pressed', 'true')
    await page.setViewportSize({width: 640, height: 720})
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    await page.keyboard.press('Escape'); await expect(page.getByTestId('views-open')).toBeFocused()
  } finally { release?.(); server.process.kill() }
})

test('ambiguous source keys save explicitly session-only and cannot be reopened after server restart', async ({ page }) => {
  const fixture = 'crates/kglite-visual-core/tests/fixtures/viewer-identity.kgl'
  let server = await launch(fixture); const name = `session-${Date.now()}`
  try {
    await page.goto(appUrl(server.info)); await ready(page)
    expect((await page.request.post(`${server.info.url}api/browse-type`, {data: {node_type: 'Person', limit: 100}})).ok()).toBe(true)
    await expect(page.getByTestId('count-loaded')).toHaveText('3'); await open(page); await save(page, name)
    await expect(page.getByTestId('views-status')).toContainText('session only; lost when this server closes')
    await expect(page.getByTestId('saved-view-marker')).toHaveAttribute('data-storage', 'session')
    const row = page.getByTestId('views-catalog').locator('[data-storage=session]').filter({hasText: name})
    await expect(row).toHaveCount(1)
    await row.getByTestId('view-restore').click(); await expect(page.getByTestId('views-status')).toContainText('Restored')
    server.process.kill(); server = await launch(fixture)
    await page.goto(appUrl(server.info)); await ready(page); await open(page)
    await expect(page.getByTestId('views-catalog')).not.toContainText(name)
  } finally { server.process.kill() }
})

test('session capacity and replacement refusals remain visible and explicit deletion recovers capacity', async ({ page }) => {
  const server = await launch('crates/kglite-visual-core/tests/fixtures/viewer-identity.kgl')
  const prefix = `capacity-${Date.now()}`
  try {
    await page.goto(appUrl(server.info)); await ready(page)
    expect((await page.request.post(`${server.info.url}api/browse-type`, {data: {node_type: 'Person', limit: 100}})).ok()).toBe(true)
    await expect(page.getByTestId('count-loaded')).toHaveText('3')
    let revision = ''
    for (let index = 0; index < 20; index += 1) {
      const response = await page.request.post(`${server.info.url}api/views/save`, {data: {name: `${prefix}-${index}`}})
      expect(response.ok(), await response.text()).toBe(true)
      revision = ((await response.json()) as {stamp: {revision: string}}).stamp.revision
    }
    await expect(page.locator('.kglv-root')).toHaveAttribute('data-shared-revision', revision)
    await open(page)
    await page.getByTestId('view-name').fill(`${prefix}-overflow`); await page.getByTestId('view-save').click()
    await expect(page.getByTestId('views-status')).toContainText('Request refused')
    await expect(page.getByTestId('views-catalog').locator('[data-storage=session]')).toHaveCount(20)
    await expect(page.getByTestId('count-loaded')).toHaveText('3')
    const row = page.getByTestId('views-catalog').locator('[data-name]').filter({has: page.locator(`span:text-is("${prefix}-0 · session only")`)})
    await row.getByTestId('view-delete').click(); await row.getByTestId('view-delete-confirm').click()
    await expect(page.getByTestId('views-status')).toContainText('Deleted')
    await expect(page.getByTestId('views-catalog').locator('[data-storage=session]')).toHaveCount(19)
    await save(page, `${prefix}-overflow`)
    await page.getByTestId('view-save').click()
    await expect(page.getByTestId('views-status')).toContainText('Request refused')
    await page.getByTestId('view-replace').check(); await page.getByTestId('view-save').click()
    await expect(page.getByTestId('views-status')).toContainText(`Saved “${prefix}-overflow”`)
    await expect(page.getByTestId('views-catalog').locator('[data-storage=session]')).toHaveCount(20)
  } finally { server.process.kill() }
})

test('a delayed save acknowledgement preserves newer local selection and reports the saved copy accurately', async ({ page }) => {
  const server = await launch(); const name = `save-delayed-${Date.now()}`
  let release: (() => void) | undefined; let held = false
  try {
    await page.goto(appUrl(server.info)); await ready(page); await browse(page, server)
    await select(page, 'Person_0'); await open(page)
    await page.route('**/api/views/save', async route => {
      const response = await route.fetch(); held = true
      await new Promise<void>(resolve => {release = resolve}); await route.fulfill({response})
    })
    await page.getByTestId('view-name').fill(name); await page.getByTestId('view-save').click()
    await expect.poll(() => held).toBe(true)
    await page.keyboard.press('Escape'); await select(page, 'Person_2')
    await page.route('**/api/views', route => route.fulfill({status: 503, contentType: 'application/json', body: JSON.stringify({error: 'catalog temporarily unavailable'})}))
    release?.()
    await expect(page.getByTestId('views-status')).toContainText('Its saved copy is intact')
    await expect(page.getByTestId('views-status')).not.toContainText('[object Object]')
    await expect(page.getByTestId('views-catalog-status')).toContainText('Catalog refresh failed')
    await expect(page.getByTestId('saved-view-marker')).toHaveAttribute('data-dirty', 'true')
    await expect(person(page, 'Person_2').getByRole('checkbox')).toBeChecked()
  } finally { release?.(); server.process.kill() }
})

test('Save waits for storage eligibility and limits before it can be requested', async ({ page }) => {
  const server = await launch(); let release: (() => void) | undefined; let held = false
  await page.route('**/api/views', async route => { held = true; await new Promise<void>(resolve => {release = resolve}); await route.continue() })
  try {
    await page.goto(appUrl(server.info)); await ready(page)
    await page.getByTestId('views-open').click(); await expect.poll(() => held).toBe(true)
    await expect(page.getByTestId('view-save')).toBeDisabled()
    await expect(page.getByTestId('view-storage-note')).toContainText('Checking save storage')
    release?.()
    await expect(page.getByTestId('view-save')).toBeEnabled()
    await expect(page.getByTestId('view-storage-note')).toContainText('Limit:')
  } finally { release?.(); server.process.kill() }
})

test('a delayed restore HTTP reply cannot turn a newer peer selection into local ownership', async ({ page }) => {
  const server = await launch(); const name = `restore-reply-${Date.now()}`
  let release: (() => void) | undefined; let held = false
  try {
    await page.goto(appUrl(server.info)); await ready(page); await browse(page, server)
    await select(page, 'Person_0'); await open(page); await save(page, name)
    await page.getByTestId('views-close').click(); await clear(page); await open(page)
    await page.route('**/api/views/restore', async route => {
      const response = await route.fetch(); held = true
      await new Promise<void>(resolve => {release = resolve}); await route.fulfill({response})
    })
    await named(page, name).getByTestId('view-restore').click(); await expect.poll(() => held).toBe(true)
    await expect(page.getByTestId('count-selected')).toHaveText('1')
    expect((await page.request.post(`${server.info.url}api/highlight`, {data: {slots: [7], concept: 'selected'}})).ok()).toBe(true)
    await expect(page.getByTestId('count-selected')).toHaveText('2')
    release?.(); await expect(page.getByTestId('views-status')).toContainText(`Restored “${name}”`)
    expect((await page.request.post(`${server.info.url}api/highlight`, {data: {slots: [], concept: 'selected'}})).ok()).toBe(true)
    await expect(page.getByTestId('count-selected')).toHaveText('1')
    await page.getByTestId('views-close').click(); await rows(page)
    await expect(person(page, 'Person_0').getByRole('checkbox')).toBeChecked()
    await expect(person(page, 'Person_2').getByRole('checkbox')).not.toBeChecked()
  } finally { release?.(); server.process.kill() }
})

test('returning to the captured selection before a delayed Save reply remains clean', async ({ page }) => {
  const server = await launch(); const name = `same-selection-${Date.now()}`
  let release: (() => void) | undefined; let held = false
  try {
    await page.goto(appUrl(server.info)); await ready(page); await browse(page, server)
    await select(page, 'Person_0'); await open(page)
    await page.route('**/api/views/save', async route => {
      const response = await route.fetch(); held = true
      await new Promise<void>(resolve => {release = resolve}); await route.fulfill({response})
    })
    await page.getByTestId('view-name').fill(name); await page.getByTestId('view-save').click(); await expect.poll(() => held).toBe(true)
    await page.keyboard.press('Escape'); await select(page, 'Person_2')
    await person(page, 'Person_2').getByRole('checkbox').uncheck()
    await expect(page.getByTestId('count-selected')).toHaveText('1')
    release?.()
    await expect(page.getByTestId('views-status')).toHaveText(`Saved “${name}” (durable).`)
    await expect(page.getByTestId('saved-view-marker')).toHaveAttribute('data-dirty', 'false')
  } finally { release?.(); server.process.kill() }
})
