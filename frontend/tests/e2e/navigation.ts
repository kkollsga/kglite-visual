import type { Page } from '@playwright/test'
import { openDestination } from './harness'

export async function openDrawer(page: Page, name: 'filters' | 'appearance'): Promise<void> {
  const button = page.getByTestId(`drawer-${name}`)
  if (await button.getAttribute('aria-expanded') !== 'true') await button.click()
}

export async function closeDrawer(page: Page): Promise<void> {
  const close = page.getByTestId('drawer-close')
  if (await close.isVisible()) await close.click()
}

/** Existing renderer suites exercise the supported mixed schema/instance presentation. */
export async function keepSchemaContext(page: Page): Promise<void> {
  await openDestination(page, 'explore')
  await page.getByTestId('schema-context').check()
}
