import { expect, test, type Page } from '@playwright/test'

import { appUrl, fillQuery, launch, openDestination, queryText } from './harness'
import { closeDrawer, openDrawer } from './navigation'

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__kglv?.ready === true)
  await expect(page.locator('.kglv-label').first()).toBeVisible()
}

test('destinations preserve renderer, selection and draft; query results reveal Data', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 720 })
  const server = await launch()
  try {
    await page.goto(appUrl(server.info))
    await ready(page)
    const labels = page.locator('.kglv-label')
    await expect(labels).toHaveCount(5)
    for (let i = 0; i < 5; i += 1) {
      await expect.poll(async () => {
        const label = await labels.nth(i).boundingBox()
        const canvas = await page.locator('.kglv-graph-host').boundingBox()
        return label !== null && canvas !== null && label.x >= canvas.x && label.y >= canvas.y &&
          label.x + label.width <= canvas.x + canvas.width && label.y + label.height <= canvas.y + canvas.height
      }).toBe(true)
    }
    const graph = await page.evaluateHandle(() => window.__kglvBench.graph)
    const canvas = await page.locator('.kglv-canvas canvas').elementHandle()
    const draft = 'MATCH (p:Person) RETURN p.title AS person LIMIT 3'
    await fillQuery(page, draft)
    await openDestination(page, 'explore')
    await page.getByTestId('browse-type-picker').selectOption('0')
    await expect(page.getByTestId('browse-type')).toHaveText('Browse Person instances')
    await page.getByTestId('expand-limit').fill('12')
    await page.getByTestId('browse-type').click()
    await expect(page.getByTestId('count-loaded')).toHaveText('12')
    await expect(page.getByTestId('scope-instances')).toHaveAttribute('aria-pressed', 'true')
    expect(await page.evaluate(() => window.__kglv.pointCount)).toBe(12)
    await expect(page.locator('.kglv-label[data-slot="0"]')).toHaveCount(0)

    const instance = page.locator('.kglv-label').first()
    await instance.click()
    await expect(page.getByTestId('selection-title')).not.toContainText('(type)')
    const selectedTitle = await page.getByTestId('selection-title').textContent()
    await expect(page.getByTestId('count-selected')).toHaveText('1')
    await openDestination(page, 'data')
    await openDestination(page, 'query')
    expect(await queryText(page)).toBe(draft)
    await page.getByTestId('query-run').click()
    await expect(page.getByTestId('destination-data')).toHaveAttribute('aria-selected', 'true')
    await expect(page.getByTestId('query-table')).toBeInViewport()
    await expect(page.getByTestId('query-table').locator('tr')).toHaveCount(4)
    await expect(page.locator('.kglv-data-destination')).toContainText('Query results · source scope')
    await openDestination(page, 'query')
    expect(await queryText(page)).toBe(draft)
    await openDestination(page, 'explore')
    await expect(page.getByTestId('selection-title')).toHaveText(selectedTitle ?? '')
    expect(await page.evaluate((saved) => saved === window.__kglvBench.graph, graph)).toBe(true)
    expect(await page.locator('.kglv-canvas canvas').evaluate((current, saved) => current === saved, canvas)).toBe(true)
    await expect(page.getByTestId('count-selected')).toHaveText('1')
  } finally {
    server.process.kill()
  }
})

test('schema navigation preserves membership and static camera; fit names visible slots', async ({ page }) => {
  const server = await launch()
  const sent: string[] = []
  page.on('websocket', (socket) => socket.on('framesent', (event) => sent.push(String(event.payload))))
  try {
    await page.goto(appUrl(server.info))
    await ready(page)
    await page.getByTestId('browse-type-picker').selectOption('0')
    await page.getByTestId('expand-limit').fill('8')
    await page.getByTestId('browse-type').click()
    await expect(page.getByTestId('count-loaded')).toHaveText('8')
    await page.evaluate(() => window.__kglvBench.graph?.zoom(0.25, 0))
    await expect.poll(() => page.evaluate(() => window.__kglvBench.graph?.getZoomLevel())).toBe(0.25)
    const start = sent.length
    await page.getByTestId('scope-schema').click()
    expect(await page.evaluate(() => window.__kglv.pointCount)).toBe(5)
    await page.getByTestId('scope-instances').click()
    expect(await page.evaluate(() => window.__kglv.pointCount)).toBe(8)
    await page.getByTestId('schema-context').check()
    expect(await page.evaluate(() => window.__kglv.pointCount)).toBe(13)
    await page.getByTestId('schema-context').uncheck()
    expect(await page.evaluate(() => window.__kglvBench.graph?.getZoomLevel())).toBe(0.25)
    await expect(page.getByTestId('count-loaded')).toHaveText('8')
    expect(sent.slice(start).filter((value) => /"type":"(?:meta-graph|reset|expand|collapse|browse-type|cypher)"/.test(value))).toEqual([])
    const fitted = await page.evaluateHandle(() => {
      const graph = window.__kglvBench.graph!
      const recorded: number[][] = []
      const fit = graph.fitViewByPointIndices.bind(graph)
      graph.fitViewByPointIndices = (indices, ...args) => {
        recorded.push([...indices])
        fit(indices, ...args)
      }
      return recorded
    })
    await page.getByTestId('fit-visible').click()
    expect(await fitted.jsonValue()).toEqual([[5, 6, 7, 8, 9, 10, 11, 12]])
  } finally {
    server.process.kill()
  }
})

