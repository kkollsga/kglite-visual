import { expect, test } from '@playwright/test'

import type { GraphSliceMeta } from '../../src/generated/GraphSliceMeta'
import type { RevisionStamp } from '../../src/generated/RevisionStamp'
import type { SharedSnapshotMeta } from '../../src/generated/SharedSnapshotMeta'
import type { SharedUpdateMessage } from '../../src/protocol'
import { launch, Listener, type Launched } from './harness'
import { McpClient } from './mcp'

test('HTTP, WS and MCP acknowledge one subset and reconnect restores the same shared state', async () => {
  let server: Launched | null = null
  const listeners: Listener[] = []
  try {
    server = await launch()
    const base = server.info.url
    const post = (route: string, body: unknown) => fetch(`${base}api/${route}`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body),
    })
    const mcp = new McpClient(server.info.mcp)
    await mcp.initialize()
    for (let i = 0; i < 2; i += 1) {
      const listener = new Listener(`${base.replace(/^http/, 'ws')}ws`)
      await listener.open()
      listeners.push(listener)
    }
    const requester = listeners[0]!
    const peer = listeners[1]!
    const waitUpdate = async (listener: Listener, requestId: string): Promise<SharedUpdateMessage> => {
      const update = await listener.waitFor((done) => done.kind === 'shared-update' && done.value.meta.request_id === requestId)
      if (update.kind !== 'shared-update') throw new Error('expected shared update')
      return update.value
    }
    const opening = await (await fetch(`${base}api/view-state`)).json() as { stamp: RevisionStamp }
    const browse = await post('browse-type', { node_type: 'Person', limit: 4, expected: opening.stamp, request_id: 'browse' })
    expect(browse.status).toBe(200)
    const loaded = await browse.json() as { meta: GraphSliceMeta; stamp: RevisionStamp }
    expect(loaded.meta.kind).toBe('query')
    expect(loaded.meta.nodes).toHaveLength(4)
    const first = loaded.meta.nodes[0]!
    await waitUpdate(peer, 'browse')

    const filtered = await mcp.call('set_subset', {
      expected: loaded.stamp, request_id: 'filter', predicates: [{
        id: 'chosen-title', enabled: true,
        predicate: { kind: 'category', field: { kind: 'property', name: 'title' }, values: [{ type: 'string', value: first.title }] },
      }],
    })
    expect(filtered.isError).toBe(false)
    const state = filtered.json<{ state: SharedSnapshotMeta }>().state
    expect(state.subset.counts.loaded_nodes).toBe(4)
    expect(state.subset.counts.visible_nodes).toBe(1)
    const a = await waitUpdate(requester, 'filter')
    const b = await waitUpdate(peer, 'filter')
    expect(a.meta.snapshot).toEqual(b.meta.snapshot)
    expect(a.meta.snapshot.subset.visible_nodes).toEqual([first.handle])
    const topologyRevision = state.topology_revision

    const appearance = await post('appearance', { color_by: 'type', size_by: null, expected: state.stamp, request_id: 'appearance' })
    expect(appearance.status).toBe(200)
    const appearanceAck = await appearance.json() as { stamp: RevisionStamp }
    requester.send({ type: 'caption', caption_by: 'id', expected: appearanceAck.stamp, request_id: 'caption' })
    const caption = await waitUpdate(peer, 'caption')
    expect(caption.meta.snapshot.caption_by).toBe('id')
    expect(caption.meta.snapshot.appearance.color_by).toBe('type')
    expect(caption.meta.snapshot.topology_revision).toBe(topologyRevision)
    expect(caption.meta.snapshot.subset_revision).toBe(state.subset_revision)

    const selected = await mcp.call('highlight', { slots: [first.slot], concept: 'selected', expected: caption.meta.snapshot.stamp, request_id: 'select' })
    expect(selected.isError).toBe(false)
    const selectedState = await waitUpdate(peer, 'select')
    requester.send({ type: 'focus', slots: [first.slot], expected: selectedState.meta.snapshot.stamp, request_id: 'focus' })
    const focused = await waitUpdate(peer, 'focus')
    expect(focused.meta.focus?.slots).toEqual([first.slot])

    const layout = await post('layout', { kernel: 'force', seed_slot: null, expected: focused.meta.snapshot.stamp, request_id: 'layout' })
    expect(layout.status).toBe(200)
    const arranged = await waitUpdate(peer, 'layout')
    expect(arranged.meta.snapshot.layout_kernel).toBe('force')
    const more = await post('browse-type', { node_type: 'Company', limit: 1, expected: arranged.meta.snapshot.stamp, request_id: 'more' })
    expect(more.status).toBe(200)
    const latest = await waitUpdate(peer, 'more')
    expect(latest.meta.snapshot.layout_kernel).toBe('simulation')
    expect(latest.meta.snapshot.layout).toBeNull()
    expect(latest.meta.snapshot.subset.counts.loaded_nodes).toBe(5)

    const late = new Listener(`${base.replace(/^http/, 'ws')}ws`)
    await late.open()
    listeners.push(late)
    const greeting = await late.waitFor((done) => done.kind === 'shared-update')
    if (greeting.kind !== 'shared-update') throw new Error('expected snapshot')
    expect(greeting.value.meta.snapshot).toEqual(latest.meta.snapshot)
    expect(greeting.value.meta.focus).toBeNull()
    expect(greeting.value.points.length).toBe(latest.points.length)

    const conflict = await post('reset', { expected: loaded.stamp, request_id: 'stale-http' })
    expect(conflict.status).toBe(409)
    expect((await conflict.json()).code).toBe('revision-conflict')
    const mcpConflict = await mcp.call('set_caption', { caption_by: 'title', expected: loaded.stamp })
    expect(mcpConflict.isError).toBe(true)
    expect(mcpConflict.json().code).toBe('revision-conflict')
    requester.send({ type: 'reset', expected: loaded.stamp, request_id: 'stale-ws' })
    const wsConflict = await requester.waitFor((done) => done.kind === 'error' && done.request_id === 'stale-ws')
    if (wsConflict.kind !== 'error') throw new Error('expected conflict')
    expect(wsConflict.conflict?.actual).toEqual(latest.meta.snapshot.stamp)
    expect((await (await fetch(`${base}api/view-state`)).json()).stamp).toEqual(latest.meta.snapshot.stamp)

    peer.received.length = 0
    requester.send({ type: 'records', handles: loaded.meta.nodes.map((node) => node.handle), fields: ['title'], offset: 0, limit: 100, request_id: 'records-private' })
    const records = await requester.waitFor((done) => done.kind === 'records' && done.request_id === 'records-private')
    if (records.kind !== 'records') throw new Error('expected records')
    expect(records.value.stamp).toEqual(latest.meta.snapshot.stamp)
    expect(records.value.rows.filter((row) => row.visible).map((row) => row.handle)).toEqual([first.handle])
    const barrier = await post('caption', { caption_by: 'id', request_id: 'barrier' })
    expect(barrier.status).toBe(200)
    await waitUpdate(peer, 'barrier')
    expect(peer.received.some((done) => done.kind === 'records')).toBe(false)
  } finally {
    for (const listener of listeners) listener.close()
    server?.process.kill()
  }
})
