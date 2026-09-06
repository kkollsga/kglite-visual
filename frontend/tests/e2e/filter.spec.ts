import { expect, test } from '@playwright/test'
import { openDrawer } from './navigation'
import { appUrl, launch } from './harness'

test('acknowledged filters hide without unloading and clearing restores hidden selection', async ({ page }) => {
  const server = await launch()
  try {
    await page.goto(appUrl(server.info))
    await page.waitForFunction(() => window.__kglv?.ready === true)
    await page.getByTestId('browse-type-picker').selectOption('0')
    await page.getByTestId('browse-type').click()
    await expect(page.getByTestId('count-loaded')).toHaveText('60')
    await page.locator('.kglv-label').first().click()
    await expect(page.getByTestId('count-selected')).toHaveText('1')
    const loaded = await page.evaluate(() => window.__kglv)
    const beforeRevision = await page.locator('.kglv-root').getAttribute('data-shared-revision')
    await openDrawer(page, 'filters')
    await page.getByTestId('subset-choices').selectOption('Company')
    await page.getByTestId('subset-apply').click()
    await expect(page.getByTestId('count-visible')).toHaveText('0')
    const hidden = await page.evaluate(() => window.__kglv)
    expect(hidden.slotCount).toBe(loaded.slotCount)
    expect(hidden.tombstoneCount).toBe(loaded.tombstoneCount)
    expect(hidden.lastMessageSeq).toBeGreaterThan(loaded.lastMessageSeq)
    expect(await page.locator('.kglv-root').getAttribute('data-shared-revision')).not.toBe(beforeRevision)
    expect(hidden.selectedCount).toBe(0)
    await expect(page.getByTestId('count-selected')).toHaveText('1')
    await expect(page.getByTestId('filter-banner')).toHaveText('filter: showing 0 of 60 drawn')
    await expect(page.locator('.kglv-label')).toHaveCount(0)
    await page.getByTestId('subset-clear').click()
    await expect(page.getByTestId('count-visible')).toHaveText('60')
    expect(await page.evaluate(() => window.__kglv.selectedCount)).toBe(1)
    await expect(page.getByTestId('filter-banner')).toHaveCount(0)

    // A property absent from every source record is missing, not unavailable.
    await page.getByTestId('subset-kind').selectOption('missing')
    await page.getByTestId('subset-field').fill('property_that_does_not_exist')
    await page.getByTestId('subset-missing').check()
    await page.getByTestId('subset-apply').click()
    await expect(page.getByTestId('subset-distributions')).toContainText('60 missing · 0 unavailable')
    await expect(page.getByTestId('count-visible')).toHaveText('60')
    await page.getByTestId('subset-clear').click()
    await expect(page.getByTestId('subset-active')).toBeEmpty()
    expect(await page.evaluate(() => window.__kglv.slotCount)).toBe(loaded.slotCount)
  } finally { server.process.kill() }
})
