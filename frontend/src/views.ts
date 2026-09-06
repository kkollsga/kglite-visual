import { apiUrl } from './urls'
import { requestNonce } from './request-id'
import type { SharedSnapshotMeta } from './generated/SharedSnapshotMeta'
import type { ViewReference } from './generated/ViewReference'
import type { RevisionStamp } from './generated/RevisionStamp'
import type { HistoryEntry } from './generated/HistoryEntry'

type Storage = 'durable' | 'session'
type CatalogEntry = {storage: Storage; name: string; saved_at: number}
type Catalog = {
  views: CatalogEntry[]; save_storage: Storage
  eligibility: {storage: Storage; reason: string | null; verification_required: boolean}
  limits: {durable: {max_views: number; max_bytes: number}; session: {max_views: number; max_bytes: number}; max_view_bytes: number}
}
type SaveResult = {saved: CatalogEntry; marker_applied: boolean; dirty: boolean; marker_error?: unknown}
type Handlers = {selection(): ViewReference[]; selectionEpoch(): number; restoreRequested(id: string, epoch: number): void; restoreRefused(id: string): void}
function el<K extends keyof HTMLElementTagNameMap>(tag: K, text = ''): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag); node.textContent = text; return node
}
function button(text: string, id: string, action: () => void): HTMLButtonElement {
  const node = el('button', text); node.type = 'button'; node.className = 'kglv-button'; node.dataset['testid'] = id; node.onclick = action; return node
}
export function selectionKey(references: ViewReference[]): string {
  return JSON.stringify([...new Set(references.map(ref => ref.kind === 'type' ? `type:${ref.name}` : `node:${ref.handle.generation}:${ref.handle.node_id}`))].sort())
}

export function saveMessage(result: SaveResult, localChanged: boolean): string {
  const storage = result.saved.storage === 'session' ? 'session only; lost when this server closes' : 'durable'
  return result.marker_applied && !result.dirty && !localChanged ? `Saved “${result.saved.name}” (${storage}).` : `Saved “${result.saved.name}” (${storage}), but the current view changed. Its saved copy is intact; current changes remain unsaved.`
}

/** Named durable/session views and explicit checkpoint restoration share the server's revision gate. */
export class SavedViews {
  private readonly dialog = el('dialog')
  private readonly marker = el('span', 'Unsaved exploration')
  private readonly status = el('p')
  private readonly storageNote = el('p', 'Checking save storage…')
  private readonly geometryNote = el('p')
  private readonly name = el('input')
  private readonly replace = el('input')
  private readonly focus = el('select')
  private readonly catalogHost = el('div')
  private readonly catalogStatus = el('p')
  private readonly historyHost = el('div')
  private readonly save: HTMLButtonElement
  private readonly opener: HTMLButtonElement
  private snapshot: SharedSnapshotMeta | null = null
  private busy = false
  private catalogReady = false
  private readToken = 0
  private operationToken = 0
  private historyKey = ''

  constructor(host: HTMLElement, private readonly handlers: Handlers) {
    this.opener = button('Saved views & history', 'views-open', () => this.open())
    this.marker.dataset['testid'] = 'saved-view-marker'; this.marker.className = 'kglv-saved-marker'
    host.className = 'kglv-saved-view-header'; host.append(this.marker, this.opener)
    this.dialog.className = 'kglv-views-dialog'; this.dialog.dataset['testid'] = 'views-dialog'
    this.dialog.setAttribute('aria-labelledby', 'views-heading')
    const heading = el('h2', 'Saved views & history'); heading.id = 'views-heading'
    const close = button('Close', 'views-close', () => this.dialog.close())
    const top = el('div'); top.className = 'kglv-data-actions'; top.append(heading, close)
    this.dialog.append(top, this.saveForm(), this.catalogSection(), this.historySection())
    document.body.append(this.dialog)
    this.dialog.addEventListener('close', () => this.opener.focus())
    this.dialog.addEventListener('keydown', event => { if (event.key === 'Escape') event.stopPropagation() })
    this.save = this.dialog.querySelector<HTMLButtonElement>('[data-testid=view-save]') as HTMLButtonElement
    this.save.disabled = true
  }

