import { expect, test } from '@playwright/test'
import type { FieldDetailResponse } from '../../src/generated/FieldDetailResponse'
import type { QueryTable } from '../../src/generated/QueryTable'
import type { SearchResponse } from '../../src/generated/SearchResponse'
import { launch, Listener, type Launched } from './harness'
import { McpClient } from './mcp'

test('private field inspection and query references agree across HTTP, WS and MCP', async () => {
  let server: Launched | null = null
  const listeners: Listener[] = []
  try {
    server = await launch('crates/kglite-visual-core/tests/fixtures/viewer-identity.kgl')
    const base = server.info.url
    const post = (route: string, body: unknown) => fetch(`${base}api/${route}`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body),
    })
    for (let i = 0; i < 2; i += 1) {
      const listener = new Listener(`${base.replace(/^http/, 'ws')}ws`)
      await listener.open()
      listener.received.length = 0
      listeners.push(listener)
    }
    const requester = listeners[0]!
    const peer = listeners[1]!
    const before = await (await fetch(`${base}api/view-state`)).json()
    const queried = await post('cypher', { query: 'MATCH (n:Person) RETURN n, id(n) AS scalar_id LIMIT 1', as_graph: false })
    expect(queried.status).toBe(200)
    const table = await queried.json() as QueryTable
    expect(table.stamp).toEqual(before.stamp)
    expect(table.row_references).toHaveLength(1)
    expect(table.row_references[0]!.nodes).toHaveLength(1)
    const handle = table.row_references[0]!.nodes[0]!
    const scalar = await (await post('cypher', { query: 'RETURN 7 AS literal_id', as_graph: false })).json() as QueryTable
    expect(scalar.row_references).toEqual([{ nodes: [], relationships: [], truncated: false }])

    const args = { handle, field: 'title', path: [], offset: 0, limit: 10 }
    const response = await post('field-detail', args)
    expect(response.status).toBe(200)
    const detail = await response.json() as FieldDetailResponse
    expect(detail.handle).toEqual(handle)
    expect(detail.page?.kind).toBe('text')
    requester.send({ type: 'field-detail', ...args, request_id: 'detail-private' })
    const ws = await requester.waitFor((done) => done.kind === 'field-detail' && done.request_id === 'detail-private')
    if (ws.kind !== 'field-detail') throw new Error('expected field detail')
    const { request_id: correlation, ...wsValue } = ws.value as FieldDetailResponse & { request_id?: string }
    expect(correlation).toBe('detail-private')
    expect(wsValue).toEqual(detail)
    const mcp = new McpClient(server.info.mcp)
    await mcp.initialize()
    const read = await mcp.call('field_detail', args)
    expect(read.isError).toBe(false)
    expect(read.json()).toEqual(detail)
    const found = await (await post('search', { query: 'Ada', node_type: 'Person', limit: 2 })).json() as SearchResponse
    expect(found.stamp).toEqual(before.stamp)
    expect(found.hits.length).toBeGreaterThan(0)
    expect(found.hits.every((hit) => hit.handle?.generation === handle.generation)).toBe(true)
    expect((await (await fetch(`${base}api/view-state`)).json()).stamp).toEqual(before.stamp)

    const relationTable = await (await post('cypher', { query: 'MATCH (a:Person)-[r:KNOWS]->(b:Person) RETURN r LIMIT 3', as_graph: false })).json() as QueryTable
    const relationships = relationTable.row_references.flatMap((row) => row.relationships)
    expect(relationships).toHaveLength(3)
    expect(new Set(relationships.map((relation) => relation.edge_id)).size).toBe(3)
    expect(relationships.filter((relation) => relation.source.node_id === relation.target.node_id)).toHaveLength(1)
    const loaded = await mcp.call('load_entities', { nodes: [], relationships, expected: before.stamp, request_id: 'exact-relations' })
    expect(loaded.isError).toBe(false)
    const changed = await peer.waitFor((done) => done.kind === 'shared-update' && done.value.meta.request_id === 'exact-relations')
    if (changed.kind !== 'shared-update') throw new Error('expected shared update')
    expect(changed.value.meta.snapshot.subset.counts.loaded_edges).toBe(3)
    expect(peer.received.some((done) => ['field-detail', 'query-table', 'search'].includes(done.kind))).toBe(false)
    const mismatched = { ...relationships[0]!, edge_id: 4_000_000_000 }
    expect((await post('load-entities', { nodes: [], relationships: [mismatched] })).status).toBe(400)
    const stale = await post('field-detail', { ...args, handle: { ...handle, generation: 'foreign' } })
    expect(stale.status).toBe(400)
  } finally {
    for (const listener of listeners) listener.close()
    server?.process.kill()
  }
})
