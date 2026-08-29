/**
 * The four interaction concepts, without a WebGL context.
 *
 * What the four sets *become* is a decision (cosmos.gl offers three
 * index-addressed channels, not four), and a decision only a browser can check
 * is a decision nothing checks. The projection is therefore pure and tested
 * here; the e2e checks that the projection reaches the renderer.
 */

import { expect, test } from '@playwright/test'

import { InteractionState, type InteractionTarget } from '../../src/interaction'
import {
  compileCategoricalColor,
  compileNumericSize,
  fillColors,
  HIGHLIGHT_COLOR,
  linkWidth,
  statLabel,
  typeRadius,
  UNSET_COLOR,
} from '../../src/appearance'
import type { PropertyStat } from '../../src/generated/PropertyStat'

/** A stand-in renderer. Records every config it is handed. */
function target(adjacency: Record<number, number[]> = {}): InteractionTarget & {
  configs: Record<string, unknown>[]
} {
  const configs: Record<string, unknown>[] = []
  return {
    configs,
    getNeighboringPointIndices: (indices) =>
      adjacency[typeof indices === 'number' ? indices : (indices[0] ?? -1)] ?? [],
    getConnectedLinkIndices: () => [7, 8],
    setConfigPartial: (config) => configs.push(config),
  }
}

test('hovering emphasises the 1-hop neighbourhood, hover included', () => {
  const renderer = target({ 3: [4, 5] })
  const state = new InteractionState()

  expect(state.hover(renderer, 3)).toBe(true)
  // The hovered point must be in the array: `highlightedPointIndices` greys out
  // everything NOT listed, so omitting it would grey out the point under the
  // cursor.
  expect(state.emphasizedSlots()).toEqual([3, 4, 5])
  expect(state.toConfig().focusedPointIndex).toBe(3)
  expect(state.toConfig().highlightedLinkIndices).toEqual([7, 8])
})

test('re-hovering the same point changes nothing', () => {
  // A mouse-move fires per pixel. Re-uploading three arrays sixty times a
  // second for an unchanged hover is the per-item work D7 exists to avoid.
  const renderer = target({ 3: [4] })
  const state = new InteractionState()
  expect(state.hover(renderer, 3)).toBe(true)
  expect(state.hover(renderer, 3)).toBe(false)
  expect(state.hover(renderer, null)).toBe(true)
})

test('not hovering leaves the view alone rather than greying all of it out', () => {
  // `[]` means "highlight nothing", which greys out the entire graph.
  // `undefined` is the only value that means "no highlighting".
  const state = new InteractionState()
  const config = state.toConfig()
  expect(config.highlightedPointIndices).toBeUndefined()
  expect(config.highlightedLinkIndices).toBeUndefined()
  expect(config.outlinedPointIndices).toBeUndefined()
  expect(config.focusedPointIndex).toBeUndefined()
})

test('the four concepts stay separate and reach the renderer in one call', () => {
  const renderer = target({ 1: [2] })
  const state = new InteractionState()
  state.hover(renderer, 1)
  state.setSelected([9])
  state.setHighlighted([4, 5])
  state.apply(renderer)

  expect(renderer.configs).toHaveLength(1)
  const config = renderer.configs[0]
  expect(config?.['focusedPointIndex']).toBe(1)
  expect(config?.['highlightedPointIndices']).toEqual([1, 2])
  expect(config?.['outlinedPointIndices']).toEqual([9])
  // Highlighted (search/query hits) rides the colour array, not a ring channel:
  // `outlinedPointRingColor` is a single value and cannot carry two meanings.
  expect(state.highlightedSlots()).toEqual([4, 5])
  expect(state.selectedSlots()).toEqual([9])
})

test('the sets are copied out, so a caller cannot mutate them in place', () => {
  const state = new InteractionState()
  state.setSelected([1, 2])
  state.selectedSlots().push(3)
  expect(state.selectedSlots()).toEqual([1, 2])
})

const CITY: PropertyStat = {
  name: 'city',
  value_type: 'String',
  non_null: 60,
  unique: 3,
  values: ['oslo', 'bergen', 'tromso'],
  sample: null,
  approx: false,
  role: 'categorical',
}

test('a categorical colour getter is compiled once and covers its value set', () => {
  const color = compileCategoricalColor(CITY)
  expect(color).not.toBeNull()
  if (color === null) return
  expect(color('oslo')).not.toEqual(color('bergen'))
  expect(color('somewhere else')).toEqual(UNSET_COLOR)
})

