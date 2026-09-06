import { expect, test, type Page } from '@playwright/test'
import { appUrl, launch, fillQuery, openDestination } from './harness'
import { openDrawer } from './navigation'

async function ready(page: Page): Promise<void> { await page.waitForFunction(() => window.__kglv?.ready === true) }
async function query(page: Page, text: string, graph = false): Promise<void> {
  await fillQuery(page, text); await page.getByTestId('query-as-graph').setChecked(graph); await page.getByTestId('query-run').click()
}

test('Data scope names its actual Records or source query lane', async ({page}) => {
  const server = await launch()
  try {
    await page.goto(appUrl(server.info)); await ready(page)
    await openDestination(page, 'data')
    await expect(page.getByTestId('scope-label')).toHaveText('Data · visible records')
    await page.getByTestId('records-scope').selectOption('selected')
    await expect(page.getByTestId('scope-label')).toHaveText('Data · selected records')
    await query(page, 'RETURN 1 AS kept')
    await expect(page.getByTestId('query-table')).toBeVisible()
    await expect(page.getByTestId('scope-label')).toHaveText('Data · query results from source')
    await page.getByTestId('data-records').click()
    await expect(page.getByTestId('scope-label')).toHaveText('Data · selected records')
  } finally { server.process.kill() }
})

test('browse and peer graph queries preserve private query rows; an own graph query replaces its own result', async ({page}) => {
  const server = await launch()
  try {
    await page.goto(appUrl(server.info)); await ready(page)
    await query(page, "UNWIND [2,1] AS kept RETURN kept")
    await page.getByTestId('sort-kept').click()
    const before = await page.getByTestId('query-table').textContent()
    expect((await page.request.post(`${server.info.url}api/browse-type`, {data: {node_type: 'Person', limit: 3}})).ok()).toBe(true)
    await expect(page.getByTestId('count-loaded')).toHaveText('3')
    await openDestination(page, 'data'); await page.getByTestId('data-query').click()
    await expect(page.getByTestId('query-table')).toHaveText(before!)
    expect((await page.request.post(`${server.info.url}api/cypher`, {data: {query: 'MATCH (n:City) RETURN n LIMIT 1', as_graph: true}})).ok()).toBe(true)
    await expect(page.getByTestId('count-loaded')).toHaveText('4')
    await openDestination(page, 'data'); await page.getByTestId('data-query').click()
    await expect(page.getByTestId('query-table')).toHaveText(before!)
    await query(page, 'MATCH (n:Company) RETURN n LIMIT 1', true)
    await expect(page.getByTestId('count-loaded')).toHaveText('5')
    await openDestination(page, 'data'); await page.getByTestId('data-query').click()
    await expect(page.getByTestId('query-table')).toHaveCount(0)
    await expect(page.getByTestId('query-status')).toContainText('1 node in the result')
  } finally { server.process.kill() }
})

test('two browsers cannot consume each other’s pending acknowledgements before their own conflict', async ({page, context}) => {
  const server = await launch(); const peer = await context.newPage()
  const pending: {id: string; release(): void}[] = []
  const hold = async (target: Page): Promise<void> => target.routeWebSocket('**/ws', route => {
    const upstream = route.connectToServer()
    route.onMessage(message => {
      const request = typeof message === 'string' ? JSON.parse(message) as {type: string; request_id: string} : null
      if (request?.type === 'presentation') pending.push({id: request.request_id, release: () => upstream.send(message)})
      else upstream.send(message)
    })
  })
  try {
    await hold(page); await hold(peer)
    await page.goto(appUrl(server.info)); await ready(page); await peer.goto(appUrl(server.info)); await ready(peer)
    await openDrawer(page, 'appearance'); await openDrawer(peer, 'appearance')
    await page.getByTestId('readability-label_density').fill('0'); await page.getByTestId('readability-label_density').press('Tab')
    await peer.getByTestId('readability-edge_opacity').fill('0.2'); await peer.getByTestId('readability-edge_opacity').press('Tab')
    await expect.poll(() => pending.length).toBe(2)
    pending[0]!.release()
    await expect(page.getByTestId('readability-status')).toContainText('Shared readability applied')
    const revision = await page.locator('.kglv-root').getAttribute('data-shared-revision')
    await expect(peer.locator('.kglv-root')).toHaveAttribute('data-shared-revision', revision!)
    await expect(peer.getByTestId('readability-status')).toContainText('Applying shared readability')
    await expect(peer.getByTestId('readability-edge_opacity')).toBeDisabled()
    expect(pending[0]!.id).not.toBe(pending[1]!.id)
    pending[1]!.release()
    await expect(peer.getByTestId('readability-status')).toContainText('revision')
    await expect(peer.getByTestId('readability-edge_opacity')).toHaveValue('1')
    await expect(peer.getByTestId('readability-edge_opacity')).toBeEnabled()
    const shared = await (await page.request.get(`${server.info.url}api/view-state`)).json()
    expect(shared.presentation.edge_opacity).toBe(1)
  } finally { await peer.close(); server.process.kill() }
})

test('an older delayed graph acknowledgement cannot erase a newer scalar query result', async ({page}) => {
  const server = await launch(); let release: (() => void) | undefined
  await page.routeWebSocket('**/ws', route => {
    const upstream = route.connectToServer()
    route.onMessage(message => {
      const request = typeof message === 'string' ? JSON.parse(message) as {type: string; as_graph?: boolean} : null
      if (request?.type === 'cypher' && request.as_graph && release === undefined) release = () => upstream.send(message)
      else upstream.send(message)
    })
  })
  try {
    await page.goto(appUrl(server.info)); await ready(page)
    await query(page, 'MATCH (n:Person) RETURN n LIMIT 1', true)
    await expect.poll(() => release !== undefined).toBe(true)
    await query(page, 'RETURN 99 AS latest')
    await expect(page.getByTestId('query-table')).toContainText('99')
    release!()
    await expect(page.getByTestId('count-loaded')).toHaveText('1')
    await openDestination(page, 'data'); await page.getByTestId('data-query').click()
    await expect(page.getByTestId('query-table')).toContainText('99')
    await expect(page.getByTestId('sort-latest')).toBeVisible()
  } finally { server.process.kill() }
})
