import {expect, test, type Page} from '@playwright/test'
import {readFile} from 'node:fs/promises'
import {resolve} from 'node:path'
import {appUrl, launch, openDestination, Listener, type Launched} from './harness'
import {openDrawer, closeDrawer} from './navigation'
import {fieldKey, fieldTestId} from '../../src/fields'
import type {QueryTable} from '../../src/generated/QueryTable'
import type {FieldRef} from '../../src/generated/FieldRef'
import type {SharedSnapshotMeta} from '../../src/generated/SharedSnapshotMeta'

const fixture = 'crates/kglite-visual-core/tests/fixtures/viewer-identity.kgl'
async function ready(page: Page): Promise<void> { await page.waitForFunction(() => window.__kglv?.ready === true) }
async function post(page: Page, server: Launched, route: string, data: unknown) {
  const response = await page.request.post(`${server.info.url}api/${route}`, {data}); expect(response.ok(), await response.text()).toBe(true); return response.json()
}
async function state(_page: Page, server: Launched): Promise<SharedSnapshotMeta> {
  const listener = new Listener(server.info.url.replace(/^http/, 'ws') + 'ws')
  try { await listener.open(); const event = listener.received.find(item => item.kind === 'shared-update'); if (event?.kind !== 'shared-update') throw new Error('No shared snapshot'); return event.value.meta.snapshot } finally {listener.close()}
}
async function load(page: Page, server: Launched): Promise<void> {
  const nodes = (await post(page, server, 'cypher', {query: 'MATCH (n) RETURN n', as_graph: false}) as QueryTable).row_references.flatMap(row => row.nodes)
  const relationships = (await post(page, server, 'cypher', {query: 'MATCH (a)-[r]->(b) RETURN r', as_graph: false}) as QueryTable).row_references.flatMap(row => row.relationships).sort((a,b) => a.edge_id-b.edge_id).slice(0,3)
  await post(page, server, 'load-entities', {nodes, relationships}); await expect(page.getByTestId('count-loaded')).toHaveText('4')
}
async function calculate(page: Page, server: Launched, kind = 'degree'): Promise<FieldRef[]> {
  await openDestination(page, 'data'); await page.getByTestId('calculation-kind').selectOption(kind); await page.getByTestId('calculation-run').click()
  await expect(page.getByTestId('calculation-status')).toContainText('ready')
  await expect(page.getByTestId('records-table')).toBeVisible()
  const calculation = (await state(page, server)).calculations.find(item => item.kind === kind)!
  expect(calculation).toBeDefined(); return calculation.fields.map(item => item.field)
}
function row(page: Page, title: string) { return page.getByTestId('records-table').locator('tr').filter({has: page.getByText(title, {exact: true})}) }
async function addField(page: Page, name: string): Promise<void> { await page.getByTestId('records-field').fill(name); await page.getByTestId('records-add-field').click(); await expect(page.getByTestId(`records-sort-${name}`)).toBeVisible() }
async function values(page: Page, title: string): Promise<string[]> { return row(page, title).locator('td').allTextContents() }

test('calculated records keep canonical source and derived columns, stable sort and scoped typed CSV', async ({page}) => {
  const server = await launch(fixture)
  try {
    await page.goto(appUrl(server.info)); await ready(page); await load(page, server)
    const fields = await calculate(page, server); const total = fields.find(field => field.kind === 'derived' && field.column === 'total')!
    await expect(page.getByTestId('calculation-scope')).toContainText('4 instances · 3 relation records')
    expect((await values(page, 'Ada')).slice(4,7)).toEqual(['0','2','2'])
    expect((await values(page, 'Duplicate Ada key')).slice(4,7)).toEqual(['3','1','4'])
    expect((await values(page, 'Outside the relationships')).slice(4,7)).toEqual(['0','0','0'])
    await page.getByTestId(`records-sort-${fieldTestId(total)}`).click(); await page.getByTestId(`records-sort-${fieldTestId(total)}`).click()
    await addField(page, '@derived:degree:total')
    await expect(page.getByTestId(`records-sort-${fieldTestId(total)}`).locator('..')).toHaveAttribute('aria-sort', 'descending')
    await expect(page.getByTestId('records-table').locator('tr').nth(1)).toContainText('Duplicate Ada key')
    expect((await values(page, 'Duplicate Ada key')).slice(4,7)).toEqual(['3','1','4'])
    const source = await post(page, server, 'records', {handles: [(await state(page, server)).slice.nodes.find(node => node.title === 'Duplicate Ada key')!.handle], fields: ['@derived:degree:total']})
    const sourceCell = source.rows[0].cells[0]
    await expect(row(page, 'Duplicate Ada key').locator('td').nth(7)).toHaveText(sourceCell.state === 'value' ? String(sourceCell.value.value) : sourceCell.state)
    const csv = page.waitForEvent('download'); await page.getByTestId('records-csv').click()
    const text = await readFile((await (await csv).path())!, 'utf8')
    expect(text).toContain('frozen input revision'); expect(text).toContain('@derived:degree:total'); expect(text).toContain('int64')
    await row(page, 'Ada').getByRole('checkbox').check(); await expect(page.getByTestId('records-selection')).toContainText('1 selected')
    await row(page, 'Ada').getByTestId('record-show-graph').click(); await expect(page.getByTestId('count-selected')).toHaveText('1')
  } finally {server.process.kill()}
})

