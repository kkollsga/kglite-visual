/**
 * Cypher this app writes on the user's behalf (plan E9).
 *
 * **Generation, not a new engine surface.** A per-type table and a multi-hop
 * path are questions the Cypher that already exists can answer; what was
 * missing was a way to ask them without writing the query. So every feature
 * built on this module ends in a string that goes down the same bounded
 * `/api/cypher` path a hand-typed query does, obeying the same row and byte
 * ceilings, with no request variant and no protocol change.
 *
 * **The generated query is shown, always.** That is the teaching-tool
 * principle, and it is also the honesty one: a UI that quietly ran Cypher the
 * user could not see would be asking them to trust a black box against their
 * own data. The string this module returns is what runs, and it is what the
 * panel puts in front of them.
 *
 * ## The two substitution rules, and why they differ
 *
 * - **Identifiers are validated, never escaped.** Cypher has no bind form for a
 *   label, a relationship type or a property key, so they have to reach the
 *   query as syntax. [`identifier`] is the same total rule `core::query`'s
 *   `validate_identifier` applies on the server — letters, digits and
 *   underscore, never empty, never leading with a digit — and anything else is
 *   refused rather than quoted. A backtick-quoted identifier would still admit
 *   a backtick.
 * - **Values are parameters, never text.** A filter's value is whatever the
 *   user typed, and concatenating it would let a quote turn a filter into a
 *   different query. Every generated query returns its `params` alongside, and
 *   the value only ever appears as `$p0`.
 *
 * These are the client's half of a rule the server keeps independently: core
 * runs whatever query arrives, through kglite's parser, under the same bound.
 * Nothing here is a security boundary the server is relying on — it is what
 * stops the app generating a query the user did not ask for.
 */

/** Values a generated query binds, by parameter name. */
export type QueryParams = Record<string, unknown>

/** A generated query and the parameters it needs. */
export type Generated = {
  /** The Cypher, formatted for reading — this is what the panel shows. */
  query: string
  params: QueryParams
}

/**
 * Columns a generated per-type table may carry.
 *
 * A table is a thing a human reads in a 340px panel, and a type with sixty
 * properties would produce a grid nobody can scan and a horizontal scrollbar
 * hiding the columns that mattered. Twelve is chosen the way the columns
 * themselves are — by coverage: the properties most of this type's nodes
 * actually have. The panel says when it dropped some, because a table that
 * silently omits columns is a table that reads as the whole record.
 */
export const MAX_TABLE_COLUMNS = 12

/**
 * True for a bare Cypher identifier.
 *
 * Deliberately total and deliberately narrow, matching `validate_identifier` in
 * `core::query` character for character. A property this rule refuses is a
 * property this app will not build a query around — the alternative is quoting,
 * and a quoted identifier admits the quote character.
 */
export function isIdentifier(name: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(name)
}

/** [`isIdentifier`], as a refusal with a message a user can act on. */
export function identifier(name: string, what: string): string {
  if (!isIdentifier(name)) {
    throw new Error(
      `${what} "${name}" is not a plain identifier (letters, digits and underscore only), ` +
        'so this query cannot be generated. Write it by hand in the Cypher box.',
    )
  }
  return name
}

/**
 * The rows behind a type's nodes on screen.
 *
 * `WHERE id(n) IN $ids` rather than `MATCH (n:Type) RETURN …`: the table is
 * about the nodes the user is *looking at*, which on a 546 850-node graph is a
 * different question from "every node of this type" by six orders of magnitude.
 * The ids are a parameter, so a hundred of them cost one bind rather than a
 * hundred concatenations.
 *
 * `id(n)` leads the columns because it is the one value that identifies a row
 * back to the graph — the label may repeat, and a table of forty `Ærfugl` rows
 * with no id is a table you cannot act on.
 */