  private saveForm(): HTMLElement {
    const section = el('section'); section.append(el('h3', 'Save this exploration'))
    const nameLabel = el('label', 'View name ')
    this.name.type = 'text'; this.name.maxLength = 128; this.name.dataset['testid'] = 'view-name'; nameLabel.append(this.name)
    this.replace.type = 'checkbox'; this.replace.dataset['testid'] = 'view-replace'
    const replaceLabel = el('label'); replaceLabel.append(this.replace, ' Replace an existing view with this name')
    const focusLabel = el('label', 'Framing on restore ')
    this.focus.dataset['testid'] = 'view-focus'; this.focus.append(new Option('Keep current camera', 'none'), new Option('Fit restored view', 'fit'), new Option('Focus saved selection', 'selection'))
    focusLabel.append(this.focus)
    this.storageNote.dataset['testid'] = 'view-storage-note'; this.geometryNote.dataset['testid'] = 'view-geometry-note'
    this.status.dataset['testid'] = 'views-status'; this.status.setAttribute('role', 'status')
    const controls = el('div'); controls.className = 'kglv-view-save-controls'
    controls.append(nameLabel, focusLabel, replaceLabel, button('Save view', 'view-save', () => { void this.saveView() }))
    section.append(this.storageNote, this.geometryNote, controls, this.status)
    return section
  }
  private catalogSection(): HTMLElement {
    this.catalogStatus.dataset['testid'] = 'views-catalog-status'; this.catalogStatus.setAttribute('role', 'status')
    const section = el('section'); section.append(el('h3', 'Named views'), el('p', 'Durable names are shared across source graphs. Restore verifies the saved source before changing this view.'), button('Refresh list', 'views-refresh', () => { void this.refresh() }), this.catalogStatus, this.catalogHost)
    this.catalogHost.dataset['testid'] = 'views-catalog'; return section
  }
  private historySection(): HTMLElement {
    const section = el('section'); section.append(el('h3', 'Shared recovery history'), el('p', 'Restore the checkpoint before an action as a new shared change. This affects every attached browser. Hover and camera movements do not add checkpoints.'), this.historyHost)
    this.historyHost.dataset['testid'] = 'views-history'; return section
  }
  private open(): void { this.dialog.showModal(); this.name.focus(); void this.refresh() }

  update(snapshot: SharedSnapshotMeta): void {
    if (this.snapshot !== null && this.snapshot.stamp.generation !== snapshot.stamp.generation) {
      this.operationToken += 1; this.readToken += 1; this.busy = false; this.catalogReady = false; this.status.replaceChildren(); this.catalogHost.replaceChildren()
    }
    this.snapshot = snapshot; this.updateSelection()
    this.geometryNote.textContent = snapshot.layout_kernel === 'simulation'
      ? 'Live layout: restoration recomputes geometry. The current camera position is not saved.'
      : 'Static layout: server positions are saved. Framing uses the restore option below; the current camera position is not saved.'
    this.save.disabled = this.busy || !this.catalogReady
    const key = JSON.stringify(snapshot.history)
    if (key !== this.historyKey) { this.historyKey = key; this.paintHistory() }
  }
  updateSelection(): void {
    const saved = this.snapshot?.saved_view
    const dirty = saved !== null && saved !== undefined && (saved.dirty || selectionKey(saved.selected) !== selectionKey(this.handlers.selection()))
    this.marker.textContent = saved == null ? 'Unsaved exploration' : `${saved.name} · ${saved.storage === 'session' ? 'session only' : 'saved'}${dirty ? ' · unsaved changes' : ''}`
    this.marker.dataset['dirty'] = String(dirty); this.marker.dataset['storage'] = saved?.storage ?? ''
    this.marker.title = saved == null ? '' : `${saved.storage} / ${saved.name}`
  }