test('derived filters use frozen values until explicit recompute and refresh cached rows outside the new input', async ({page}) => {
  const server = await launch(fixture)
  try {
    await page.goto(appUrl(server.info)); await ready(page); await load(page, server)
    const fields = await calculate(page, server); const total = fields.find(field => field.kind === 'derived' && field.column === 'total')!
    await page.getByTestId('records-scope').selectOption('loaded')
    await openDrawer(page, 'filters'); await page.getByTestId('subset-kind').selectOption('numeric-range')
    await page.getByTestId('subset-field-kind').selectOption('calculated field'); await page.getByTestId('subset-derived-field').selectOption(fieldKey(total))
    await page.getByTestId('subset-min').fill('3'); await page.getByTestId('subset-apply').click()
    await expect(page.getByTestId('count-visible')).toHaveText('1'); await expect(page.getByTestId('subset-active')).toContainText('Degree')
    await closeDrawer(page); await expect(page.getByTestId('calculation-scope')).toContainText('earlier visible subset')
    expect((await values(page, 'Duplicate Ada key')).slice(4,7)).toEqual(['3','1','4'])
    await page.getByTestId('calculation-recompute').click(); await expect(page.getByTestId('calculation-status')).toContainText('1 instances · 1 relation records')
    await expect(page.getByTestId('count-visible')).toHaveText('0')
    await expect.poll(async () => (await values(page, 'Duplicate Ada key')).slice(4,7)).toEqual(['1','1','2'])
    await expect(row(page, 'Ada')).toContainText('unavailable')
    await expect(page.getByTestId('calculation-scope')).toContainText('1 instances · 1 relation records')
    await openDrawer(page, 'filters'); await page.getByTestId('subset-clear').click(); await expect(page.getByTestId('count-visible')).toHaveText('4')
    await closeDrawer(page); await expect(row(page, 'Ada')).toContainText('unavailable')
    await page.getByTestId('calculation-recompute').click(); await expect(page.getByTestId('calculation-status')).toContainText('4 instances · 3 relation records')
    await expect.poll(async () => (await values(page, 'Ada')).slice(4,7)).toEqual(['0','2','2'])
  } finally {server.process.kill()}
})

