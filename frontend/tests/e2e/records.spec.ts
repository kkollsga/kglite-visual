import { expect, test } from '@playwright/test'

import type { GraphSliceMeta } from '../../src/generated/GraphSliceMeta'
import type { RecordTable } from '../../src/generated/RecordTable'
import { launch, Listener, type Launched } from './harness'
import { McpClient } from './mcp'

test('HTTP, WebSocket and MCP share bounded records and exact source handles', async () => {
  let server: Launched | null = null
  const listeners: Listener[] = []
  try {
    server = await launch()
    const base = server.info.url
    const post = (route: string, body: unknown) => fetch(`${base}api/${route}`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body),
    })
    for (let i = 0; i < 2; i += 1) {
      const listener = new Listener(`${base.replace(/^http/, 'ws')}ws`)
      await listener.open()
      listeners.push(listener)
    }
    const requester = listeners[0]!
    const peer = listeners[1]!
    const browse = await post('browse-type', { node_type: 'Person', limit: 3 })
    expect(browse.status).toBe(200)
    const slice = await browse.json() as { meta: GraphSliceMeta }
    expect(slice.meta.nodes).toHaveLength(3)
    const handles = slice.meta.nodes.map((node) => node.handle)
    for (const listener of listeners) {
      const update = await listener.waitFor((m) => m.kind === 'shared-update' && m.value.meta.snapshot.slice.nodes.length === 3)
      if (update.kind !== 'shared-update') throw new Error('expected slice')
      expect(update.value.meta.snapshot.slice.nodes.map((node) => node.handle)).toEqual(handles)
      listener.received.length = 0
    }
    const args = { handles, fields: ['id', 'type', 'title', 'does-not-exist'], limit: 2, offset: 0 }
    const before = await (await fetch(`${base}api/view-state`)).json()
    const response = await post('records', args)
    expect(response.status).toBe(200)
    const table = await response.json() as RecordTable
    expect(table.rows).toHaveLength(2)
    expect(table.next_offset).toBe(2)
    expect(table.rows[0]!.handle).toEqual(handles[0])
    expect(table.rows[0]!.cells[3]).toEqual({ state: 'missing' })
    requester.send({ type: 'records', ...args })
    const ws = await requester.waitFor((m) => m.kind === 'records')
    expect(ws).toEqual({ kind: 'records', value: table })

    const mcp = new McpClient(server.info.mcp)
    await mcp.initialize()
    const inspected = await mcp.call('records', args)
    expect(inspected.isError).toBe(false)
    expect(inspected.json()).toEqual(table)
    expect(await (await fetch(`${base}api/view-state`)).json()).toEqual(before)

    // A following shared mutation is the ordering barrier: private record answers
    // must not have reached the peer before the next update arrives.
    const load = await mcp.call('load_nodes', { handles: [handles[0]!] })
    expect(load.isError).toBe(false)
    await peer.waitFor((m) => m.kind === 'shared-update')
    expect(peer.received.some((m) => m.kind === 'records')).toBe(false)

    const stale = { ...handles[0]!, generation: 'a-different-session' }
    const rejected = await post('load-nodes', { handles: [stale] })
    expect(rejected.status).toBe(400)
    expect((await rejected.json()).error).toMatch(/generation|session/i)
    const refused = await mcp.call('records', { ...args, handles: [stale] })
    expect(refused.isError).toBe(true)
    requester.received.length = 0
    requester.send({ type: 'records', ...args, handles: [stale] })
    expect((await requester.waitFor((m) => m.kind === 'error')).kind).toBe('error')

    const badFields = await post('records', { ...args, fields: Array.from({ length: 33 }, (_, i) => `f${i}`) })
    expect(badFields.status).toBe(400)
    const missingType = await mcp.call('browse_type', { node_type: 'NoSuchType' })
    expect(missingType.isError).toBe(true)
  } finally {
    for (const listener of listeners) listener.close()
    server?.process.kill()
  }
})
