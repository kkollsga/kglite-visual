import type { SubsetFilter } from './generated/SubsetFilter'
import type { SubsetPredicate } from './generated/SubsetPredicate'
import type { SubsetSnapshot } from './generated/SubsetSnapshot'
import type { TypedValue } from './generated/TypedValue'

function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = ''): HTMLElementTagNameMap[K] {
  const result = document.createElement(tag)
  result.textContent = text
  return result
}
function select(values: string[], testid: string): HTMLSelectElement {
  const result = node('select')
  result.className = 'kglv-select'
  result.dataset['testid'] = testid
  for (const value of values) { const option = node('option', value); option.value = value; result.append(option) }
  return result
}
function input(label: string, testid: string): HTMLInputElement {
  const result = node('input')
  result.className = 'kglv-input'; result.placeholder = label; result.setAttribute('aria-label', label)
  result.dataset['testid'] = testid
  return result
}

/** Edits predicates, never evaluates them. Counts and distributions are the core's answer. */
export class Filters {
  private predicates: SubsetFilter[] = []
  private readonly list = node('div')
  private readonly status = node('p', 'Filters apply to loaded instances and their relationships.')
  private readonly kind = select(['type', 'category', 'numeric-range', 'missing', 'relation', 'hide-isolated'], 'subset-kind')
  private readonly field = input('Property name', 'subset-field')
  private readonly values = input('Values, separated by commas', 'subset-values')
  private readonly valueType = select(['string', 'int64', 'float64', 'boolean'], 'subset-value-type')
  private readonly min = input('Minimum (inclusive, optional)', 'subset-min')
  private readonly max = input('Maximum (inclusive, optional)', 'subset-max')
  private readonly includeNull = input('Include null', 'subset-null')
  private readonly includeMissing = input('Include missing', 'subset-missing')
  private readonly choices = select([], 'subset-choices')
  private readonly add = node('button', 'Apply filter')
  private readonly clear = node('button', 'Clear filters')
  private readonly detail = node('div')
  private serial = 0
  private types: string[] = []
  private relations: string[] = []

  constructor(host: HTMLElement, private readonly apply: (filters: SubsetFilter[]) => void) {
    const title = node('h2', 'Filters')
    this.status.dataset['testid'] = 'subset-status'; this.status.setAttribute('role', 'status')
    this.list.dataset['testid'] = 'subset-active'; this.detail.dataset['testid'] = 'subset-distributions'
    this.choices.multiple = true; this.choices.setAttribute('aria-label', 'Types or relationships')
    this.kind.setAttribute('aria-label', 'Filter kind'); this.valueType.setAttribute('aria-label', 'Value type')
    this.includeNull.type = 'checkbox'; this.includeMissing.type = 'checkbox'
    const nullLabel = node('label', ' Include null'); nullLabel.prepend(this.includeNull)
    const missingLabel = node('label', ' Include missing'); missingLabel.prepend(this.includeMissing)
    this.add.className = this.clear.className = 'kglv-button'
    this.add.dataset['testid'] = 'subset-apply'; this.clear.dataset['testid'] = 'subset-clear'
    this.add.onclick = () => { try { this.apply([...this.predicates, { id: `ui-${Date.now()}-${++this.serial}`, enabled: true, predicate: this.predicate() }]); this.pending() } catch (error) { this.error(String(error)) } }
    this.clear.onclick = () => { this.apply([]); this.pending() }
    this.kind.onchange = () => this.form()
    const form = node('div'); form.className = 'kglv-filter-form'
    form.append(this.kind, this.choices, this.field, this.valueType, this.values, this.min, this.max, nullLabel, missingLabel, this.add, this.clear)
    host.append(title, this.status, this.list, form, this.detail)
    this.form()
  }

  setSchema(types: string[], relations: string[]): void { this.types = types; this.relations = relations; this.form() }
  error(message: string): void { this.status.textContent = message; this.add.disabled = this.clear.disabled = false }
  private pending(): void { this.status.textContent = 'Applying to the shared view…'; this.add.disabled = this.clear.disabled = true }

