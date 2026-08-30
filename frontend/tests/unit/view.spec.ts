/**
 * Client-side slot bookkeeping — no browser, no server.
 *
 * Two operations here can silently mislabel the whole view rather than fail:
 * a tombstone written as a splice renumbers everything after the hole, and a
 * compaction applied to the wrong maps re-labels whatever the user has
 * selected. Neither produces an error; both produce a plausible wrong picture.
 * That is why they are unit-tested rather than left to the e2e.
 */

import { expect, test } from '@playwright/test'

import type { Compaction } from '../../src/generated/Compaction'
import type { GraphSliceMeta } from '../../src/generated/GraphSliceMeta'
import type { MetaGraphMeta } from '../../src/generated/MetaGraphMeta'
import { SlotView } from '../../src/view'

function metaGraph(names: string[]): MetaGraphMeta {
  return {
    protocol_version: 2,
    tier: 'full',
    stats: {
      node_count: 0,
      edge_count: 0,
      node_type_count: names.length,
      relationship_type_count: 0,
      core_type_count: names.length,
    },
    nodes: names.map((name, slot) => ({
      slot,
      name,
      count: 10 - slot,
      capabilities: [],
      supporting: false,
    })),
    edges: [],
    node_bound: { returned: names.length, total: names.length, truncated: false },
    edge_bound: { returned: 0, total: 0, truncated: false },
  }
}

function slice(overrides: Partial<GraphSliceMeta>): GraphSliceMeta {
  return {
    protocol_version: 2,
    kind: 'expand',
    first_slot: 0,
    nodes: [],
    tombstones: [],
    edges: [],
    slot_count: 0,
    tombstone_count: 0,
    bound: { returned: 0, total: 0, truncated: false },
    link_bound: { returned: 0, total: 0, truncated: false },
    ...overrides,
  }
}

function seeded(): SlotView {
  const view = new SlotView()
  view.setMetaGraph(
    metaGraph(['Person', 'City']),
    Float32Array.from([0, 0, 140, 0]),
    Float32Array.from([]),
  )
  view.applySlice(
    slice({
      first_slot: 2,
      nodes: [
        { slot: 2, node_id: 100, node_type: 'Person', title: 'ada', key: null },
        { slot: 3, node_id: 101, node_type: 'Person', title: 'linus', key: null },
        { slot: 4, node_id: 102, node_type: 'Person', title: '', key: null },
      ],
      slot_count: 5,
    }),
    null,
    Float32Array.from([140, 140, 0, 140, -140, 140]),
    Float32Array.from([0, 2, 0, 3, 2, 3]),
  )
  return view
}

test('an expansion appends without moving what was already drawn', () => {
  const view = seeded()
  expect(view.slotCount).toBe(5)
  expect(view.liveCount).toBe(5)
  expect(view.linkCount).toBe(3)
  expect(Array.from(view.positions.slice(0, 4))).toEqual([0, 0, 140, 0])
  expect(Array.from(view.positions.slice(4))).toEqual([140, 140, 0, 140, -140, 140])
  expect(view.label(0)?.text).toBe('Person')
  expect(view.label(0)?.isType).toBe(true)
  expect(view.label(2)?.text).toBe('ada')
  expect(view.label(2)?.isType).toBe(false)
  expect(view.slotForNode(101)).toBe(3)
})

test('a titleless node falls back to its type and id rather than a blank label', () => {
  // A *display* fallback. The payload carries the empty string the server
  // actually found; inventing a title in the data would make the absence
  // unrecoverable.
  expect(seeded().label(4)?.text).toBe('Person 102')
})

test('a tombstone is a NaN in place, never a splice', () => {
  const view = seeded()
  view.applySlice(
    slice({ kind: 'collapse', first_slot: 5, tombstones: [2, 3], slot_count: 5, tombstone_count: 2 }),
    null,
    Float32Array.from([]),
    Float32Array.from([]),
  )

  expect(view.slotCount).toBe(5)
  expect(view.tombstoneCount).toBe(2)
  expect(view.liveCount).toBe(3)
  expect(Number.isNaN(view.positions[4])).toBe(true)
  expect(Number.isNaN(view.positions[7])).toBe(true)
  // The survivor did NOT move. A splice would have pulled slot 4's position
  // down to index 2 and every label after it would name the wrong point.
  expect(Array.from(view.positions.slice(8))).toEqual([-140, 140])
  expect(view.label(4)?.text).toBe('Person 102')
  expect(view.slotForNode(100)).toBeUndefined()
  expect(view.liveSlots()).toEqual([0, 1, 4])
})

