/**
 * Slot detection and alias resolution — the two decisions completion gets wrong.
 *
 * Both are pure functions of the document text and the caret offset, so they
 * are testable here rather than through a browser, a server and a graph. The
 * cases below are the ones that separate a working completion list from one
 * that confidently offers the wrong vocabulary: a colon that is not a pattern,
 * an alias that appears twice under different labels, a decimal point that
 * looks like a property read.
 *
 * `|` marks the caret in each fixture; nothing else in these strings is
 * notation.
 */

import { expect, test } from '@playwright/test'

import { labelForAlias, slotAt, type CompletionSlot } from '../../src/editor/slots'

/** Read a fixture with `|` for the caret. */
function at(fixture: string): CompletionSlot {
  const pos = fixture.indexOf('|')
  return slotAt(fixture.replace('|', ''), pos)
}

test('a colon inside a node pattern asks for labels, with or without an alias', () => {
  expect(at('MATCH (w:Well|')).toEqual({ kind: 'label', from: 9 })
  expect(at('MATCH (:Well|')).toEqual({ kind: 'label', from: 8 })
  // The colon just typed, nothing after it: the list must open on an empty
  // word, because the point of `:` is not knowing the name yet.
  expect(at('MATCH (w:|')).toEqual({ kind: 'label', from: 9 })
  // A second pattern in the same MATCH, after a comma.
  expect(at('MATCH (a:A), (b:B|')).toEqual({ kind: 'label', from: 16 })
})

test('a colon inside brackets asks for relationship types, not labels', () => {
  expect(at('MATCH (w)-[:DRILL|')).toEqual({ kind: 'relationship', from: 12 })
  expect(at('MATCH (w)-[r:DRILL|')).toEqual({ kind: 'relationship', from: 13 })
  // The bracket is what decides, so the same alias-and-colon shape flips.
  expect(at('MATCH (r:DRILL|').kind).toBe('label')
})

test('a colon that is not a pattern offers the general vocabulary', () => {
  // A map literal. Offering node labels here would name the wrong thing with
  // full confidence, which is the failure this branch exists to prevent.
  expect(at('RETURN {name: val|').kind).toBe('general')
  expect(at('MATCH (n) RETURN n AS x, {a: b|').kind).toBe('general')
})

test('a dot after an alias asks for that alias’s properties', () => {
  expect(at('MATCH (w:Wellbore) RETURN w.tit|')).toEqual({
    kind: 'property',
    from: 28,
    alias: 'w',
  })
  // The dot just typed.
  expect(at('MATCH (w:Wellbore) RETURN w.|')).toEqual({
    kind: 'property',
    from: 28,
    alias: 'w',
  })
})

test('a decimal point is a number, not a property read', () => {
  // `3.14` would otherwise resolve an alias called "3" and offer its columns.
  expect(at('RETURN 3.14|').kind).toBe('general')
})

test('an empty word is not a completion slot unless it is a schema position', () => {
  // A general slot on an empty word returns `from === pos`; the source uses
  // that to stay quiet rather than opening a list of every keyword on a space.
  const slot = at('MATCH (n) |')
  expect(slot.kind).toBe('general')
  expect(slot.from).toBe(10)
})

test('an alias resolves to the label it was bound to', () => {
  const query = 'MATCH (w:Wellbore)-[:DRILLED_IN]->(f:Field) RETURN w.title'
  expect(labelForAlias(query, 'w', query.length)).toBe('Wellbore')
  expect(labelForAlias(query, 'f', query.length)).toBe('Field')
  expect(labelForAlias(query, 'q', query.length)).toBeNull()
})

test('a rebound alias resolves to the binding the caret is under', () => {
  const query = 'MATCH (n:Field) WITH n MATCH (n:Wellbore) RETURN n.'
  // Under the second MATCH the name means Wellbore, and completing `n.` with
  // Field's columns would offer properties the query cannot read.
  expect(labelForAlias(query, 'n', query.length)).toBe('Wellbore')
  // Between the two, it still means the first.
  expect(labelForAlias(query, 'n', query.indexOf('WITH'))).toBe('Field')
})

test('a binding after the caret still resolves, for an edited query', () => {
  // Editing the middle of a finished query is the ordinary case; refusing to
  // look forward would make completion work only while writing left to right.
  const query = 'RETURN w. MATCH (w:Wellbore)'
  expect(labelForAlias(query, 'w', 9)).toBe('Wellbore')
})

test('a relationship alias is not a node label binding', () => {
  // `[r:DRILLED_IN]` binds a relationship, and `r.` is a relationship property
  // — a different vocabulary the scan deliberately does not claim to know.
  const query = 'MATCH (w:Wellbore)-[r:DRILLED_IN]->(f) RETURN r.'
  expect(labelForAlias(query, 'r', query.length)).toBeNull()
})
