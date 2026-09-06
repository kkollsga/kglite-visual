import { expect, test, type Page } from '@playwright/test'
import { readFile } from 'node:fs/promises'
import { appUrl, launch, Listener, openDestination, fillQuery, type Launched } from './harness'
import { openDrawer } from './navigation'
import type { SharedSnapshotMeta } from '../../src/generated/SharedSnapshotMeta'
import type { QueryTable } from '../../src/generated/QueryTable'

async function ready(page: Page): Promise<void> { await page.waitForFunction(() => window.__kglv?.ready === true) }
async function browse(page: Page, server: Launched): Promise<void> {
  expect((await page.request.post(`${server.info.url}api/browse-type`, {data: {node_type: 'Person', limit: 100}})).ok()).toBe(true)
  await expect(page.getByTestId('count-loaded')).toHaveText('60')
}
async function openExport(page: Page): Promise<void> {
  if (await page.getByTestId('export-toggle').getAttribute('aria-expanded') !== 'true') await page.getByTestId('export-toggle').click()
  const card = await page.getByTestId('export').boundingBox(); const graph = await page.locator('.kglv-graph-host').boundingBox()
  expect(card!.y).toBeGreaterThanOrEqual(graph!.y)
  expect(card!.y + card!.height).toBeLessThanOrEqual(graph!.y + graph!.height)
  await page.getByTestId('export-scoped').click()
}
async function snapshot(server: Launched): Promise<SharedSnapshotMeta> {
  const listener = new Listener(server.info.url.replace(/^http/, 'ws') + 'ws')
  try { await listener.open(); const message = listener.received.find(message => message.kind === 'shared-update'); if (message?.kind !== 'shared-update') throw new Error('No shared snapshot'); return message.value.meta.snapshot }
  finally { listener.close() }
}

test('shared readability keeps selected labels, schema labels, camera and renderer stable across browsers', async ({page, context}) => {
  const server = await launch(); const peer = await context.newPage()
  try {
    await page.goto(appUrl(server.info)); await ready(page); await browse(page, server)
    await openDestination(page, 'data')
    await page.getByTestId('records-table').locator('tr').filter({hasText: /Person_0$/}).getByRole('checkbox').check()
    await openDestination(page, 'explore')
    const selected = await page.evaluate(() => window.__kglv.selectedCount)
    expect(selected).toBe(1)
    await peer.goto(appUrl(server.info)); await ready(peer)
    const probe = await page.evaluateHandle(() => {
      const graph = window.__kglvBench.graph!; graph.zoom(0.25, 0)
      const state = {positions: 0, links: 0, zoom: graph.getZoomLevel(), opacity: 1}
      const positions = graph.setPointPositions.bind(graph); const links = graph.setLinks.bind(graph); const config = graph.setConfigPartial.bind(graph)
      graph.setPointPositions = (...args) => {state.positions += 1; return positions(...args)}
      graph.setLinks = (...args) => {state.links += 1; return links(...args)}
      graph.setConfigPartial = value => {if (value.linkOpacity !== undefined) state.opacity = value.linkOpacity; return config(value)}
      return state
    })
    await openDrawer(peer, 'appearance')
    await peer.getByTestId('readability-label_density').fill('0'); await peer.getByTestId('readability-label_density').press('Tab')
    await expect(peer.getByTestId('readability-status')).toContainText('applied')
    await expect(page.locator('.kglv-label')).toHaveCount(1)
    await expect(page.locator('.kglv-label')).toContainText('Person_0')
    await peer.getByTestId('readability-edge_opacity').fill('0.25'); await peer.getByTestId('readability-edge_opacity').press('Tab')
    await expect.poll(() => probe.evaluate(value => value.opacity)).toBe(0.25)
    await peer.getByTestId('readability-legend_visible').uncheck()
    await expect(page.getByTestId('legend')).toBeHidden()
    expect(await probe.evaluate(value => ({positions: value.positions, links: value.links}))).toEqual({positions: 0, links: 0})
    expect(await page.evaluate(() => window.__kglvBench.graph!.getZoomLevel())).toBeCloseTo(await probe.evaluate(value => value.zoom), 5)
    await page.getByTestId('scope-schema').click(); await page.getByTestId('fit-visible').click()
    await expect(page.locator('.kglv-label')).toHaveCount(5)
    await page.getByTestId('scope-instances').click()
    await peer.getByTestId('readability-prioritize_selected_labels').uncheck()
    await expect(page.locator('.kglv-label')).toHaveCount(0)
    await peer.getByTestId('readability-prioritize_hovered_labels').check()
    await page.getByTestId('fit-visible').click()
    const point = await page.evaluate(() => {
      const graph = window.__kglvBench.graph!; const xy = graph.getPointPositions(); const host = document.querySelector('.kglv-graph-host')!.getBoundingClientRect()
      for (let slot = 5; slot < window.__kglv.slotCount; slot += 1) {
        const [x, y] = graph.spaceToScreenPosition([xy[slot * 2]!, xy[slot * 2 + 1]!])
        if (x > 45 && x < host.width - 45 && y > 100 && y < host.height - 120) return {slot, x: host.x + x, y: host.y + y}
      }
      return null
    })
    expect(point).not.toBeNull()
    await page.mouse.move(point!.x, point!.y)
    await expect(page.locator(`.kglv-label[data-slot="${point!.slot}"]`)).toBeVisible()
    await page.mouse.move(2, 2)
    await expect(page.locator('.kglv-label')).toHaveCount(0)
    await peer.getByTestId('readability-node_size_min').fill('23'); await peer.getByTestId('readability-node_size_min').press('Tab')
    await expect(peer.getByTestId('readability-status')).toContainText('size')
    await expect(peer.getByTestId('readability-node_size_min')).toHaveValue('4')
    await peer.getByTestId('readability-reset').click()
    await expect(page.getByTestId('legend')).toBeVisible()
    await expect.poll(async () => page.locator('.kglv-label').count()).toBeGreaterThan(1)
  } finally { await peer.close(); server.process.kill() }
})