test('peer calculated appearance and saved frozen fields restore together through narrow keyboard controls', async ({page, context}) => {
  const server = await launch(fixture); const peer = await context.newPage(); const name = `calculated-${Date.now()}`
  try {
    await page.goto(appUrl(server.info)); await ready(page); await load(page, server)
    const fields = await calculate(page, server, 'weak-components'); const size = fields.find(field => field.kind === 'derived' && field.column === 'component_size')!
    const original = await state(page, server)
    await peer.goto(appUrl(server.info)); await ready(peer); await openDestination(peer, 'data')
    await expect(peer.getByTestId('calculation-scope')).toContainText('4 instances · 3 relation records')
    await peer.getByTestId('calculation-inspect').click(); await expect(peer.getByTestId(`records-sort-${fieldTestId(size)}`)).toBeVisible()
    await openDrawer(peer, 'appearance'); await peer.getByTestId('size-by').selectOption(fieldKey(size)); await expect.poll(async () => (await state(page, server)).appearance.size_field).toEqual(size); await peer.getByTestId('color-by').selectOption(fieldKey(fields[0]!))
    await openDestination(page, 'explore'); await expect(page.getByTestId('legend-body')).toContainText('Frozen visible input revision')
    const styled = await state(page, server); expect(styled.appearance.size_field).toEqual(size)
    expect(styled.appearance_mapping.nodes.every(node => node.radius !== null)).toBe(true)
    await page.setViewportSize({width: 640, height: 720}); await page.getByTestId('views-open').click()
    await expect(page.getByTestId('view-storage-note')).not.toContainText('Checking')
    await page.getByTestId('view-name').fill(name); await page.getByTestId('view-save').click(); await expect(page.getByTestId('views-status')).toContainText(`Saved “${name}”`)
    const catalog = page.getByTestId('views-catalog').locator('[data-name]').filter({hasText: name})
    await page.keyboard.press('Escape'); await expect(page.getByTestId('views-open')).toBeFocused()
    await post(page, server, 'reset', {}); await expect(page.getByTestId('count-loaded')).toHaveText('0')
    await page.getByTestId('views-open').click(); await catalog.getByTestId('view-restore').click(); await expect(page.getByTestId('views-status')).toContainText(`Restored “${name}”`)
    await page.keyboard.press('Escape'); await openDestination(page, 'data')
    await expect(page.getByTestId('calculation-scope')).toContainText('4 instances · 3 relation records')
    await page.getByTestId('calculation-inspect').focus(); await page.keyboard.press('Enter')
    await expect(page.getByTestId(`records-sort-${fieldTestId(size)}`)).toBeVisible()
    const restored = await state(page, server); expect(restored.calculations[0]?.input_stamp).toEqual(original.calculations[0]?.input_stamp)
    expect(restored.appearance.size_field).toEqual(size)
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    await page.screenshot({path: resolve('../dev-docs/bench/out/green-room-calculations.png'), fullPage: true})
  } finally {await peer.close();server.process.kill()}
})

test('a delayed refused calculation stays pending through a peer action and never claims a result', async ({page}) => {
  const server = await launch(fixture); let release: (() => void) | undefined
  await page.routeWebSocket('**/ws', route => {const upstream = route.connectToServer(); route.onMessage(message => {
    const request = typeof message === 'string' ? JSON.parse(message) as {type: string} : null
    if (request?.type === 'calculate') release = () => upstream.send(message); else upstream.send(message)
  })})
  try {
    await page.goto(appUrl(server.info)); await ready(page); await load(page, server); await openDestination(page, 'data'); await page.getByTestId('calculation-run').click()
    await expect.poll(() => release !== undefined).toBe(true)
    await post(page, server, 'focus', {slots: []})
    await expect(page.getByTestId('calculation-status')).toContainText('Calculating'); await expect(page.getByTestId('calculation-run')).toBeDisabled()
    release!(); await expect(page.getByTestId('calculation-status')).toContainText('revision'); await expect(page.getByTestId('calculation-run')).toBeEnabled()
    await expect(page.locator('[data-calculation-id]')).toHaveCount(0)
  } finally {server.process.kill()}
})


test('Inspect fields refuses all new columns at the 32-field cap and preserves the previous 31', async ({page}) => {
  const server = await launch(fixture)
  try {
    await page.goto(appUrl(server.info)); await ready(page); await load(page, server); await openDestination(page, 'data')
    for (let index = 0; index < 29; index += 1) await addField(page, `literal-${index}`)
    await expect(page.getByTestId('records-status')).toContainText('31 source fields · 0 frozen calculated fields')
    await page.getByTestId('calculation-run').click(); await expect(page.getByTestId('calculation-status')).toContainText('ready')
    await expect(page.getByTestId('records-field-status')).toContainText('No calculated fields were added')
    await expect(page.getByTestId('records-table').locator('th')).toHaveCount(33)
    await expect(page.getByTestId('records-sort-literal-28')).toBeVisible()
    await expect(page.getByTestId('records-table').locator('button[data-testid^="records-sort-derived-"]')).toHaveCount(0)
    await page.getByTestId('records-remove-literal-28').click(); await page.getByTestId('records-remove-literal-27').click()
    await page.getByTestId('calculation-inspect').click()
    await expect(page.getByTestId('records-status')).toContainText('29 source fields · 3 frozen calculated fields')
    await expect(page.getByTestId('records-field-status')).toHaveText('')
  } finally {server.process.kill()}
})
