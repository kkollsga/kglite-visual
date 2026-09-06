import { compareCells, recordCell } from './cells'
import type { NodeHandle } from './generated/NodeHandle'
import type { RecordRow } from './generated/RecordRow'
import type { RecordTable } from './generated/RecordTable'
import type { SharedSnapshotMeta } from './generated/SharedSnapshotMeta'
import { apiUrl } from './urls'

export const handleKey = (handle: NodeHandle): string => `${handle.generation}:${handle.node_id}`
type Scope = 'visible' | 'loaded' | 'selected'
type Handlers = {
  select(handles: NodeHandle[]): void
  showGraph(handles: NodeHandle[]): void
  inspectValue(handle: NodeHandle, field: string): void
  reveal(): void
}
function el<K extends keyof HTMLElementTagNameMap>(tag: K, text = ''): HTMLElementTagNameMap[K] {
  const result = document.createElement(tag); result.textContent = text; return result
}
function button(text: string, testid: string, action: () => void): HTMLButtonElement {
  const result = el('button', text); result.type = 'button'; result.className = 'kglv-button'
  result.dataset['testid'] = testid; result.onclick = action; return result
}
const MAX_BATCH_BYTES = 16 * 1024 * 1024

/** Bounded source records, independent of the most recent query result. */
export class DataWorkspace {
  readonly queryHost = el('section')
  private readonly recordsHost = el('section')
  private readonly queryTab: HTMLButtonElement
  private readonly recordsTab: HTMLButtonElement
  private readonly scope = el('select')
  private readonly status = el('p', 'No instances loaded. Browse a type in Explore to inspect records.')
  private readonly fieldsHost = el('div')
  private readonly typeNote = el('div')
  private readonly grid = el('div')
  private readonly pager = el('div')
  private readonly selectionStatus = el('span')
  private readonly showSelected: HTMLButtonElement
  private readonly fieldInput = el('input')
  private snapshot: SharedSnapshotMeta | null = null
  private fields = ['id', 'title']
  private rows: RecordRow[] = []
  private selected = new Map<string, NodeHandle>()
  private selectedKey = ''
  private sort: {field: number; descending: boolean} | null = null
  private page = 0
  private pageSize = 100
  private nodeType: string | null = null
  private active = true
  private presented = false
  private dirty = true
  private token = 0
  private abort: AbortController | null = null
  private loading = false
  private note = ''

