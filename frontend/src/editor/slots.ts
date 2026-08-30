/**
 * Where the caret is, in Cypher terms — the half of completion that is a pure
 * function of the text.
 *
 * Deliberately free of CodeMirror and of any schema: it takes a document and a
 * caret offset and answers "the user is naming a node label here", so the two
 * decisions that actually go wrong — reading the slot, and working out which
 * label an alias stands for — are testable without a browser, a server or a
 * graph. `completions.ts` is the thin layer that turns an answer into a
 * CodeMirror `CompletionResult`.
 *
 * **A regex scan, not a parser** (plan E2). The alternative is a second Cypher
 * grammar in the browser, which this project already refused once for
 * highlighting and for the same reason. The scan is honest about its limits:
 * it reads `(alias:Label)` bindings out of the text, and an alias that came
 * from anywhere else — a `WITH n AS m`, an `UNWIND`, a function call — is
 * simply not resolved, and the caller offers nothing rather than guessing.
 */

/** What the caret is asking for. */
export type CompletionSlot =
  /** After `:` in a node pattern — `(n:Fo|`, `(:Fo|`, `, :Fo|`. */
  | { kind: 'label'; from: number }
  /** After `:` inside brackets — `[r:DRILL|`, `[:DRILL|`. */
  | { kind: 'relationship'; from: number }
  /** After `alias.` — the alias is resolved separately. */
  | { kind: 'property'; from: number; alias: string }
  /** Anywhere else a word is being typed. */
  | { kind: 'general'; from: number }

const IDENTIFIER = /[A-Za-z0-9_]/

/** Walk back over identifier characters from `at`, returning where they start. */
function wordStart(text: string, at: number): number {
  let index = at
  while (index > 0 && IDENTIFIER.test(text.charAt(index - 1))) index -= 1
  return index
}

/** Walk back over spaces and tabs from `at`. Newlines count as space too. */
function skipSpaceBack(text: string, at: number): number {
  let index = at
  while (index > 0 && /\s/.test(text.charAt(index - 1))) index -= 1
  return index
}

/**
 * Read the slot at `pos`.
 *
 * The word under the caret is found first, so `from` is where a replacement
 * starts; everything after that is a question about the characters in front of
 * it. `(`/`,` before a colon means a node pattern and `[` means a relationship
 * pattern — with an optional alias in between, which is the case that made this
 * a walk rather than a single character look-back: in `(n:Fo` the character
 * before the colon is `n`, not `(`.
 */
export function slotAt(text: string, pos: number): CompletionSlot {
  const from = wordStart(text, pos)

  // `alias.prop` — the dot must be immediately before the word, because
  // `alias . prop` with spaces is not something a user is mid-typing.
  if (from > 0 && text.charAt(from - 1) === '.') {
    const alias = text.slice(wordStart(text, from - 1), from - 1)
    // A digit run before the dot is a decimal literal, not an alias.
    if (alias !== '' && !/^[0-9]/.test(alias)) return { kind: 'property', from, alias }
    return { kind: 'general', from }
  }

  const beforeWord = skipSpaceBack(text, from)
  if (beforeWord > 0 && text.charAt(beforeWord - 1) === ':') {
    // Skip the colon, then an optional alias, then whitespace, and look at
    // what opened the pattern.
    const beforeColon = skipSpaceBack(text, beforeWord - 1)
    const opener = skipSpaceBack(text, wordStart(text, beforeColon))
    const ch = opener > 0 ? text.charAt(opener - 1) : ''
    if (ch === '[') return { kind: 'relationship', from }
    if (ch === '(' || ch === ',') return { kind: 'label', from }
    // A colon somewhere else — a map literal, a `CASE` label. Offering node
    // labels there would be offering the wrong vocabulary confidently.
    return { kind: 'general', from }
  }

  return { kind: 'general', from }
}

/** Every `(alias:Label)` binding in the text, in source order. */
const BINDING = /\(\s*([A-Za-z_][A-Za-z0-9_]*)\s*:\s*([A-Za-z_][A-Za-z0-9_]*)/g

/**
 * The node label an alias stands for, or `null`.
 *
 * The binding *nearest before the caret* wins. A query can rebind a name —
 * `MATCH (n:Field) … MATCH (n:Wellbore) …` — and the one the user is typing
 * under is the one they just wrote. A binding that appears only after the caret
 * still counts, because a user editing the middle of a finished query is the
 * ordinary case; it is only consulted when nothing earlier matched.
 */
export function labelForAlias(text: string, alias: string, pos: number): string | null {
  BINDING.lastIndex = 0
  let before: string | null = null
  let after: string | null = null
  for (let match = BINDING.exec(text); match !== null; match = BINDING.exec(text)) {
    if (match[1] !== alias) continue
    const label = match[2] ?? null
    if (match.index < pos) before = label
    else if (after === null) after = label
  }
  return before ?? after
}
