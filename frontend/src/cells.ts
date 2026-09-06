import type { RecordCell } from './generated/RecordCell'
import type { TypedValue } from './generated/TypedValue'

/** Human text preserves decimal integers and distinguishes empty strings from missing cells. */
export function typedText(value: TypedValue): string {
  switch (value.type) {
    case 'null': return 'null'
    case 'string': return /^(?:\s*|[-+]?\d+(?:\.\d+)?(?:e[-+]?\d+)?|true|false|null)$/i.test(value.value) ? JSON.stringify(value.value) : value.value
    case 'point': return `${value.value.lat}, ${value.value.lon}`
    case 'duration': return `${value.value.months} months, ${value.value.days} days, ${value.value.seconds} seconds`
    case 'list': return `[${value.value.map(typedText).join(', ')}]`
    case 'map': return `{${value.value.map(([key, item]) => `${key}: ${typedText(item)}`).join(', ')}}`
    default: return String(value.value)
  }
}

export function recordText(cell: RecordCell): string {
  switch (cell.state) {
    case 'value': return typedText(cell.value)
    case 'null': return 'null'
    case 'missing': return 'missing'
    case 'unavailable': return `unavailable: ${cell.reason}`
    case 'truncated': return `${cell.preview}… [partial: ${cell.reason}]`
  }
}

const stateOrder = {value: 0, null: 1, missing: 2, truncated: 3, unavailable: 4} as const
const valueOrder = ['unique-id', 'int64', 'float64', 'boolean', 'date', 'timestamp', 'string', 'point', 'duration', 'list', 'map', 'null']
function numeric(value: TypedValue): bigint | number | null {
  if (value.type === 'int64' || value.type === 'unique-id') return BigInt(value.value)
  return value.type === 'float64' ? value.value : null
}

/** Non-values remain last in either direction; numeric comparisons never round int64 keys. */
export function compareCells(a: RecordCell, b: RecordCell, descending = false): number {
  const states = stateOrder[a.state] - stateOrder[b.state]
  if (states !== 0) return states
  if (a.state !== 'value' || b.state !== 'value') return 0
  const left = numeric(a.value)
  const right = numeric(b.value)
  let compared: number
  if (left !== null && right !== null) compared = left < right ? -1 : left > right ? 1 : 0
  else if (a.value.type !== b.value.type) compared = valueOrder.indexOf(a.value.type) - valueOrder.indexOf(b.value.type)
  else compared = typedText(a.value).localeCompare(typedText(b.value), undefined, {numeric: false})
  return descending ? -compared : compared
}

export function recordCell(cell: RecordCell, inspect?: () => void): HTMLElement {
  const content = document.createElement('span')
  content.dataset['cellState'] = cell.state
  content.className = `kglv-record-cell kglv-cell-${cell.state}`
  const text = recordText(cell)
  content.textContent = text.length > 120 ? `${text.slice(0, 120)}…` : text
  if (cell.state === 'value') content.title = cell.value.type
  if (inspect === undefined && text.length > 120) {
    const details = document.createElement('details')
    const summary = document.createElement('summary'); summary.textContent = `${text.slice(0, 120)}…`
    const full = document.createElement('pre'); full.textContent = text
    details.append(summary, full); content.replaceChildren(details)
  }
  if (inspect !== undefined && (text.length > 120 || cell.state === 'truncated' || (cell.state === 'value' && ['list', 'map'].includes(cell.value.type)))) {
    const button = document.createElement('button')
    button.type = 'button'; button.className = 'kglv-button kglv-button-small'; button.textContent = 'Inspect value'
    button.onclick = inspect
    content.append(' ', button)
  }
  return content
}
