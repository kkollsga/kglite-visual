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