test('an approximate value set never becomes a palette', () => {
  // The D12 rule: kglite sets `approx` when it sampled or hit its distinct-value
  // cap, so the value set is a LOWER BOUND. Colouring by it leaves the values
  // nobody enumerated silently uncoloured, which reads as missing data.
  expect(compileCategoricalColor({ ...CITY, approx: true })).toBeNull()
  expect(compileCategoricalColor({ ...CITY, values: [] })).toBeNull()
})

test('the approximate label says "approximate", verbatim', () => {
  // Phase 0 finding: never present sampled statistics as exact. The word is
  // asserted rather than the styling, because the styling is not what a
  // screen reader or a careful user reads.
  expect(statLabel({ ...CITY, approx: true })).toContain('approximate')
  expect(statLabel({ ...CITY, approx: true })).toContain('3+')
  expect(statLabel(CITY)).not.toContain('approximate')
})

test('a numeric size ramp spreads four orders of magnitude', () => {
  const size = compileNumericSize([1, 10, 100, 10_000])
  expect(size(1)).toBeCloseTo(4, 5)
  expect(size(10_000)).toBeCloseTo(22, 5)
  // Fourth root, measured against the alternative rather than a guessed
  // threshold: a value 1% of the way up the range lands at 9.7 px, where a
  // linear ramp would put it at 4.2 — within a rounding error of the minimum,
  // and therefore invisible.
  const linear = 4 + 18 * (99 / 9999)
  expect(size(100)).toBeGreaterThan(linear + 4)
  expect(size(100)).toBeCloseTo(9.678, 3)
  expect(size('not a number')).toBe(4)
})

test('one colour fill covers every slot, with highlights winning', () => {
  const colors = fillColors(3, () => UNSET_COLOR, new Set([1]))
  expect(colors).toHaveLength(12)
  // Channel by channel, not deep equality: the array is `Float32Array`, so
  // 0.86 comes back as 0.8600000143051147. That rounding is the renderer's
  // storage, not a wrong colour.
  for (const [channel, expected] of HIGHLIGHT_COLOR.entries()) {
    expect(colors[4 + channel]).toBeCloseTo(expected, 6)
  }
  for (const [channel, expected] of UNSET_COLOR.entries()) {
    expect(colors[channel]).toBeCloseTo(expected, 6)
  }
})

test('the type size ramp keeps a 3-member type and a 102k one both visible and apart', () => {
  // The real spread from the graph this was tuned against.
  const smallest = typeRadius(3, 102_420, false)
  const median = typeRadius(1_051, 102_420, false)
  const largest = typeRadius(102_420, 102_420, false)
  expect(smallest).toBeGreaterThanOrEqual(6)
  expect(largest).toBeCloseTo(36, 5)
  // The failure this replaces: the fourth-root ramp put the median type at
  // 16.3px in an 8-34px range, i.e. in the bottom third with everything else.
  // A log ramp has to put it near the middle or it has not fixed anything.
  expect(median).toBeGreaterThan((smallest + largest) / 2 - 5)
  expect(median - smallest).toBeGreaterThan(10)
  expect(largest - median).toBeGreaterThan(10)
})

test('a supporting type is drawn smaller than a core type of the same size', () => {
  expect(typeRadius(1_051, 102_420, true)).toBeLessThan(typeRadius(1_051, 102_420, false))
  // Still ranked among themselves: quieter, not flattened.
  expect(typeRadius(102_420, 102_420, true)).toBeGreaterThan(typeRadius(3, 102_420, true))
})

test('a link with no count gets the floor rather than a fabricated width', () => {
  expect(linkWidth(0, 102_420)).toBe(0.5)
  expect(linkWidth(1, 102_420)).toBeGreaterThan(0.5)
  expect(linkWidth(102_420, 102_420)).toBeCloseTo(5, 5)
  expect(linkWidth(1_760, 102_420)).toBeGreaterThan(linkWidth(1, 102_420))
})

test('a collapse drops the slots it killed from every set it was in', () => {
  // The counts in `window.__kglv` are the instrument every agent assertion
  // reads, and a highlight naming a tombstoned slot leaves them describing
  // nodes that are gone. Nothing wrong is drawn — a colour at a NaN position
  // draws no point — which is exactly why nothing else catches it.
  const state = new InteractionState()
  const graph = target({ 1: [2], 2: [1, 3] })
  state.hover(graph, 1)
  state.setHighlighted([1, 2, 5])
  state.setSelected([2])

  state.dropSlots([1, 2])

  expect(state.highlightedSlots()).toEqual([5])
  expect(state.selectedSlots()).toEqual([])
  expect(state.hoveredSlot()).toBeNull()
  expect(state.emphasizedSlots()).toEqual([])
  // The renderer is told nothing is hovered, rather than being left pointing
  // at a slot that no longer draws.
  expect(state.toConfig().focusedPointIndex).toBeUndefined()
})