test('peer property appearance applies the published handle mapping without a local statistics fetch', async ({page}) => {
  const server = await launch('crates/kglite-visual-core/tests/fixtures/viewer-appearance.kgl')
  try {
    await page.goto(appUrl(server.info)); await ready(page); expect((await page.request.post(`${server.info.url}api/browse-type`, {data: {node_type: 'Person', limit: 100}})).ok()).toBe(true)
    await expect(page.getByTestId('count-loaded')).toHaveText('3')
    let propertyReads = 0; let recordReads = 0
    page.on('request', request => {if (request.url().endsWith('/api/property-stats')) propertyReads += 1; if (request.url().endsWith('/api/records')) recordReads += 1})
    const arrays = await page.evaluateHandle(() => {
      const graph = window.__kglvBench.graph!; const state = {colors: [] as number[], sizes: [] as number[]}
      const colors = graph.setPointColors.bind(graph); const sizes = graph.setPointSizes.bind(graph)
      graph.setPointColors = value => {state.colors = Array.from(value); return colors(value)}
      graph.setPointSizes = value => {state.sizes = Array.from(value); return sizes(value)}
      return state
    })
    expect((await page.request.post(`${server.info.url}api/appearance`, {data: {color_by: 'mixed', size_by: 'score'}})).ok()).toBe(true)
    await expect(page.getByTestId('legend-body')).toContainText('Domain: loaded instances')
    const state = await snapshot(server)
    expect(state.appearance_mapping.nodes.length).toBe(3)
    expect(state.appearance_mapping.categories.map(item => item.value)).toEqual(expect.arrayContaining([
      {type: 'int64', value: '2'}, {type: 'int64', value: '9007199254740993'}, {type: 'string', value: '9007199254740993'},
    ]))
    expect(new Set(state.appearance_mapping.nodes.map(item => JSON.stringify(item.color))).size).toBe(3)
    expect(state.appearance_mapping.size_min).toEqual({type: 'int64', value: '9007199254740993'})
    expect(state.appearance_mapping.size_max).toEqual({type: 'int64', value: '9007199254740995'})
    const actual = await arrays.jsonValue()
    for (const node of state.appearance_mapping.nodes) {
      const slot = state.slice.nodes.find(item => item.handle.node_id === node.handle.node_id)!.slot
      expect(actual.sizes[slot]).toBeCloseTo(node.radius!, 5)
      for (let channel = 0; channel < 4; channel += 1) expect(actual.colors[slot * 4 + channel]).toBeCloseTo(node.color![channel]!, 5)
    }
    expect(propertyReads).toBe(0); expect(recordReads).toBe(0)
  } finally { server.process.kill() }
})

