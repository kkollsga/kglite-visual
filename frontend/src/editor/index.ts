/**
 * The CodeMirror 6 query editor — this app's only dynamically imported chunk
 * of its own.
 *
 * **Progressive enhancement, not a replacement.** `panels.ts` builds a plain
 * `<textarea>` first and asks for this module afterwards; if the chunk never
 * arrives the panel keeps the textarea and says so. That is the reason for the
 * split: the editor is authoring comfort, and a query box that cannot be typed
 * into because a script failed would be the app's central control lost to a
 * nicety.
 *
 * **Hand-picked extensions, no `basicSetup`** (plan E1). `basicSetup` pulls
 * search, folding, bracket matching, a gutter and a rectangular-selection
 * handler — every one of them bytes in a 340px panel that shows four lines of
 * Cypher. What is here is what the panel uses: the tokenizer, the theme, undo,
 * one keybinding, and (from the commits that follow) completions and
 * diagnostics.
 *
 * **`history()` is here because its absence was a regression, not because the
 * list looked short.** Measured on sodir, 2026-08-30, before it was added:
 * typing `RETURN 2` and pressing Ctrl/Cmd+Z left the text unchanged, because
 * CodeMirror takes the document away from the browser and a `contenteditable`
 * with no history extension has no undo at all — while the `<textarea>` this
 * replaces got the browser's for free. Everything else CodeMirror does without
 * a keymap was checked the same way and does work: Enter inserts a line,
 * Backspace deletes, select-all selects.
 */

import { history, historyKeymap } from '@codemirror/commands'
import { EditorState } from '@codemirror/state'
import { EditorView, keymap } from '@codemirror/view'

import type { QueryEditor, QueryEditorOptions } from './contract'
import { cypherLanguage } from './cypher'
import { cypherTheme } from './theme'

export function mountCypherEditor(options: QueryEditorOptions): QueryEditor {
  const view = new EditorView({
    parent: options.parent,
    state: EditorState.create({
      doc: options.doc,
      extensions: [
        // Ahead of everything else: a `Mod-Enter` bound further down the
        // precedence chain would lose to any extension that also wants it.
        keymap.of([
          {
            key: 'Mod-Enter',
            run: () => {
              options.onRun()
              return true
            },
          },
        ]),
        history(),
        keymap.of(historyKeymap),
        cypherLanguage,
        cypherTheme,
        EditorView.lineWrapping,
      ],
    }),
  })

  return {
    value: () => view.state.doc.toString(),
    setValue: (text) => {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: text },
        // The caret goes to the end of what was just loaded, not to wherever
        // it happened to be in the query this one replaced.
        selection: { anchor: text.length },
      })
    },
    focus: () => view.focus(),
  }
}
