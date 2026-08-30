/**
 * Schema-aware completion, over data the app already has.
 *
 * **Synchronous** (plan E2). Everything offered here is cached — the
 * meta-graph arrived with the entry screen, and a type's properties were
 * fetched the first time the editor asked. A source that awaited a round trip
 * would open its list after the user had typed past it; instead the list opens
 * with what is known, and `SchemaCache.onChange` re-runs the query when a lazy
 * fetch lands.
 *
 * The two hard parts — reading the slot and resolving an alias to a label — are
 * pure functions in `slots.ts` with their own unit tests. What is left here is
 * the mapping from a slot to a list, and the `validFor` that stops CodeMirror
 * re-asking on every keystroke of the same word.
 */

import type { Completion, CompletionContext, CompletionResult } from '@codemirror/autocomplete'

import type { SchemaSource } from './contract'
import { CYPHER_KEYWORDS } from './cypher'
import { labelForAlias, slotAt } from './slots'

/**
 * The word being completed. While the user is still typing letters of it,
 * CodeMirror filters the list it already has instead of calling back.
 */
const WORD = /^\w*$/

function options(names: readonly string[], type: string, detail?: string): Completion[] {
  return names.map((label) => (detail === undefined ? { label, type } : { label, type, detail }))
}

/**
 * Build the completion source for one schema.
 *
 * `explicit` is honoured only in the sense that an empty word still completes:
 * the point of `:` is that pressing it should show the labels, and requiring a
 * character first would mean the feature only helps people who already know
 * the name.
 */
export function cypherCompletions(schema: SchemaSource) {
  return (context: CompletionContext): CompletionResult | null => {
    const text = context.state.doc.toString()
    const slot = slotAt(text, context.pos)

    switch (slot.kind) {
      case 'label':
        return { from: slot.from, options: options(schema.labels(), 'class'), validFor: WORD }
      case 'relationship':
        return {
          from: slot.from,
          options: options(schema.relationshipTypes(), 'type'),
          validFor: WORD,
        }
      case 'property': {
        const label = labelForAlias(text, slot.alias, context.pos)
        // An alias the scan could not bind to a label. Offering every property
        // of every type would be a list the user has to filter by knowing the
        // answer already, so nothing is offered and the editor says nothing.
        if (label === null) return null
        return {
          from: slot.from,
          options: options(schema.propertiesFor(label), 'property', label),
          validFor: WORD,
        }
      }
      case 'general':
        // Not offered on an empty word: a list of every keyword and every type
        // name, unprompted on each space, is a list in the way of typing.
        if (slot.from === context.pos && !context.explicit) return null
        return {
          from: slot.from,
          options: [
            ...options(CYPHER_KEYWORDS, 'keyword'),
            ...options(schema.labels(), 'class'),
            ...options(schema.relationshipTypes(), 'type'),
          ],
          validFor: WORD,
        }
    }
  }
}