test('scoped graph previews name exact versus induced relations and stale offers cannot download', async ({page}) => {
  const server = await launch('crates/kglite-visual-core/tests/fixtures/viewer-identity.kgl')
  try {
    await page.goto(appUrl(server.info)); await ready(page)
    const nodes = await (await page.request.post(`${server.info.url}api/cypher`, {data: {query: 'MATCH (n) RETURN n', params: {}, limit: 100, as_graph: false}})).json() as QueryTable
    const edges = await (await page.request.post(`${server.info.url}api/cypher`, {data: {query: 'MATCH (a)-[r]->(b) RETURN r', params: {}, limit: 100, as_graph: false}})).json() as QueryTable
    expect((await page.request.post(`${server.info.url}api/load-entities`, {data: {nodes: nodes.row_references.flatMap(row => row.nodes), relationships: edges.row_references.flatMap(row => row.relationships)}})).ok()).toBe(true)
    await expect(page.getByTestId('count-loaded')).toHaveText('4')
    expect((await page.request.post(`${server.info.url}api/subset`, {data: {predicates: [{id: 'relations', enabled: true, predicate: {kind: 'relation', names: ['LIKES']}}]}})).ok()).toBe(true)
    await openExport(page); await page.getByTestId('export-format').selectOption('json')
    await page.getByTestId('export-preview').click(); await expect(page.getByTestId('export-preview-status')).toContainText('4 nodes · 1 relations')
    const first = page.waitForEvent('download'); await page.getByTestId('export-download').click()
    const download = await first; const text = await readFile((await download.path())!, 'utf8')
    const data = JSON.parse(text) as {nodes: unknown[]; links?: unknown[]; edges?: unknown[]}
    expect(data.nodes).toHaveLength(4); expect(data.links ?? data.edges).toHaveLength(1)
    await page.getByTestId('export-scope').selectOption('loaded-induced')
    await expect(page.getByTestId('export-download')).toBeDisabled()
    await page.getByTestId('export-preview').click(); await expect(page.getByTestId('export-preview-status')).toContainText('4 nodes · 4 relations')
    expect((await page.request.post(`${server.info.url}api/focus`, {data: {slots: []}})).ok()).toBe(true)
    await expect(page.getByTestId('export-preview-status')).toContainText('shared view changed')
    await expect(page.getByTestId('export-download')).toBeDisabled()
    await page.keyboard.press('Escape'); await expect(page.getByTestId('export-scoped')).toBeFocused()
  } finally { server.process.kill() }
})

test('narrow deterministic SVG and PNG previews show dimensions and download the same captured bytes', async ({page}) => {
  const server = await launch()
  try {
    await page.setViewportSize({width: 640, height: 720}); await page.goto(appUrl(server.info)); await ready(page); await browse(page, server)
    await openExport(page)
    for (const format of ['svg', 'png']) {
      await page.getByTestId('export-format').selectOption(format)
      await page.getByTestId('export-width').fill('640'); await page.getByTestId('export-height').fill('480')
      await page.getByTestId('export-preview').click(); await expect(page.getByTestId('export-preview-status')).toContainText('640 × 480 pixels')
      await expect(page.getByTestId('export-preview-status')).toContainText('names shown')
      const source = await page.getByTestId('export-image-preview').getAttribute('src'); expect(source).toContain(';base64,')
      const pending = page.waitForEvent('download'); await page.getByTestId('export-download').click(); const download = await pending
      const bytes = await readFile((await download.path())!)
      expect(bytes.equals(Buffer.from(source!.split(',')[1]!, 'base64'))).toBe(true)
    }
    const bounds = await page.getByTestId('scoped-export-dialog').boundingBox(); expect(bounds!.width).toBeLessThanOrEqual(640)
    await page.keyboard.press('Escape'); await expect(page.getByTestId('export-scoped')).toBeFocused()
  } finally { server.process.kill() }
})

