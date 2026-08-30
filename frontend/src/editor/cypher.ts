/**
 * A Cypher tokenizer for CodeMirror's `StreamLanguage`.
 *
 * **Highlighting only. It has no opinion about validity** — that is the whole
 * design (plan E1). kglite's parser is the authority on whether a query parses,
 * and it answers with a line, a column and the token it expected; a second
 * grammar in the browser would be a weaker opinion that disagrees with the
 * engine on exactly the inputs a user needs help with. So this file never
 * reports an error: an unrecognised character is consumed and left unstyled.
 *
 * A `StreamLanguage` rather than a Lezer grammar for the same reason. Lezer
 * would buy incremental parsing and a syntax tree; nothing here needs a tree,
 * and a grammar that must accept every kglite extension is a second dialect
 * definition to keep in step with the engine's.
 *
 * The returned strings are resolved against `@lezer/highlight`'s tag names by
 * `StreamLanguage` itself, so "typeName" here and `tags.typeName` in
 * `theme.ts` are the same tag by construction.
 */

import { StreamLanguage, type StringStream } from '@codemirror/language'

/**
 * The words the editor colours and offers.
 *
 * Standard openCypher plus `CALL`/`YIELD`, checked against kglite's own
 * `CYPHER.md` on 2026-08-30 — every word below appears there. Deliberately not
 * a function list: `count`, `collect` and friends are ordinary identifiers to
 * the tokenizer, and a stale function list would colour a name the engine has
 * never heard of as if it were built in.
 */
export const CYPHER_KEYWORDS: readonly string[] = [
  'ALL',
  'AND',
  'AS',
  'ASC',
  'ASCENDING',
  'BY',
  'CALL',
  'CASE',
  'CONSTRAINT',
  'CONTAINS',
  'CREATE',
  'CSV',
  'DELETE',
  'DESC',
  'DESCENDING',
  'DETACH',
  'DISTINCT',
  'ELSE',
  'END',
  'ENDS',
  'EXISTS',
  'EXPLAIN',
  'FOREACH',
  'FROM',
  'HEADERS',
  'IN',
  'INDEX',
  'IS',
  'LIMIT',
  'LOAD',
  'MATCH',
  'MERGE',
  'NOT',
  'ON',
  'OPTIONAL',
  'OR',
  'ORDER',
  'PROFILE',
  'REMOVE',
  'RETURN',
  'SET',
  'SKIP',
  'STARTS',
  'UNION',
  'UNIQUE',
  'UNWIND',
  'USING',
  'WHEN',
  'WHERE',
  'WITH',
  'XOR',
  'YIELD',
]

/** Words that are values rather than clauses. Coloured as atoms, not keywords. */
const CYPHER_ATOMS = new Set(['NULL', 'TRUE', 'FALSE'])

const KEYWORD_SET = new Set(CYPHER_KEYWORDS)

const IDENTIFIER_START = /[A-Za-z_]/
const IDENTIFIER_PART = /[A-Za-z0-9_]/
const DIGIT = /[0-9]/

/**
 * What the tokenizer must remember between calls.
 *
 * `relationship` is why this is a state machine at all: `:FOO` means a node
 * label outside brackets and a relationship type inside them, and the two are
 * different colours because they are different halves of the schema. `comment`
 * carries an unterminated block comment across the line break.
 */
type CypherState = { relationship: number; comment: boolean }

/** Consume a line comment to end of line, or a block comment across lines. */
function comment(stream: StringStream, state: CypherState): boolean {
  if (state.comment) {
    while (!stream.eol()) {
      if (stream.next() === '*' && stream.eat('/')) {
        state.comment = false
        return true
      }
    }
    return true
  }
  if (stream.match('//')) {
    stream.skipToEnd()
    return true
  }
  if (stream.match('/*')) {
    state.comment = true
    return comment(stream, state)
  }
  return false
}

/** A quoted run, honouring backslash escapes. Unterminated runs end at EOL. */
function quoted(stream: StringStream, quote: string): void {
  let escaped = false
  while (!stream.eol()) {
    const ch = stream.next()
    if (escaped) {
      escaped = false
    } else if (ch === '\\') {
      escaped = true
    } else if (ch === quote) {
      return
    }
  }
}

/** The identifier under the cursor, or `''` when there is none. */
function identifier(stream: StringStream): string {
  if (!stream.match(IDENTIFIER_START, false)) return ''
  let name = ''
  while (!stream.eol() && stream.match(IDENTIFIER_PART, false)) name += stream.next()
  return name
}

function token(stream: StringStream, state: CypherState): string | null {
  if (stream.eatSpace()) return null
  if (comment(stream, state)) return 'comment'

  const ch = stream.peek() ?? ''

  if (ch === "'" || ch === '"') {
    stream.next()
    quoted(stream, ch)
    return 'string'
  }
  // A backtick run is an escaped *identifier* (`` `weird name` ``), not a
  // string literal, and colouring it as one would say the opposite.
  if (ch === '`') {
    stream.next()
    quoted(stream, '`')
    return 'variableName'
  }
  if (DIGIT.test(ch) || (ch === '.' && DIGIT.test(stream.string.charAt(stream.pos + 1)))) {
    stream.match(/^[0-9]*\.?[0-9]*(?:[eE][+-]?[0-9]+)?/)
    return 'number'
  }
  if (ch === '$') {
    stream.next()
    if (stream.eat('`')) quoted(stream, '`')
    else identifier(stream)
    return 'variableName.special'
  }
  // `.foo` — a property read. `.` alone (a lone dot, or one before a bracket)
  // is punctuation; only a dot with a name after it is a property.
  if (ch === '.') {
    stream.next()
    return identifier(stream) === '' ? 'punctuation' : 'propertyName'
  }
  // `:Foo` outside brackets is a node label; `[:FOO]` inside them is a
  // relationship type. Whitespace between the colon and the name is legal.
  if (ch === ':') {
    stream.next()
    stream.eatSpace()
    if (identifier(stream) === '') return 'punctuation'
    return state.relationship > 0 ? 'labelName' : 'typeName'
  }
  if (ch === '[') {
    stream.next()
    state.relationship += 1
    return 'punctuation'
  }
  if (ch === ']') {
    stream.next()
    state.relationship = Math.max(0, state.relationship - 1)
    return 'punctuation'
  }
  if (IDENTIFIER_START.test(ch)) {
    const name = identifier(stream)
    const upper = name.toUpperCase()
    if (CYPHER_ATOMS.has(upper)) return 'atom'
    return KEYWORD_SET.has(upper) ? 'keyword' : 'variableName'
  }
  if (stream.match(/^(?:<->|<-|->|<>|<=|>=|=~|\|\||[-+*/%^=<>|!])/)) return 'operator'
  if (stream.match(/^[(){},;]/)) return 'punctuation'

  // Nothing recognised it. Consume one character and leave it unstyled rather
  // than marking it invalid — see the module header.
  stream.next()
  return null
}

/** The language extension. One instance; `StreamLanguage.define` is not cheap. */
export const cypherLanguage = StreamLanguage.define<CypherState>({
  name: 'cypher',
  startState: () => ({ relationship: 0, comment: false }),
  token,
  languageData: {
    commentTokens: { line: '//', block: { open: '/*', close: '*/' } },
    closeBrackets: { brackets: ['(', '[', '{', "'", '"', '`'] },
  },
})
