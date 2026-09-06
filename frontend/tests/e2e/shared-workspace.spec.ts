import { expect, test, type Page } from '@playwright/test'
import { appUrl, launch } from './harness'
import { openDrawer } from './navigation'

async function revision(page: Page): Promise<string | null> {
  return page.locator('.kglv-root').getAttribute('data-shared-revision')
}

test('core filters preserve local selection, camera and GPU topology across tabs', async ({ page, context }) => {
  const server = await launch('crates/kglite-visual-core/tests/fixtures/viewer-identity.kgl')
  try {
    await page.goto(appUrl(server.info))
    await page.waitForFunction(() => window.__kglv?.ready === true)
    const loaded = await page.request.post(`${server.info.url}api/cypher`, {data: {query: 'MATCH (n) RETURN n', params: {}, limit: 100, as_graph: true}})
    expect(loaded.ok()).toBe(true)
    await expect(page.getByTestId('count-loaded')).toHaveText('4')
    await page.getByTestId('fit-visible').click()
    await page.locator('.kglv-label').filter({hasText: 'Outside the relationships'}).click()
    const title = await page.getByTestId('selection-title').textContent()
    const other = await context.newPage()
    await other.goto(appUrl(server.info))
    await other.waitForFunction(() => window.__kglv?.ready === true)
    await expect(other.getByTestId('count-loaded')).toHaveText('4')
    await page.evaluate(() => window.__kglvBench.graph!.zoom(0.25, 0))
    const uploads = await page.evaluateHandle(() => {
      const graph = window.__kglvBench.graph!
      const calls = {positions: 0, links: 0}
      const positions = graph.setPointPositions.bind(graph)
      const links = graph.setLinks.bind(graph)
      graph.setPointPositions = (...args) => { calls.positions += 1; return positions(...args) }
      graph.setLinks = (...args) => { calls.links += 1; return links(...args) }
      return calls
    })
    await openDrawer(page, 'filters')
    await page.getByTestId('subset-choices').selectOption('Person')
    await page.getByTestId('subset-apply').click()
    await expect(page.getByTestId('count-visible')).toHaveText('3')
    await expect(other.getByTestId('count-visible')).toHaveText('3')
    await expect(page.getByTestId('count-loaded')).toHaveText('4')
    await expect(page.getByTestId('count-selected')).toHaveText('1')
    await expect(page.getByTestId('subset-status')).toContainText('3 / 4 loaded instances')
    expect(await uploads.jsonValue()).toEqual({positions: 0, links: 0})
    expect(await page.evaluate(() => window.__kglvBench.graph!.getZoomLevel())).toBe(0.25)
    expect(await revision(other)).toBe(await revision(page))
    await page.getByTestId('subset-clear').click()
    await expect(page.getByTestId('count-visible')).toHaveText('4')
    await page.keyboard.press('Escape')
    await expect(page.getByTestId('selection-title')).toHaveText(title ?? '')
    await other.close()
  } finally { server.process.kill() }
})

test('numeric, missing, relation and isolate predicates use acknowledged core counts', async ({ page }) => {
  const server = await launch('crates/kglite-visual-core/tests/fixtures/viewer-identity.kgl')
  try {
    await page.goto(appUrl(server.info))
    await page.waitForFunction(() => window.__kglv?.ready === true)
    expect((await page.request.post(`${server.info.url}api/cypher`, {data: {query: 'MATCH (a)-[r]->(b) RETURN a,r,b', params: {}, limit: 100, as_graph: true}})).ok()).toBe(true)
    await expect(page.getByTestId('count-loaded')).toHaveText('3')
    await openDrawer(page, 'filters')
    await expect(page.getByTestId('subset-status')).toContainText('4 / 4 loaded relationships')
    await page.getByTestId('subset-kind').selectOption('relation')
    await page.getByTestId('subset-choices').selectOption('KNOWS')
    await page.getByTestId('subset-apply').click()
    await expect(page.getByTestId('subset-status')).toContainText('3 / 4 loaded relationships')
    await expect(page.getByTestId('count-visible')).toHaveText('3')
    await page.getByTestId('subset-kind').selectOption('hide-isolated')
    await page.getByTestId('subset-apply').click()
    await expect(page.getByTestId('count-visible')).toHaveText('2')
    await page.getByTestId('subset-clear').click()
    await expect(page.getByTestId('count-visible')).toHaveText('3')
    await page.getByTestId('subset-kind').selectOption('numeric-range')
    await page.getByTestId('subset-field').fill('score')
    await page.getByTestId('subset-min').fill('0')
    await page.getByTestId('subset-max').fill('5')
    await page.getByTestId('subset-apply').click()
    await expect(page.getByTestId('count-visible')).toHaveText('2')
    await page.getByTestId('subset-clear').click()
    await expect(page.getByTestId('count-visible')).toHaveText('3')
    await page.getByTestId('subset-kind').selectOption('missing')
    await page.getByTestId('subset-field').fill('active')
    await page.getByTestId('subset-missing').check()
    await page.getByTestId('subset-apply').click()
    await expect(page.getByTestId('count-visible')).toHaveText('1')
  } finally { server.process.kill() }
})

