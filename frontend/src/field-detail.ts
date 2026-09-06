import { recordCell, recordText } from './cells'
import type { FieldDetailRequest } from './generated/FieldDetailRequest'
import type { FieldDetailResponse } from './generated/FieldDetailResponse'
import type { FieldPathSegment } from './generated/FieldPathSegment'
import type { NodeHandle } from './generated/NodeHandle'
import { apiUrl } from './urls'

function button(text: string, action: () => void): HTMLButtonElement {
  const result = document.createElement('button'); result.type = 'button'; result.className = 'kglv-button'
  result.textContent = text; result.onclick = action; return result
}

/** Reads bounded source-value pages; copy names exactly the page it copies. */
export class FieldDetails {
  private readonly dialog = document.createElement('dialog')
  private readonly heading = document.createElement('h2')
  private readonly note = document.createElement('p')
  private readonly content = document.createElement('div')
  private readonly actions = document.createElement('div')
  private readonly pathBar = document.createElement('div')
  private request: FieldDetailRequest | null = null
  private offsets: number[] = []
  private token = 0
  private returnFocus: HTMLElement | null = null

  constructor(host: HTMLElement, private readonly generation: () => string | null) {
    this.dialog.className = 'kglv-field-detail'; this.dialog.dataset['testid'] = 'field-detail'
    this.heading.id = 'field-detail-title'; this.dialog.setAttribute('aria-labelledby', this.heading.id)
    this.note.dataset['testid'] = 'field-detail-status'; this.note.setAttribute('role', 'status')
    this.content.className = 'kglv-field-detail-content'; this.actions.className = 'kglv-data-actions'
    const close = button('Close', () => this.dialog.close()); close.dataset['testid'] = 'field-detail-close'
    this.dialog.append(close, this.heading, this.pathBar, this.note, this.content, this.actions); host.append(this.dialog)
    this.dialog.onclose = () => { this.token += 1; this.returnFocus?.focus() }
  }
  open(handle: NodeHandle, field: string): void {
    this.returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null
    this.offsets = []; this.request = {handle, field, path: [], offset: 0, limit: null}
    this.heading.textContent = `${field} · source value`
    if (!this.dialog.open) this.dialog.showModal()
    void this.read()
  }
  private async read(): Promise<void> {
    const request = this.request
    if (request === null) return
    const token = ++this.token
    this.content.replaceChildren(); this.actions.replaceChildren(); this.note.textContent = 'Reading value…'
    this.paintPath(request)
    try {
      const response = await fetch(apiUrl('api/field-detail'), {method: 'POST', headers: {'content-type': 'application/json'},
        body: JSON.stringify({...request, request_id: `field-${token}`})})
      if (!response.ok) throw new Error(`Value read refused (${response.status}): ${await response.text()}`)
      const value = await response.json() as FieldDetailResponse
      if (token !== this.token) return
      if (value.stamp.generation !== this.generation() || value.handle.generation !== request.handle.generation || value.handle.node_id !== request.handle.node_id) throw new Error('This source handle has expired. Select the record again.')
      this.paint(value)
    } catch (error) { if (token === this.token) this.note.textContent = error instanceof Error ? error.message : String(error) }
  }
  private paintPath(request: FieldDetailRequest): void {
    const root = button(request.field, () => { this.request = {...request, path: [], offset: 0}; this.offsets = []; void this.read() })
    this.pathBar.replaceChildren(root)
    request.path.forEach((segment, index) => this.pathBar.append(button(segment.kind === 'index' ? `[${segment.index}]` : segment.key, () => {
      this.request = {...request, path: request.path.slice(0, index + 1), offset: 0}; this.offsets = []; void this.read()
    })))
  }
  private descend(segment: FieldPathSegment): void {
    if (this.request === null) return
    this.request = {...this.request, path: [...this.request.path, segment], offset: 0}; this.offsets = []; void this.read()
  }
  private paint(value: FieldDetailResponse): void {
    const page = value.page
    let copy = recordText(value.cell)
    if (page === null) {
      this.note.textContent = value.cell.state === 'truncated' ? 'Partial value; no expanded page is available.' : 'Bounded source value'
      const pre = document.createElement('pre'); pre.textContent = copy; this.content.append(pre)
    } else if (page.kind === 'text') {
      copy = page.text
      const end = page.offset + new TextEncoder().encode(page.text).length
      this.note.textContent = `Text bytes ${page.offset}–${end} / ${page.total_bytes}${page.offset > 0 || page.next_offset !== null ? ' · partial page' : ' · complete value'}`
      const pre = document.createElement('pre'); pre.textContent = page.text; this.content.append(pre)
    } else {
      const entries = page.kind === 'list' ? page.items.map((cell, index) => ({cell, label: String(page.offset + index), path: {kind: 'index' as const, index: page.offset + index}}))
        : page.entries.map(entry => ({cell: entry.cell, label: entry.key, path: {kind: 'key' as const, key: entry.key}}))
      this.note.textContent = `${page.kind} items ${page.offset}–${page.offset + entries.length} / ${page.total_items}${page.offset > 0 || page.next_offset !== null ? ' · partial page' : ''}`
      copy = entries.map(entry => `${entry.label}: ${recordText(entry.cell)}`).join('\n')
      for (const entry of entries) {
        const row = document.createElement('div'); row.className = 'kglv-field-entry'
        const name = document.createElement('strong'); name.textContent = `${entry.label}: `
        row.append(name, recordCell(entry.cell, () => this.descend(entry.path))); this.content.append(row)
      }
    }
    const copyPage = button(page === null ? 'Copy displayed value' : 'Copy current page', () => {
      void navigator.clipboard.writeText(copy).then(() => { this.note.textContent += ' · copied displayed text' }).catch(error => { this.note.textContent = String(error) })
    }); copyPage.dataset['testid'] = 'field-detail-copy'
    this.actions.append(copyPage)
    if (page !== null) this.paintPager(page.next_offset)
  }
  private paintPager(nextOffset: number | null): void {
    const previous = button('Previous page', () => {
      const offset = this.offsets.pop(); if (this.request === null || offset === undefined) return
      this.request = {...this.request, offset}; void this.read()
    })
    previous.disabled = this.offsets.length === 0; previous.dataset['testid'] = 'field-detail-previous'
    const next = button('Next page', () => {
      if (this.request === null || nextOffset === null) return
      this.offsets.push(this.request.offset); this.request = {...this.request, offset: nextOffset}; void this.read()
    })
    next.disabled = nextOffset === null; next.dataset['testid'] = 'field-detail-next'
    this.actions.append(previous, next)
  }
}
