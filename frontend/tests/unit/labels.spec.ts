/**
 * The label grid's collision rule.
 *
 * A flickering overlay is invisible to a screenshot assert and to a state
 * assert alike — the only thing that catches it is pinning the tie-break.
 */

import { expect, test } from '@playwright/test'

import { chooseLabels } from '../../src/labels'
import { rendersGraph } from '../../src/tiers'

test('one label per screen cell, heaviest wins', () => {
  const placed = chooseLabels([
    { slot: 0, x: 10, y: 10, weight: 5 },
    { slot: 1, x: 20, y: 12, weight: 50 },
    { slot: 2, x: 900, y: 400, weight: 1 },
  ])
  expect(placed.map((p) => p.slot)).toEqual([1, 2])
})

test('an exact tie is broken by slot id, in both input orders', () => {
  const ascending = chooseLabels([
    { slot: 3, x: 10, y: 10, weight: 7 },
    { slot: 9, x: 12, y: 11, weight: 7 },
  ])
  const descending = chooseLabels([
    { slot: 9, x: 12, y: 11, weight: 7 },
    { slot: 3, x: 10, y: 10, weight: 7 },
  ])
  expect(ascending.map((p) => p.slot)).toEqual([3])
  expect(descending.map((p) => p.slot)).toEqual([3])
})

test('well-separated points all keep their labels', () => {
  const spread = [0, 1, 2, 3, 4].map((slot) => ({
    slot,
    x: slot * 200,
    y: slot * 60,
    weight: 1,
  }))
  expect(chooseLabels(spread)).toHaveLength(5)
})

test('only the summary tier drops out of the graph view', () => {
  expect(rendersGraph('full')).toBe(true)
  expect(rendersGraph('compact')).toBe(true)
  expect(rendersGraph('top-types')).toBe(true)
  expect(rendersGraph('summary')).toBe(false)
})

test('placeAll nudges a loser into a free neighbouring cell instead of dropping it', () => {
  // Two names on the same circle. The meta-graph must show both: an unlabelled
  // type node is a dot, which is the entry screen this mode exists to fix.
  const stacked = [
    { slot: 0, x: 100, y: 100, weight: 10 },
    { slot: 1, x: 105, y: 102, weight: 5 },
  ]
  expect(chooseLabels(stacked)).toHaveLength(1)
  const all = chooseLabels(stacked, true)
  expect(all.map((p) => p.slot)).toEqual([0, 1])
  // The heavier one keeps the spot it earned; the other moves by exactly one
  // cell, so it is still recognisably attached to its own circle.
  expect(all[0]).toEqual({ slot: 0, x: 100, y: 100 })
  expect(all[1]?.x).toBe(105)
  expect(Math.abs((all[1]?.y ?? 0) - 102)).toBe(30)
})

test('placeAll draws an overlap rather than losing a label when nothing is free', () => {
  // Eleven candidates on one point: one cell plus every nudge the search knows.
  const pile = Array.from({ length: 12 }, (_, i) => ({
    slot: i,
    x: 400,
    y: 400,
    weight: 100 - i,
  }))
  expect(chooseLabels(pile, true)).toHaveLength(12)
})

test('the placement is a function of the input, not of its order', () => {
  const spread = [
    { slot: 4, x: 100, y: 100, weight: 3 },
    { slot: 7, x: 108, y: 104, weight: 3 },
    { slot: 2, x: 640, y: 400, weight: 9 },
  ]
  const forward = chooseLabels(spread, true)
  const reversed = chooseLabels([...spread].reverse(), true)
  expect(forward).toEqual(reversed)
})