test('a compaction rewrites every slot-keyed map from the remap it was given', () => {
  const view = seeded()
  const compaction: Compaction = {
    protocol_version: 2,
    // Slots 2 and 3 were tombstoned and reclaimed; 0, 1 and 4 close up.
    old_to_new: [0, 1, null, null, 2],
    slot_count: 3,
    reclaimed: 2,
  }
  view.applySlice(
    slice({
      kind: 'collapse',
      first_slot: 0,
      tombstones: [2, 3],
      slot_count: 3,
      tombstone_count: 0,
    }),
    compaction,
    Float32Array.from([0, 0, 140, 0, -140, 140]),
    Float32Array.from([]),
  )

  expect(view.slotCount).toBe(3)
  expect(view.tombstoneCount).toBe(0)
  expect(view.liveCount).toBe(3)
  expect(view.liveSlots()).toEqual([0, 1, 2])
  expect(view.label(0)?.text).toBe('Person')
  expect(view.label(1)?.text).toBe('City')
  // The survivor moved from slot 4 to slot 2, and BOTH maps followed it. A
  // client that updated positions but not the id map would answer "node 102 is
  // at slot 4" for a slot that now holds something else.
  expect(view.label(2)?.text).toBe('Person 102')
  expect(view.slotForNode(102)).toBe(2)
  expect(view.slotForNode(100)).toBeUndefined()
  expect(Array.from(view.positions)).toEqual([0, 0, 140, 0, -140, 140])
})

test('a remap that omits a slot drops it rather than leaving it in place', () => {
  // `undefined` (past the end of the remap) and `null` (reclaimed) both mean
  // gone. Treating a short remap as "unchanged" would keep a label pointing at
  // a slot the server no longer has.
  const view = seeded()
  view.applyRemap([0])
  expect(view.liveSlots()).toEqual([0])
  expect(view.label(0)?.text).toBe('Person')
})

test('re-expanding after a collapse revives the slot the server chose', () => {
  const view = seeded()
  view.applySlice(
    slice({ kind: 'collapse', first_slot: 5, tombstones: [2], slot_count: 5, tombstone_count: 1 }),
    null,
    Float32Array.from([]),
    Float32Array.from([]),
  )
  expect(view.tombstoneCount).toBe(1)

  // The server never reuses slot 2 — it appends 5. The tombstone stays a
  // tombstone, and the count reflects that.
  view.applySlice(
    slice({
      first_slot: 5,
      nodes: [{ slot: 5, node_id: 100, node_type: 'Person', title: 'ada', key: null }],
      slot_count: 6,
      tombstone_count: 1,
    }),
    null,
    Float32Array.from([280, 140]),
    Float32Array.from([]),
  )
  expect(view.slotCount).toBe(6)
  expect(view.tombstoneCount).toBe(1)
  expect(view.slotForNode(100)).toBe(5)
  expect(Number.isNaN(view.positions[4])).toBe(true)
})

test('a slice that both adds nodes and compacts lands them at their new slots', () => {
  // The client half of a defect found by driving the running server: an
  // expansion into an already-sparse view compacts, so one slice carries new
  // nodes AND a remap. Both lists are in the PRE-compaction space and the remap
  // is applied last — any other order puts the new labels on the wrong points.
  const view = seeded()
  view.applySlice(
    slice({ kind: 'collapse', first_slot: 5, tombstones: [2, 3], slot_count: 5, tombstone_count: 2 }),
    null,
    Float32Array.from([]),
    Float32Array.from([]),
  )
  expect(view.tombstoneCount).toBe(2)

  view.applySlice(
    slice({
      first_slot: 5,
      nodes: [
        { slot: 5, node_id: 200, node_type: 'Person', title: 'grace', key: null },
        { slot: 6, node_id: 201, node_type: 'Person', title: 'alan', key: null },
      ],
      slot_count: 5,
      tombstone_count: 0,
    }),
    {
      protocol_version: 2,
      // Slots 2 and 3 reclaimed; 0, 1, 4 close up to 0, 1, 2 and the two new
      // nodes land at 3 and 4.
      old_to_new: [0, 1, null, null, 2, 3, 4],
      slot_count: 5,
      reclaimed: 2,
    },
    Float32Array.from([0, 0, 140, 0, -140, 140, 280, 140, 280, 0]),
    Float32Array.from([]),
  )

  expect(view.slotCount).toBe(5)
  expect(view.tombstoneCount).toBe(0)
  expect(view.liveSlots()).toEqual([0, 1, 2, 3, 4])
  expect(view.label(3)?.text).toBe('grace')
  expect(view.label(4)?.text).toBe('alan')
  expect(view.slotForNode(200)).toBe(3)
  expect(view.slotForNode(201)).toBe(4)
  // The survivor of the collapse is still itself, one slot to the left.
  expect(view.label(2)?.text).toBe('Person 102')
})