test('a missing shared revision reconnects before taking a fresh full snapshot', async ({ page }) => {
  const server = await launch()
  let connections = 0
  let dropNext = false
  let dropping = false
  await page.routeWebSocket('**/ws', route => {
    connections += 1
    const remote = route.connectToServer()
    remote.onMessage(message => {
      if (typeof message !== 'string') {
        const type = message.readUInt32LE(4)
        if (dropNext && type === 18) { dropNext = false; dropping = true }
        if (dropping) { if ((message.readUInt32LE(12) & 1) !== 0) dropping = false; return }
      }
      route.send(message)
    })
  })
  try {
    await page.goto(appUrl(server.info))
    await page.waitForFunction(() => window.__kglv?.ready === true)
    await expect.poll(() => revision(page)).toBe('0')
    const graph = await page.evaluateHandle(() => window.__kglvBench.graph)
    dropNext = true
    expect((await page.request.post(`${server.info.url}api/subset`, {data: {predicates: []}})).ok()).toBe(true)
    expect((await page.request.post(`${server.info.url}api/appearance`, {data: {color_by: null, size_by: null}})).ok()).toBe(true)
    await expect.poll(() => connections).toBe(2)
    await expect.poll(() => revision(page)).toBe('2')
    expect(await page.evaluate(saved => saved === window.__kglvBench.graph, graph)).toBe(true)
    await expect(page.getByTestId('count-loaded')).toHaveText('0')
  } finally { server.process.kill() }
})

test('a cleared caption rejects a delayed field read even when core processes it at the new revision', async ({ page }) => {
  const server = await launch('crates/kglite-visual-core/tests/fixtures/viewer-identity.kgl')
  let release: (() => void) | undefined
  let held = false
  let done = false
  await page.route('**/api/records', async route => {
    const request = route.request().postDataJSON() as {fields: string[]}
    if (!held && request.fields.includes('category')) {
      held = true
      await new Promise<void>(resolve => { release = resolve })
      await route.continue()
      done = true
    } else await route.continue()
  })
  try {
    await page.goto(appUrl(server.info))
    await page.waitForFunction(() => window.__kglv?.ready === true)
    expect((await page.request.post(`${server.info.url}api/cypher`, {data: {query: 'MATCH (n) RETURN n', params: {}, limit: 100, as_graph: true}})).ok()).toBe(true)
    await expect(page.getByTestId('count-loaded')).toHaveText('4')
    await page.getByTestId('fit-visible').click()
    expect((await page.request.post(`${server.info.url}api/caption`, {data: {caption_by: 'category'}})).ok()).toBe(true)
    await expect.poll(() => held).toBe(true)
    await openDrawer(page, 'appearance')
    await expect(page.getByTestId('caption-by')).toHaveValue('category')
    expect((await page.request.post(`${server.info.url}api/caption`, {data: {caption_by: null}})).ok()).toBe(true)
    await expect.poll(() => revision(page)).toBe('3')
    await expect(page.getByTestId('caption-by')).toHaveValue('')
    await page.keyboard.press('Escape')
    const before = await page.locator('.kglv-label').allTextContents()
    const response = page.waitForResponse('**/api/records')
    release?.()
    await response
    await expect.poll(() => done).toBe(true)
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))
    expect(await page.locator('.kglv-label').allTextContents()).toEqual(before)
    expect(before.join(' ')).toContain('Ada')
  } finally { release?.(); server.process.kill() }
})
