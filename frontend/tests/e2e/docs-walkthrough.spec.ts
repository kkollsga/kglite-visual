import { expect, test, type Page } from '@playwright/test'
import { readFile } from 'node:fs/promises'
import { copyFileSync, mkdtempSync, rmSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { appUrl, fillQuery, launch, Listener, openDestination, REPO, type Launched } from './harness'
import { closeDrawer, openDrawer } from './navigation'
import { fieldKey, fieldTestId } from '../../src/fields'
import type { SharedSnapshotMeta } from '../../src/generated/SharedSnapshotMeta'

const SAMPLE = 'docs/_static/team.kgl'

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__kglv?.ready === true)
  await expect(page.locator('.kglv-label').first()).toBeVisible()
}

async function addField(page: Page, field: string): Promise<void> {
  await page.getByTestId('records-field').fill(field)
  await page.getByTestId('records-add-field').click()
  await expect(page.getByTestId(`records-sort-${field}`)).toBeVisible()
}

async function openExport(page: Page): Promise<void> {
  await page.getByTestId('workspace-export').click()
  await expect(page.getByTestId('scoped-export-dialog')).toBeVisible()
}

async function stop(server: Launched): Promise<void> {
  const exited = new Promise<{ code: number | null, signal: NodeJS.Signals | null }>(resolve => {
    server.process.once('exit', (code, signal) => resolve({ code, signal }))
  })
  server.process.kill()
  await expect(exited).resolves.toEqual({ code: 0, signal: null })
  await expect.poll(async () => fetch(server.info.url).then(() => false).catch(() => true)).toBe(true)
}

async function snapshot(server: Launched): Promise<SharedSnapshotMeta> {
  const listener = new Listener(server.info.url.replace(/^http/, 'ws') + 'ws')
  try {
    await listener.open()
    const message = listener.received.find(item => item.kind === 'shared-update')
    if (message?.kind !== 'shared-update') throw new Error('No shared snapshot')
    return message.value.meta.snapshot
  } finally {
    listener.close()
  }
}

