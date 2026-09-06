/**
 * L3: the acceptance scenario for P10, in the user's own words — *the agent
 * will show and help the user navigate the graph*.
 *
 * One real browser holding the view, one MCP client driving it over streamable
 * HTTP at the URL the launch contract printed, and `window.__kglv` proving
 * after every step that the picture the human is looking at followed. Nothing
 * in the page is clicked; the only inputs are MCP tool calls.
 *
 * This is deliberately one long test rather than five short ones. The claim
 * being made is that a *sequence* works — show, point, zoom, put back — and a
 * suite that proved each verb in isolation would not have proved the thing the
 * phase is for.
 */

import { keepSchemaContext } from './navigation'

import { expect, test, type Page } from '@playwright/test'

import { appUrl, launch, Listener, type Launched } from './harness'
import { McpClient } from './mcp'

const PERSON_SLOT = 0
const META_POINTS = 5
/**
 * The query below names one company explicitly, so the node set it loads is
 * exact rather than "whatever LIMIT 3 happened to reach": three Persons who
 * work at Company_3, plus Company_3 itself.
 */
const PERSONS_SHOWN = 3
const SHOWN = PERSONS_SHOWN + 1

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__kglv?.ready === true, undefined, { timeout: 30_000 })
}

function state(page: Page) {
  return page.evaluate(() => window.__kglv)
}

test('MCP protocol: initialize, list_tools, call_tool over streamable HTTP', async () => {
  let server: Launched | null = null
  try {
    server = await launch()
    // The launch contract IS the attach step (D14): no discovery file, no
    // second process, just a key on the line the server already printed.
    expect(server.info.mcp).toBe(`http://127.0.0.1:${server.info.port}/mcp`)

    const mcp = new McpClient(server.info.mcp)
    const info = await mcp.initialize()
    expect(info.serverInfo.name).toBe('kglite-visual')
    expect(info.capabilities).toHaveProperty('tools')
    // The instructions string is part of the product, not decoration: it is
    // what stops an agent treating a shared screen as a scratchpad. Three of
    // its four load-bearing claims, asserted where they actually ship.
    expect(info.instructions).toContain('human being is looking at')
    expect(info.instructions).toContain('Shared changes are ordered and acknowledged')
    expect(info.instructions).toContain('Pass `expected` from `view_state`')
    expect(info.instructions).toContain('A revision conflict changes nothing')
    expect(info.instructions).toContain('stale prepared work always refuses')
    // The geometry rule is CONDITIONAL since G3 (plan E5): an agent that read
    // only "you cannot know geometry" would never reach for `set_layout`, and
    // the caveat it must not lose is the one naming the condition.
    expect(info.instructions).toContain('depends on `view_state.layout_kernel`')

    const tools = await mcp.listTools()
    expect(tools.map((tool) => tool.name).sort()).toEqual([
      'browse_type',
      'calculate',
      'collapse',
      'delete_view',
      'expand',
      'export_view',
      'field_detail',
      'focus',
      'highlight',
      'list_saved_queries',
      'list_views',
      'load_entities',
      'load_nodes',
      'records',
      'render',
      'reset_view',
      'restore_history',
      'restore_view',
      'run_saved_query',
      'save_view',
      'set_appearance',
      'set_caption',
      'set_layout',
      'set_subset',
      'show_cypher',
      'view_history',
      'view_state',
    ])
    for (const tool of tools) {
      expect(tool.description, `${tool.name} has no description`).toBeTruthy()
      expect(tool.inputSchema.type, `${tool.name} has no object schema`).toBe('object')
    }

    // call_tool, and the honest-diagnostics rule with it: a refusal is a tool
    // error carrying kglite's own message, never a protocol error that clients
    // render as "tool result missing".
    const broken = await mcp.call('show_cypher', { query: 'MATCH (n:Nope RETURN n' })
    expect(broken.isError).toBe(true)
    expect(broken.text).toContain('Cypher syntax error')

    const bounds = await mcp.call('focus', { slots: [4000] })
    expect(bounds.isError).toBe(true)
    expect(bounds.text).toContain('slot 4000 is not in this view')
    expect(bounds.text).toContain(`holds slots 0..${META_POINTS}`)

    // ── one store, two faces ─────────────────────────────────────────────
    // Saved over the JSON twin, read back over MCP. The two are not two
    // stores agreeing: they are one file, reached through the same AppState,
    // which is why `show()` gets the same one without arranging anything.
    const saved = await fetch(`${server.info.url}api/queries/save`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ name: 'people', query: 'MATCH (p:Person) RETURN p LIMIT 2' }),
    })
    expect(saved.status).toBe(200)

    const listed = await mcp.call('list_saved_queries')
    expect(listed.isError).toBe(false)
    expect(listed.json<{ saved: { name: string }[] }>().saved.map((q) => q.name)).toEqual([
      'people',
    ])

    // Running it goes through the ordinary Cypher path, so the answer is an
    // ordinary slice report.
    const ran = await mcp.call('run_saved_query', { name: 'people' })
    expect(ran.isError).toBe(false)
    expect(ran.json<{ added: unknown[] }>().added).toHaveLength(2)

    // ...and the run is in the recent list, which is the same store again.
    const after = await mcp.call('list_saved_queries')
    expect(after.json<{ recent: { query: string }[] }>().recent[0]?.query).toBe(
      'MATCH (p:Person) RETURN p LIMIT 2',
    )

    const missing = await mcp.call('run_saved_query', { name: 'nope' })
    expect(missing.isError).toBe(true)
    expect(missing.text).toContain('no saved query named')
  } finally {
    server?.process.kill()
  }
})

