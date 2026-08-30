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
import type { QueryEditor, SchemaSource } from './editor/contract'
import type { EdgeDirection } from './generated/EdgeDirection'
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
  /** Keep this query under this name. The store's ceilings answer as refusals. */
  saveQuery(name: string, query: string): void
  /** Forget a saved query. */
  deleteQuery(name: string): void
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

export class Panels {
  readonly root: HTMLDivElement
  private readonly queryInput: HTMLTextAreaElement
  /** Where the CodeMirror view mounts, and what holds the textarea until it does. */
  private readonly queryHost: HTMLDivElement
  private readonly editorNote: HTMLDivElement
  /** The textarea, or CodeMirror once its chunk has landed. Never null. */
  private editor: QueryEditor
  private readonly queryAsGraph: HTMLInputElement
  private readonly queryStatus: HTMLDivElement
  private readonly queryDiagnostics: HTMLDivElement
  private readonly queryResults: HTMLDivElement
  private readonly selection: HTMLDivElement
  private readonly expandLimit: HTMLInputElement
  private readonly searchInput: HTMLInputElement
  private readonly searchType: HTMLSelectElement
  private readonly searchResults: HTMLDivElement
  private readonly colorBy: HTMLSelectElement
  private readonly sizeBy: HTMLSelectElement
  private readonly appearanceNote: HTMLDivElement
  private readonly savedList: HTMLSelectElement
  private readonly savedNote: HTMLDivElement
  private readonly historyList: HTMLDivElement
  private lastHits: SearchResponse | null = null
  private lastSaved: SavedQueries | null = null

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
    appearance.append(
      this.labelled('colour by', this.colorBy),
      this.labelled('size by', this.sizeBy),
    )
    this.appearanceNote = element('div', 'kglv-hint')
    this.appearanceNote.setAttribute('data-testid', 'appearance-note')
    appearance.appendChild(this.appearanceNote)
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
    // The engine's own diagnostics, between the count and the rows: a warning
    // printed under the table is a warning read after the conclusion was drawn.
    this.queryDiagnostics = element('div', 'kglv-diagnostics')
    this.queryDiagnostics.setAttribute('data-testid', 'query-diagnostics')
    this.queryResults = element('div', 'kglv-results')
    query.append(
      this.queryHost,
      this.editorNote,
      queryRow,
      savedRow,
      this.savedNote,
      this.historyList,
      this.queryStatus,
      this.queryDiagnostics,
      this.queryResults,
    )
    this.root.appendChild(this.section('Cypher', query))

    void this.upgradeEditor()
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
      })
      this.editorNote.remove()
    } catch (err) {
      this.editorNote.className = 'kglv-hint kglv-warn'
      this.editorNote.textContent = `plain text box — the syntax editor did not load (${
        err instanceof Error ? err.message : String(err)
      })`
    }
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

    const grid = element('table', 'kglv-table')
    grid.setAttribute('data-testid', 'query-table')
    const head = element('tr')
    for (const column of table.columns) head.appendChild(element('th', undefined, column))
    grid.appendChild(head)
    for (let row = 0; row < rows; row += 1) {
      const tr = element('tr')
      for (let column = 0; column < table.columns.length; column += 1) {
        tr.appendChild(element('td', undefined, formatCell(table.data[column]?.[row])))
      }
      grid.appendChild(tr)
    }
    this.queryResults.appendChild(grid)
    return rows
  }

  showQueryError(message: string): void {
    this.queryResults.replaceChildren()
    // The previous run's advisories described the previous query. Left up, they
    // would read as an explanation of this failure.
    this.queryDiagnostics.replaceChildren()
    this.queryStatus.className = 'kglv-hint kglv-error'
    // kglite's own diagnostic, verbatim: position, expected token, the schema
    // name it could not resolve. A friendlier summary would delete the only
    // part the user can act on.
    this.queryStatus.textContent = message
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
  showPropertyStats(stats: PropertyStatsResponse): [number, number] {
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

    const offered: PropertyStat[] = [
      ...stats.categorical_candidates,
      ...stats.numeric_candidates,
    ]
      .map((name) => byName.get(name))
      .filter((stat): stat is PropertyStat => stat !== undefined)
    const approximate = offered.filter((stat) => stat.approx).length

    const notes: string[] = [`${stats.node_type}: ${count(stats.node_count)} nodes`]
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
