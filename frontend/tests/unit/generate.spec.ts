/**
 * The Cypher this app writes on the user's behalf (plan E9).
 *
 * Two properties are asserted here and nowhere else, because they are the two
 * ways a query generator goes wrong: an identifier that escapes into syntax,
 * and a value that was concatenated instead of bound.
 */

import { expect, test } from '@playwright/test'

import {
  MAX_TABLE_COLUMNS,
  isIdentifier,
  tableColumns,
  typeTableQuery,
} from '../../src/generate'

test('an identifier that could escape the query is refused, not quoted', () => {
  // The same list `core::query`'s own test refuses, because these two rules
  // have to agree: this one decides whether the app *generates* a query, and
  // that one decides whether the server *runs* one it was handed.
  for (const bad of [
    'name`',
    'name RETURN 1',
    'n.name',
    '',
    '1name',
    "name'",
    'name)-[:X]->(m',
    'name WHERE 1=1 //',
  ]) {
    expect(isIdentifier(bad), `${bad} must be refused`).toBe(false)
    expect(() => typeTableQuery(bad, [], [1])).toThrow()
    expect(() => typeTableQuery('Item', [bad], [1])).toThrow()
  }
  for (const good of ['title', 'name', '_private', 'col_2', 'wlbWellboreName']) {
    expect(isIdentifier(good), good).toBe(true)
  }
})

test('a type table projects the columns it was given and binds the ids', () => {
  const { query, params } = typeTableQuery('Wellbore', ['wlbWellType', 'wlbTotalDepth'], [7, 9])

  expect(query).toBe(
    'MATCH (n:Wellbore)\n' +
      'WHERE id(n) IN $ids\n' +
      'RETURN id(n) AS id, n.wlbWellType AS wlbWellType, n.wlbTotalDepth AS wlbTotalDepth',
  )
  // The ids are BOUND, never written into the text: a hundred nodes on screen
  // is one parameter, and nothing a caller can put in the array becomes syntax.
  expect(query).not.toContain('7')
  expect(params).toEqual({ ids: [7, 9] })

  // A type with no properties worth showing is still a table of ids, not an
  // error: `RETURN` with nothing in it would not parse.
  expect(typeTableQuery('Bare', [], [1]).query).toContain('RETURN id(n) AS id')
})

test('a property called id does not collide with the node handle', () => {
  // kglite refuses a RETURN with two columns under one name, and the fixture's
  // own Person carries an `id` property — so `id(n) AS id` beside `n.id AS id`
  // made the table action a syntax error on the first type anybody clicked.
  const { query } = typeTableQuery('Person', ['id', 'age'], [1])
  expect(query).toContain('RETURN id(n) AS node_id, n.id AS id, n.age AS age')

  // …and again if the data also has a `node_id`.
  expect(typeTableQuery('Person', ['id', 'node_id'], [1]).query).toContain(
    'RETURN id(n) AS node_node_id,',
  )
})

test('columns are the best-covered properties, capped and stable', () => {
  const stats = [
    { name: 'rare', non_null: 2 },
    { name: 'everywhere', non_null: 900 },
    { name: 'common', non_null: 900 },
    // Refused by the identifier rule, so it cannot reach a generated query at
    // all — dropped here rather than thrown, because one awkward property name
    // must not cost the type its whole table.
    { name: 'has space', non_null: 1000 },
  ]
  expect(tableColumns(stats)).toEqual(['common', 'everywhere', 'rare'])

  const many = Array.from({ length: 40 }, (_, i) => ({
    name: `p${String(i).padStart(2, '0')}`,
    non_null: 100 - i,
  }))
  const capped = tableColumns(many)
  expect(capped).toHaveLength(MAX_TABLE_COLUMNS)
  expect(capped[0]).toBe('p00')
  expect(capped.at(-1)).toBe('p11')
  // Two calls over the same statistics produce the same table: a cap plus an
  // unstable order would silently change which columns a user sees.
  expect(tableColumns(many)).toEqual(capped)
})
