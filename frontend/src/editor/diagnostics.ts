/**
 * kglite's findings, drawn where the caret is.
 *
 * The engine is the only opinion about validity (plan E3), so everything here
 * is placement: turning a 1-indexed `(line, col)` from `KgError` into a
 * document range CodeMirror can underline, and phrasing the two cases where
 * there is no position.
 */

import type { Diagnostic as LintDiagnostic } from '@codemirror/lint'
import type { Text } from '@codemirror/state'

import type { Diagnostic } from './contract'

const IDENTIFIER = /[A-Za-z0-9_]/

/**
 * The range to underline for one finding.
 *
 * **No position means the whole document**, not "drop it" and not "line 1". A
 * parser error that could not be pinned ("expected end of input") and every
 * schema advisory — kglite's advisory channel is a list of sentences, not
 * positions — are both real findings about the query as a whole, and searching
 * the text for the name they mention would underline the wrong occurrence on a
 * query that mentions it twice.
 *
 * With a position, the token under it is underlined; at the very end of a line,
 * where the engine points *after* what it was reading, the preceding character
 * is underlined instead, because a zero-width squiggle is invisible.
 */
export function rangeFor(doc: Text, found: Diagnostic): { from: number; to: number } {
  if (found.line === null || found.col === null) return { from: 0, to: doc.length }

  const line = doc.line(Math.min(Math.max(found.line, 1), doc.lines))
  const text = line.text
  // 1-indexed, and clamped: the engine counts in its own copy of the query, and
  // a column past the end of the line must not produce a range outside the doc.
  const column = Math.min(Math.max(found.col - 1, 0), text.length)

  let end = column
  while (end < text.length && IDENTIFIER.test(text.charAt(end))) end += 1
  if (end > column) return { from: line.from + column, to: line.from + end }

  // Not on a word. One character forward if there is one, otherwise one back.
  if (column < text.length) return { from: line.from + column, to: line.from + column + 1 }
  if (column > 0) return { from: line.from + column - 1, to: line.from + column }
  return { from: line.from, to: line.to }
}

/** One server finding as a CodeMirror diagnostic. */
export function toLintDiagnostic(doc: Text, found: Diagnostic): LintDiagnostic {
  return {
    ...rangeFor(doc, found),
    severity: found.severity,
    // kglite's own sentence, verbatim — position, expected token, the schema
    // name it could not resolve, and its own caret rendering. A friendlier
    // summary would delete the only part the user can act on.
    message: found.message,
  }
}
