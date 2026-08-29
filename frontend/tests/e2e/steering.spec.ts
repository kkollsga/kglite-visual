/**
 * L3: the three steering commands, driven from outside the page (plan D14).
 *
 * `focus`, `highlight` and `appearance` are the only messages on this wire that
 * change nothing about *what* is on screen — they change where the user is
 * looking, what stands out, and how it is coloured. So none of them can be
 * asserted through a slot count, and all three are asserted through the
 * renderer's own read-back in `window.__kglv`: the zoom cosmos.gl reports, the
 * interaction-concept sizes, and the two appearance channel names.
 *
 * Every command here arrives over HTTP from the test process. Nothing in the
 * page is clicked.
 */

import { expect, test, type Page } from '@playwright/test'

import { appUrl, launch, type Launched } from './harness'

const PERSON_SLOT = 0
/** The fixture's five types are interned in count order: Person, Project, Skill, City, Company. */
const PROJECT_SLOT = 1
const META_POINTS = 5

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__kglv?.ready === true, undefined, { timeout: 30_000 })
}

async function post(url: string, path: string, body: unknown): Promise<Response> {
  return fetch(`${url}api/${path}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

test('focus moves the camera on a client that asked for nothing', async ({ page }) => {
  let server: Launched | null = null
  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)

    const before = await page.evaluate(() => window.__kglv)
    expect(before.zoomLevel).not.toBeNull()
    expect(before.focusedSlots).toEqual([])

    const response = await post(server.info.url, 'focus', { slots: [PERSON_SLOT, PROJECT_SLOT] })
    expect(response.status).toBe(200)
    // The audience size is the answer: a steering command that reached nobody
    // is indistinguishable from one that reached the user unless the server
    // says so.
    expect(await response.json()).toEqual({ clients: 1 })

    // Wait on the ZOOM, not on `focusedSlots`: the command is recorded before
    // the fit is asked for, and cosmos.gl runs the fit through a d3 transition
    // that has not applied when `fitViewByPointIndices` returns.
    await page.waitForFunction(
      (zoom) => window.__kglv.zoomLevel !== zoom,
      before.zoomLevel,
      { timeout: 15_000 },
    )
    const after = await page.evaluate(() => window.__kglv)
    expect(after.focusedSlots).toEqual([PERSON_SLOT, PROJECT_SLOT])
    // Two of five type nodes occupy less of the plane than all five, so
    // framing them is a zoom IN. The direction is the assertion, not the
    // number: the number is a function of the viewport and the settled layout.
    expect(after.zoomLevel).not.toBeNull()
    expect(after.zoomLevel as number).toBeGreaterThan(before.zoomLevel as number)
    // Nothing about the view itself moved. `focus` is a camera, and a camera
    // that allocated a slot would be a different message.
    expect(after.slotCount).toBe(before.slotCount)
    expect(after.pointCount).toBe(META_POINTS)
  } finally {
    server?.process.kill()
  }
})

test('an out-of-range focus is refused by name rather than narrowed', async ({ page }) => {
  let server: Launched | null = null
  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)

    const response = await post(server.info.url, 'focus', { slots: [PERSON_SLOT, 4000] })
    expect(response.status).toBe(400)
    const body = (await response.json()) as { error: string }
    // The bound is named, not implied: an agent holding a wrong model of the
    // view has to be told what the view actually is.
    expect(body.error).toContain('slot 4000 is not in this view')
    expect(body.error).toContain(`holds slots 0..${META_POINTS}`)

    // And the refusal is total — the legal half of the list was not applied.
    const after = await page.evaluate(() => window.__kglv)
    expect(after.focusedSlots).toEqual([])
  } finally {
    server?.process.kill()
  }
})

test('highlight drives the two remote-addressable interaction concepts', async ({ page }) => {
  let server: Launched | null = null
  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)

    expect((await page.evaluate(() => window.__kglv)).highlightedCount).toBe(0)

    await post(server.info.url, 'highlight', { slots: [PERSON_SLOT, PROJECT_SLOT] })
    await page.waitForFunction(() => window.__kglv.highlightedCount === 2, undefined, {
      timeout: 15_000,
    })

    // `selected` is a different channel, and setting it must not clear the
    // other: the four concepts are separate by design (plan D7).
    await post(server.info.url, 'highlight', { slots: [PROJECT_SLOT], concept: 'selected' })
    await page.waitForFunction(() => window.__kglv.selectedCount === 1, undefined, {
      timeout: 15_000,
    })
    const both = await page.evaluate(() => window.__kglv)
    expect(both.highlightedCount).toBe(2)
    expect(both.selectedCount).toBe(1)
    // A remote selection still opens the panel that describes it, or the ring
    // names a node with nothing behind it.
    await expect(page.getByTestId('selection-title')).toHaveText('Project (type)')

    // An empty list clears, rather than being ignored as a degenerate case.
    await post(server.info.url, 'highlight', { slots: [] })
    await page.waitForFunction(() => window.__kglv.highlightedCount === 0, undefined, {
      timeout: 15_000,
    })
    expect((await page.evaluate(() => window.__kglv)).selectedCount).toBe(1)
  } finally {
    server?.process.kill()
  }
})

test('appearance moves both channels and the menus that show them', async ({ page }) => {
  let server: Launched | null = null
  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)

    const before = await page.evaluate(() => window.__kglv)
    expect(before.colorBy).toBeNull()
    expect(before.sizeBy).toBeNull()

    const response = await post(server.info.url, 'appearance', {
      color_by: 'department',
      size_by: null,
    })
    expect(response.status).toBe(200)

    await page.waitForFunction(() => window.__kglv.colorBy === 'department', undefined, {
      timeout: 15_000,
    })
    const after = await page.evaluate(() => window.__kglv)
    expect(after.sizeBy).toBeNull()
    // The menu says what the graph is doing. A dropdown still reading
    // "capability" while the points are coloured by `department` is a UI lying
    // about its own state.
    await expect(page.getByTestId('color-by')).toHaveValue('department')

    // Clearing is an instruction, not an omission.
    await post(server.info.url, 'appearance', { color_by: null, size_by: null })
    await page.waitForFunction(() => window.__kglv.colorBy === null, undefined, { timeout: 15_000 })
    await expect(page.getByTestId('color-by')).toHaveValue('')
  } finally {
    server?.process.kill()
  }
})
