/**
 * Client-side protocol unit tests — no browser, no server.
 *
 * The decoder's refusals are what this covers. A client that parses a frame it
 * does not understand renders a wrong picture instead of an error, and the
 * version-skew case is the one that only ever appears in the field, between a
 * wheel and a server built weeks apart (test-plan §L2).
 */

import { expect, test } from '@playwright/test'

import {
  asFloat32,
  decodeFrame,
  fnv1a,
  ProtocolError,
  ResponseAssembler,
} from '../../src/protocol'
import {
  FLAG_TERMINAL,
  HEADER_BYTES,
  MessageType,
  PROTOCOL_VERSION,
} from '../../src/generated/protocol-constants'

/** Build a frame the way the Rust encoder does. */
function frame(
  msgType: number,
  payload: Uint8Array,
  { seq = 0, terminal = true, offset = 0, version = PROTOCOL_VERSION } = {},
): ArrayBuffer {
  const padded = payload.byteLength + ((4 - (payload.byteLength % 4)) % 4)
  const buffer = new ArrayBuffer(HEADER_BYTES + padded)
  const view = new DataView(buffer)
  view.setUint32(0, version, true)
  view.setUint32(4, msgType, true)
  view.setUint32(8, seq, true)
  view.setUint32(12, terminal ? FLAG_TERMINAL : 0, true)
  view.setUint32(16, payload.byteLength, true)
  view.setUint32(20, offset, true)
  new Uint8Array(buffer, HEADER_BYTES).set(payload)
  return buffer
}

function f32Payload(values: number[]): Uint8Array {
  return new Uint8Array(Float32Array.from(values).buffer)
}

test('a version mismatch fails decode loudly', () => {
  const foreign = frame(MessageType.SESSION_INFO, new TextEncoder().encode('{}'), {
    version: PROTOCOL_VERSION + 1,
  })
  expect(() => decodeFrame(foreign)).toThrow(ProtocolError)
  expect(() => decodeFrame(foreign)).toThrow(/protocol version mismatch/)
})

test('a truncated frame is refused rather than half-read', () => {
  expect(() => decodeFrame(new ArrayBuffer(8))).toThrow(/shorter than/)

  const lying = frame(MessageType.POINTS, f32Payload([1, 2]))
  new DataView(lying).setUint32(16, 9999, true)
  expect(() => decodeFrame(lying)).toThrow(/declares a 9999-byte payload/)
})

test('an unknown message type is refused', () => {
  const assembler = new ResponseAssembler()
  expect(() => assembler.push(decodeFrame(frame(77, new Uint8Array(0))))).toThrow(
    /unknown message type 77/,
  )
})

test('a float payload decodes as an aligned zero-copy view', () => {
  const decoded = decodeFrame(frame(MessageType.POINTS, f32Payload([1.5, -2.25])))
  expect(Array.from(asFloat32(decoded))).toEqual([1.5, -2.25])
})

test('chunks are reassembled by their own offset, not by arrival order', () => {
  const assembler = new ResponseAssembler()
  assembler.push(
    decodeFrame(
      frame(MessageType.META_GRAPH_META, new TextEncoder().encode('{"tier":"full"}'), {
        terminal: false,
      }),
    ),
  )
  // Deliberately out of order: seq 2 before seq 1. The header carries the
  // offset precisely so this still lands correctly.
  assembler.push(
    decodeFrame(
      frame(MessageType.POINTS, f32Payload([3, 4]), {
        seq: 2,
        offset: 8,
        terminal: false,
      }),
    ),
  )
  const done = assembler.push(
    decodeFrame(frame(MessageType.POINTS, f32Payload([1, 2]), { seq: 1, offset: 0 })),
  )
  expect(done?.kind).toBe('meta-graph')
  if (done?.kind !== 'meta-graph') return
  expect(Array.from(done.value.points)).toEqual([1, 2, 3, 4])
})

test('fnv1a distinguishes layouts and repeats exactly', () => {
  const a = Float32Array.from([1, 2, 3, 4])
  const b = Float32Array.from([1, 2, 3, 4.5])
  expect(fnv1a(a)).toBe(fnv1a(Float32Array.from([1, 2, 3, 4])))
  expect(fnv1a(a)).not.toBe(fnv1a(b))
  expect(fnv1a(a)).toMatch(/^[0-9a-f]{8}$/)
})


