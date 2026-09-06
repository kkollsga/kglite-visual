import type { SharedSnapshotMeta } from './generated/SharedSnapshotMeta'
import type { OutputPreview as Preview } from './generated/OutputPreview'
import type { CaptureOutputRequest } from './generated/CaptureOutputRequest'
import type { RenderOutputSettings } from './generated/RenderOutputSettings'
import type { OutputScope } from './generated/OutputScope'
import { apiUrl } from './urls'
import { boundedBlob, downloadBlob, downloadFilename } from './download'

type Request = CaptureOutputRequest & {format: string} & Partial<Omit<RenderOutputSettings, 'format'>>
function element<K extends keyof HTMLElementTagNameMap>(tag: K, text = ''): HTMLElementTagNameMap[K] { const node = document.createElement(tag); node.textContent = text; return node }

/** A preview is a revision-bound offer; changing any input retires it. */
export class ScopedExport {
  private readonly dialog = element('dialog')
  private readonly scope = element('select')
  private readonly format = element('select')
  private readonly width = element('input')
  private readonly height = element('input')
  private readonly imageSettings = element('fieldset')
  private readonly status = element('p')
  private readonly image = element('img')
  private readonly previewButton = element('button', 'Preview export')
  private readonly downloadButton = element('button', 'Download previewed export')
  private snapshot: SharedSnapshotMeta | null = null
  private offer: {preview: Preview; request: Request; image: boolean} | null = null
  private token = 0
  private busy = false
  private opener: HTMLElement | null = null
  constructor(host: HTMLElement) {
    this.dialog.className = 'kglv-output-dialog'; this.dialog.dataset['testid'] = 'scoped-export-dialog'
    const heading = element('h2', 'Export a captured view')
    const hint = element('p', 'Choose an instance scope. Schema navigation does not change these sets. Images capture shared selection and settings in a deterministic server layout. Local hover, local selection and the current camera are not captured.')
    this.scope.append(new Option('Visible instance subset — exact relations', 'visible'), new Option('Loaded nodes + induced relations', 'loaded-induced'))
    this.scope.dataset['testid'] = 'export-scope'
    for (const [value, label] of [['graphml', 'GraphML'], ['gexf', 'GEXF'], ['csv', 'CSV nodes'], ['csv-edges', 'CSV edges'], ['json', 'JSON graph'], ['svg', 'SVG image'], ['png', 'PNG image']]) this.format.append(new Option(label, value))
    this.format.dataset['testid'] = 'export-format'
    this.imageSettings.append(element('legend', 'Deterministic image'))
    for (const [input, name, value] of [[this.width, 'Width', 2000], [this.height, 'Height', 1250]] as const) {
      input.type = 'number'; input.min = '200'; input.max = '8000'; input.step = '1'; input.value = String(value); input.required = true
      input.dataset['testid'] = `export-${name.toLowerCase()}`; this.imageSettings.append(this.label(name, input))
    }
    this.imageSettings.append(element('p', 'Pixels · dark theme · seed 0 · automatic structural layout. Label omissions and bounds are reported with the preview.'))
    this.status.dataset['testid'] = 'export-preview-status'; this.status.setAttribute('role', 'status')
    this.image.alt = 'Deterministic preview of the captured instance scope'; this.image.hidden = true; this.image.dataset['testid'] = 'export-image-preview'
    this.previewButton.dataset['testid'] = 'export-preview'; this.previewButton.onclick = () => { void this.preview() }
    this.downloadButton.dataset['testid'] = 'export-download'; this.downloadButton.onclick = () => { void this.download() }
    const close = element('button', 'Close'); close.dataset['testid'] = 'export-close'; close.onclick = () => this.dialog.close()
    for (const button of [this.previewButton, this.downloadButton, close]) { button.type = 'button'; button.className = 'kglv-button' }
    const actions = element('div'); actions.className = 'kglv-data-actions'; actions.append(this.previewButton, this.downloadButton, close)
    this.dialog.append(heading, hint, this.label('Scope', this.scope), this.label('Format', this.format), this.imageSettings, actions, this.status, this.image)
    host.append(this.dialog)
    for (const input of [this.scope, this.format, this.width, this.height]) input.onchange = () => this.invalidate('Settings changed. Preview again before downloading.')
    this.dialog.onclose = () => { this.token += 1; this.busy = false; this.opener?.focus(); this.paint() }
    this.paint()
  }
  private label(text: string, input: HTMLElement): HTMLLabelElement { const label = element('label', text); label.append(input); return label }
  open(opener: HTMLElement): void { this.opener = opener; this.dialog.showModal(); this.scope.focus(); this.paint() }
  update(snapshot: SharedSnapshotMeta): void {
    const changed = this.snapshot?.stamp.generation !== snapshot.stamp.generation || this.snapshot?.stamp.revision !== snapshot.stamp.revision
    this.snapshot = snapshot
    if (changed && (this.offer !== null || this.busy)) this.invalidate('The shared view changed. Preview again before downloading.')
    this.paint()
  }
  private invalidate(message: string): void { this.token += 1; this.offer = null; this.busy = false; this.image.hidden = true; this.image.removeAttribute('src'); this.status.textContent = message; this.paint() }
  private isImage(): boolean { return this.format.value === 'svg' || this.format.value === 'png' }
  private paint(): void {
    this.imageSettings.hidden = !this.isImage()
    this.previewButton.disabled = this.snapshot === null || this.busy
    this.downloadButton.disabled = this.offer === null || this.busy
    for (const input of [this.scope, this.format, this.width, this.height]) input.disabled = this.busy
  }
  private async post(path: string, body: object): Promise<Response> {
    const response = await fetch(apiUrl(path), {method: 'POST', headers: {'content-type': 'application/json'}, body: JSON.stringify(body)})
    if (!response.ok) {
      const error = await response.json().catch(() => ({})) as {message?: string; error?: string}
      throw new Error(error.message ?? error.error ?? `Export refused (${response.status}).`)
    }
    return response
  }
  private async preview(): Promise<void> {
    if (this.snapshot === null || (this.isImage() && (!this.width.reportValidity() || !this.height.reportValidity()))) return
    const token = ++this.token; const image = this.isImage()
    const request: Request = {scope: this.scope.value as OutputScope, expected: {...this.snapshot.stamp}, subset_revision: this.snapshot.subset_revision, format: this.format.value}
    if (image) Object.assign(request, {width: Number(this.width.value), height: Number(this.height.value), seed: 0, theme: 'dark', kernel: 'auto'})
    this.offer = null; this.image.hidden = true; this.busy = true; this.status.textContent = 'Capturing preview…'; this.paint()
    try {
      const response = await this.post(image ? 'api/render/preview' : 'api/export/preview', request)
      const body = await response.json() as Preview & {preview?: Preview; image_base64?: string; rendered?: {names_shown?: number; nodes: number; folded: number; banners: string[]; layout_kernel: string}}
      if (token !== this.token) return
      const preview = image ? body.preview : body
      if (!preview) throw new Error('The server returned no export preview.')
      this.offer = {preview, request, image}
      this.status.textContent = `${image ? 'Deterministic server image' : 'Graph export'} · ${preview.scope} · ${preview.nodes} nodes · ${preview.edges} relations · revision ${preview.stamp.revision}${image ? ` · ${request.width} × ${request.height} pixels` : ''}. ${preview.notes.join(' ')}${body.rendered ? ` Layout: ${body.rendered.layout_kernel}. ${body.rendered.names_shown ?? body.rendered.nodes} names shown; ${body.rendered.folded} nodes folded. ${body.rendered.banners.join(' ')}` : ''}`
      if (image && body.image_base64) { this.image.src = `data:image/${request.format === 'svg' ? 'svg+xml' : 'png'};base64,${body.image_base64}`; this.image.hidden = false }
    } catch (error) { if (token === this.token) this.status.textContent = String(error instanceof Error ? error.message : error) }
    finally { if (token === this.token) { this.busy = false; this.paint() } }
  }
  private async download(): Promise<void> {
    const offer = this.offer; if (!offer) return
    const token = ++this.token; this.busy = true; this.paint()
    try {
      const response = await this.post(offer.image ? 'api/render/download' : 'api/export/download', {...offer.request, preview_digest: offer.preview.preview_digest})
      const blob = await boundedBlob(response); if (token !== this.token) return
      downloadBlob(blob, downloadFilename(response.headers.get('content-disposition'), `graph-${offer.preview.scope}.${offer.request.format}`))
      this.status.textContent = `Downloaded ${offer.preview.nodes} nodes and ${offer.preview.edges} relations from revision ${offer.preview.stamp.revision}.`
    } catch (error) { if (token === this.token) { this.offer = null; this.status.textContent = `${error instanceof Error ? error.message : String(error)} Preview again before downloading.` } }
    finally { if (token === this.token) { this.busy = false; this.paint() } }
  }
}