test('guided navigation: an agent shows the user a slice, points at it, and puts it back', async ({
  page,
}, testInfo) => {
  let server: Launched | null = null
  const consoleErrors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })

  try {
    server = await launch()
    await page.goto(appUrl(server.info))
    await ready(page)
    await keepSchemaContext(page)

    const mcp = new McpClient(server.info.mcp)
    await mcp.initialize()

    // ── the agent reads the screen before it touches it ──────────────────
    const before = await mcp.call('view_state')
    const view = before.json<{
      slot_count: number
      live_count: number
      connected_viewers: number
      types: { name: string; slot: number; instances_on_screen: number }[]
      geometry_caveat: string
      last_slice: unknown
    }>()
    expect(view.slot_count).toBe(META_POINTS)
    // Somebody IS watching. An agent steering an unwatched view is talking to
    // itself, and this is the only field that can tell it apart.
    expect(view.connected_viewers).toBe(1)
    expect(view.types.map((type) => type.name)).toContain('Person')
    expect(view.geometry_caveat).toContain('geometry-different')
    expect(view.last_slice).toBeNull()
    // The agent's view of the content matches the browser's, exactly. That is
    // the `view_state` contract: same truth, two readers.
    const browserBefore = await state(page)
    expect(view.slot_count).toBe(browserBefore.slotCount)
    expect(view.live_count).toBe(browserBefore.pointCount)

    // ── show: a Cypher result arrives on the human's screen ──────────────
    const shown = await mcp.call('show_cypher', {
      query:
        "MATCH (p:Person)-[:WORKS_AT]->(c:Company) WHERE c.title = 'Company_3' " +
        'RETURN p, c LIMIT 3',
    })
    expect(shown.isError).toBe(false)
    const slice = shown.json<{
      kind: string
      added: { slot: number; type: string; title: string }[]
      slot_count: number
      connected_viewers: number
    }>()
    expect(slice.kind).toBe('query')
    expect(slice.added.length).toBe(SHOWN)
    expect(slice.slot_count).toBe(META_POINTS + SHOWN)

    await page.waitForFunction(
      (expected) => window.__kglv.slotCount === expected,
      META_POINTS + SHOWN,
      { timeout: 15_000 },
    )
    const arrived = await state(page)
    expect(arrived.lastSliceKind).toBe('query')
    expect(arrived.pointCount).toBe(META_POINTS + SHOWN)
    // Drawn, not merely counted: the nodes the query named are labelled on
    // screen by their own titles.
    await expect(page.locator('.kglv-label:has-text("Person_")').first()).toBeVisible()

    // ── point: highlight what the agent is talking about ─────────────────
    const targets = slice.added.map((node) => node.slot)
    const highlighted = await mcp.call('highlight', { slots: targets })
    expect(highlighted.isError).toBe(false)
    expect(highlighted.json<{ marked: number }>().marked).toBe(SHOWN)
    await page.waitForFunction(
      (expected) => window.__kglv.highlightedCount === expected,
      SHOWN,
      { timeout: 15_000 },
    )

    // ── zoom: the honest way to say "look at this" ───────────────────────
    const zoomBefore = (await state(page)).zoomLevel
    const focused = await mcp.call('focus', { slots: targets })
    expect(focused.isError).toBe(false)
    expect(focused.json<{ framed: number }>().framed).toBe(SHOWN)
    await page.waitForFunction((zoom) => window.__kglv.zoomLevel !== zoom, zoomBefore, {
      timeout: 15_000,
    })
    const pointedAt = await state(page)
    expect(pointedAt.zoomLevel).not.toBe(zoomBefore)
    expect(pointedAt.highlightedCount).toBe(SHOWN)
    // Pointing changed no content. That separation is the whole reason focus
    // and highlight are their own messages.
    expect(pointedAt.slotCount).toBe(META_POINTS + SHOWN)

    // ── render: the agent looks at what it built ─────────────────────────
    const rendered = await mcp.call('render', {
      target: 'live-view',
      format: 'png',
      width: 900,
      height: 560,
    })
    expect(rendered.isError).toBe(false)
    const picture = rendered.json<{ nodes: number; links: number; geometry_caveat: string }>()
    // The image is OF the live view: every live slot, type nodes included,
    // which is what makes it a picture of the human's screen rather than of a
    // query the human never saw.
    expect(picture.nodes).toBe(META_POINTS + SHOWN)
    expect(picture.geometry_caveat).toContain('geometry-different')
    expect(rendered.images).toHaveLength(1)
    expect(rendered.images[0]?.mimeType).toBe('image/png')
    expect(rendered.images[0]?.base64.length).toBeGreaterThan(1000)

    // ── put it back ──────────────────────────────────────────────────────
    const collapsed = await mcp.call('collapse', { slot: PERSON_SLOT })
    expect(collapsed.isError).toBe(false)
    await page.waitForFunction(() => window.__kglv.lastSliceKind === 'collapse', undefined, {
      timeout: 15_000,
    })
    const afterCollapse = await state(page)
    // The three Persons went; Company_3 stays, because `collapse` on a type
    // slot collapses that type and no other.
    expect(afterCollapse.pointCount).toBe(META_POINTS + SHOWN - PERSONS_SHOWN)

    const reset = await mcp.call('reset_view')
    expect(reset.isError).toBe(false)
    await page.waitForFunction(
      (expected) => window.__kglv.pointCount === expected,
      META_POINTS,
      { timeout: 15_000 },
    )
    const restored = await state(page)
    expect(restored.pointCount).toBe(META_POINTS)
    expect(restored.linkCount).toBe(7)
    // Tombstones, not a re-index. Nine slots is well under the compaction
    // minimum, so the four instance slots stay allocated and dead — which is
    // the honest outcome: the remap would cost more than the waste, and every
    // slot the agent is still holding keeps meaning what it meant.
    expect(restored.slotCount).toBe(META_POINTS + SHOWN)
    expect(restored.tombstoneCount).toBe(SHOWN)
    expect(restored.compactions).toBe(0)

    // And the agent's own reading agrees with the screen again.
    const finalState = (await mcp.call('view_state')).json<{
      slot_count: number
      live_count: number
      tombstone_count: number
      instances_by_type: [string, number][]
    }>()
    expect(finalState.live_count).toBe(META_POINTS)
    expect(finalState.slot_count).toBe(META_POINTS + SHOWN)
    expect(finalState.tombstone_count).toBe(SHOWN)
    expect(finalState.instances_by_type).toEqual([])

    const shotPath = testInfo.outputPath('guided-navigation.png')
    await page.screenshot({ path: shotPath })
    await testInfo.attach('guided-navigation.png', { path: shotPath, contentType: 'image/png' })

    expect(consoleErrors, `browser console errors: ${consoleErrors.join(' | ')}`).toEqual([])
  } finally {
    server?.process.kill()
  }
})

