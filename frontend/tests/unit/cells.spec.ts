import { expect, test } from '@playwright/test'
import { compareCells, recordText, typedText } from '../../src/cells'
import type { RecordCell } from '../../src/generated/RecordCell'

const integer = (value: string): RecordCell => ({state: 'value', value: {type: 'int64', value}})
test('typed cell sorting preserves integers beyond Number precision and genuine numeric strings', () => {
  const low = integer('9007199254740992'); const high = integer('9007199254740993')
  expect(compareCells(low, high)).toBeLessThan(0)
  expect(compareCells(integer('2'), high)).toBeLessThan(0)
  const string: RecordCell = {state: 'value', value: {type: 'string', value: '9007199254740993'}}
  expect(compareCells(high, string)).toBeLessThan(0)
  expect(recordText(high)).toBe('9007199254740993')
  expect(recordText(string)).toBe('"9007199254740993"')
  expect(compareCells({state: 'missing'}, high, true)).toBeGreaterThan(0)
  expect(compareCells({state: 'null'}, {state: 'missing'})).toBeLessThan(0)
})
test('human cell values distinguish false, zero, empty, null, missing and partial values', () => {
  expect(typedText({type: 'boolean', value: false})).toBe('false')
  expect(typedText({type: 'int64', value: '0'})).toBe('0')
  expect(typedText({type: 'string', value: ''})).toBe('""')
  expect(recordText({state: 'null'})).toBe('null')
  expect(recordText({state: 'missing'})).toBe('missing')
  expect(recordText({state: 'truncated', preview: 'part', reason: 'byte bound'})).toContain('partial')
  expect(typedText({type: 'list', value: [{type: 'int64', value: '9007199254740993'}]})).toBe('[9007199254740993]')
})