export function typeTableQuery(
  nodeType: string,
  properties: string[],
  ids: number[],
): Generated {
  const label = identifier(nodeType, 'node type')
  const columns = properties.map((name) => identifier(name, 'property'))
  const alias = nodeIdAlias(columns)
  const projection = [
    `id(n) AS ${alias}`,
    ...columns.map((name) => `n.${name} AS ${name}`),
  ]
  return {
    query: `MATCH (n:${label})\nWHERE id(n) IN $ids\nRETURN ${projection.join(', ')}`,
    params: { ids },
  }
}

/**
 * What to call the `id(n)` column when the type already has a property by that
 * name.
 *
 * Not a nicety: kglite refuses a `RETURN` with two columns under one name
 * ("Multiple result columns with the same name are not supported"), and the
 * fixture's own `Person` carries an `id` property — so the obvious `id(n) AS
 * id` turns the table action into a syntax error on the very first type
 * somebody clicks. Found by running it, not by reading the grammar.
 *
 * Prefixing rather than dropping either column, because they are different
 * facts: `id(n)` is the handle every other request in this app names a node by,
 * and `n.id` is whatever the data calls its own identifier. A user comparing
 * the two is doing something reasonable.
 */
function nodeIdAlias(columns: readonly string[]): string {
  const taken = new Set(columns)
  let alias = 'id'
  while (taken.has(alias)) alias = `node_${alias}`
  return alias
}

/**
 * The properties a type's table should carry, most-covered first.
 *
 * Coverage rather than alphabetical or declaration order: a property four of a
 * thousand nodes carry is a column of blanks, and it would push out one every
 * node has. Ties break on the name so two calls over the same statistics
 * produce the same table.
 */
export function tableColumns(
  properties: readonly { name: string; non_null: number }[],
): string[] {
  return [...properties]
    .filter((stat) => isIdentifier(stat.name))
    .sort((a, b) => b.non_null - a.non_null || a.name.localeCompare(b.name))
    .slice(0, MAX_TABLE_COLUMNS)
    .map((stat) => stat.name)
}

/** How a step's value is compared. */
export type FilterOperator = '=' | 'contains' | '>' | '<'

/** One optional per-step narrowing: a property, an operator, and a value. */
export type StepFilter = {
  property: string
  operator: FilterOperator
  /** As typed. Bound as a parameter; never written into the query text. */
  value: string
}

/** Which way a hop's arrow points. `both` draws no arrowhead. */
export type HopDirection = 'out' | 'in' | 'both'

/** One hop of a path: an edge, and the node on the far side of it. */
export type PathStep = {
  relationship: string
  direction: HopDirection
  nodeType: string
  filter: StepFilter | null
}

/** A path, as the builder holds it. */
export type PathSpec = {
  start: string
  startFilter: StepFilter | null
  steps: PathStep[]
}

/**
 * Hops past which the builder does not go.
 *
 * Not a performance guess — the response bound is what protects the server,
 * and it applies whatever the depth. It is about the *picture*: three hops from
 * a type with a fan-out of a hundred is already a million-row question, and a
 * builder that let a user assemble one by clicking is a builder that produces
 * timeouts rather than paths. Beyond three, the honest answer is Cypher.
 */
export const MAX_PATH_DEPTH = 3

/**
 * The path a spec describes, as a graph query.
 *
 * `RETURN n0, r1, n1, …` rather than a projection: this result is meant for the
 * canvas, and "show in graph" maps the nodes and relationships a result
 * *names* into the slot space. A table of property values would draw nothing.
 *
 * **No bound is re-armed here.** The query runs down the same path a typed one
 * does, so the row ceiling and the byte ceiling apply once, at the end, to the
 * whole answer (plan E9). A per-hop limit would be a second, weaker bound that
 * disagreed with the banner the user reads.
 */
export function pathQuery(spec: PathSpec): Generated {
  const { pattern, where, params, names } = compilePath(spec, spec.steps.length)
  const clauses = [`MATCH ${pattern}`]
  if (where.length > 0) clauses.push(`WHERE ${where.join('\n  AND ')}`)
  clauses.push(`RETURN ${names.join(', ')}`)
  return { query: clauses.join('\n'), params }
}

