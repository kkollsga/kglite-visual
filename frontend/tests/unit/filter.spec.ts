/**
 * The filter's grammar and its refusal (plan E7) — no browser, no server.
 *
 * Unit-tested rather than left to the e2e because the failure that matters is
 * silent: a term the matcher cannot answer and drops anyway leaves a filter
 * that looks like it worked and is filtering on less than it was told. That is
 * a wrong picture with a confident caption, and it is indistinguishable from a
 * correct one in a screenshot.
 */

import { expect, test } from '@playwright/test'

import {
  filterLine,
  matches,
  parseFilter,
  unknownKeys,
  type SlotFacts,
} from '../../src/filter'

function facts(text: string, nodeType: string | null, values: [string, unknown][] = []): SlotFacts {
  return { text, nodeType, values: new Map(values) }
}

test('a bare word matches the label or the type, case-insensitively', () => {
  const terms = parseFilter('WELL')
  expect(matches(terms, facts('31/2-1 Troll', 'Wellbore'))).toBe(true)
  expect(matches(terms, facts('Wellhead north', 'Facility'))).toBe(true)
  expect(matches(terms, facts('Troll', 'Field'))).toBe(false)
})

test('several terms narrow rather than widen', () => {
  // AND, because narrowing is what a filter box is for. If this were OR, a
  // second word would show MORE than the first did, which is the opposite of
  // what typing more means.
  const terms = parseFilter('troll type:Field')
  expect(matches(terms, facts('Troll', 'Field'))).toBe(true)
  expect(matches(terms, facts('Troll', 'Wellbore'))).toBe(false)
  expect(matches(terms, facts('Oseberg', 'Field'))).toBe(false)
})

test('a property term reads a loaded value, and a slot without one does not match', () => {
  const terms = parseFilter('status:producing')
  expect(matches(terms, facts('Troll', 'Field', [['status', 'PRODUCING']]))).toBe(true)
  expect(matches(terms, facts('Troll', 'Field', [['status', 'SHUT DOWN']]))).toBe(false)
  // No value at all is a genuine non-match, not a pass: `status:producing`
  // must hide the nodes whose status nobody knows, or the filter's answer
  // includes rows it cannot vouch for.
  expect(matches(terms, facts('Troll', 'Field'))).toBe(false)
})

test('a key nothing loaded carries is refused BY NAME, never dropped', () => {
  const terms = parseFilter('depth:2000 type:Wellbore name')
  expect(unknownKeys(terms, new Set(['status']))).toEqual(['depth'])
  // `type` is always answerable, and a bare word needs no key at all.
  expect(unknownKeys(parseFilter('type:Field troll'), new Set())).toEqual([])
  // Reported once, in the order typed, however many times it appears.
  expect(unknownKeys(parseFilter('a:1 b:2 a:3'), new Set())).toEqual(['a', 'b'])
})

test('a half-typed term does not empty the screen', () => {
  // `type:` on the way to `type:Field` would otherwise be a term with an empty
  // needle, which matches nothing — the whole view blinking out mid-keystroke.
  expect(parseFilter('type:')).toEqual([])
  expect(parseFilter('  ')).toEqual([])
  expect(parseFilter(':value')).toEqual([])
})

test('the honesty line appears only when something is hidden', () => {
  expect(filterLine(12, 98)).toBe('filter: showing 12 of 98 drawn')
  // A line that is always there stops being read, so "nothing hidden" is
  // silence rather than "98 of 98".
  expect(filterLine(98, 98)).toBeNull()
  expect(filterLine(1200, 103719)).toBe('filter: showing 1,200 of 103,719 drawn')
})
