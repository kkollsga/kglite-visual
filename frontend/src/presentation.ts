import type { PresentationSettings } from './generated/PresentationSettings'

/** Mirrors core PresentationSettings::default; structural type/instance sizes stay separate. */
export const DEFAULT_PRESENTATION: PresentationSettings = {
  label_density: 1, prioritize_selected_labels: true, prioritize_hovered_labels: false,
  edge_opacity: 1, node_size_min: 4, node_size_max: 22, legend_visible: true,
}
type Key = keyof PresentationSettings
export class PresentationControls {
  private readonly inputs = new Map<Key, HTMLInputElement>()
  private readonly status = document.createElement('p')
  private readonly reset = document.createElement('button')
  private acknowledged = {...DEFAULT_PRESENTATION}
  private pending: string | null = null
  private draft: PresentationSettings | null = null
  constructor(host: HTMLElement, private readonly submit: (settings: PresentationSettings) => string) {
    const section = document.createElement('section'); section.className = 'kglv-readability'
    const heading = document.createElement('h3'); heading.textContent = 'Readability'
    const hint = document.createElement('p'); hint.textContent = 'Shared across browsers. Density controls ordinary instance labels; schema labels remain available. Size range affects numeric size-by only.'
    section.append(heading, hint)
    this.number(section, 'label_density', 'Instance label density', 0, 1, 0.1)
    this.flag(section, 'prioritize_selected_labels', 'Keep selected labels')
    this.flag(section, 'prioritize_hovered_labels', 'Prioritize hovered labels')
    this.number(section, 'edge_opacity', 'Edge opacity', 0, 1, 0.05)
    this.number(section, 'node_size_min', 'Numeric size minimum', 0.1, 64, 0.1)
    this.number(section, 'node_size_max', 'Numeric size maximum', 0.1, 64, 0.1)
    this.flag(section, 'legend_visible', 'Show legend')
    this.reset.type = 'button'; this.reset.className = 'kglv-button'; this.reset.textContent = 'Restore readability defaults'; this.reset.dataset['testid'] = 'readability-reset'
    this.reset.onclick = () => this.send({...DEFAULT_PRESENTATION})
    this.status.dataset['testid'] = 'readability-status'; this.status.setAttribute('role', 'status')
    section.append(this.reset, this.status); host.append(section); this.paint()
  }
  private number(host: HTMLElement, key: Key, text: string, min: number, max: number, step: number): void {
    const input = document.createElement('input'); input.type = 'number'; input.min = String(min); input.max = String(max); input.step = String(step)
    input.onchange = () => { if (input.reportValidity()) this.send({...this.acknowledged, [key]: Number(input.value)}) }
    this.attach(host, key, text, input)
  }
  private flag(host: HTMLElement, key: Key, text: string): void {
    const input = document.createElement('input'); input.type = 'checkbox'; input.onchange = () => this.send({...this.acknowledged, [key]: input.checked})
    this.attach(host, key, text, input)
  }
  private attach(host: HTMLElement, key: Key, text: string, input: HTMLInputElement): void {
    const label = document.createElement('label'); label.textContent = text; label.append(input)
    input.dataset['testid'] = `readability-${key}`; this.inputs.set(key, input); host.append(label)
  }
  private send(settings: PresentationSettings): void {
    this.draft = settings; this.pending = this.submit(settings); this.status.textContent = 'Applying shared readability…'; this.paint()
  }
  update(settings: PresentationSettings, requestId: string | null): void {
    this.acknowledged = settings
    if (requestId !== null && requestId === this.pending) { this.pending = null; this.draft = null; this.status.textContent = 'Shared readability applied.' }
    this.paint()
  }
  disconnected(): void { if (this.pending !== null) this.error('Connection changed. Review the acknowledged settings before applying again.') }
  error(message: string): void { this.pending = null; this.draft = null; this.status.textContent = message; this.paint() }
  private paint(): void {
    for (const [key, input] of this.inputs) {
      const value = (this.draft ?? this.acknowledged)[key]
      if (typeof value === 'boolean') input.checked = value; else input.value = String(value)
      input.disabled = this.pending !== null
    }
    this.reset.disabled = this.pending !== null
  }
}
