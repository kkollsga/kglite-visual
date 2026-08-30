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