  constructor(host: HTMLElement, private readonly handlers: Handlers) {
    this.recordsTab = button('Records', 'data-records', () => this.showRecords())
    this.queryTab = button('Query results · source', 'data-query', () => this.showQuery())
    const tabs = el('div'); tabs.className = 'kglv-data-lanes'; tabs.setAttribute('role', 'tablist')
    for (const tab of [this.recordsTab, this.queryTab]) {
      tab.setAttribute('role', 'tab')
      tab.onkeydown = event => {
        if (['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) {
          event.preventDefault()
          const target = event.key === 'Home' ? this.recordsTab : event.key === 'End' ? this.queryTab : tab === this.recordsTab ? this.queryTab : this.recordsTab
          target.click(); target.focus()
        }
      }
    }
    tabs.append(this.recordsTab, this.queryTab)
    this.queryHost.dataset['testid'] = 'query-lane'; this.recordsHost.dataset['testid'] = 'records-lane'
    this.status.dataset['testid'] = 'records-status'; this.status.setAttribute('role', 'status')
    this.grid.className = 'kglv-record-grid'; this.fieldsHost.className = 'kglv-field-chips'
    this.showSelected = button('Show selection in Explore', 'records-show-selected', () => this.handlers.showGraph([...this.selected.values()]))
    const actions = el('div'); actions.className = 'kglv-data-actions'
    actions.append(this.scopeControl(), this.showSelected, this.selectionStatus)
    this.recordsHost.append(actions, this.typeNote, this.fieldControls(), this.fieldsHost, this.status, this.grid, this.pager)
    host.append(tabs, this.recordsHost, this.queryHost)
    this.paintFields(); this.paintLanes(); this.paintSelection()
  }

  private scopeControl(): HTMLElement {
    this.scope.dataset['testid'] = 'records-scope'; this.scope.className = 'kglv-select'
    this.scope.setAttribute('aria-label', 'Records scope')
    this.scope.append(new Option('Visible instances', 'visible'), new Option('Loaded instances', 'loaded'), new Option('Selected source nodes', 'selected'))
    this.scope.onchange = () => { this.page = 0; this.reload() }
    return this.scope
  }
  private fieldControls(): HTMLElement {
    const row = el('div'); row.className = 'kglv-data-actions'
    this.fieldInput.className = 'kglv-input'; this.fieldInput.placeholder = 'Add source property'
    this.fieldInput.setAttribute('aria-label', 'Source property to add'); this.fieldInput.dataset['testid'] = 'records-field'
    const add = button('Add field', 'records-add-field', () => {
      const field = this.fieldInput.value.trim()
      if (!field || this.fields.includes(field)) return
      if (this.fields.length >= 32) { this.status.textContent = 'Choose at most 32 source fields.'; return }
      this.fields.push(field); this.fieldInput.value = ''; this.page = 0; this.paintFields(); this.reload()
    })
    this.fieldInput.onkeydown = event => { if (event.key === 'Enter') { event.preventDefault(); add.click() } }
    row.append(this.fieldInput, add)
    return row
  }
  private paintFields(): void {
    this.fieldsHost.replaceChildren(...this.fields.map(field => {
      const chip = button(`${field} ×`, `records-remove-${field}`, () => {
        if (this.fields.length === 1) { this.status.textContent = 'Keep at least one source field.'; return }
        this.fields = this.fields.filter(item => item !== field); this.sort = null; this.page = 0; this.paintFields(); this.reload()
      })
      chip.setAttribute('aria-label', `Remove field ${field}`)
      return chip
    }))
  }
  private paintLanes(): void {
    this.recordsHost.hidden = !this.active; this.queryHost.hidden = this.active
    this.recordsTab.setAttribute('aria-selected', String(this.active)); this.queryTab.setAttribute('aria-selected', String(!this.active))
    this.recordsTab.tabIndex = this.active ? 0 : -1; this.queryTab.tabIndex = this.active ? -1 : 0
  }
  showQuery(): void { this.active = false; this.paintLanes() }
  showRecords(): void { this.active = true; this.paintLanes(); if (this.dirty && this.presented) void this.fetchRows() }
  setPresented(presented: boolean): void { this.presented = presented; if (presented && this.active && this.dirty) void this.fetchRows() }
  showSelection(): void {
    this.nodeType = null; this.typeNote.replaceChildren(); this.scope.value = 'selected'
    this.page = 0; this.dirty = true; this.showRecords(); this.handlers.reveal()
  }
  showType(nodeType: string, fields: string[]): void {
    this.nodeType = nodeType; this.scope.value = 'loaded'
    this.fields = [...new Set(['id', 'title', ...fields])].slice(0, 12)
    this.sort = null; this.page = 0; this.paintFields()
    const clear = button('All types', 'records-all-types', () => { this.nodeType = null; this.typeNote.replaceChildren(); this.reload() })
    this.typeNote.replaceChildren(el('span', `Loaded ${nodeType} records `), clear)
    this.dirty = true; this.showRecords(); this.handlers.reveal()
  }
  update(snapshot: SharedSnapshotMeta): void {
    const before = this.membershipKey()
    const presentationChanged = this.snapshot?.subset_revision !== snapshot.subset_revision || this.snapshot?.topology_revision !== snapshot.topology_revision
    this.snapshot = snapshot
    if (before !== this.membershipKey()) this.reload()
    else if (presentationChanged && this.rows.length > 0) {
      this.reconcileRows()
      if (this.active && this.presented) this.paintGrid()
    }
    this.paintSelection()
  }
  setSelection(handles: NodeHandle[]): void {
    const key = handles.map(handleKey).join(',')
    if (key === this.selectedKey) return
    this.selectedKey = key
    this.selected = new Map(handles.map(handle => [handleKey(handle), handle]))
    this.paintSelection()
    if (this.scope.value === 'selected') this.reload()
    for (const input of this.grid.querySelectorAll<HTMLInputElement>('input[data-handle]')) {
      input.checked = this.selected.has(input.dataset['handle'] ?? '')
      input.closest('tr')?.setAttribute('aria-selected', String(input.checked))
    }
  }
  private paintSelection(): void {
    const visible = new Set(this.snapshot?.subset.visible_nodes.map(handleKey) ?? [])
    const hidden = [...this.selected.keys()].filter(key => !visible.has(key)).length
    this.selectionStatus.textContent = `${this.selected.size} selected${hidden > 0 ? ` · ${hidden} hidden or unloaded` : ''}`
    this.selectionStatus.dataset['testid'] = 'records-selection'
    this.showSelected.disabled = this.selected.size === 0
  }
  private reload(): void {
    this.dirty = true; this.token += 1; this.abort?.abort()
    if (this.active && this.presented) void this.fetchRows()
  }
  private handles(): NodeHandle[] {
    if (this.scope.value === 'selected') return [...this.selected.values()].filter(handle => handle.generation === this.snapshot?.stamp.generation)
    const visible = new Set(this.snapshot?.subset.visible_nodes.map(handleKey) ?? [])
    return this.snapshot?.slice.nodes.filter(node => (this.nodeType === null || node.node_type === this.nodeType) &&
      (this.scope.value as Scope === 'loaded' || visible.has(handleKey(node.handle)))).map(node => node.handle) ?? []
  }
  private membershipKey(): string { return `${this.snapshot?.stamp.generation ?? ''}|${this.scope.value}|${JSON.stringify(this.fields)}|${this.handles().map(handleKey).join(',')}` }
  private reconcileRows(): void {
    const slots = new Map(this.snapshot?.slice.nodes.map(node => [handleKey(node.handle), node.slot]) ?? [])
    const visible = new Set(this.snapshot?.subset.visible_nodes.map(handleKey) ?? [])
    this.rows = this.rows.map(row => ({...row, slot: slots.get(handleKey(row.handle)) ?? null, visible: visible.has(handleKey(row.handle))}))
  }
  private async fetchRows(): Promise<void> {
    const snapshot = this.snapshot
    if (snapshot === null) return
    const token = ++this.token
    const membership = this.membershipKey()
    this.abort?.abort(); this.abort = new AbortController()
    const handles = this.handles(); const fields = [...this.fields]
    this.dirty = false; this.loading = true; this.rows = []; this.note = ''; this.paintGrid()
    const rows: RecordRow[] = []
    let offset: number | null = 0; let bytes = 0
    try {
      while (offset !== null && handles.length > 0) {
        this.status.textContent = `Reading ${rows.length} / ${handles.length} ${this.scope.value} records…`
        const response = await fetch(apiUrl('api/records'), {method: 'POST', headers: {'content-type': 'application/json'}, signal: this.abort.signal,
          body: JSON.stringify({handles, fields, offset, limit: 500, request_id: `records-${token}-${offset}`})})
        if (!response.ok) throw new Error(`Records read refused (${response.status}): ${await response.text()}`)
        const text = await response.text()
        const table = JSON.parse(text) as RecordTable
        if (token !== this.token || this.snapshot === null || table.stamp.generation !== this.snapshot.stamp.generation || membership !== this.membershipKey()) return
        bytes += new TextEncoder().encode(text).length
        if (bytes > MAX_BATCH_BYTES) { this.note = 'Partial records: the 16 MiB Data limit was reached. Choose fewer fields to read more rows.'; break }
        rows.push(...table.rows)
        if (table.next_offset !== null && table.next_offset <= offset) throw new Error('Records pagination did not advance.')
        offset = table.next_offset
        if (table.bound.truncated && offset === null) this.note = `Partial records: ${table.bound.returned} / ${table.bound.total} returned.`
      }
      if (token !== this.token) return
      this.rows = rows; this.reconcileRows(); this.loading = false; this.paintGrid(); this.paintSelection()
    } catch (error) {
      if (token !== this.token) return
      this.loading = false; this.status.textContent = error instanceof Error ? error.message : String(error)
    }
  }
  private ordered(): RecordRow[] {
    if (this.sort === null) return this.rows
    const sort = this.sort
    return this.rows.map((row, index) => ({row, index})).sort((a, b) => {
      const left = a.row.cells[sort.field] ?? {state: 'missing' as const}
      const right = b.row.cells[sort.field] ?? {state: 'missing' as const}
      return compareCells(left, right, sort.descending) || a.index - b.index
    }).map(item => item.row)
  }
  private paintGrid(): void {
    if (this.loading) { this.grid.replaceChildren(); this.pager.replaceChildren(); return }
    const rows = this.ordered()
    this.page = Math.min(this.page, Math.max(0, Math.ceil(rows.length / this.pageSize) - 1))
    const from = this.page * this.pageSize; const shown = rows.slice(from, from + this.pageSize)
    this.status.textContent = `${rows.length} / ${this.handles().length} ${this.scope.value} records${this.nodeType === null ? '' : ` · ${this.nodeType}`} · ${this.fields.length} source fields${this.note ? ` · Sort covers these ${rows.length} fetched records. ${this.note}` : ''}`
    const table = el('table'); table.className = 'kglv-table'; table.dataset['testid'] = 'records-table'
    const head = el('tr'); head.append(el('th', 'Select'), el('th', 'Graph'))
    this.fields.forEach((field, index) => {
      const th = el('th'); const active = this.sort?.field === index
      th.setAttribute('aria-sort', active ? this.sort?.descending ? 'descending' : 'ascending' : 'none')
      const sort = button(`${field}${active ? this.sort?.descending ? ' ▾' : ' ▴' : ''}`, `records-sort-${field}`, () => {
        this.sort = {field: index, descending: active && this.sort !== null ? !this.sort.descending : false}; this.page = 0; this.paintGrid()
      })
      sort.className = 'kglv-th-sort'; th.append(sort); head.append(th)
    })
    table.append(head, ...shown.map(row => this.paintRow(row)))
    this.grid.replaceChildren(table); this.paintPager(from, shown.length, rows.length)
  }
  private paintRow(row: RecordRow): HTMLTableRowElement {
    const tr = el('tr'); const key = handleKey(row.handle)
    tr.dataset['handle'] = key; tr.setAttribute('aria-selected', String(this.selected.has(key)))
    const select = el('input'); select.type = 'checkbox'; select.checked = this.selected.has(key)
    select.dataset['handle'] = key; select.setAttribute('aria-label', `Select record ${row.handle.node_id}`)
    select.onchange = () => {
      if (select.checked) this.selected.set(key, row.handle); else this.selected.delete(key)
      this.handlers.select([...this.selected.values()]); this.paintSelection(); tr.setAttribute('aria-selected', String(select.checked))
    }
    const check = el('td'); check.append(select)
    const graph = el('td'); graph.append(button(row.visible ? 'Show in Explore' : row.slot === null ? 'Unloaded · show in Explore' : 'Hidden · show in Explore', 'record-show-graph', () => this.handlers.showGraph([row.handle])))
    tr.append(check, graph)
    row.cells.forEach((cell, index) => {
      const td = el('td'); const field = this.fields[index]
      td.append(recordCell(cell, field === undefined ? undefined : () => this.handlers.inspectValue(row.handle, field)))
      tr.append(td)
    })
    return tr
  }
  private paintPager(from: number, shown: number, total: number): void {
    const previous = button('Previous', 'records-previous', () => { this.page -= 1; this.paintGrid() })
    const next = button('Next', 'records-next', () => { this.page += 1; this.paintGrid() })
    previous.disabled = this.page === 0; next.disabled = from + shown >= total
    const size = el('select'); size.className = 'kglv-select'; size.dataset['testid'] = 'records-page-size'; size.setAttribute('aria-label', 'Records per page')
    for (const count of [100, 250, 500]) size.append(new Option(`${count} per page`, String(count)))
    size.value = String(this.pageSize); size.onchange = () => { this.pageSize = Number(size.value); this.page = 0; this.paintGrid() }
    const label = el('span', `${total === 0 ? 0 : from + 1}–${from + shown} of ${total}`); label.dataset['testid'] = 'records-page'
    this.pager.className = 'kglv-data-actions'; this.pager.replaceChildren(previous, label, next, size)
  }
}