test('a disconnected type is browsable without relationship expansion', async ({ page }) => {
  const server = await launch('crates/kglite-visual-core/tests/fixtures/spill.kgl')
  try {
    await page.goto(appUrl(server.info))
    await ready(page)
    await page.getByTestId('browse-type-picker').selectOption('0')
    await expect(page.getByTestId('preview-summary')).toHaveText('no relationships to expand')
    await page.getByTestId('expand-limit').fill('1')
    await page.getByTestId('browse-type').click()
    await expect(page.getByTestId('count-loaded')).toHaveText('1')
    expect(await page.evaluate(() => window.__kglv.pointCount)).toBe(1)
    await expect(page.getByTestId('scope-instances')).toHaveAttribute('aria-pressed', 'true')
  } finally {
    server.process.kill()
  }
})

test('selecting an unlabelled canvas node promotes its label immediately', async ({ page }) => {
  const server = await launch()
  try {
    await page.goto(appUrl(server.info))
    await ready(page)
    await page.getByTestId('browse-type-picker').selectOption('0')
    await page.getByTestId('browse-type').click()
    await expect(page.getByTestId('count-loaded')).toHaveText('60')
    const candidate = await page.evaluate(() => {
      const graph = window.__kglvBench.graph!
      const host = document.querySelector('.kglv-graph-host')!.getBoundingClientRect()
      const labels = [...document.querySelectorAll<HTMLElement>('.kglv-label')]
      const labelled = new Set(labels.map((node) => Number(node.dataset['slot'])))
      const rectangles = labels.map((node) => node.getBoundingClientRect())
      const positions = graph.getPointPositions()
      for (let slot = 5; slot < window.__kglv.slotCount; slot += 1) {
        if (labelled.has(slot)) continue
        const point: [number, number] = [positions[slot * 2]!, positions[slot * 2 + 1]!]
        const [localX, localY] = graph.spaceToScreenPosition(point)
        const x = host.x + localX
        const y = host.y + localY
        if (localX < 30 || localX > host.width - 30 || localY < 85 || localY > host.height - 105) continue
        if (rectangles.some((r) => x >= r.left - 8 && x <= r.right + 8 && y >= r.top - 8 && y <= r.bottom + 8)) continue
        return { slot, x, y }
      }
      return null
    })
    expect(candidate, 'fixture must offer an unlabelled, unobstructed instance to select').not.toBeNull()
    await page.mouse.click(candidate!.x, candidate!.y)
    await expect(page.getByTestId('count-selected')).toHaveText('1')
    await expect(page.locator(`.kglv-label[data-slot="${candidate!.slot}"]`)).toBeVisible()
  } finally {
    server.process.kill()
  }
})

test('narrow keyboard navigation restores focus and the selected-record inspector', async ({ page }) => {
  await page.setViewportSize({ width: 640, height: 720 })
  const server = await launch()
  try {
    await page.goto(appUrl(server.info))
    await ready(page)
    await page.getByTestId('browse-type-picker').selectOption('0')
    await expect(page.getByTestId('browse-type')).toBeVisible()
    await page.getByTestId('expand-limit').fill('4')
    await page.getByTestId('browse-type').click()
    await expect(page.getByTestId('count-loaded')).toHaveText('4')
    await page.locator('.kglv-label').first().click()
    await expect(page.getByTestId('selection-title')).not.toContainText('(type)')
    const title = await page.getByTestId('selection-title').textContent()
    await page.keyboard.press('Escape')
    await expect(page.getByTestId('inspector-open')).toBeFocused()
    await fillQuery(page, 'RETURN 1 AS value')
    await page.getByTestId('query-run').click()
    await expect(page.getByTestId('destination-data')).toHaveAttribute('aria-selected', 'true')
    await openDestination(page, 'explore')
    await page.getByTestId('inspector-open').click()
    await expect(page.getByTestId('selection-title')).toHaveText(title ?? '')
    await page.keyboard.press('Escape')
    await openDrawer(page, 'filters')
    await page.keyboard.press('Escape')
    await expect(page.getByTestId('drawer-filters')).toBeFocused()
    await page.getByTestId('destination-explore').focus()
    await page.keyboard.press('ArrowRight')
    await expect(page.getByTestId('destination-data')).toBeFocused()
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    await closeDrawer(page)
  } finally {
    server.process.kill()
  }
})
