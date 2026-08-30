/**
 * The Cypher this app writes on the user's behalf (plan E9).
 *
 * Two properties are asserted here and nowhere else, because they are the two
 * ways a query generator goes wrong: an identifier that escapes into syntax,
 * and a value that was concatenated instead of bound.
 */

import { expect, test } from '@playwright/test'

import {
  MAX_PATH_DEPTH,
  MAX_TABLE_COLUMNS,
  isIdentifier,
  pathCountQuery,
  pathQuery,
  tableColumns,
  typeTableQuery,
  type PathSpec,
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

test('a path becomes a MATCH whose arrows follow the directions chosen', () => {
  const { query, params } = pathQuery({
    start: 'Field',
    startFilter: null,
    steps: [
      { relationship: 'OWNED_BY', direction: 'out', nodeType: 'Company', filter: null },
      { relationship: 'LOCATED_IN', direction: 'in', nodeType: 'City', filter: null },
      { relationship: 'KNOWS', direction: 'both', nodeType: 'Person', filter: null },
    ],
  })
  expect(query).toBe(
    'MATCH (n0:Field)-[r1:OWNED_BY]->(n1:Company)<-[r2:LOCATED_IN]-(n2:City)' +
      '-[r3:KNOWS]-(n3:Person)\n' +
      'RETURN n0, r1, n1, r2, n2, r3, n3',
  )
  // Nothing to bind, so nothing is bound. An empty `params` beside a query with
  // no `$` is the pair that says no value was concatenated.
  expect(params).toEqual({})
  expect(query).not.toContain('$')
})

test('every filter value is bound, and the operator decides its type', () => {
  const { query, params } = pathQuery({
    start: 'Field',
    startFilter: { property: 'fldName', operator: 'contains', value: 'TROLL' },
    steps: [
      {
        relationship: 'OWNED_BY',
        direction: 'out',
        nodeType: 'Company',
        filter: { property: 'cmpShare', operator: '>', value: '50' },
      },
    ],
  })
  expect(query).toBe(
    'MATCH (n0:Field)-[r1:OWNED_BY]->(n1:Company)\n' +
      'WHERE toLower(toString(n0.fldName)) CONTAINS $p0\n' +
      '  AND n1.cmpShare > $p1\n' +
      'RETURN n0, r1, n1',
  )
  // `contains` folds case, exactly as the server's own search does; `>` binds a
  // NUMBER, because a numeric property compared against the string "50" is a
  // lexical comparison wearing a numeric operator.
  expect(params).toEqual({ p0: 'troll', p1: 50 })
  expect(query).not.toContain('TROLL')
  expect(query).not.toContain('50')
})

test('equality takes the value type the text implies', () => {
  const value = (text: string): unknown =>
    pathQuery({
      start: 'T',
      startFilter: { property: 'p', operator: '=', value: text },
      steps: [],
    }).params.p0
  expect(value('3')).toBe(3)
  expect(value('3.5')).toBe(3.5)
  expect(value('true')).toBe(true)
  expect(value('false')).toBe(false)
  expect(value('34/2-A')).toBe('34/2-A')
})

test('a > against text is refused rather than compared lexically', () => {
  expect(() =>
    pathQuery({
      start: 'T',
      startFilter: { property: 'p', operator: '>', value: 'north' },
      steps: [],
    }),
  ).toThrow(/is not a number/)
})

test('an injection attempt in a path spec is refused at every identifier', () => {
  const hostile = "x`) MATCH (m) DETACH DELETE m //"
  const base: PathSpec = { start: 'T', startFilter: null, steps: [] }
  expect(() => pathQuery({ ...base, start: hostile })).toThrow(/not a plain identifier/)
  expect(() =>
    pathQuery({
      ...base,
      steps: [{ relationship: hostile, direction: 'out', nodeType: 'T', filter: null }],
    }),
  ).toThrow(/not a plain identifier/)
  expect(() =>
    pathQuery({
      ...base,
      steps: [{ relationship: 'R', direction: 'out', nodeType: hostile, filter: null }],
    }),
  ).toThrow(/not a plain identifier/)
  expect(() =>
    pathQuery({
      ...base,
      startFilter: { property: hostile, operator: '=', value: 'x' },
    }),
  ).toThrow(/not a plain identifier/)

  // …and the one place hostile text is ALLOWED, because it is bound rather
  // than written: a value can be anything at all.
  const { query, params } = pathQuery({
    ...base,
    startFilter: { property: 'p', operator: '=', value: hostile },
  })
  expect(query).toBe('MATCH (n0:T)\nWHERE n0.p = $p0\nRETURN n0')
  expect(params.p0).toBe(hostile)
})

test('a count probe is the same path truncated, with the same bindings', () => {
  const spec: PathSpec = {
    start: 'Field',
    startFilter: { property: 'fldName', operator: 'contains', value: 'troll' },
    steps: [
      { relationship: 'OWNED_BY', direction: 'out', nodeType: 'Company', filter: null },
      {
        relationship: 'EMPLOYS',
        direction: 'out',
        nodeType: 'Person',
        filter: { property: 'age', operator: '<', value: '40' },
      },
    ],
  }

  // Depth 0 is the start type alone — the number that says whether the first
  // filter did anything.
  expect(pathCountQuery(spec, 0).query).toBe(
    'MATCH (n0:Field)\nWHERE toLower(toString(n0.fldName)) CONTAINS $p0\nRETURN count(*) AS matches',
  )
  const one = pathCountQuery(spec, 1)
  expect(one.query).toContain('(n0:Field)-[r1:OWNED_BY]->(n1:Company)')
  expect(one.query).not.toContain('EMPLOYS')
  // A hop that is not in the probe must not drag its filter's binding in with
  // it: an unused `$p1` is a parameter the engine has no placeholder for.
  expect(one.params).toEqual({ p0: 'troll' })

  const two = pathCountQuery(spec, 2)
  expect(two.query).toContain('AND n2.age < $p1')
  expect(two.params).toEqual({ p0: 'troll', p1: 40 })
  // The probe answers a count, never rows: this is the whole reason it is
  // cheap enough to fire on every change.
  expect(two.query.endsWith('RETURN count(*) AS matches')).toBe(true)
})

test('the depth cap is enforced by the generator, not only by the UI', () => {
  const spec: PathSpec = {
    start: 'T',
    startFilter: null,
    steps: Array.from({ length: MAX_PATH_DEPTH + 2 }, () => ({
      relationship: 'R',
      direction: 'out' as const,
      nodeType: 'T',
      filter: null,
    })),
  }
  // A builder bug that pushed a fourth step must not produce a four-hop query:
  // the cap lives where the query is written.
  expect(pathQuery(spec).query.match(/\[r\d+:R\]/g)).toHaveLength(MAX_PATH_DEPTH)
})
