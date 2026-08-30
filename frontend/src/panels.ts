/**
 * The side panels: Cypher, selection, search, appearance.
 *
 * Plain DOM, no framework, and — the rule that matters — **no graph data in
 * here** (plan D7). These panels hold at most a few hundred rows a human is
 * reading; nodes and edges stay in typed arrays on their way to the GPU. The
 * comparable-repo study found the opposite arrangement in every graph UI that
 * froze.
 *
 * The query editor is a plain `<textarea>` that upgrades itself. CodeMirror 6
 * arrives through a dynamic `import()` — its own chunk, fetched after first
 * paint — and replaces the textarea when it lands. Everything here talks to the
 * {@link QueryEditor} contract, which both satisfy, so the panel never learns
 * which one it has and the card keeps working when the chunk does not arrive.
 */

import { statLabel } from './appearance'
import type { Diagnostic } from './generated/Diagnostic'
import type { QueryEditor, SchemaSource } from './editor/contract'
import type { EdgeDirection } from './generated/EdgeDirection'
import type { LayoutKernel } from './generated/LayoutKernel'
import type { ExpansionPreview } from './generated/ExpansionPreview'
import type { NodeDetail } from './generated/NodeDetail'
import type { PropertyStat } from './generated/PropertyStat'
import type { PropertyStatsResponse } from './generated/PropertyStatsResponse'
import type { QueryTable } from './generated/QueryTable'
import type { SavedQueries } from './queries'
import type { SearchResponse } from './generated/SearchResponse'

/** What the panels ask the app to do. */
export type PanelHandlers = {
  runQuery(query: string, asGraph: boolean): void
  expand(
    slot: number,
    relationship: string,
    direction: EdgeDirection,
    limit: number | null,
  ): void
  collapse(slot: number): void
  search(query: string, nodeType: string | null): void
  loadHits(nodeIds: number[], nodeType: string | null): void
  focusSlot(slot: number): void
  setColorBy(property: string | null): void
  setSizeBy(property: string | null): void
  /**
   * Draw this type's nodes under a different property's value instead of the
   * title kglite chose (plan E11). `null` restores the title.
   */
  setCaptionBy(nodeType: string, property: string | null): void
  /**
   * Ask the server for an arrangement — or, with `simulation`, hand the
   * layout back to the viewer's GPU (plan E5).
   */
  setLayoutKernel(kernel: LayoutKernel): void
  /**
   * Hide everything on screen that does not match (plan E7). Client-only —
   * a term that would need a fetch is refused, not fetched.
   */
  setFilter(query: string): void
  /** Keep this query under this name. The store's ceilings answer as refusals. */
  saveQuery(name: string, query: string): void
  /** Forget a saved query. */
  deleteQuery(name: string): void
  /** Ask the server what is wrong with this query, without running it. */
  validateQuery(query: string): Promise<Diagnostic[]>
  /**
   * Show this type's on-screen nodes as a table of their properties (plan E9).
   *
   * The panel asks; the app generates the Cypher, puts it in the editor where
   * the user can read and edit it, and runs it down the ordinary bounded path.
   */
  showTypeTable(nodeType: string): void
}

function element<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag)
  if (className !== undefined) node.className = className
  if (text !== undefined) node.textContent = text
  return node
}

/** A number a human reads, grouped. */
function count(value: number): string {
  return value.toLocaleString('en-US')
}

/**
 * The layout picker's entries, in the order a user reads them.
 *
 * `simulation` is first and is the one the session starts in, so the list
 * reads as "the live one, then the ones that hold still".
 */
const LAYOUT_CHOICES: readonly (readonly [LayoutKernel, string])[] = [
  ['simulation', 'live force (GPU)'],
  ['auto', 'auto — from the structure'],
  ['radial', 'hop rings'],
  ['islands', 'packed islands'],
  ['force', 'force, held still'],
]

/**
 * The `geo` entry, added and removed as the view gains and loses nodes that
 * have somewhere to be.
 *
 * **Conditional rather than always present, and that is the same decision as
 * before with a new answer.** While the server refused `geo` outright, an entry
 * that always errored was worse than no entry; now that it computes, an entry
 * that errors *on this view* is still worse than no entry — so it appears
 * exactly when the view holds instances of a type with coordinates and
 * disappears when it does not. The condition is the app's, not a guess: it
 * reads the same `geo` / `loc` capability flags the meta-graph already sends
 * and the legend already explains.
 */
const GEO_CHOICE: readonly [LayoutKernel, string] = ['geo', 'map — where they are']

/**
 * What the live map is and is not, said where the choice is made.
 *
 * cosmos.gl draws points and links and has no background layer, so the live
 * view gets positions and nothing else — no coastline, no graticule. Saying so
 * here is cheaper than a user concluding the map is broken, and the render is
 * one sentence away.
 */
const GEO_HINT = 'positions only — render for the map picture'

/**
 * What the filter box is for, said where the two boxes sit together.
 *
 * One constant because it is written twice — once at construction and once
 * when a filter is cleared — and two copies of the sentence that keeps Search
 * and Filter apart is one copy that stops saying it.
 */
const FILTER_HINT =
  'hides what is already loaded — nothing is fetched. Try "type:Wellbore", or a property ' +
  'you are colouring or sizing by. Use Search above to bring nodes in.'

