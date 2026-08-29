/**
 * Decoding the binary protocol (plan D4).
 *
 * The constants and message-type codes are GENERATED from the Rust enum
 * (`./generated/protocol-constants`), so a wire change that is not mirrored
 * here fails the gate rather than the browser. Nothing in this file may
 * hard-code a number that lives in that module.
 */

import {
  FLAG_TERMINAL,
  HEADER_BYTES,
  MessageType,
  PROTOCOL_VERSION,
} from './generated/protocol-constants'
import type { Compaction } from './generated/Compaction'
import type { ExpansionPreview } from './generated/ExpansionPreview'
import type { GraphSliceMeta } from './generated/GraphSliceMeta'
import type { MetaGraphMeta } from './generated/MetaGraphMeta'
import type { NodeDetail } from './generated/NodeDetail'
import type { PropertyStatsResponse } from './generated/PropertyStatsResponse'
import type { QueryTable } from './generated/QueryTable'
import type { SearchResponse } from './generated/SearchResponse'
import type { SessionInfo } from './generated/SessionInfo'

/** One decoded frame. `payload` is a view *into* the received buffer. */
export type Frame = {
  msgType: number
  seq: number
  terminal: boolean
  payloadOffset: number
  payload: Uint8Array
  /** The frame's own buffer, kept so an aligned typed-array view is possible. */
  buffer: ArrayBuffer
  payloadLength: number
}

export class ProtocolError extends Error {}

/**
 * Assert the host is little-endian, once at startup.
 *
 * Every multi-byte field on the wire is LE and every typed-array view here
 * reads in *host* order. On a big-endian host that mismatch does not throw —
 * it silently yields transposed floats, which look like a layout bug. No
 * mainstream browser runs big-endian today, which is exactly why the failure
 * would go unexplained if it ever appeared.
 */
export function assertLittleEndian(): void {
  const probe = new Uint8Array(new Uint32Array([1]).buffer)
  if (probe[0] !== 1) {
    throw new ProtocolError(
      'this host is big-endian; the kglite-visual wire format is little-endian',
    )
  }
}

export function decodeFrame(buffer: ArrayBuffer): Frame {
  if (buffer.byteLength < HEADER_BYTES) {
    throw new ProtocolError(
      `frame is ${buffer.byteLength} bytes, shorter than the ${HEADER_BYTES}-byte header`,
    )
  }
  const header = new DataView(buffer, 0, HEADER_BYTES)
  // Word 0 first, before anything else is interpreted: a peer speaking another
  // wire format must be reported as a version mismatch, not as whatever the
  // re-ordered words happen to decode to.
  const version = header.getUint32(0, true)
  if (version !== PROTOCOL_VERSION) {
    throw new ProtocolError(
      `protocol version mismatch: this client speaks v${PROTOCOL_VERSION}, ` +
        `the server sent v${version}`,
    )
  }

  const msgType = header.getUint32(4, true)
  const seq = header.getUint32(8, true)
  const flags = header.getUint32(12, true)
  const payloadLength = header.getUint32(16, true)
  const payloadOffset = header.getUint32(20, true)

  if (HEADER_BYTES + payloadLength > buffer.byteLength) {
    throw new ProtocolError(
      `frame declares a ${payloadLength}-byte payload but carries ` +
        `${buffer.byteLength - HEADER_BYTES}`,
    )
  }

  return {
    msgType,
    seq,
    terminal: (flags & FLAG_TERMINAL) !== 0,
    payloadOffset,
    payload: new Uint8Array(buffer, HEADER_BYTES, payloadLength),
    buffer,
    payloadLength,
  }
}

/**
 * A frame's payload as a `Float32Array`.
 *
 * A zero-copy view, which is only legal because the header is 24 bytes and
 * every frame is padded to a multiple of 4: `Float32Array` throws outright on
 * an unaligned `byteOffset` rather than copying.
 */
export function asFloat32(frame: Frame): Float32Array {
  return new Float32Array(frame.buffer, HEADER_BYTES, frame.payloadLength / 4)
}

function asJson<T>(frame: Frame): T {
  return JSON.parse(new TextDecoder().decode(frame.payload)) as T
}

/** A fully-received meta-graph response. */
export type MetaGraphMessage = {
  meta: MetaGraphMeta
  points: Float32Array
  links: Float32Array
}

/** A fully-received graph slice: an expansion, a collapse, or query results. */
export type GraphSliceMessage = {
  meta: GraphSliceMeta
  /** Present only when the server reclaimed tombstones this round. */
  compaction: Compaction | null
  points: Float32Array
  links: Float32Array
}

/** Everything a completed response can be. */
export type Completed =
  | { kind: 'meta-graph'; value: MetaGraphMessage }
  | { kind: 'session'; value: SessionInfo }
  | { kind: 'slice'; value: GraphSliceMessage }
  | { kind: 'query-table'; value: QueryTable }
  | { kind: 'preview'; value: ExpansionPreview }
  | { kind: 'node-detail'; value: NodeDetail }
  | { kind: 'search'; value: SearchResponse }
  | { kind: 'property-stats'; value: PropertyStatsResponse }
  | { kind: 'error'; value: string }

/**
 * Reassembles one response from its frames.
 *
 * Chunks are placed by their own `payloadOffset`, not by arrival order — the
 * header carries the offset precisely so a decoder never has to trust
 * sequencing it cannot verify.
 */
