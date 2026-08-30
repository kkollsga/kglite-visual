/**
 * What the panel needs from a query editor, with no CodeMirror in sight.
 *
 * Types only, and that is load-bearing: `panels.ts` imports this file with
 * `import type`, which TypeScript erases, so the panel carries no static
 * reference to the editor module and the bundler keeps CodeMirror out of the
 * entry chunk. The one import that does reach it is the `import()` in
 * `panels.ts`, which is what puts the editor in its own chunk.
 */

import type { Diagnostic } from '../generated/Diagnostic'

/**
 * What a validation request answers, seen from the editor.
 *
 * Re-exported here rather than imported from `generated/` inside the editor's
 * own files, so every one of the editor's inputs arrives through one door.
 */
export type { Diagnostic }

/** The editor, as the panel drives it. A `<textarea>` satisfies this too. */
export type QueryEditor = {
  /** The query text as it stands. */
  value(): string
  /** Replace the whole document — a saved query, a history entry. */
  setValue(text: string): void
  focus(): void
}

/**
 * What completion reads. `SchemaCache` implements it; the editor never learns
 * where any of it came from, which is what keeps the fetching in the entry
 * chunk and out of the editor's.
 *
 * Every method answers **now**, from cache. `onChange` is how a lazily fetched
 * answer reaches an already-open completion list.
 */
export type SchemaSource = {
  labels(): readonly string[]
  relationshipTypes(): readonly string[]
  /** Cached property names of one node label; empty until a fetch lands. */
  propertiesFor(label: string): readonly string[]
  onChange(listener: () => void): void
}

/** Everything the editor needs at mount time. */
export type QueryEditorOptions = {
  /** Where the editor's DOM goes. */
  parent: HTMLElement
  /** The text the fallback `<textarea>` was holding. */
  doc: string
  /** Ctrl/Cmd+Enter. The panel decides what running means. */
  onRun: () => void
  /** Labels, relationship types and per-type properties, already in hand. */
  schema: SchemaSource
  /**
   * Ask the server what is wrong with this query, without running it.
   *
   * Called on an idle timer while the user types, so it must be cheap and it
   * must never throw: the editor turns the answer into underlines, and an
   * exception would become a red squiggle claiming the query is invalid on the
   * evidence that the connection dropped.
   */
  validate(query: string): Promise<Diagnostic[]>
  /**
   * The same findings, for the panel to list under the editor.
   *
   * The underline is easy to miss in a 340px column and its message lives in a
   * hover tooltip, which is a place a user has to already suspect something to
   * look. One request, two renderings.
   */
  onDiagnostics(found: Diagnostic[]): void
}