test('table CSV names returned query, visible and selected fetched record scopes with typed metadata', async ({page}) => {
  const server = await launch()
  try {
    await page.goto(appUrl(server.info)); await ready(page); await browse(page, server); await openDestination(page, 'data')
    await expect(page.getByTestId('records-csv')).toContainText('60 fetched visible')
    await page.getByTestId('records-table').getByRole('checkbox').first().check()
    await page.getByTestId('records-scope').selectOption('selected')
    await expect(page.getByTestId('records-csv')).toContainText('1 fetched selected')
    const selected = page.waitForEvent('download'); await page.getByTestId('records-csv').click(); const selectedFile = await selected
    const selectedText = await readFile((await selectedFile.path())!, 'utf8')
    expect(selectedText).toContain('id [cell state]'); expect(selectedText.trim().split('\r\n')).toHaveLength(2)
    await fillQuery(page, 'RETURN 9007199254740993 AS exact, "null" AS literal')
    await page.getByTestId('query-run').click(); await expect(page.getByTestId('query-csv')).toContainText('1 returned query rows')
    const query = page.waitForEvent('download'); await page.getByTestId('query-csv').click(); const queryFile = await query
    const queryText = await readFile((await queryFile.path())!, 'utf8')
    expect(queryText).toContain('"9007199254740993","value","int64"')
    expect(queryText).toContain('"""null""","value","string"')
    await expect(page.getByTestId('count-loaded')).toHaveText('60')
  } finally { server.process.kill() }
})

test('header Export opens one persistent dialog from Data and Query with narrow keyboard focus return', async ({page}) => {
  const server = await launch()
  try {
    await page.setViewportSize({width: 640, height: 720}); await page.goto(appUrl(server.info)); await ready(page)
    await openDestination(page, 'data')
    const button = page.getByTestId('workspace-export'); await button.focus(); await page.keyboard.press('Enter')
    await expect(page.getByTestId('scoped-export-dialog')).toBeVisible(); await expect(page.getByTestId('export-scope')).toBeFocused()
    await page.getByTestId('export-scope').selectOption('loaded-induced')
    await page.keyboard.press('Escape'); await expect(button).toBeFocused()
    await openDestination(page, 'query'); await button.focus(); await page.keyboard.press('Enter')
    await expect(page.getByTestId('scoped-export-dialog')).toBeVisible()
    await expect(page.getByTestId('export-scope')).toHaveValue('loaded-induced')
    await expect(page.getByTestId('scoped-export-dialog')).toHaveCount(1)
    await page.keyboard.press('Escape'); await expect(button).toBeFocused()
    const bounds = await button.boundingBox(); expect(bounds!.x).toBeGreaterThanOrEqual(0); expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(640)
  } finally { server.process.kill() }
})

test('export Escape closes only the modal over an open Appearance drawer and returns header focus', async ({page}) => {
  const server = await launch()
  try {
    await page.setViewportSize({width: 640, height: 720}); await page.goto(appUrl(server.info)); await ready(page)
    await openDrawer(page, 'appearance')
    await page.getByTestId('workspace-export').click()
    await expect(page.getByTestId('scoped-export-dialog')).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(page.getByTestId('scoped-export-dialog')).not.toBeVisible()
    await expect(page.getByTestId('drawer-appearance')).toHaveAttribute('aria-expanded', 'true')
    await expect(page.getByTestId('workspace-export')).toBeFocused()
  } finally { server.process.kill() }
})