test('saved views and recovery share the HTTP/MCP catalog and revision contract', async ({ page }) => {
  const server = await launch()
  try {
    const mcp = new McpClient(server.info.mcp)
    await mcp.initialize()
    const post = (route: string, data: unknown) => fetch(`${server.info.url}api/${route}`, {
      method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify(data),
    })
    await page.goto(appUrl(server.info))
    await ready(page)
    expect((await mcp.call('browse_type', {node_type: 'Person', limit: 100})).isError).toBe(false)
    await expect(page.getByTestId('count-loaded')).toHaveText('60')
    const name = `mcp-view-${Date.now()}`
    const captureStamp = (await (await fetch(`${server.info.url}api/view-state`)).json()).stamp
    const saved = await mcp.call('save_view', {name, expected: captureStamp, request_id: 'save-mcp'})
    expect(saved.isError).toBe(false)
    const savedValue = saved.json<{saved: {storage: string; name: string}; marker_applied: boolean; dirty: boolean}>()
    expect(savedValue.saved).toMatchObject({storage: 'durable', name})
    expect(savedValue.marker_applied).toBe(true)
    expect(savedValue.dirty).toBe(false)
    const catalog = await (await fetch(`${server.info.url}api/views`)).json()
    expect(catalog.views).toContainEqual(expect.objectContaining(savedValue.saved))
    expect(catalog.views.every((entry: object) => !('bookmark' in entry))).toBe(true)
    expect((await mcp.call('save_view', {name})).isError).toBe(true)
    expect((await mcp.call('reset_view')).isError).toBe(false)
    await expect(page.getByTestId('count-loaded')).toHaveText('0')
    await expect(page.getByTestId('scope-schema')).toHaveAttribute('aria-pressed', 'true')
    expect((await post('caption', {caption_by: 'id'})).ok).toBe(true)
    const changedStamp = (await (await fetch(`${server.info.url}api/view-state`)).json()).stamp
    const refused = await post('views/restore', {...savedValue.saved, expected: captureStamp})
    expect(refused.status).toBe(409)
    expect((await refused.json()).code).toBe('revision-conflict')
    const restored = await mcp.call('restore_view', {...savedValue.saved, expected: changedStamp})
    expect(restored.isError).toBe(false)
    await expect(page.getByTestId('count-loaded')).toHaveText('60')
    await expect(page.getByTestId('scope-instances')).toHaveAttribute('aria-pressed', 'true')
    await expect.poll(async () => (await state(page)).pointCount).toBe(60)
    const history = (await mcp.call('view_history')).json<{history: {entries: {id: string}[]}}>()
    const checkpoint = history.history.entries.at(-1)!
    const current = (await (await fetch(`${server.info.url}api/view-state`)).json()).stamp
    expect((await mcp.call('restore_history', {id: checkpoint.id, expected: current})).isError).toBe(false)
    const restoredState = new Listener(`${server.info.url.replace(/^http/, 'ws')}ws`)
    try {
      await restoredState.open()
      const snapshot = await restoredState.waitFor(done => done.kind === 'shared-update')
      if (snapshot.kind !== 'shared-update') throw new Error('expected shared snapshot')
      expect(snapshot.value.meta.snapshot.caption_by).toBe('id')
    } finally { restoredState.close() }
    expect((await mcp.call('delete_view', savedValue.saved)).isError).toBe(false)
    const listed = (await mcp.call('list_views')).json<{views: {name: string}[]}>()
    expect(listed.views.some(entry => entry.name === name)).toBe(false)
  } finally { server.process.kill() }
})
