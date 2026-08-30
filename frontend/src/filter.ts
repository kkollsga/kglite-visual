/**
 * Remove-never-find: hiding what is already on screen (plan E7).
 *
 * **This is not search, and the distinction is the whole design.** Search asks
 * the server a question about the graph and can bring answers *in*; a filter
 * decides which of the things already loaded stay drawn. They are two cards for
 * that reason, and a filter that quietly fetched would be a search wearing the
 * wrong label — a user who typed a name into the box that hides things and got
 * five hundred new nodes would have no model left of what either control does.
 *
 * So the matcher only ever reads what the client already holds, and a term that
 * would need a fetch is **refused by name** rather than silently ignored: an
 * ignored term is a filter that appears to work and is filtering on less than
 * it was told.
 *
 * ## What a term can be
 *
 * - `wellbore` — plain text, matched case-insensitively as a substring of the
 *   slot's label and of its node type. Substring rather than a fuzzy
 *   subsequence: subsequence matching over short type names turns almost every
 *   query into "everything" (`abc` matches `AnnouncedBlock`), and a filter
 *   whose failure mode is "nothing was hidden" is a filter nobody trusts.
 * - `type:Wellbore` — the node type, same substring rule.
 * - `<property>:<value>` — a property whose values this client has FETCHED, so
 *   in practice the colour-by, size-by or caption channel. Anything else is
 *   the refusal.
 *
 * Terms are separated by whitespace and combined with AND, because that is what
 * narrowing means and because OR needs a syntax nobody asked for yet.
 */

/** One term: a key to match against, or `null` for "anything visible". */
export type FilterTerm = {
  /** Lower-cased property key, or `null` for a bare word. */
  key: string | null
  /** Lower-cased needle. Never empty — a term with no needle is dropped. */
  needle: string
}

/** What a slot offers a filter, gathered by the caller from what it holds. */
export type SlotFacts = {
  /** The label the user can read on screen. */
  text: string
  nodeType: string | null
  /** Loaded property values, keyed by lower-cased property name. */
  values: ReadonlyMap<string, unknown>
}

/**
 * Split a filter box's contents into terms.
 *
 * A `key:` with nothing after it is dropped rather than treated as a bare word:
 * it is what a half-typed term looks like, and hiding the whole view on every
 * keystroke of `type:` is a box that fights the person using it.
 */
export function parseFilter(text: string): FilterTerm[] {
  const terms: FilterTerm[] = []
  for (const raw of text.trim().split(/\s+/)) {
    if (raw === '') continue
    const at = raw.indexOf(':')
    if (at < 0) {
      terms.push({ key: null, needle: raw.toLowerCase() })
      continue
    }
    const key = raw.slice(0, at).toLowerCase()
    const needle = raw.slice(at + 1).toLowerCase()
    if (key === '' || needle === '') continue
    terms.push({ key, needle })
  }
  return terms
}

/**
 * Keys the client cannot answer, in the order they were typed.
 *
 * `type` is always answerable — every slot carries one. Everything else has to
 * be a property whose values were actually fetched, which is what `loaded`
 * holds.
 */
export function unknownKeys(terms: FilterTerm[], loaded: ReadonlySet<string>): string[] {
  const unknown: string[] = []
  for (const term of terms) {
    if (term.key === null || term.key === 'type') continue
    if (loaded.has(term.key)) continue
    if (!unknown.includes(term.key)) unknown.push(term.key)
  }
  return unknown
}

/** Does this slot survive every term? */
export function matches(terms: FilterTerm[], facts: SlotFacts): boolean {
  return terms.every((term) => matchesTerm(term, facts))
}

function matchesTerm(term: FilterTerm, facts: SlotFacts): boolean {
  if (term.key === null) {
    return (
      facts.text.toLowerCase().includes(term.needle) ||
      (facts.nodeType ?? '').toLowerCase().includes(term.needle)
    )
  }
  if (term.key === 'type') {
    return (facts.nodeType ?? '').toLowerCase().includes(term.needle)
  }
  const value = facts.values.get(term.key)
  // An unknown key reaches here only when the caller chose to filter anyway
  // after being told; a slot with no value for a known key genuinely does not
  // match, which is what makes `field:troll` hide the nodes that have no field.
  if (value === undefined || value === null) return false
  return String(value).toLowerCase().includes(term.needle)
}

/**
 * The "n of m drawn" line, in the truncation banner's voice.
 *
 * Deliberately the same shape as `showing 120 of 103,719 nodes` — the two say
 * the same kind of thing (you are not looking at all of it) and a reader should
 * not have to learn two phrasings. `null` when nothing is hidden, because a
 * line that is always there stops being read.
 */
export function filterLine(shown: number, total: number): string | null {
  if (shown >= total) return null
  const count = (n: number): string => n.toLocaleString('en-US')
  return `filter: showing ${count(shown)} of ${count(total)} drawn`
}
