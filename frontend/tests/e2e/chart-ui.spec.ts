import { expect, test, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'

import { appUrl, fillQuery, launch, openDestination, type Launched } from './harness'

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__kglv?.ready === true, undefined, {timeout: 30_000})
}

async function runTable(page: Page, query: string): Promise<void> {
  await fillQuery(page, query)
  await page.getByTestId('query-as-graph').uncheck()
  await page.getByTestId('query-run').click()
  await openDestination(page, 'data')
  await expect(page.getByTestId('query-table')).toBeVisible()
  await expect(page.getByTestId('query-chart-panel')).toContainText(query)
}

test('a bounded query result becomes an inspectable chart without changing graph selection', async ({page}) => {
  let server: Launched | null = null
  try {
    server = await launch()
    await page.goto(appUrl(server.info)); await ready(page)
    const selectedBefore = await page.evaluate(() => window.__kglv?.selectedCount)

    const query = 'MATCH (p:Person) RETURN p.title AS person, p.age AS age ORDER BY age LIMIT 12'
    await runTable(page, query)
    await page.getByTestId('chart-open').click()
    await expect(page.getByTestId('chart-status')).toContainText('12 complete source rows profiled')
    await page.getByTestId('chart-kind').selectOption('bar')
    await page.getByTestId('chart-shape').selectOption('rows')
    await page.getByTestId('chart-x').selectOption('person')
    await page.getByTestId('chart-y').selectOption('age')
    await page.getByTestId('chart-title').fill('Fixture ages')
    await page.getByTestId('chart-unit').fill('years')
    await page.getByTestId('chart-build').click()

    const svg = page.getByTestId('chart-picture').locator('svg')
    await expect(svg).toBeVisible()
    await expect(svg.locator('#chart-title')).toHaveText('Fixture ages')
    await expect(page.getByTestId('chart-values')).toContainText('12 visible')
    expect(await page.evaluate(() => window.__kglv?.selectedCount)).toBe(selectedBefore)

    const download = page.waitForEvent('download')
    await page.getByTestId('chart-svg').click()
    const saved = await download
    expect(saved.suggestedFilename()).toBe('fixture-ages.svg')
    const savedPath = await saved.path()
    expect(savedPath).not.toBeNull()
    const exported = readFileSync(savedPath!, 'utf8')
    expect(exported).toContain(query)
    expect(exported).toContain('&quot;resultRowsReturned&quot;:12')

    await page.getByTestId('chart-table-view').click()
    await expect(page.getByTestId('query-table')).toBeVisible()
    await page.getByTestId('chart-chart-view').click()
    await expect(svg).toBeVisible()

    await runTable(page, "UNWIND ['2024-01-01','2024-03-01'] AS month RETURN month, 31.0 AS volume LIMIT 12")
    await page.getByTestId('chart-open').click()
    await expect(page.getByTestId('chart-title')).toHaveValue('')
    await expect(page.getByTestId('chart-unit')).toHaveValue('')
    await expect(page.getByTestId('chart-monthly')).not.toBeChecked()
    await page.getByTestId('chart-kind').selectOption('line')
    await page.getByTestId('chart-shape').selectOption('rows')
    await page.getByTestId('chart-x').selectOption('month')
    await page.getByTestId('chart-y').selectOption('volume')
    await page.getByTestId('chart-monthly').check()
    await page.getByTestId('chart-monthly-confirm').check()
    await page.getByTestId('chart-scale').fill('1000000')
    await page.getByTestId('chart-build').click()
    await expect(page.getByTestId('chart-status')).toContainText('2 plotted points; 0 missing y values; 1 calendar gaps inserted')
    await expect(page.getByTestId('chart-values')).toContainText('Missing — gap')

    await runTable(page, "RETURN [{time:'2024-01-01',value:1.0},{time:'2024-02-01',value:2.0}] AS points, 'Field A' AS field LIMIT 1")
    await page.getByTestId('chart-open').click()
    await page.getByTestId('chart-shape').selectOption('point-map-array')
    await page.getByTestId('chart-points').selectOption('points')
    await page.getByTestId('chart-x-key').fill('time')
    await page.getByTestId('chart-y-key').fill('value')
    await page.getByTestId('chart-series').selectOption('field')
    await page.getByTestId('chart-build').click()
    await expect(page.getByTestId('chart-status')).toContainText('2 plotted points')
    await expect(page.getByTestId('chart-values')).toContainText('Field A')

    await runTable(page, "RETURN ['2024-01-01','2024-02-01'] AS months, [1.0,2.0] AS values LIMIT 1")
    await page.getByTestId('chart-open').click()
    await page.getByTestId('chart-kind').selectOption('line')
    await page.getByTestId('chart-shape').selectOption('paired-arrays')
    await page.getByTestId('chart-x').selectOption('months')
    await page.getByTestId('chart-y').selectOption('values')
    await page.getByTestId('chart-build').click()
    await expect(page.getByTestId('chart-status')).toContainText('2 plotted points')

    await runTable(page, 'MATCH (a:Person), (b:Person), (c:Person) RETURN a.age AS x, b.age AS y')
    await page.getByTestId('chart-open').click()
    await expect(page.getByTestId('chart-status')).toContainText('Visualization refused: this result contains 5000 of')
    await page.getByTestId('chart-build').click()
    await expect(page.getByTestId('chart-status')).toContainText('narrow or aggregate it before charting')
  } finally {
    server?.process.kill()
  }
})

test('chart provenance keeps the executed query when the editor changes', async ({page}) => {
  let server: Launched | null = null
  try {
    server = await launch()
    await page.goto(appUrl(server.info)); await ready(page)
    const executed = 'MATCH (a:Person), (b:Person) RETURN a.age AS x, b.age AS y LIMIT 3600'
    await fillQuery(page, executed)
    await page.getByTestId('query-as-graph').uncheck()
    await page.getByTestId('query-run').click()
    await fillQuery(page, 'RETURN "edited after send" AS draft')
    await openDestination(page, 'data')
    await expect(page.getByTestId('query-table')).toBeVisible()
    const source = page.getByText('Chart source and lifetime').locator('..')
    await source.locator('summary').click()
    await expect(source).toContainText(executed)
    await expect(source).not.toContainText('edited after send')
  } finally {
    server?.process.kill()
  }
})