  update(snapshot: SubsetSnapshot): void {
    this.predicates = snapshot.predicates
    this.add.disabled = this.clear.disabled = false
    const c = snapshot.counts
    this.status.textContent = `${c.visible_nodes} / ${c.loaded_nodes} loaded instances · ${c.visible_edges} / ${c.loaded_edges} loaded relationships visible`
    this.list.replaceChildren()
    for (const filter of this.predicates) {
      const row = node('div'); row.className = 'kglv-row'
      const toggle = node('input'); toggle.type = 'checkbox'; toggle.checked = filter.enabled
      toggle.setAttribute('aria-label', `Enable ${filter.predicate.kind} filter`)
      toggle.onchange = () => { this.apply(this.predicates.map(item => item.id === filter.id ? {...item, enabled: toggle.checked} : item)); this.pending() }
      const remove = node('button', 'Remove'); remove.className = 'kglv-button kglv-button-small'
      remove.setAttribute('aria-label', `Remove ${filter.predicate.kind} filter`)
      remove.onclick = () => { this.apply(this.predicates.filter(item => item.id !== filter.id)); this.pending() }
      row.append(toggle, node('span', describe(filter.predicate)), remove); this.list.append(row)
    }
    this.detail.replaceChildren()
    for (const distribution of snapshot.distributions) {
      this.detail.append(node('p', `${distribution.scope}: ${distribution.matching_nodes} / ${distribution.input_nodes} match · ${distribution.null} null · ${distribution.missing} missing · ${distribution.unavailable} unavailable`))
      if (distribution.categories.length > 0) this.detail.append(node('p', distribution.categories.map(item => `${JSON.stringify(item.value)}: ${item.count}`).join(' · ')))
      if (distribution.min !== null || distribution.max !== null) this.detail.append(node('p', `Range ${JSON.stringify(distribution.min)} to ${JSON.stringify(distribution.max)}`))
      if (distribution.other_values > 0) this.detail.append(node('p', `${distribution.other_values} other values`))
    }
  }

  private form(): void {
    const kind = this.kind.value
    this.choices.hidden = kind !== 'type' && kind !== 'relation'
    const choices = kind === 'relation' ? this.relations : this.types
    this.choices.replaceChildren(...choices.map(value => { const option = node('option', value); option.value = value; return option }))
    this.field.hidden = !['category', 'numeric-range', 'missing'].includes(kind)
    this.valueType.hidden = kind !== 'category'
    this.values.hidden = kind !== 'category'
    this.min.hidden = this.max.hidden = kind !== 'numeric-range'
    ;(this.includeNull.parentElement as HTMLElement).hidden = (this.includeMissing.parentElement as HTMLElement).hidden = this.field.hidden
  }

  private predicate(): SubsetPredicate {
    const kind = this.kind.value
    const names = [...this.choices.selectedOptions].map(option => option.value)
    if (kind === 'type' || kind === 'relation') {
      if (names.length === 0) throw new Error('Select at least one type or relationship.')
      return kind === 'type' ? {kind, node_types: names} : {kind, names}
    }
    if (kind === 'hide-isolated') return {kind}
    const name = this.field.value.trim()
    if (!name) throw new Error('Enter a source property name.')
    const common = {field: {kind: 'property' as const, name}, include_null: this.includeNull.checked, include_missing: this.includeMissing.checked}
    if (kind === 'missing') return {kind, ...common}
    if (kind === 'numeric-range') return {kind, ...common, min: numeric(this.min.value), max: numeric(this.max.value)}
    const values = this.values.value.split(',').map(value => typed(value.trim(), this.valueType.value))
    return {kind: 'category', ...common, values}
  }
}
function numeric(text: string): TypedValue | null {
  if (!text.trim()) return null
  if (/^-?\d+$/.test(text.trim())) return {type: 'int64', value: text.trim()}
  const value = Number(text)
  if (!Number.isFinite(value)) throw new Error('A numeric range needs finite numbers.')
  return {type: 'float64', value}
}
function typed(value: string, type: string): TypedValue {
  if (type === 'int64') { if (!/^-?\d+$/.test(value)) throw new Error('Integer values need decimal digits.'); return {type, value} }
  if (type === 'float64') { const number = Number(value); if (!Number.isFinite(number)) throw new Error('Enter a finite number.'); return {type, value: number} }
  if (type === 'boolean') { if (!['true','false'].includes(value)) throw new Error('Boolean values are true or false.'); return {type, value: value === 'true'} }
  return {type: 'string', value}
}
function describe(predicate: SubsetPredicate): string {
  if (predicate.kind === 'type') return `Types: ${predicate.node_types.join(', ')}`
  if (predicate.kind === 'relation') return `Relationships: ${predicate.names.join(', ')}`
  if (predicate.kind === 'hide-isolated') return 'Hide isolated instances'
  return `${predicate.kind}: ${predicate.field.kind === 'property' ? predicate.field.name : predicate.field.column}`
}