  private async refresh(): Promise<void> {
    const token = ++this.readToken
    this.catalogReady = false; this.save.disabled = true; this.storageNote.textContent = 'Checking save storage…'
    this.catalogStatus.replaceChildren()
    try {
      const response = await fetch(apiUrl('api/views'))
      if (!response.ok) throw new Error(await this.failure(response))
      const catalog = await response.json() as Catalog
      if (token !== this.readToken) return
      this.catalogReady = true; this.save.disabled = this.busy || this.snapshot === null
      const scope = catalog.save_storage === 'session' ? `Session only — saved views are lost when this server closes. ${catalog.eligibility.reason ?? ''}` : 'Durable candidate — source identity is checked on save; the result may be session only.'
      const limits = catalog.limits[catalog.save_storage]
      this.storageNote.textContent = `${scope} Limit: ${limits.max_views} views / ${Math.round(limits.max_bytes / 1048576)} MiB; ${Math.round(catalog.limits.max_view_bytes / 1048576)} MiB per view.`
      this.catalogHost.replaceChildren(...catalog.views.map(entry => this.catalogRow(entry)))
      if (catalog.views.length === 0) this.catalogHost.append(el('p', 'No named views saved.'))
    } catch (error) {
      if (token === this.readToken) {
        this.catalogStatus.textContent = `Catalog refresh failed. ${String(error)}`
        this.storageNote.textContent = 'Save storage is unavailable. Refresh the list before saving.'
      }
    }
  }
  private catalogRow(entry: CatalogEntry): HTMLElement {
    const row = el('div'); row.className = 'kglv-view-row'; row.dataset['storage'] = entry.storage; row.dataset['name'] = entry.name
    const text = el('span', `${entry.name} · ${entry.storage === 'session' ? 'session only' : 'durable'}`)
    text.title = `Saved ${new Date(entry.saved_at * 1000).toLocaleString()}`
    const restore = button('Restore', 'view-restore', () => { void this.restoreView(entry) })
    const remove = button('Delete', 'view-delete', () => {
      const confirm = button(`Delete “${entry.name}” from ${entry.storage}?`, 'view-delete-confirm', () => { void this.deleteView(entry) })
      const cancel = button('Cancel', 'view-delete-cancel', () => { row.replaceChildren(text, restore, remove) })
      row.replaceChildren(text, confirm, cancel)
    })
    row.append(text, restore, remove); return row
  }
  private paintHistory(): void {
    const history = this.snapshot?.history
    if (history === undefined) return
    const note = el('p', `${history.entries.length} retained checkpoints · ${history.evicted_count} evicted. Up to 20 checkpoints / 16 MiB are retained in this session.`)
    const list = el('ol'); list.append(...[...history.entries].reverse().map(entry => this.historyRow(entry)))
    this.historyHost.replaceChildren(note, list)
  }
  private historyRow(entry: HistoryEntry): HTMLElement {
    const row = el('li'); row.dataset['historyId'] = entry.id
    const action = entry.action
    const name = [action.kind, action.label, action.node_type, action.relationship, action.direction].filter(value => value !== null).join(' · ')
    row.append(el('strong', name), el('span', ` · revision ${entry.before.revision} → ${entry.after.revision}`))
    if (action.nodes !== null) row.append(el('p', `${action.nodes.returned} nodes returned${action.nodes.truncated ? ` / at least ${action.nodes.total} found` : ''}`))
    if (action.relations !== null) row.append(el('p', `${action.relations.returned} relationships returned${action.relations.truncated ? ` / up to ${action.relations.total} found` : ''}`))
    if (action.query !== null) {
      const details = el('details'); details.append(el('summary', action.query_truncated ? 'Query provenance · partial' : 'Query provenance'), el('pre', action.query)); row.append(details)
    }
    row.append(button('Restore before this action', 'history-restore', () => { void this.restoreHistory(entry.id) }))
    return row
  }
  private async failure(response: Response): Promise<string> {
    let text = await response.text()
    try { const body = JSON.parse(text) as {error?: string; message?: string}; text = body.error ?? body.message ?? text } catch { /* Preserve a proxy's non-JSON refusal. */ }
    return response.status === 409 ? `Shared view changed. Review the current state before retrying. ${text}` : `Request refused (${response.status}): ${text}`
  }
  private async mutate<T>(path: string, body: object, completed: (value: T) => void, restoreEpoch?: number): Promise<void> {
    if (this.busy || this.snapshot === null) return
    const token = ++this.operationToken; const expected: RevisionStamp = {...this.snapshot.stamp}
    const request_id = `view-${requestNonce()}`
    if (restoreEpoch !== undefined) this.handlers.restoreRequested(request_id, restoreEpoch)
    this.busy = true; this.save.disabled = true; this.status.textContent = 'Waiting for acknowledgement…'
    try {
      const response = await fetch(apiUrl(path), {method: 'POST', headers: {'content-type': 'application/json'}, body: JSON.stringify({...body, expected, request_id})})
      if (!response.ok) throw new Error(await this.failure(response))
      const value = await response.json() as T
      if (token !== this.operationToken) return
      completed(value); await this.refresh()
    } catch (error) {
      if (restoreEpoch !== undefined) this.handlers.restoreRefused(request_id)
      if (token === this.operationToken) this.status.textContent = String(error)
    }
    finally { if (token === this.operationToken) { this.busy = false; this.save.disabled = this.snapshot === null || !this.catalogReady } }
  }
  private async saveView(): Promise<void> {
    const name = this.name.value.trim()
    if (name === '') { this.status.textContent = 'Enter a name for this view.'; this.name.focus(); return }
    const selected = this.handlers.selection()
    if (this.focus.value === 'selection' && selected.length === 0) { this.status.textContent = 'Select a node or type before saving selection framing.'; return }
    const focus = this.focus.value === 'fit' ? {kind: 'fit'} : this.focus.value === 'selection' ? {kind: 'references', references: selected} : undefined
    await this.mutate<SaveResult>('api/views/save', {name, replace: this.replace.checked, selected, focus}, result => {
      this.status.textContent = saveMessage(result, selectionKey(selected) !== selectionKey(this.handlers.selection()))
    })
  }
  private async restoreView(entry: CatalogEntry): Promise<void> {
    const epoch = this.handlers.selectionEpoch()
    await this.mutate('api/views/restore', {storage: entry.storage, name: entry.name}, () => { this.status.textContent = `Restored “${entry.name}” (${entry.storage}) as a shared change.` }, epoch)
  }
  private async deleteView(entry: CatalogEntry): Promise<void> {
    await this.mutate<{marker_applied: boolean}>('api/views/delete', {storage: entry.storage, name: entry.name}, result => { this.status.textContent = `Deleted “${entry.name}” (${entry.storage}).${result.marker_applied ? '' : ' The current view association was not changed.'}` })
  }
  private async restoreHistory(id: string): Promise<void> {
    const epoch = this.handlers.selectionEpoch()
    await this.mutate('api/history/restore', {id}, () => { this.status.textContent = 'Restored the checkpoint as a new shared change.' }, epoch)
  }
}
