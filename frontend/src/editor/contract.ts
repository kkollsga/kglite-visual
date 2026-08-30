/**
 * What the panel needs from a query editor, with no CodeMirror in sight.
 *
 * Types only, and that is load-bearing: `panels.ts` imports this file with
 * `import type`, which TypeScript erases, so the panel carries no static
 * reference to the editor module and the bundler keeps CodeMirror out of the
 * entry chunk. The one import that does reach it is the `import()` in
 * `panels.ts`, which is what puts the editor in its own chunk.
 */

/** The editor, as the panel drives it. A `<textarea>` satisfies this too. */
export type QueryEditor = {
  /** The query text as it stands. */
  value(): string
  /** Replace the whole document — a saved query, a history entry. */
  setValue(text: string): void
  focus(): void
}

/** Everything the editor needs at mount time. */
export type QueryEditorOptions = {
  /** Where the editor's DOM goes. */
  parent: HTMLElement
  /** The text the fallback `<textarea>` was holding. */
  doc: string
  /** Ctrl/Cmd+Enter. The panel decides what running means. */
  onRun: () => void
}