/**
 * How many rows the path would have after `depth` hops.
 *
 * A separate query rather than a `LIMIT` on the real one, and cheap for the
 * reason the expansion preview is cheap: `count(*)` asks the engine for a
 * number and returns one row, so a builder can say "1 240 matches" beside a
 * step before anybody has decided to draw 1 240 nodes. `depth` of 0 counts the
 * start type alone, which is the number that tells a user their *first* filter
 * did something.
 */
export function pathCountQuery(spec: PathSpec, depth: number): Generated {
  const { pattern, where, params } = compilePath(spec, depth)
  const clauses = [`MATCH ${pattern}`]
  if (where.length > 0) clauses.push(`WHERE ${where.join('\n  AND ')}`)
  clauses.push('RETURN count(*) AS matches')
  return { query: clauses.join('\n'), params }
}

/** The pattern, the predicates and the bindings a path of `depth` hops needs. */
function compilePath(
  spec: PathSpec,
  depth: number,
): { pattern: string; where: string[]; params: QueryParams; names: string[] } {
  const steps = spec.steps.slice(0, Math.min(depth, MAX_PATH_DEPTH))
  const where: string[] = []
  const params: QueryParams = {}
  const names: string[] = ['n0']

  let pattern = `(n0:${identifier(spec.start, 'node type')})`
  pushFilter(where, params, 'n0', spec.startFilter)

  steps.forEach((step, index) => {
    const edge = `r${index + 1}`
    const node = `n${index + 1}`
    const type = identifier(step.relationship, 'relationship type')
    const label = identifier(step.nodeType, 'node type')
    const left = step.direction === 'in' ? '<-' : '-'
    const right = step.direction === 'out' ? '->' : '-'
    pattern += `${left}[${edge}:${type}]${right}(${node}:${label})`
    pushFilter(where, params, node, step.filter)
    names.push(edge, node)
  })

  return { pattern, where, params, names }
}

/**
 * One `WHERE` predicate, with its value bound.
 *
 * **The value never reaches the query text.** That is the search precedent
 * (`core::query::search`): the needle is a parameter because a quote in a text
 * box would otherwise turn a filter into a different query. What *does* reach
 * the text is the property name, which is an identifier and is validated
 * instead.
 *
 * `contains` folds both sides to lower case, exactly as the server's own search
 * does — a filter box that matched case-sensitively while the Search card above
 * it did not would be two different meanings of the same word on one screen.
 */
function pushFilter(
  where: string[],
  params: QueryParams,
  node: string,
  filter: StepFilter | null,
): void {
  if (filter === null || filter.property === '' || filter.value === '') return
  const property = identifier(filter.property, 'property')
  const name = `p${Object.keys(params).length}`
  const reference = `${node}.${property}`
  switch (filter.operator) {
    case 'contains':
      where.push(`toLower(toString(${reference})) CONTAINS $${name}`)
      params[name] = filter.value.toLowerCase()
      break
    case '>':
    case '<':
      // A comparison against a string is not a smaller-than question, it is a
      // different one — kglite would compare lexically and the user would read
      // the answer as numeric. Refused here, where the message can say so.
      if (!Number.isFinite(Number(filter.value))) {
        throw new Error(
          `"${filter.value}" is not a number, so ${reference} ${filter.operator} it would ` +
            'compare as text. Use "contains" for a text match.',
        )
      }
      where.push(`${reference} ${filter.operator} $${name}`)
      params[name] = Number(filter.value)
      break
    default:
      // Equality takes the value's own type where it has one: `= "3"` and
      // `= 3` are different questions to the engine, and a numeric property
      // compared against a string matches nothing while looking correct.
      where.push(`${reference} = $${name}`)
      params[name] = coerce(filter.value)
  }
}

/** A typed value from a text box: a number if it reads as one, else the text. */
function coerce(value: string): unknown {
  if (value === 'true') return true
  if (value === 'false') return false
  const asNumber = Number(value)
  return value.trim() !== '' && Number.isFinite(asNumber) ? asNumber : value
}