test('the downloadable team graph supports the complete documentation walkthrough', async ({ page }, testInfo) => {
  const sampleDir = mkdtempSync(path.join(os.tmpdir(), 'kglv-team-walkthrough-'))
  const samplePath = path.join(sampleDir, 'team.kgl')
  copyFileSync(path.join(REPO, SAMPLE), samplePath)
  let server = await launch(samplePath)
  let running = true
  const viewName = `Team overview ${Date.now()}`
  try {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto(appUrl(server.info))
    await ready(page)
    await expect(page.locator('.kglv-label')).toHaveCount(3)
    expect(await page.evaluate(() => ({ points: window.__kglv.pointCount, links: window.__kglv.linkCount }))).toEqual({ points: 3, links: 4 })
    await page.screenshot({ path: testInfo.outputPath('team-overview.png'), fullPage: true })

    await page.getByTestId('browse-type-picker').selectOption({ label: 'Person' })
    await page.getByTestId('expand-limit').fill('20')
    await page.getByTestId('browse-type').click()
    await expect(page.getByTestId('count-loaded')).toHaveText('9')
    await expect(page.getByTestId('count-visible')).toHaveText('9')
    expect(await page.evaluate(() => window.__kglv.linkCount)).toBe(0)

    await openDestination(page, 'data')
    await page.getByTestId('data-records').click()
    await expect(page.getByTestId('records-table')).toBeVisible()
    await addField(page, 'role')
    await addField(page, 'location')
    await addField(page, 'years')
    const ada = page.getByTestId('records-table').locator('tr').filter({ has: page.getByText('Ada', { exact: true }) })
    await expect(ada).toContainText('engineer')
    await expect(ada).toContainText('Oslo')
    await expect(ada).toContainText('6')
    await ada.getByRole('checkbox').check()
    await ada.getByTestId('record-show-graph').click()
    await expect(page.getByTestId('selection-title')).toHaveText('Ada — Person')
    await openDestination(page, 'data')
    await page.screenshot({ path: testInfo.outputPath('team-records.png'), fullPage: true })

    await fillQuery(page, 'MATCH (a)-[r]->(b) RETURN a, r, b LIMIT 100')
    await page.getByTestId('query-as-graph').check()
    await page.getByTestId('query-run').click()
    await expect(page.getByTestId('count-loaded')).toHaveText('17')
    await expect(page.getByTestId('count-visible')).toHaveText('17')
    expect(await page.evaluate(() => window.__kglv.linkCount)).toBe(34)
    await expect(page.getByTestId('truncation-banner')).toHaveCount(0)

    await openDrawer(page, 'filters')
    await page.getByTestId('subset-kind').selectOption('category')
    await page.getByTestId('subset-field').fill('role')
    await page.getByTestId('subset-values').fill('engineer')
    await page.getByTestId('subset-apply').click()
    await expect(page.getByTestId('count-visible')).toHaveText('5')
    await expect(page.getByTestId('subset-status')).toContainText('5 / 17 loaded instances')
    await page.getByTestId('subset-clear').click()
    await expect(page.getByTestId('count-visible')).toHaveText('17')
    await closeDrawer(page)

    await openDestination(page, 'data')
    await page.getByTestId('calculation-kind').selectOption('degree')
    await page.getByTestId('calculation-run').click()
    await expect(page.getByTestId('calculation-status')).toContainText('17 instances · 34 relation records')
    const calculated = await snapshot(server)
    const degree = calculated.calculations.find(item => item.kind === 'degree')
    const total = degree?.fields.find(item => item.field.kind === 'derived' && item.field.column === 'total')?.field
    const adaHandle = calculated.slice.nodes.find(item => item.title === 'Ada')?.handle
    expect(total).toBeDefined()
    expect(adaHandle).toBeDefined()
    const degreeRow = await page.request.post(`${server.info.url}api/records`, {
      data: { handles: [adaHandle], fields: [], field_refs: [total], offset: 0, limit: 1 },
    })
    expect(degreeRow.ok(), await degreeRow.text()).toBe(true)
    expect((await degreeRow.json()).rows[0].cells[0]).toEqual({ state: 'value', value: { type: 'int64', value: '6' } })
    const calculatedAda = page.getByTestId('records-table').locator('tr').filter({ has: page.getByText('Ada', { exact: true }) })
    const headers = await page.getByTestId('records-table').locator('th').allTextContents()
    const totalColumn = headers.findIndex(text => text.includes('Total degree'))
    expect(totalColumn).toBeGreaterThan(1)
    await expect(page.getByTestId(`records-sort-${fieldTestId(total!)}`)).toBeVisible()
    await expect(calculatedAda.locator('td').nth(totalColumn)).toHaveText('6')

    await openDestination(page, 'explore')
    await openDrawer(page, 'appearance')
    await page.getByTestId('size-by').selectOption(fieldKey(total!))
    await expect(page.getByTestId('legend-body')).toContainText('Degree')
    await closeDrawer(page)

    await page.getByTestId('views-open').click()
    await expect(page.getByTestId('view-storage-note')).not.toContainText('Checking')
    await page.getByTestId('view-name').fill(viewName)
    await page.getByTestId('view-focus').selectOption('fit')
    await page.getByTestId('view-save').click()
    await expect(page.getByTestId('views-status')).toContainText(`Saved “${viewName}”`)
    await page.getByTestId('views-close').click()

    await stop(server)
    running = false
    server = await launch(samplePath)
    running = true
    await page.goto(appUrl(server.info))
    await ready(page)
    await expect(page.getByTestId('count-loaded')).toHaveText('0')
    await page.getByTestId('views-open').click()
    const saved = page.getByTestId('views-catalog').locator('[data-name]').filter({ hasText: viewName })
    await saved.getByTestId('view-restore').click()
    await expect(page.getByTestId('views-status')).toContainText(`Restored “${viewName}”`)
    await expect(page.getByTestId('count-loaded')).toHaveText('17')
    await page.getByTestId('views-close').click()
    await openDestination(page, 'data')
    await expect(page.getByTestId('calculation-scope')).toContainText('17 instances · 34 relation records')

    await openDestination(page, 'explore')
    await openExport(page)
    await page.getByTestId('export-scope').selectOption('visible')
    await page.getByTestId('export-format').selectOption('graphml')
    await page.getByTestId('export-preview').click()
    await expect(page.getByTestId('export-preview-status')).toContainText('17 nodes · 34 relations')
    const pending = page.waitForEvent('download')
    await page.getByTestId('export-download').click()
    const graphml = await readFile((await (await pending).path())!, 'utf8')
    expect(graphml.match(/<node\b/g)).toHaveLength(17)
    expect(graphml.match(/<edge\b/g)).toHaveLength(34)

    await page.getByTestId('export-format').selectOption('png')
    await page.getByTestId('export-width').fill('1200')
    await page.getByTestId('export-height').fill('800')
    await page.getByTestId('export-preview').click()
    await expect(page.getByTestId('export-preview-status')).toContainText('1200 × 800 pixels')
    await expect(page.getByTestId('export-image-preview')).toBeVisible()
    await page.screenshot({ path: testInfo.outputPath('team-export.png'), fullPage: true })
  } finally {
    if (running) await stop(server)
    rmSync(sampleDir, { recursive: true, force: true })
  }
})
