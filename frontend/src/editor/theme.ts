/**
 * The editor's colours, taken from the panel chrome rather than invented.
 *
 * Every value below is one the panels already use (`styles.css`): `#0d1117`
 * is the input background, `#30363d` the border, `#e6edf3` the text, `#8b949e`
 * the muted label, and the warn/error hues are the ones `.kglv-warn` and
 * `.kglv-error` carry. The editor sits inside a `.kglv-card`, so a theme with
 * its own palette would read as a widget dropped into somebody else's app.
 *
 * **One theme, because the chrome has one.** The panels are dark-only —
 * `styles.css` has no `prefers-color-scheme` block and no light palette — so a
 * light editor theme here would be a light box on a dark panel, which is worse
 * than no light mode at all. When the chrome grows a light palette this file
 * grows the matching half; until then a second theme would be untested code
 * claiming a capability the app does not have.
 */

import { HighlightStyle, syntaxHighlighting } from '@codemirror/language'
import { EditorView } from '@codemirror/view'
import { tags } from '@lezer/highlight'

/** Panel chrome, single-sourced here so the theme and the styles cannot drift. */
const CHROME = {
  background: '#0d1117',
  text: '#e6edf3',
  muted: '#8b949e',
  border: '#30363d',
  selection: '#1f6feb55',
  cursor: '#58a6ff',
  keyword: '#ff7b72',
  string: '#a5d6ff',
  number: '#79c0ff',
  comment: '#6e7681',
  label: '#7ee787',
  relationship: '#d2a8ff',
  property: '#ffa657',
  parameter: '#f0883e',
  error: '#f85149',
  warn: '#d29922',
} as const

const editorTheme = EditorView.theme(
  {
    '&': {
      background: CHROME.background,
      color: CHROME.text,
      border: `1px solid ${CHROME.border}`,
      borderRadius: '4px',
      fontSize: '12px',
    },
    // The editor grows with its content but stops: an unbounded editor pushes
    // the Run button off the bottom of a 340px panel, which is the one control
    // the card exists for.
    '.cm-scroller': {
      fontFamily: 'ui-monospace, SFMono-Regular, monospace',
      lineHeight: '1.5',
      maxHeight: '200px',
    },
    '.cm-content': { padding: '4px 0', caretColor: CHROME.cursor },
    '.cm-line': { padding: '0 6px' },
    '&.cm-focused': { outline: `1px solid ${CHROME.cursor}` },
    '.cm-cursor, .cm-dropCursor': { borderLeftColor: CHROME.cursor },
    '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection':
      { background: CHROME.selection },
    // Completion and diagnostic surfaces. They are `position: absolute`
    // children of the editor, so they inherit nothing from `.kglv-card`.
    '.cm-tooltip': {
      background: '#161b22',
      border: `1px solid ${CHROME.border}`,
      borderRadius: '4px',
      color: CHROME.text,
      fontFamily: 'ui-sans-serif, system-ui, sans-serif',
    },
    '.cm-tooltip.cm-tooltip-autocomplete > ul > li': { padding: '2px 6px' },
    '.cm-tooltip.cm-tooltip-autocomplete > ul > li[aria-selected]': {
      background: '#1f6feb',
      color: '#ffffff',
    },
    '.cm-completionDetail': { color: CHROME.muted, fontStyle: 'normal', marginLeft: '8px' },
    '.cm-completionIcon': { display: 'none' },
    '.cm-diagnostic': { padding: '4px 8px', borderLeftWidth: '4px' },
    '.cm-diagnostic-error': { borderLeftColor: CHROME.error },
    '.cm-diagnostic-warning': { borderLeftColor: CHROME.warn },
    '.cm-lintRange-error': { backgroundImage: 'none', textDecoration: `underline wavy ${CHROME.error}` },
    '.cm-lintRange-warning': { backgroundImage: 'none', textDecoration: `underline wavy ${CHROME.warn}` },
    '.cm-panels': { background: '#161b22', color: CHROME.text },
  },
  { dark: true },
)

/**
 * Tag → colour. The two schema tags are deliberately different hues: a node
 * label and a relationship type are the two halves of the meta-graph, and the
 * editor is where a user tells them apart before the engine does.
 */
const highlightStyle = HighlightStyle.define([
  { tag: tags.keyword, color: CHROME.keyword },
  { tag: tags.atom, color: CHROME.number },
  { tag: tags.string, color: CHROME.string },
  { tag: tags.number, color: CHROME.number },
  { tag: tags.comment, color: CHROME.comment, fontStyle: 'italic' },
  { tag: tags.typeName, color: CHROME.label },
  { tag: tags.labelName, color: CHROME.relationship },
  { tag: tags.propertyName, color: CHROME.property },
  { tag: tags.special(tags.variableName), color: CHROME.parameter },
  { tag: tags.variableName, color: CHROME.text },
  { tag: tags.operator, color: CHROME.muted },
  { tag: tags.punctuation, color: CHROME.muted },
])

export const cypherTheme = [editorTheme, syntaxHighlighting(highlightStyle)]