export class ResponseAssembler {
  private meta: MetaGraphMeta | null = null
  private session: SessionInfo | null = null
  private error: string | null = null
  private slice: GraphSliceMeta | null = null
  private compaction: Compaction | null = null
  private table: QueryTable | null = null
  private preview: ExpansionPreview | null = null
  private detail: NodeDetail | null = null
  private search: SearchResponse | null = null
  private stats: PropertyStatsResponse | null = null
  private readonly chunks = new Map<number, { offset: number; bytes: Uint8Array }[]>()
  lastSeq = -1

  /** Feed one frame. Returns the completed response, if this frame ended one. */
  push(frame: Frame): Completed | null {
    this.lastSeq = frame.seq

    switch (frame.msgType) {
      case MessageType.META_GRAPH_META:
        this.meta = asJson<MetaGraphMeta>(frame)
        break
      case MessageType.SESSION_INFO:
        this.session = asJson<SessionInfo>(frame)
        break
      case MessageType.ERROR:
        this.error = asJson<{ message: string }>(frame).message
        break
      case MessageType.GRAPH_SLICE:
        this.slice = asJson<GraphSliceMeta>(frame)
        break
      case MessageType.COMPACTION:
        this.compaction = asJson<Compaction>(frame)
        break
      case MessageType.QUERY_TABLE:
        this.table = asJson<QueryTable>(frame)
        break
      case MessageType.EXPANSION_PREVIEW:
        this.preview = asJson<ExpansionPreview>(frame)
        break
      case MessageType.NODE_DETAIL:
        this.detail = asJson<NodeDetail>(frame)
        break
      case MessageType.SEARCH_RESULT:
        this.search = asJson<SearchResponse>(frame)
        break
      case MessageType.PROPERTY_STATS:
        this.stats = asJson<PropertyStatsResponse>(frame)
        break
      case MessageType.POINTS:
      case MessageType.LINKS: {
        const bucket = this.chunks.get(frame.msgType) ?? []
        // A copy, not the view: the frame's buffer is the socket's, and the
        // next message may reuse it. The offset travels with it — see
        // `joinFloats`.
        bucket.push({ offset: frame.payloadOffset, bytes: new Uint8Array(frame.payload) })
        this.chunks.set(frame.msgType, bucket)
        break
      }
      default:
        throw new ProtocolError(`unknown message type ${frame.msgType}`)
    }

    if (!frame.terminal) return null
    return this.complete()
  }

  private complete(): Completed {
    const finished = this.classify()
    this.meta = null
    this.session = null
    this.error = null
    this.slice = null
    this.compaction = null
    this.table = null
    this.preview = null
    this.detail = null
    this.search = null
    this.stats = null
    this.chunks.clear()
    return finished
  }

  /**
   * Decide what the frames that just arrived add up to.
   *
   * Error first, deliberately: a response that carries a failure *and* a
   * partial payload is a failure, and rendering the payload would show the user
   * a view built from an answer the server did not stand behind.
   */
  private classify(): Completed {
    if (this.error !== null) return { kind: 'error', value: this.error }
    if (this.meta !== null) {
      return {
        kind: 'meta-graph',
        value: {
          meta: this.meta,
          points: this.joinFloats(MessageType.POINTS),
          links: this.joinFloats(MessageType.LINKS),
        },
      }
    }
    if (this.slice !== null) {
      return {
        kind: 'slice',
        value: {
          meta: this.slice,
          compaction: this.compaction,
          points: this.joinFloats(MessageType.POINTS),
          links: this.joinFloats(MessageType.LINKS),
        },
      }
    }
    if (this.table !== null) return { kind: 'query-table', value: this.table }
    if (this.preview !== null) return { kind: 'preview', value: this.preview }
    if (this.detail !== null) return { kind: 'node-detail', value: this.detail }
    if (this.search !== null) return { kind: 'search', value: this.search }
    if (this.stats !== null) return { kind: 'property-stats', value: this.stats }
    if (this.session !== null) return { kind: 'session', value: this.session }
    throw new ProtocolError('a response ended without carrying anything')
  }

  /**
   * Place each chunk at the byte offset its own header declared.
   *
   * Not concatenation in arrival order: the offset word exists precisely so a
   * decoder never has to trust sequencing it cannot verify, and a reassembler
   * that appends instead silently transposes a response whose frames arrive
   * out of order.
   */
  private joinFloats(msgType: number): Float32Array {
    const bucket = this.chunks.get(msgType) ?? []
    const bytes = bucket.reduce(
      (total, chunk) => Math.max(total, chunk.offset + chunk.bytes.byteLength),
      0,
    )
    const joined = new Uint8Array(bytes)
    for (const chunk of bucket) joined.set(chunk.bytes, chunk.offset)
    return new Float32Array(joined.buffer, 0, bytes / 4)
  }
}

/**
 * FNV-1a over a float array's bytes.
 *
 * The e2e determinism assert (D2): a hash the test can compare against a
 * committed expectation without shipping 10 000 coordinates through the
 * browser bridge. Cheap and non-cryptographic on purpose — it is detecting a
 * changed layout, not resisting an adversary.
 */
export function fnv1a(values: Float32Array): string {
  const bytes = new Uint8Array(values.buffer, values.byteOffset, values.byteLength)
  let hash = 0x811c9dc5
  for (const byte of bytes) {
    hash ^= byte
    hash = Math.imul(hash, 0x01000193) >>> 0
  }
  return hash.toString(16).padStart(8, '0')
}
