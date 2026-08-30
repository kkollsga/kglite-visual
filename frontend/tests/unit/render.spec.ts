/**
 * The layout axes, without a GPU.
 *
 * `seedAuthority` is the axis with a silent failure mode: under `gpu` the
 * server's positions are a seed and get overwritten by whatever is already on
 * screen, so a computed layout requested at the wrong authority is *applied,
 * uploaded, and then discarded* — the picture simply does not move, and nothing
 * errors. That is the pre-mortem's first entry, and it is testable here because
 * the merge is a pure function of two arrays.
 */

import { expect, test } from '@playwright/test'

import { axesFor, layoutModeFromSearch, mergePositions, toRendererSpace } from '../../src/render'

test('the query flag selects the deterministic preset and nothing else does', () => {
  expect(layoutModeFromSearch('?deterministic=1')).toBe('deterministic')
  expect(layoutModeFromSearch('')).toBe('force')
  expect(layoutModeFromSearch('?deterministic=0')).toBe('force')
  expect(layoutModeFromSearch('?deterministic=true')).toBe('force')
})

test('the two presets differ on all three axes, and each preset is its own copy', () => {
  // If a future axis is added to one preset and forgotten in the other, this is
  // where it shows: both objects are compared whole.
  expect(axesFor('force')).toEqual({
    simulation: true,
    drag: true,
    seedAuthority: 'gpu',
  })
  expect(axesFor('deterministic')).toEqual({
    simulation: false,
    drag: false,
    seedAuthority: 'server',
  })

  // A mutated preset must not reach the next caller — the axes are a value, and
  // the layout switch this split exists for will hand around modified copies.
  const mine = axesFor('force')
  mine.simulation = false
  expect(axesFor('force').simulation).toBe(true)
})

test('gpu authority keeps what is already drawn and seeds only what is not', () => {
  // Slot 0 is on screen at (10, 10); slot 1 has never been drawn (the live
  // array is short, which is what "never drawn" looks like on the GPU).
  const seeded = new Float32Array([1, 1, 2, 2])
  const live = new Float32Array([10, 10])
  expect([...mergePositions(seeded, live, 'gpu')]).toEqual([10, 10, 2, 2])
})

test('server authority replaces the whole array, on-screen slots included', () => {
  const seeded = new Float32Array([1, 1, 2, 2])
  const live = new Float32Array([10, 10, 20, 20])
  expect([...mergePositions(seeded, live, 'server')]).toEqual([1, 1, 2, 2])
})

test('a server NaN is a tombstone and wins under either authority', () => {
  // Absence is the server's call (D4). Under `gpu` this is the one case where
  // the server still overrides a live position.
  const gpu = mergePositions(new Float32Array([NaN, NaN]), new Float32Array([5, 5]), 'gpu')
  expect(Number.isNaN(gpu[0] as number)).toBe(true)
  const server = mergePositions(new Float32Array([NaN, NaN]), new Float32Array([5, 5]), 'server')
  expect(Number.isNaN(server[0] as number)).toBe(true)
})

test('a non-finite live position is ignored rather than uploaded', () => {
  // A tombstoned slot the server has since revived: the GPU still holds the
  // NaN, and carrying it forward would keep the revived node invisible.
  const merged = mergePositions(
    new Float32Array([3, 3]),
    new Float32Array([NaN, NaN]),
    'gpu',
  )
  expect([...merged]).toEqual([3, 3])
})

/**
 * North is up, on both sides of the wire.
 *
 * The server lays out in canvas coordinates (`y` down, the SVG convention its
 * own emitter draws in); cosmos.gl's `y` grows up. Every server layout has
 * always crossed that boundary, and until the geographic kernel there was no
 * picture that could tell — a ring, an island field and a force cloud all look
 * the same reflected. On sodir the live view drew Goliat at 71 deg N below Yme
 * at 59, and put the no-coordinate tray along the top of the screen.
 */
test('the server-to-renderer conversion flips y and shifts both axes', () => {
  const shifted = [...toRendererSpace(new Float32Array([10, 20, -10, -20]))]
  // x keeps its sign, y does not; both land centred in the renderer's space.
  expect((shifted[0] ?? 0) - (shifted[2] ?? 0)).toBe(20)
  expect((shifted[1] ?? 0) - (shifted[3] ?? 0)).toBe(-40)
  // Two points at the same x differ only in y, and the one the server put
  // LOWER on its canvas is the one the renderer draws lower.
  const [northX, northY, southX, southY] = [
    ...toRendererSpace(new Float32Array([0, -100, 0, 100])),
  ]
  expect(northX).toBe(southX)
  expect(northY ?? 0).toBeGreaterThan(southY ?? 0)
})

test('a tombstone survives the conversion as a NaN on both axes', () => {
  const shifted = [...toRendererSpace(new Float32Array([Number.NaN, Number.NaN, 1, 1]))]
  expect(Number.isNaN(shifted[0] ?? 0)).toBe(true)
  expect(Number.isNaN(shifted[1] ?? 0)).toBe(true)
})
