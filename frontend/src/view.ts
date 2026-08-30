/**
 * The client's half of the slot space (plan D4).
 *
 * The server owns the indices; this holds what the renderer needs to draw them
 * and what the UI needs to name them. Three rules it exists to keep:
 *
 * - **Positions are spliced, not rebuilt.** An expansion appends slots and
 *   sends only their coordinates, so the array grows at `first_slot`. Rebuilding
 *   would make every expansion an O(V) upload of data the GPU already has.
 * - **A tombstone is a NaN.** cosmos.gl reads a NaN position as absence: the
 *   point stops drawing and the links touching it are faded, with no
 *   re-indexing of anything else. Splicing the array instead would silently
 *   renumber every slot after the hole.
 * - **A compaction is applied, never inferred.** The server sends an explicit
 *   old→new remap; this rewrites its maps from that and nothing else. A client
 *   that guessed slots had moved would re-label whatever the user has selected.
 */

import type { Compaction } from './generated/Compaction'
import type { GraphSliceMeta } from './generated/GraphSliceMeta'
import type { MetaGraphMeta } from './generated/MetaGraphMeta'

/** What the UI knows about one occupied slot. */
export type SlotLabel = {
  /** Display text. */
  text: string
  /** Capability flags on a type node; empty for an instance. */
  badges: string[]
  /** Bigger wins a label cell. Member count for a type, 1 for an instance. */
  weight: number
  /** True for a meta-graph type node. */
  isType: boolean
  /**
   * True when this type has a parent in kglite's `is_a` forest.
   *
   * Carried through to the appearance because it is the one classification the
   * server already makes about *importance*: a graph with 98 types where 63 are
   * supporting is really a graph of 35, and drawing all 98 at equal weight is
   * why the entry screen reads as a cloud.
   */
  supporting: boolean
  /** kglite node id, for an instance node. */
  nodeId: number | null
  /**
   * The node's `id` **field** — what Cypher's `id(n)` evaluates to.
   *
   * Not {@link nodeId}, which is the engine's internal index and what every
   * request in this app names a node by. The two are different values and a
   * generated query needs this one; see `SliceNode.key` for the defect that
   * bought the distinction. `null` when the node has no `id` field, and for a
   * type node, which is not a node in the graph at all.
   */
  nodeKey: unknown
  /** Node type, for an instance node. */
  nodeType: string | null
}

/** Undirected key for a slot pair. */
function pairKey(a: number, b: number): string {
  return a <= b ? `${a},${b}` : `${b},${a}`
}

export class SlotView {
  /** Server-space positions, `[x0, y0, …]`. NaN at a tombstone. */
  positions = new Float32Array(0)
  /** `[src0, tgt0, …]` slot indices, always the whole set (D4). */
  links = new Float32Array(0)

  private readonly labels = new Map<number, SlotLabel>()
  private readonly slotOfNode = new Map<number, number>()
  private readonly tombstones = new Set<number>()
  /**
   * Edges between two type slots, summed over relationship names.
   *
   * Keyed by the slot pair rather than carried alongside `links`, because the
   * link array is re-sent whole on every slice (D4) and a parallel weight array
   * would have to be re-derived from it anyway. Two type nodes can be joined by
   * several relationship types; the width channel is answering "how much
   * traffic is there between these two", so they add.
   */
  private readonly typeLinkEdges = new Map<string, number>()

  /** Slots allocated, tombstones included. */
  get slotCount(): number {
    return this.positions.length / 2
  }

  get tombstoneCount(): number {
    return this.tombstones.size
  }

  /** Slots that currently draw something. */
  get liveCount(): number {
    return this.slotCount - this.tombstones.size
  }

  /**
   * Slots this client holds an identity for.
   *
   * Equal to `liveCount` on a client that has seen every slice; smaller on one
   * that joined a session already in progress and was never told what the
   * earlier slices added. That gap is the defect `Session::sync_slice` closes,
   * and this is the number it is measured by.
   */
  get namedCount(): number {
    return this.labels.size
  }

  get linkCount(): number {
    return this.links.length / 2
  }

  label(slot: number): SlotLabel | undefined {
    return this.labels.get(slot)
  }

  /** Every occupied slot, ascending. */
  liveSlots(): number[] {
    return [...this.labels.keys()].sort((a, b) => a - b)
  }

  slotForNode(nodeId: number): number | undefined {
    return this.slotOfNode.get(nodeId)
  }

  /**
   * Edges between two type slots, or 0 when the pair is not a meta-graph link.
   *
   * Order-insensitive: the meta-graph draws one line per pair and the width
   * channel is about the pair, not about a direction the line cannot show.
   */
  typeLinkWeight(source: number, target: number): number {
    return this.typeLinkEdges.get(pairKey(source, target)) ?? 0
  }