export class Panels {
  readonly root: HTMLDivElement
  private readonly queryInput: HTMLTextAreaElement
  /** Where the CodeMirror view mounts, and what holds the textarea until it does. */
  private readonly queryHost: HTMLDivElement
  private readonly editorNote: HTMLDivElement
  /** kglite's parse-only findings, listed under the editor. */
  private readonly editorDiagnostics: HTMLDivElement
  /** The textarea, or CodeMirror once its chunk has landed. Never null. */
  private editor: QueryEditor
  private readonly queryAsGraph: HTMLInputElement
  private readonly queryStatus: HTMLDivElement
  private readonly queryDiagnostics: HTMLDivElement
  private readonly queryResults: HTMLDivElement
  /** What a generated table left out, when it left anything out. */
  private readonly tableNote: HTMLDivElement
  private readonly selection: HTMLDivElement
  private readonly expandLimit: HTMLInputElement
  private readonly searchInput: HTMLInputElement
  private readonly searchType: HTMLSelectElement
  private readonly searchResults: HTMLDivElement
  private readonly filterInput: HTMLInputElement
  private readonly filterNote: HTMLDivElement
  private readonly colorBy: HTMLSelectElement
  private readonly sizeBy: HTMLSelectElement
  private readonly captionBy: HTMLSelectElement
  private readonly captionRow: HTMLElement
  private readonly appearanceNote: HTMLDivElement
  /** The "table of the N on screen" action, and the row it lives in. */
  private readonly tableButton: HTMLButtonElement
  private readonly tableRow: HTMLElement
  /** Instance nodes on screen, by type — what the table button counts. */
  private instanceCounts = new Map<string, number>()
  /** The type the three per-type channels are currently describing. */
  private statsType: string | null = null
  private readonly layoutKernel: HTMLSelectElement
  /** The picker's whole row, so `?deterministic=1` can remove it. */
  private readonly layoutRow: HTMLElement
  private readonly layoutNote: HTMLDivElement
  private readonly savedList: HTMLSelectElement
  private readonly savedNote: HTMLDivElement
  private readonly historyList: HTMLDivElement
  private lastHits: SearchResponse | null = null
  private lastSaved: SavedQueries | null = null
  /**
   * The rows currently on screen, kept so a header click can re-order them
   * without re-running the query. Null until a result arrives, and replaced
   * whole by the next one.
   */
  private lastTable: QueryTable | null = null
  /** The column the grid is sorted by, or null for the engine's own order. */
  private sortBy: { column: number; descending: boolean } | null = null