test('typed record identities and missing states survive decoding without number coercion', () => {
  const assembler = new ResponseAssembler()
  const table = {
    generation: 'test-generation',
    rows: [{
      handle: { generation: 'test-generation', node_id: 7 },
      slot: null,
      cells: [
        { state: 'value', value: { type: 'int64', value: '9007199254740993' } },
        { state: 'value', value: { type: 'boolean', value: false } },
        { state: 'value', value: { type: 'string', value: '' } },
        { state: 'null' }, { state: 'missing' },
        { state: 'unavailable', reason: 'source field unavailable' },
        { state: 'truncated', preview: '1000 items', reason: 'collection limit' },
      ],
    }],
  }
  const decoded = assembler.push(decodeFrame(frame(MessageType.RECORDS,
    new TextEncoder().encode(JSON.stringify(table)))))
  expect(decoded).toEqual({ kind: 'records', value: table })
  const next = assembler.push(decodeFrame(frame(MessageType.QUERY_TABLE,
    new TextEncoder().encode('{"columns":["count"],"rows":[[1]]}'))))
  expect(next?.kind).toBe('query-table')
})

test('a shared update becomes visible only after its complete typed arrays arrive', () => {
  const assembler = new ResponseAssembler()
  const meta = {
    snapshot: { stamp: { generation: 'fixture-generation', revision: '9' }, topology_revision: '2' },
    request_id: 'change-9', focus: null, mutation_kind: 'query', compacted: false,
  }
  expect(assembler.push(decodeFrame(frame(MessageType.SHARED_UPDATE,
    new TextEncoder().encode(JSON.stringify(meta)), { terminal: false })))).toBeNull()
  expect(assembler.push(decodeFrame(frame(MessageType.POINTS, f32Payload([1, 2]),
    { terminal: false, seq: 1 })))).toBeNull()
  const completed = assembler.push(decodeFrame(frame(MessageType.LINKS, f32Payload([]), { seq: 2 })))
  expect(completed?.kind).toBe('shared-update')
  if (completed?.kind !== 'shared-update') throw new Error('expected atomic shared update')
  expect(completed.request_id).toBe('change-9')
  expect(completed.value.meta).toEqual(meta)
  expect([...completed.value.points]).toEqual([1, 2])
  expect([...completed.value.links]).toEqual([])

  const privateReply = assembler.push(decodeFrame(frame(MessageType.QUERY_TABLE,
    new TextEncoder().encode('{"request_id":"query-10","columns":[],"rows":[]}'))))
  expect(privateReply?.kind).toBe('query-table')
  expect(privateReply?.request_id).toBe('query-10')
})

test('revision conflicts retain expected and actual stamps and request correlation', () => {
  const payload = {
    code: 'revision-conflict', message: 'shared view changed', request_id: 'stale-change',
    expected: { generation: 'fixture-generation', revision: '2' },
    actual: { generation: 'fixture-generation', revision: '3' },
  }
  const reply = new ResponseAssembler().push(decodeFrame(frame(MessageType.ERROR,
    new TextEncoder().encode(JSON.stringify(payload)))))
  expect(reply?.kind).toBe('error')
  if (reply?.kind !== 'error') throw new Error('expected conflict error')
  expect(reply.request_id).toBe('stale-change')
  expect(reply.conflict?.expected.revision).toBe('2')
  expect(reply.conflict?.actual.revision).toBe('3')
})


test('field pages and query provenance retain source identity and independent correlation', () => {
  const assembler = new ResponseAssembler()
  const handle = { generation: 'fixture-generation', node_id: 7 }
  const detail = {
    stamp: { generation: handle.generation, revision: '4' }, subset_revision: '2',
    handle, field: 'metadata', path: [{ kind: 'key', key: 'children' }],
    cell: { state: 'value', value: { type: 'string', value: 'preview' } },
    page: { kind: 'list', offset: 0, total_items: 2, items: [{ state: 'missing' }, { state: 'null' }], next_offset: null },
    request_id: 'field-1',
  }
  const reply = assembler.push(decodeFrame(frame(MessageType.FIELD_DETAIL,
    new TextEncoder().encode(JSON.stringify(detail)))))
  expect(reply?.kind).toBe('field-detail')
  expect(reply?.request_id).toBe('field-1')
  if (reply?.kind !== 'field-detail') throw new Error('expected field detail')
  expect(reply.value.handle).toEqual(handle)
  expect(reply.value.page).toEqual(detail.page)

  const table = { request_id: 'query-2', columns: ['n', 'literal_id'], rows: [[{}, 7]],
    row_references: [{ nodes: [handle], relationships: [], truncated: false }] }
  const query = assembler.push(decodeFrame(frame(MessageType.QUERY_TABLE,
    new TextEncoder().encode(JSON.stringify(table)))))
  expect(query?.kind).toBe('query-table')
  expect(query?.request_id).toBe('query-2')
  if (query?.kind !== 'query-table') throw new Error('expected query')
  expect(query.value.row_references).toEqual(table.row_references)
})
