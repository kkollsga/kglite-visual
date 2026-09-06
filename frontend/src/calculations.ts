import type { SharedSnapshotMeta } from './generated/SharedSnapshotMeta'
import type { CalculationKind } from './generated/CalculationKind'
import type { FieldRef } from './generated/FieldRef'
import { calculationLabel, calculationMatchesSubset, calculationListKey } from './fields'

function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = ''): HTMLElementTagNameMap[K] { const result = document.createElement(tag); result.textContent = text; return result }
function button(text: string, id: string, action: () => void): HTMLButtonElement { const result = node('button', text); result.type = 'button'; result.className = 'kglv-button'; result.dataset['testid'] = id; result.onclick = action; return result }
type Handlers = {calculate(kind: CalculationKind, id?: string): string; inspect(fields: FieldRef[]): void}

/** Calculations are frozen core results; this controller never derives graph values. */
export class Calculations {
  private readonly list = node('div')
  private readonly status = node('p', 'Calculate over the acknowledged visible instances and exact relation records.')
  private readonly kind = node('select')
  private readonly run: HTMLButtonElement
  private snapshot: SharedSnapshotMeta | null = null
  private pending: {requestId: string; existingId?: string; previousIds: Set<string>} | null = null
  private rendered = ''
  constructor(host: HTMLElement, private readonly handlers: Handlers) {
    const section = node('section'); section.className = 'kglv-calculations'; section.dataset['testid'] = 'calculations'
    const heading = node('h2', 'Calculated fields')
    const explanation = node('p', 'Input: the current visible subset, including parallel relation records and self-loops. Values stay frozen until explicitly recomputed; source properties are unchanged.')
    this.kind.append(new Option('Directed degree', 'degree'), new Option('Weak components', 'weak-components'))
    this.kind.className = 'kglv-select'; this.kind.dataset['testid'] = 'calculation-kind'; this.kind.setAttribute('aria-label', 'Calculation kind')
    this.run = button('Calculate visible subset', 'calculation-run', () => this.start(this.kind.value as CalculationKind))
    const actions = node('div'); actions.className = 'kglv-data-actions'; actions.append(this.kind, this.run)
    this.status.dataset['testid'] = 'calculation-status'; this.status.setAttribute('role', 'status')
    section.append(heading, explanation, actions, this.status, this.list); host.append(section); this.busy()
  }
  private start(kind: CalculationKind, existingId?: string): void {
    this.pending = {requestId: this.handlers.calculate(kind, existingId), existingId, previousIds: new Set(this.snapshot?.calculations.map(item => item.id) ?? [])}
    this.status.textContent = existingId ? 'Recomputing against the current visible subset…' : 'Calculating the current visible subset…'; this.busy()
  }
  update(snapshot: SharedSnapshotMeta, requestId: string | null): void {
    this.snapshot = snapshot
    if (requestId !== null && this.pending?.requestId === requestId) {
      const calculation = snapshot.calculations.find(item => this.pending?.existingId ? item.id === this.pending.existingId : !this.pending?.previousIds.has(item.id))
      this.pending = null
      this.status.textContent = calculation ? `${calculationLabel(calculation.kind)} ${calculation.id} ready · ${calculation.node_count} instances · ${calculation.edge_count} relation records.` : 'Calculation acknowledged.'
      if (calculation) this.handlers.inspect(calculation.fields.map(item => item.field))
    }
    const key = calculationListKey(snapshot)
    if (key !== this.rendered) { this.rendered = key; this.renderList(snapshot) }
    this.busy()
  }
  error(message: string): void { this.pending = null; this.status.textContent = message; this.busy() }
  disconnected(): void { if (this.pending) this.error('Connection changed. Inspect the acknowledged calculation list before retrying.') }
  private busy(): void {
    this.run.disabled = this.snapshot === null || this.pending !== null || this.snapshot.calculations.length >= 8
    this.kind.disabled = this.pending !== null
    for (const button of this.list.querySelectorAll<HTMLButtonElement>('button[data-testid="calculation-recompute"]')) button.disabled = this.pending !== null
    this.run.title = (this.snapshot?.calculations.length ?? 0) >= 8 ? 'Eight calculations are retained. Recompute an existing result to reuse its fields.' : ''
  }
  private renderList(snapshot: SharedSnapshotMeta): void {
    this.list.replaceChildren()
    for (const calculation of snapshot.calculations) {
      const row = node('article'); row.className = 'kglv-calculation'; row.dataset['calculationId'] = calculation.id
      const title = node('h3', `${calculationLabel(calculation.kind)} · ${calculation.id}`)
      const scope = node('p', `${calculation.node_count} instances · ${calculation.edge_count} relation records · input revision ${calculation.input_stamp.revision} · ${calculationMatchesSubset(calculation, snapshot) ? 'Frozen; matches current visible subset' : 'Frozen from an earlier visible subset'}`)
      scope.dataset['testid'] = 'calculation-scope'
      const inspect = button('Inspect fields', 'calculation-inspect', () => this.handlers.inspect(calculation.fields.map(item => item.field)))
      const recompute = button('Recompute on current visible subset', 'calculation-recompute', () => this.start(calculation.kind, calculation.id))
      const details = node('details'); details.append(node('summary', 'Definition and fields'), node('p', calculation.semantics), node('p', calculation.fields.map(item => `${item.label} (${item.value_type})`).join(' · ')))
      row.append(title, scope, inspect, recompute, details); this.list.append(row)
    }
  }
}