  constructor(
    container: HTMLElement,
    private readonly handlers: PanelHandlers,
    private readonly schema: SchemaSource,
  ) {
    this.root = element('div', 'kglv-panels')
    container.appendChild(this.root)

    // ── selection ─────────────────────────────────────────────────────────
    this.selection = element('div', 'kglv-card kglv-selection')
    this.selection.appendChild(
      element('div', 'kglv-hint', 'Click a node to see what expanding it would add.'),
    )
    // The bound, as a number the user can choose. The preview above it says how
    // many edges there are; this says how many nodes to take. Leaving it blank
    // takes the server's default, and whatever is typed is still clamped in
    // core (D5) — the field asks for less, it can never ask for more.
    this.expandLimit = element('input', 'kglv-input')
    this.expandLimit.type = 'number'
    this.expandLimit.min = '1'
    this.expandLimit.placeholder = 'server default'
    this.expandLimit.setAttribute('data-testid', 'expand-limit')
    const limitRow = this.labelled('max nodes', this.expandLimit)
    this.root.appendChild(this.section('Selection', this.selection))
    this.selection.parentElement?.appendChild(limitRow)

    // ── search ────────────────────────────────────────────────────────────
    const searchBox = element('div', 'kglv-card')
    const searchRow = element('div', 'kglv-row')
    this.searchInput = element('input', 'kglv-input')
    this.searchInput.placeholder = 'title contains…'
    this.searchInput.setAttribute('data-testid', 'search-input')
    this.searchType = element('select', 'kglv-select')
    this.searchType.setAttribute('data-testid', 'search-type')
    const searchButton = element('button', 'kglv-button', 'Search')
    searchButton.setAttribute('data-testid', 'search-run')
    searchButton.addEventListener('click', () => this.emitSearch())
    this.searchInput.addEventListener('keydown', (event) => {
      if (event.key === 'Enter') this.emitSearch()
    })
    searchRow.append(this.searchInput, this.searchType, searchButton)
    this.searchResults = element('div', 'kglv-results')
    searchBox.append(searchRow, this.searchResults)
    this.root.appendChild(this.section('Search', searchBox))

    // ── filter ────────────────────────────────────────────────────────────
    // Its own card, directly under Search, and the hint under the box exists
    // to keep them apart: they are the two boxes you type a name into, and one
    // brings nodes IN while the other takes them off the screen. A user who
    // confuses them either loses their view or wonders why nothing arrived.
    const filterBox = element('div', 'kglv-card')
    const filterRow = element('div', 'kglv-row')
    this.filterInput = element('input', 'kglv-input')
    this.filterInput.placeholder = 'hide all but…'
    this.filterInput.setAttribute('data-testid', 'filter-input')
    this.filterInput.addEventListener('input', () =>
      this.handlers.setFilter(this.filterInput.value),
    )
    const clear = element('button', 'kglv-button kglv-button-small', 'clear')
    clear.setAttribute('data-testid', 'filter-clear')
    clear.addEventListener('click', () => {
      this.filterInput.value = ''
      this.handlers.setFilter('')
    })
    filterRow.append(this.filterInput, clear)
    this.filterNote = element('div', 'kglv-hint')
    this.filterNote.setAttribute('data-testid', 'filter-note')
    this.filterNote.textContent = FILTER_HINT
    filterBox.append(filterRow, this.filterNote)
    this.root.appendChild(this.section('Filter', filterBox))

    // ── appearance ────────────────────────────────────────────────────────
    const appearance = element('div', 'kglv-card')
    this.colorBy = element('select', 'kglv-select')
    this.colorBy.setAttribute('data-testid', 'color-by')
    this.sizeBy = element('select', 'kglv-select')
    this.sizeBy.setAttribute('data-testid', 'size-by')
    this.colorBy.addEventListener('change', () =>
      this.handlers.setColorBy(this.colorBy.value === '' ? null : this.colorBy.value),
    )
    this.sizeBy.addEventListener('change', () =>
      this.handlers.setSizeBy(this.sizeBy.value === '' ? null : this.sizeBy.value),
    )
    // The third per-type display channel, beside the two it belongs with.
    // The plan called it "the type panel"; this card IS that panel for these
    // channels — its dropdowns are filled from the selected type's property
    // statistics — and splitting one type's three display choices across two
    // cards would make the third look like it was about something else.
    this.captionBy = element('select', 'kglv-select')
    this.captionBy.setAttribute('data-testid', 'caption-by')
    this.captionBy.addEventListener('change', () => {
      if (this.statsType === null) return
      this.handlers.setCaptionBy(
        this.statsType,
        this.captionBy.value === '' ? null : this.captionBy.value,
      )
    })
    this.captionRow = this.labelled('caption by', this.captionBy)
    // The fourth thing this card does with a type, and the one that is not a
    // display channel: read its nodes as rows. It sits here because this card
    // IS the type panel — its contents are the selected type's property
    // statistics — and the table's columns come from exactly those statistics.
    this.tableButton = element('button', 'kglv-button kglv-button-small', 'table')
    this.tableButton.setAttribute('data-testid', 'type-table')
    this.tableButton.addEventListener('click', () => {
      if (this.statsType !== null) this.handlers.showTypeTable(this.statsType)
    })
    this.tableRow = this.labelled('rows', this.tableButton)
    this.tableRow.hidden = true
    appearance.append(
      this.labelled('colour by', this.colorBy),
      this.labelled('size by', this.sizeBy),
      this.captionRow,
      this.tableRow,
    )
    this.appearanceNote = element('div', 'kglv-hint')
    this.appearanceNote.setAttribute('data-testid', 'appearance-note')
    appearance.appendChild(this.appearanceNote)

    // ── layout ────────────────────────────────────────────────────────────
    // In the appearance card rather than a card of its own: colour, size and
    // arrangement are the three channels that change how the same graph looks
    // without changing what is in it, and a fourth heading for one select is a
    // sidebar that scrolls.
    this.layoutKernel = element('select', 'kglv-select')
    this.layoutKernel.setAttribute('data-testid', 'layout-kernel')
    for (const [value, label] of LAYOUT_CHOICES) {
      const option = element('option', undefined, label)
      option.value = value
      this.layoutKernel.appendChild(option)
    }
    this.layoutKernel.addEventListener('change', () =>
      this.handlers.setLayoutKernel(this.layoutKernel.value as LayoutKernel),
    )
    this.layoutRow = this.labelled('layout', this.layoutKernel)
    this.layoutNote = element('div', 'kglv-hint')
    this.layoutNote.setAttribute('data-testid', 'layout-note')
    appearance.append(this.layoutRow, this.layoutNote)

    this.root.appendChild(this.section('Appearance', appearance))

    // ── cypher ────────────────────────────────────────────────────────────
    const query = element('div', 'kglv-card')
    this.queryHost = element('div', 'kglv-editor')
    this.queryHost.setAttribute('data-testid', 'query-editor')
    this.queryInput = element('textarea', 'kglv-textarea')
    this.queryInput.setAttribute('data-testid', 'query-input')
    this.queryInput.rows = 4
    this.queryInput.spellcheck = false
    this.queryInput.value = 'MATCH (p:Person)-[:WORKS_AT]->(c:Company)\nRETURN c.title AS company, count(p) AS staff\nORDER BY staff DESC'
    this.queryHost.appendChild(this.queryInput)
    this.editor = {
      value: () => this.queryInput.value,
      setValue: (text) => {
        this.queryInput.value = text
      },
      focus: () => this.queryInput.focus(),
    }
    this.editorNote = element('div', 'kglv-hint')
    this.editorNote.setAttribute('data-testid', 'editor-note')
    // Under the editor and above the Run button, because it is about the query
    // that has not run yet — the block below the button is about the one that
    // has.
    this.editorDiagnostics = element('div', 'kglv-diagnostics')
    this.editorDiagnostics.setAttribute('data-testid', 'editor-diagnostics')
    const queryRow = element('div', 'kglv-row')
    const run = element('button', 'kglv-button', 'Run')
    run.setAttribute('data-testid', 'query-run')
    run.addEventListener('click', () => this.runCurrentQuery())
    this.queryAsGraph = element('input')
    this.queryAsGraph.type = 'checkbox'
    this.queryAsGraph.setAttribute('data-testid', 'query-as-graph')
    const asGraphLabel = element('label', 'kglv-checkbox')
    asGraphLabel.append(this.queryAsGraph, document.createTextNode(' show in graph'))
    // Ctrl/Cmd+Enter runs, because a multi-line editor cannot use Enter alone.
    // The CodeMirror half binds the same chord as a keymap; this one covers the
    // textarea, which is what a user types into until the chunk lands.
    this.queryInput.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) this.runCurrentQuery()
    })

    // ── saved queries + recent ────────────────────────────────────────────
    // Above the results and below the run row, because they are things to put
    // INTO the editor: reading down the card is write, run, read.
    this.savedList = element('select', 'kglv-select')
    this.savedList.setAttribute('data-testid', 'saved-list')
    this.savedList.addEventListener('change', () => this.loadSaved())
    const save = element('button', 'kglv-button kglv-button-small', 'save')
    save.setAttribute('data-testid', 'query-save')
    save.addEventListener('click', () => this.emitSave())
    const forget = element('button', 'kglv-button kglv-button-small', 'forget')
    forget.setAttribute('data-testid', 'query-delete')
    forget.addEventListener('click', () => {
      if (this.savedList.value !== '') this.handlers.deleteQuery(this.savedList.value)
    })
    // Its own row, not appended to the run row: five controls on one line left
    // the picker too narrow to read a query's name in, which is the one thing
    // the picker is for.
    const savedRow = element('div', 'kglv-row')
    savedRow.append(this.savedList, save, forget)
    queryRow.append(run, asGraphLabel)

    this.savedNote = element('div', 'kglv-hint')
    this.savedNote.setAttribute('data-testid', 'saved-note')
    this.historyList = element('div', 'kglv-history')
    this.historyList.setAttribute('data-testid', 'query-history')

    this.queryStatus = element('div', 'kglv-hint')
    this.queryStatus.setAttribute('data-testid', 'query-status')
    // What a *generated* query left out, said above the rows it produced.
    // Its own line rather than folded into the status: the status describes
    // the response bound, and a column cap is a different kind of clipping —
    // conflating them would make "12 of 40" ambiguous between rows and
    // columns.
    this.tableNote = element('div', 'kglv-hint')
    this.tableNote.setAttribute('data-testid', 'table-note')
    this.tableNote.hidden = true
    // The engine's own diagnostics, between the count and the rows: a warning
    // printed under the table is a warning read after the conclusion was drawn.
    this.queryDiagnostics = element('div', 'kglv-diagnostics')
    this.queryDiagnostics.setAttribute('data-testid', 'query-diagnostics')
    this.queryResults = element('div', 'kglv-results')
    query.append(
      this.queryHost,
      this.editorNote,
      this.editorDiagnostics,
      queryRow,
      savedRow,
      this.savedNote,
      this.historyList,
      this.queryStatus,
      this.queryDiagnostics,
      this.tableNote,
      this.queryResults,
    )
    this.root.appendChild(this.section('Cypher', query))

    void this.upgradeEditor()
  }

  /**
   * List what the engine said about the query that has not run yet.
   *
   * The editor underlines each finding where kglite positioned it, and the
   * message lives in a hover tooltip — a place a user has to already suspect
   * something to look. In a 340px column that is easy to miss entirely, so the
   * same findings are also read out here, in the same warn/error voice the
   * post-run diagnostics use.
   */
  private showEditorDiagnostics(found: Diagnostic[]): void {
    this.editorDiagnostics.replaceChildren()
    for (const one of found) {
      const line = element(
        'div',
        one.severity === 'error' ? 'kglv-hint kglv-error' : 'kglv-hint kglv-warn',
      )
      line.setAttribute('data-testid', `editor-${one.severity}`)
      // kglite's own wording, first line only: the engine renders its own ASCII
      // caret under the offending line, and the editor is already drawing that
      // caret as an underline. The whole message is in the hover tooltip.
      line.textContent = one.message.split('\n')[0] ?? one.message
      this.editorDiagnostics.appendChild(line)
    }
  }

  /** Run whatever is in the editor — the one path both the button and the chord take. */
  private runCurrentQuery(): void {
    this.handlers.runQuery(this.editor.value(), this.queryAsGraph.checked)
  }

  /**
   * Fetch the CodeMirror chunk and, if it arrives, swap the textarea out.
   *
   * The failure branch is the point. A dynamic import can fail for reasons that
   * have nothing to do with this code — a chunk that did not get embedded, a
   * proxy that rewrote the path, a browser that refused the module — and the
   * silent version of that is a query box that is *quietly* worse than the one
   * the last release shipped. So the note says which editor is on screen, and
   * the textarea it says that about is still fully working.
   */
  private async upgradeEditor(): Promise<void> {
    try {
      const { mountCypherEditor } = await import('./editor')
      const text = this.queryInput.value
      this.queryInput.remove()
      this.editor = mountCypherEditor({
        parent: this.queryHost,
        doc: text,
        onRun: () => this.runCurrentQuery(),
        schema: this.schema,
        validate: (query) => this.handlers.validateQuery(query),
        onDiagnostics: (found) => this.showEditorDiagnostics(found),
      })
      this.editorNote.remove()
    } catch (err) {
      this.editorNote.className = 'kglv-hint kglv-warn'
      this.editorNote.textContent = `plain text box — the syntax editor did not load (${
        err instanceof Error ? err.message : String(err)
      })`
    }
  }

  /**
   * Put another card in the sidebar, under its own heading.
   *
   * The path builder (plan E9) is a card this class does not own: it holds a
   * spec, a generator and a probe queue, none of which belong in a file whose
   * rule is "no graph data in here". So it builds its own DOM and asks for a
   * place to stand — which keeps the section chrome in one place without making
   * this class a second home for it.
   */
  addSection(title: string, body: HTMLElement): void {
    this.root.appendChild(this.section(title, body))
  }

  private section(title: string, body: HTMLElement): HTMLElement {
    const wrapper = element('section', 'kglv-section')
    wrapper.append(element('h2', 'kglv-section-title', title), body)
    return wrapper
  }

  private labelled(text: string, control: HTMLElement): HTMLElement {
    const row = element('label', 'kglv-field')
    row.append(element('span', 'kglv-field-label', text), control)
    return row
  }

  /** The `max nodes` box, or null for the server's default. */
  private requestedLimit(): number | null {
    const value = Number.parseInt(this.expandLimit.value, 10)
    return Number.isFinite(value) && value > 0 ? value : null
  }

  private emitSearch(): void {
    const type = this.searchType.value === '' ? null : this.searchType.value
    this.handlers.search(this.searchInput.value, type)
  }

  /** The query text currently in the editor. */
  queryText(): string {
    return this.editor.value()
  }

  /**
   * Put a query in the editor without running it.
   *
   * Loading and running are deliberately two actions. A saved query is
   * somebody's Cypher against a graph that may have moved on since; showing it
   * first is what lets them read it before it executes, and it keeps one Run
   * button as the only thing that runs anything.
   */
  private setQueryText(query: string): void {
    this.editor.setValue(query)
    this.editor.focus()
  }

  /**
   * Put a generated query where the user can read it (plan E9).
   *
   * The teaching-tool rule: a table or a path this app built is a *query*, and
   * showing it in the same box the user types into is what makes it something
   * they can learn from, edit and re-run rather than a button whose workings
   * are ours. Public, unlike {@link setQueryText}, because the caller is the
   * app rather than this panel.
   */
  loadGeneratedQuery(query: string): void {
    this.setQueryText(query)
  }

  /**
   * How many instance nodes of each type are on screen.
   *
   * Refreshed wherever the view changes rather than when statistics arrive: an
   * expansion moves the count without moving the type, and a "table of 40" on a
   * view holding 4 000 would be describing the moment the panel was last
   * opened.
   */
  setInstanceCounts(counts: Map<string, number>): void {
    this.instanceCounts = counts
    this.refreshTableAction()
  }

  /**
   * Offer the table exactly while there is something to put in it.
   *
   * A type with no instances loaded has no rows — the query would be
   * `id(n) IN []` — so the action is hidden rather than left to produce an
   * empty grid the user would read as "this type has no properties".
   */
  private refreshTableAction(): void {
    const loaded =
      this.statsType === null ? 0 : (this.instanceCounts.get(this.statsType) ?? 0)
    this.tableRow.hidden = loaded === 0
    this.tableButton.textContent = `table of ${count(loaded)} on screen`
  }

  private loadSaved(): void {
    const chosen = this.lastSaved?.saved.find((saved) => saved.name === this.savedList.value)
    if (chosen !== undefined) this.setQueryText(chosen.query)
  }

  private emitSave(): void {
    const query = this.editor.value().trim()
    if (query === '') return
    // The selected name is the default, so re-saving an edited query is one
    // click; a fresh name is one word.
    const suggested = this.savedList.value
    const name = window.prompt('Save this query as:', suggested)
    if (name === null || name.trim() === '') return
    this.handlers.saveQuery(name.trim(), query)
  }

  /**
   * Draw the store: saved queries in a picker, recent runs as a list.
   *
   * Called after every read AND every write, because the store is the server's
   * and a `curl`, another tab or an agent's `run_saved_query` may have moved it
   * since. Nothing here is authoritative — this renders what the server said.
   */
  showSavedQueries(store: SavedQueries): void {
    this.lastSaved = store
    const selected = this.savedList.value

    this.savedList.replaceChildren()
    const blank = element('option', undefined, `saved (${store.saved.length})`)
    blank.value = ''
    this.savedList.appendChild(blank)
    for (const saved of store.saved) {
      const option = element('option', undefined, saved.name)
      option.value = saved.name
      this.savedList.appendChild(option)
    }
    // Keep the user's selection across a refresh they did not ask for — unless
    // what they had selected is what just got deleted.
    this.savedList.value = store.saved.some((saved) => saved.name === selected) ? selected : ''

    this.savedNote.className = 'kglv-hint'
    this.savedNote.textContent =
      store.store === null
        ? 'saved queries are unavailable: this machine has no config directory'
        : `${store.saved.length} of ${store.max_saved} saved for ${store.graph_label}`

    this.historyList.replaceChildren()
    for (const entry of store.history) {
      const row = element('button', 'kglv-hit')
      // One line, whatever the query's own line breaks say: this is a list to
      // pick from, not a place to read Cypher.
      row.textContent = entry.query.replace(/\s+/g, ' ').slice(0, 120)
      row.title = entry.query
      row.addEventListener('click', () => this.setQueryText(entry.query))
      this.historyList.appendChild(row)
    }
  }

  /**
   * What the filter is doing, in the truncation banner's voice.
   *
   * `refused` is the honest half: a term naming a property this client has not
   * loaded cannot be answered without a fetch, and a filter that quietly
   * dropped the term would be filtering on less than the user typed while
   * looking like it worked.
   */
  showFilterState(line: string | null, refused: string[]): void {
    if (refused.length > 0) {
      this.filterNote.className = 'kglv-hint kglv-error'
      this.filterNote.textContent =
        `nothing loaded carries ${refused.map((key) => `"${key}"`).join(', ')} — this box only ` +
        'reads values already on screen. Colour or size by it first, or use Search above to ' +
        'ask the server for it.'
      return
    }
    if (line === null) {
      this.filterNote.className = 'kglv-hint'
      this.filterNote.textContent = FILTER_HINT
      return
    }
    this.filterNote.className = 'kglv-hint kglv-warn'
    this.filterNote.textContent = line
  }

  /** A store refusal, in the store's own words. */
  showQueriesError(message: string): void {
    this.savedNote.className = 'kglv-hint kglv-error'
    this.savedNote.textContent = message
  }

  /** Populate the type filter from the meta-graph. */
  setNodeTypes(types: string[]): void {
    this.searchType.replaceChildren()
    const any = element('option', undefined, 'any type')
    any.value = ''
    this.searchType.appendChild(any)
    for (const type of types) {
      const option = element('option', undefined, type)
      option.value = type
      this.searchType.appendChild(option)
    }
  }

  /** The expansion preview — counts BEFORE anything is fetched (plan D12). */
  showPreview(preview: ExpansionPreview, detail: NodeDetail | null): number {
    this.selection.replaceChildren()

    const heading = element('div', 'kglv-selection-title')
    heading.setAttribute('data-testid', 'selection-title')
    heading.textContent =
      preview.scope === 'type'
        ? `${preview.node_type} (type)`
        : `${preview.title || `${preview.node_type} node`} — ${preview.node_type}`
    this.selection.appendChild(heading)

    if (detail !== null && detail.properties.length > 0) {
      const list = element('dl', 'kglv-props')
      list.setAttribute('data-testid', 'node-properties')
      for (const [key, value] of detail.properties) {
        const dt = element('dt', undefined, key)
        const dd = element('dd', undefined, formatCell(value))
        list.append(dt, dd)
      }
      this.selection.appendChild(list)
    }

    const summary = element('div', 'kglv-hint')
    summary.setAttribute('data-testid', 'preview-summary')
    summary.textContent =
      preview.relationships.length === 0
        ? 'no relationships to expand'
        : `${count(preview.total_edges)} edges across ${preview.relationships.length} relationship${
            preview.relationships.length === 1 ? '' : 's'
          } — up to ${count(preview.max_nodes)} nodes per expansion`
    this.selection.appendChild(summary)

    const table = element('div', 'kglv-preview')
    table.setAttribute('data-testid', 'preview-rows')
    for (const relationship of preview.relationships) {
      const row = element('div', 'kglv-preview-row')
      const arrow = relationship.direction === 'out' ? '→' : '←'
      row.appendChild(
        element(
          'span',
          'kglv-preview-name',
          `${relationship.name} ${arrow} ${relationship.other_type}`,
        ),
      )
      row.appendChild(element('span', 'kglv-preview-count', count(relationship.count)))
      const button = element('button', 'kglv-button kglv-button-small', 'expand')
      button.setAttribute(
        'data-testid',
        `expand-${relationship.name}-${relationship.direction}`,
      )
      button.addEventListener('click', () =>
        this.handlers.expand(
          preview.slot,
          relationship.name,
          relationship.direction,
          this.requestedLimit(),
        ),
      )
      row.appendChild(button)
      table.appendChild(row)
    }
    this.selection.appendChild(table)

    const collapse = element('button', 'kglv-button kglv-button-small', 'collapse')
    collapse.setAttribute('data-testid', 'collapse')
    collapse.addEventListener('click', () => this.handlers.collapse(preview.slot))
    this.selection.appendChild(collapse)

    return preview.relationships.length
  }

  clearSelection(): void {
    this.selection.replaceChildren(
      element('div', 'kglv-hint', 'Click a node to see what expanding it would add.'),
    )
  }

  /**
   * What the engine said about the query, beside what it answered.
   *
   * Two separate facts, and neither is the response bound: `truncated` means
   * "there is more of this answer", `timed_out` means "this is not the answer",
   * and a warning means "the answer may not be to the question you typed". The
   * last one is the reason this block exists at all — kglite's unknown-label
   * advisory used to reach only the *server's* stderr, so a mistyped label
   * rendered as an empty result and read as an empty graph.
   */
  private showQueryDiagnostics(table: QueryTable): void {
    this.queryDiagnostics.replaceChildren()
    if (table.timed_out) {
      const line = element(
        'div',
        'kglv-hint kglv-error',
        'the engine cancelled this query at its time limit — the rows below are ' +
          'a partial answer, not a short one',
      )
      line.setAttribute('data-testid', 'query-timed-out')
      this.queryDiagnostics.appendChild(line)
    }
    for (const warning of table.warnings) {
      // kglite's own wording, verbatim: it carries the offending name and a
      // "did you mean?" hint, which is the whole of what the user can act on.
      const line = element('div', 'kglv-hint kglv-warn', warning)
      line.setAttribute('data-testid', 'query-warning')
      this.queryDiagnostics.appendChild(line)
    }
  }

  /**
   * An `EXPLAIN` plan, drawn as a plan rather than as three columns of data.
   *
   * The rows already arrive — `explain` has been on `QueryTable` since the flag
   * existed — and the generic table renders them truthfully and unreadably: a
   * `step` column of 1..n beside an `operation` column is a numbered list
   * wearing a grid, and `estimated_rows` reads as a value of the row rather
   * than as the planner's guess about it. Monospace, the step in the gutter,
   * the estimate on the right, so the shape of the plan is the shape on screen.
   *
   * **The rows are a plan, not data**, which is also why nothing here offers
   * "show in graph" and why the status line below says the query did not run.
   */
  private showExplainPlan(table: QueryTable, rows: number): void {
    const column = (name: string): unknown[] => table.data[table.columns.indexOf(name)] ?? []
    const steps = column('step')
    const operations = column('operation')
    const estimates = column('estimated_rows')

    const plan = element('div', 'kglv-plan')
    plan.setAttribute('data-testid', 'query-plan')
    for (let row = 0; row < rows; row += 1) {
      const line = element('div', 'kglv-plan-row')
      line.append(
        element('span', 'kglv-plan-step', formatCell(steps[row])),
        element('span', 'kglv-plan-op', formatCell(operations[row])),
      )
      // Only where the planner produced one. A blank cell is the honest
      // rendering of "no estimate"; a `0` would be a number it never gave.
      const estimate = estimates[row]
      if (typeof estimate === 'number') {
        line.appendChild(element('span', 'kglv-plan-rows', `~${count(estimate)}`))
      }
      plan.appendChild(line)
    }
    this.queryResults.appendChild(plan)
  }

  /**
   * True when this result is a plan *and* has the shape this panel can draw.
   *
   * The column check is not belt-and-braces: an engine that reshapes `EXPLAIN`
   * would otherwise get a plan panel of empty rows, and an unreadable table is
   * a better failure than a confident blank one.
   */
  private static isDrawablePlan(table: QueryTable): boolean {
    return table.explain && table.columns.includes('operation')
  }

  /** The results table. Rows are already bounded by the server (D5). */
  showQueryTable(table: QueryTable): number {
    this.queryResults.replaceChildren()
    this.showQueryDiagnostics(table)
    const rows = table.data[0]?.length ?? 0

    if (Panels.isDrawablePlan(table)) {
      // No bound wording: `EXPLAIN` is exempt from the row cap in the engine,
      // so "n of m" would be describing a ceiling that did not apply. And no
      // elapsed time worth reading — the query was planned, not run.
      this.queryStatus.className = 'kglv-hint'
      this.queryStatus.textContent = `query plan — ${count(rows)} step${
        rows === 1 ? '' : 's'
      }, not executed`
      this.showExplainPlan(table, rows)
      return rows
    }

    const bound = table.bound.truncated
      ? `showing ${count(table.bound.returned)} of ${count(table.bound.total)} rows`
      : `${count(table.bound.total)} row${table.bound.total === 1 ? '' : 's'}`
    this.queryStatus.className = table.bound.truncated ? 'kglv-hint kglv-warn' : 'kglv-hint'
    this.queryStatus.textContent = `${bound} in ${count(table.elapsed_ms)} ms`

    this.lastTable = table
    this.sortBy = null
    this.drawGrid()
    return rows
  }

  /**
   * Draw the results grid, in whatever order {@link sortBy} says.
   *
   * Redrawn rather than re-fetched: sorting is a way of reading rows that have
   * already arrived, and asking the server for an `ORDER BY` would turn a
   * column click into a round trip that re-runs the query — against a graph
   * that may have moved, under a bound that may cut a different subset. The
   * rows on screen stay the rows the status line above them describes.
   */
  private drawGrid(): void {
    const table = this.lastTable
    if (table === null) return
    const rows = table.data[0]?.length ?? 0
    const order = this.rowOrder(table, rows)

    const grid = element('table', 'kglv-table')
    grid.setAttribute('data-testid', 'query-table')
    const head = element('tr')
    table.columns.forEach((column, index) => {
      const cell = element('th')
      const button = element('button', 'kglv-th-sort')
      button.setAttribute('data-testid', `sort-${column}`)
      const active = this.sortBy?.column === index
      button.textContent = active ? `${column} ${this.sortBy?.descending ? '▾' : '▴'}` : column
      button.addEventListener('click', () => {
        // First click on a new column sorts ascending; clicking the active one
        // flips it. Never a third state that clears the sort — a user who
        // wanted the original order clicked a column by mistake, and re-running
        // the query is a worse answer than one more click.
        this.sortBy =
          active && this.sortBy !== null
            ? { column: index, descending: !this.sortBy.descending }
            : { column: index, descending: false }
        this.drawGrid()
      })
      cell.appendChild(button)
      head.appendChild(cell)
    })
    grid.appendChild(head)

    for (const row of order) {
      const tr = element('tr')
      for (let column = 0; column < table.columns.length; column += 1) {
        tr.appendChild(element('td', undefined, formatCell(table.data[column]?.[row])))
      }
      grid.appendChild(tr)
    }
    this.queryResults.replaceChildren(grid)
  }

  /**
   * Row indices in display order.
   *
   * **Stable, and typed per column.** Stable because two rows the sort cannot
   * separate must keep the order the engine returned — an unstable sort makes a
   * table shuffle under a second click on the same header, which reads as data
   * changing. Typed because `10` and `9` compare one way as numbers and the
   * other as strings, and a column of counts sorted lexically is a column
   * sorted wrong; the type is read off the values rather than off the column
   * name, since a `RETURN` can name anything. A column that mixes numbers and
   * text is sorted as text, which is the only comparison both halves answer to.
   *
   * Empty cells sort last in both directions. They are not a value, so ranking
   * them as one would put a block of blanks at the top of a descending sort.
   */
  private rowOrder(table: QueryTable, rows: number): number[] {
    const order = [...Array(rows).keys()]
    const sort = this.sortBy
    if (sort === null) return order
    const values = table.data[sort.column] ?? []
    const numeric = values.every((value) => value === null || typeof value === 'number')
    const sign = sort.descending ? -1 : 1
    return order.sort((a, b) => {
      const left = values[a]
      const right = values[b]
      const leftEmpty = left === null || left === undefined
      const rightEmpty = right === null || right === undefined
      if (leftEmpty || rightEmpty) {
        // Not multiplied by `sign`: blanks are last whichever way the column is
        // pointing.
        if (leftEmpty && rightEmpty) return a - b
        return leftEmpty ? 1 : -1
      }
      const compared = numeric
        ? (left as number) - (right as number)
        : formatCell(left).localeCompare(formatCell(right))
      // The index tiebreak is what makes this stable across engines: Array
      // .prototype.sort is specified stable, but a comparator that returns 0
      // for two rows still lets a *different* column's later sort reorder them.
      return compared === 0 ? a - b : compared * sign
    })
  }

  /**
   * A query that answered with a *graph* rather than a table.
   *
   * Without this the results card kept the previous query's status line —
   * "120 rows in 10 ms" sitting under a run that returned no rows at all,
   * which reads as the row count of the thing that just happened. Found on
   * sodir in G5, running a path with "show in graph" after a table.
   *
   * Both numbers, because they differ and the difference is the interesting
   * part: a result whose nodes were all already on screen adds nothing, and a
   * bare "0" would read as a query that matched nothing.
   */
  showGraphResult(inResult: number, added: number): void {
    this.queryResults.replaceChildren()
    this.queryDiagnostics.replaceChildren()
    this.showTableNote(null)
    this.lastTable = null
    this.sortBy = null
    this.queryStatus.className = 'kglv-hint'
    this.queryStatus.textContent =
      `${count(inResult)} node${inResult === 1 ? '' : 's'} in the result — ` +
      `${count(added)} new on screen`
  }

  showQueryError(message: string): void {
    this.queryResults.replaceChildren()
    // The failed query's predecessor is not this query's result. Left in place,
    // a header click on the old grid would re-draw rows for a question that is
    // no longer on screen.
    this.lastTable = null
    this.sortBy = null
    // The previous run's advisories described the previous query. Left up, they
    // would read as an explanation of this failure.
    this.queryDiagnostics.replaceChildren()
    this.queryStatus.className = 'kglv-hint kglv-error'
    // kglite's own diagnostic, verbatim: position, expected token, the schema
    // name it could not resolve. A friendlier summary would delete the only
    // part the user can act on.
    this.queryStatus.textContent = message
  }

  /**
   * Say what a generated query is not showing, or clear the line.
   *
   * Called before the query runs, so the caveat is on screen with the rows
   * rather than after the user has read them.
   */
  showTableNote(note: string | null): void {
    this.tableNote.hidden = note === null
    this.tableNote.className = note === null ? 'kglv-hint' : 'kglv-hint kglv-warn'
    this.tableNote.textContent = note ?? ''
  }

  /** Search hits: already-loaded ones focus, cold ones load into the view. */
  showSearch(response: SearchResponse): number {
    this.lastHits = response
    this.searchResults.replaceChildren()

    const status = element('div', response.bound.truncated ? 'kglv-hint kglv-warn' : 'kglv-hint')
    status.setAttribute('data-testid', 'search-status')
    status.textContent = response.bound.truncated
      ? `showing ${count(response.bound.returned)} of ${count(response.bound.total)} hits on ${response.property}`
      : `${count(response.bound.returned)} hit${response.bound.returned === 1 ? '' : 's'} on ${response.property}`
    this.searchResults.appendChild(status)

    const cold = response.hits.filter((hit) => hit.slot === null)
    if (cold.length > 0) {
      const load = element(
        'button',
        'kglv-button kglv-button-small',
        `load ${count(cold.length)} into view`,
      )
      load.setAttribute('data-testid', 'search-load')
      load.addEventListener('click', () =>
        this.handlers.loadHits(
          cold.map((hit) => hit.node_id),
          response.node_type,
        ),
      )
      this.searchResults.appendChild(load)
    }

    const list = element('div', 'kglv-hits')
    list.setAttribute('data-testid', 'search-hits')
    for (const hit of response.hits) {
      const row = element('button', 'kglv-hit')
      row.textContent = `${hit.label} · ${hit.node_type}${hit.slot === null ? ' (not loaded)' : ''}`
      if (hit.slot !== null) {
        const slot = hit.slot
        row.addEventListener('click', () => this.handlers.focusSlot(slot))
      } else {
        row.disabled = true
      }
      list.appendChild(row)
    }
    this.searchResults.appendChild(list)
    return response.hits.length
  }

  /** Slots the last search found that are already on screen. */
  loadedHitSlots(): number[] {
    return (this.lastHits?.hits ?? [])
      .map((hit) => hit.slot)
      .filter((slot): slot is number => slot !== null)
  }

  /**
   * Take the picker off the page.
   *
   * `?deterministic=1` is the fixture mode: the positions are the server's
   * lattice and `positionsHash` is asserted against them, so a control that
   * replaced them with a kernel's output would be a button for breaking the
   * test suite. Removed rather than disabled — a greyed-out control invites
   * the question "why can I not use this", and the answer is not about this
   * screen.
   */
  hideLayoutPicker(): void {
    this.layoutRow.remove()
    this.layoutNote.remove()
  }

  /**
   * Offer — or withdraw — the map, as the view gains and loses placeable nodes.
   *
   * See {@link GEO_CHOICE}. Withdrawing is the half that has to be right: a
   * user who selected `geo` and then collapsed every wellbore is left with a
   * picker naming an arrangement the current view cannot be drawn in, so the
   * option goes and the note says what happened rather than leaving a dead
   * entry selected.
   */
  setGeoAvailable(available: boolean): void {
    const [value, text] = GEO_CHOICE
    const existing = [...this.layoutKernel.options].find((option) => option.value === value)
    if (available === (existing !== undefined)) return
    if (available) {
      const option = element('option', undefined, text)
      option.value = value
      this.layoutKernel.appendChild(option)
      return
    }
    const wasChosen = this.layoutKernel.value === value
    existing?.remove()
    if (wasChosen) {
      // The select falls back to its first option on its own; the note is what
      // stops that looking like the server changed its mind.
      this.layoutNote.className = 'kglv-hint kglv-warn'
      this.layoutNote.textContent =
        'nothing in this view has a coordinate any more, so the map is not on offer'
    }
  }

  /**
   * Show the arrangement the shared view is actually in.
   *
   * Driven from the server's answer, never from the click that asked for it:
   * the kernel that ran can differ from the one requested (`islands` over a
   * graph with no communities falls back), another tab or an agent can change
   * it, and a picker showing what the user *asked* for is a UI lying about the
   * picture beside it — the same rule `setAppearanceSelection` follows.
   */
  showLayoutKernel(requested: LayoutKernel, chosen: LayoutKernel, placed: number): void {
    this.layoutKernel.value = chosen
    this.layoutNote.className = requested === chosen ? 'kglv-hint' : 'kglv-hint kglv-warn'
    if (chosen === 'simulation') {
      this.layoutNote.textContent = 'the layout runs live on this machine — drag a node to move it'
      return
    }
    const fellBack =
      requested === chosen || requested === 'auto'
        ? ''
        : ` (${requested} had nothing to work with here)`
    const aside = chosen === 'geo' ? ` — ${GEO_HINT}` : ''
    this.layoutNote.textContent =
      `${count(placed)} nodes placed by the server's ${chosen} layout${fellBack}` +
      ' — held still, so dragging is off' +
      aside
  }

  /** A layout request the server refused, in its own words. */
  showLayoutError(message: string): void {
    this.layoutNote.className = 'kglv-hint kglv-error'
    this.layoutNote.textContent = message
  }

  /**
   * Show a colour-by / size-by choice this panel did not make.
   *
   * A remote `appearance` command (plan D14) moves the same two channels the
   * menus do, and a menu still reading "capability" while the graph is
   * coloured by `field` is a UI lying about its own state. An option the menus
   * have not been filled with yet is added rather than dropped: the agent
   * named a property the server accepted, and the human needs to see which.
   */
  setAppearanceSelection(colorBy: string | null, sizeBy: string | null): void {
    const choose = (select: HTMLSelectElement, value: string | null) => {
      const wanted = value ?? ''
      if (wanted !== '' && ![...select.options].some((option) => option.value === wanted)) {
        const option = element('option', undefined, wanted)
        option.value = wanted
        select.appendChild(option)
      }
      select.value = wanted
    }
    choose(this.colorBy, colorBy)
    choose(this.sizeBy, sizeBy)
  }

  /**
   * Fill the appearance menus.
   *
   * Returns `[candidates, approximate]` — the second is what the e2e asserts
   * the "approximate" labelling on, because a sampled stat presented as exact
   * is the failure D12 named.
   */
  showPropertyStats(stats: PropertyStatsResponse, caption: string | null): [number, number] {
    this.statsType = stats.node_type
    this.refreshTableAction()
    const byName = new Map(stats.properties.map((stat) => [stat.name, stat]))
    const fill = (select: HTMLSelectElement, names: string[], none: string) => {
      select.replaceChildren()
      const blank = element('option', undefined, none)
      blank.value = ''
      select.appendChild(blank)
      for (const name of names) {
        const stat = byName.get(name)
        const option = element('option', undefined, stat ? statLabel(stat) : name)
        option.value = name
        select.appendChild(option)
      }
    }
    fill(this.colorBy, stats.categorical_candidates, 'capability')
    fill(this.sizeBy, stats.numeric_candidates, 'uniform')

    // Every string property is offered, not only the server's own candidate:
    // the candidate is a heuristic about what a human finds readable, and the
    // human is right here. The blank entry names what it restores rather than
    // saying "none" — the title is a real choice, not the absence of one.
    this.captionBy.replaceChildren()
    const keepTitle = element('option', undefined, 'title (as stored)')
    keepTitle.value = ''
    this.captionBy.appendChild(keepTitle)
    for (const stat of stats.properties) {
      if (!/string/i.test(stat.value_type)) continue
      const option = element('option', undefined, statLabel(stat))
      option.value = stat.name
      this.captionBy.appendChild(option)
    }
    // A caption the server suggested but this list does not contain would be a
    // silently ignored suggestion, so it is added rather than dropped — the
    // same rule `setAppearanceSelection` follows for a remote channel choice.
    if (caption !== null && ![...this.captionBy.options].some((o) => o.value === caption)) {
      const option = element('option', undefined, caption)
      option.value = caption
      this.captionBy.appendChild(option)
    }
    this.captionBy.value = caption ?? ''
    this.captionRow.hidden = this.captionBy.options.length <= 1

    const offered: PropertyStat[] = [
      ...stats.categorical_candidates,
      ...stats.numeric_candidates,
    ]
      .map((name) => byName.get(name))
      .filter((stat): stat is PropertyStat => stat !== undefined)
    const approximate = offered.filter((stat) => stat.approx).length

    const notes: string[] = [`${stats.node_type}: ${count(stats.node_count)} nodes`]
    // The approx rule applies to a caption exactly as it applies to a colour:
    // a value drawn from a sampled population is a value some nodes will not
    // have, and a label falling back to the title on those looks like a bug in
    // the labelling rather than a fact about the statistics.
    const captionStat = caption === null ? undefined : byName.get(caption)
    if (captionStat?.approx === true) {
      notes.push(`captions come from ${caption}, whose values kglite only sampled`)
    }
    if (stats.sampled) {
      notes.push(
        `statistics are approximate — kglite samples above ${count(stats.exact_scan_ceiling)} nodes`,
      )
    } else if (approximate > 0) {
      notes.push(
        `${approximate} of ${offered.length} channels report approximate distinct counts`,
      )
    }
    this.appearanceNote.className = approximate > 0 || stats.sampled ? 'kglv-hint kglv-warn' : 'kglv-hint'
    this.appearanceNote.textContent = notes.join(' · ')
    return [offered.length, approximate]
  }
}

/** One result cell as text. `null` reads as an empty cell, not as "null". */
export function formatCell(value: unknown): string {
  if (value === null || value === undefined) return ''
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'boolean') return String(value)
  return JSON.stringify(value)
}