  /** Seed from the meta-graph: the entry screen owns slots 0..n. */
  setMetaGraph(meta: MetaGraphMeta, points: Float32Array, links: Float32Array): void {
    this.positions = new Float32Array(points)
    this.links = new Float32Array(links)
    this.labels.clear()
    this.slotOfNode.clear()
    this.tombstones.clear()
    this.typeLinkEdges.clear()
    for (const node of meta.nodes) {
      this.labels.set(node.slot, {
        text: node.name,
        badges: node.capabilities,
        weight: node.count,
        isType: true,
        supporting: node.supporting,
        nodeId: null,
        nodeKey: null,
        nodeType: node.name,
      })
    }
    for (const edge of meta.edges) {
      const key = pairKey(edge.source_slot, edge.target_slot)
      this.typeLinkEdges.set(key, (this.typeLinkEdges.get(key) ?? 0) + edge.count)
    }
  }

  /**
   * Apply one slice.
   *
   * Order is load-bearing: append, then tombstone, then remap. The tombstone
   * list and the remap are both in the PRE-compaction index space, so applying
   * the remap first would make the tombstone list point at the wrong slots.
   */
  applySlice(
    meta: GraphSliceMeta,
    compaction: Compaction | null,
    points: Float32Array,
    links: Float32Array,
  ): void {
    if (compaction === null) this.splicePositions(meta.first_slot, points)

    for (const node of meta.nodes) {
      this.labels.set(node.slot, {
        // A node with no title gets its type and id rather than a blank label.
        // That is a *display* fallback: the server sends the empty string it
        // actually found, and inventing a title in the payload would make the
        // absence unrecoverable.
        text: node.title === '' ? `${node.node_type} ${node.node_id}` : node.title,
        badges: [],
        weight: 1,
        isType: false,
        supporting: false,
        nodeId: node.node_id,
        nodeKey: node.key ?? null,
        nodeType: node.node_type,
      })
      this.slotOfNode.set(node.node_id, node.slot)
      this.tombstones.delete(node.slot)
    }

    for (const slot of meta.tombstones) {
      const label = this.labels.get(slot)
      if (label?.nodeId !== null && label !== undefined) this.slotOfNode.delete(label.nodeId)
      this.labels.delete(slot)
      this.tombstones.add(slot)
      if (compaction === null) {
        // cosmos.gl's absence semantics. Not a splice: the array index IS the
        // slot id, and closing the hole would renumber everything after it.
        this.positions[slot * 2] = Number.NaN
        this.positions[slot * 2 + 1] = Number.NaN
      }
    }

    if (compaction !== null) {
      // The server already reclaimed these slots, so there is no hole to mark:
      // it sends the whole post-compaction array from slot zero. Writing the
      // tombstone NaNs here as well would blank whatever moved INTO those
      // indices — the bug this ordering exists to avoid, and the one the unit
      // test caught.
      this.positions = new Float32Array(points)
      this.applyRemap(compaction.old_to_new)
    }
    this.links = new Float32Array(links)
  }

  /**
   * Take a server-computed arrangement (plan E5).
   *
   * Whole-array, from slot zero, because that is what a layout is: every point
   * moved at once, so there is no `first_slot` and nothing to splice around. A
   * tombstone arrives as NaN and stays one.
   *
   * Written through the same splice as an expansion so a layout that was
   * computed *before* a slice this client has already applied leaves the newer
   * slots alone rather than truncating them off the end. The next layout covers
   * them; a shortened positions array would unrender them immediately.
   */
  applyLayout(points: Float32Array): void {
    this.splicePositions(0, points)
  }

  /**
   * Rewrite every slot-keyed map through an old→new remap.
   *
   * Exported behaviour rather than an internal detail because this is the one
   * operation that can silently mislabel the whole view, and it has its own
   * unit test for that reason.
   */
  applyRemap(oldToNew: (number | null)[]): void {
    const labels = new Map<number, SlotLabel>()
    const slotOfNode = new Map<number, number>()
    for (const [oldSlot, label] of this.labels) {
      const next = oldToNew[oldSlot]
      // `undefined` (index past the remap) and `null` (a reclaimed tombstone)
      // both mean "gone". Treating them differently would be a distinction the
      // server does not make.
      if (next === undefined || next === null) continue
      labels.set(next, label)
      if (label.nodeId !== null) slotOfNode.set(label.nodeId, next)
    }
    this.labels.clear()
    for (const [slot, label] of labels) this.labels.set(slot, label)
    this.slotOfNode.clear()
    for (const [nodeId, slot] of slotOfNode) this.slotOfNode.set(nodeId, slot)
    this.tombstones.clear()

    // The type-link weights are keyed by slot, so they are as stale as any
    // other slot-keyed map after a remap. A pair whose either end was
    // reclaimed no longer names a link and is dropped, exactly as its labels
    // were.
    const weights = new Map<string, number>()
    for (const [key, count] of this.typeLinkEdges) {
      const [oldSource, oldTarget] = key.split(',').map(Number) as [number, number]
      const source = oldToNew[oldSource]
      const target = oldToNew[oldTarget]
      if (source == null || target == null) continue
      weights.set(pairKey(source, target), count)
    }
    this.typeLinkEdges.clear()
    for (const [key, count] of weights) this.typeLinkEdges.set(key, count)
  }

  private splicePositions(firstSlot: number, points: Float32Array): void {
    const needed = firstSlot * 2 + points.length
    if (needed > this.positions.length) {
      const grown = new Float32Array(needed)
      grown.set(this.positions)
      this.positions = grown
    }
    this.positions.set(points, firstSlot * 2)
  }
}
