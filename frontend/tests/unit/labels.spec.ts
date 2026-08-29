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
