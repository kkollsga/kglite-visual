import { expect, test } from '@playwright/test'
import { SharedState } from '../../src/shared'
import type { SharedSnapshotMeta } from '../../src/generated/SharedSnapshotMeta'

function snapshot(generation: string, revision: string): SharedSnapshotMeta {
  return {stamp: {generation, revision}} as SharedSnapshotMeta
}

test('full shared snapshots reject old, duplicate and foreign revisions, and require a fresh baseline after a gap', () => {
  const state = new SharedState()
  state.begin('one')
  expect(state.accept(snapshot('one', '9007199254740992'))).toBe(true)
  expect(state.accept(snapshot('one', '9007199254740993'))).toBe(true)
  expect(state.accept(snapshot('one', '9007199254740992'))).toBe(false)
  expect(state.accept(snapshot('one', '9007199254740993'))).toBe(false)
  expect(state.accept(snapshot('other', '9007199254740994'))).toBe(false)
  expect(state.accept(snapshot('one', 'invalid'))).toBe(false)
  expect(state.accept(snapshot('one', '9007199254740997'))).toBe(false)
  expect(state.gaps).toBe(1)
  expect(state.needsResync).toBe(true)
  expect(state.matches({generation: 'one', revision: '9007199254740993'})).toBe(true)
  state.begin('one')
  expect(state.accept(snapshot('one', '9007199254740997'))).toBe(true)
  state.begin('two')
  expect(state.matches({generation: 'one', revision: '9007199254740997'})).toBe(false)
  expect(state.accept(snapshot('one', '9007199254740998'))).toBe(false)
  expect(state.accept(snapshot('two', '0'))).toBe(true)
})
