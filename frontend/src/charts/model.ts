import type { QueryTable } from '../generated/QueryTable'
import type { TypedValue } from '../generated/TypedValue'
import { expandChartValues } from './arrays'
import {
  ChartModelError,
  MAX_BAR_CATEGORIES,
  MAX_CHART_POINTS,
  MAX_CHART_SERIES,
  type ChartAnalysis,
  type ChartBuildOptions,
  type ChartColumnProfile,
  type ChartMapping,
  type ChartModel,
  type ChartPoint,
  type ChartSeries,
  type ChartXKind,
} from './types'

const ISO_DATE = /^(\d{4})-(\d{2})-(\d{2})$/

function strictDate(value: TypedValue): Date | null {
  if (value.type !== 'date' && value.type !== 'string') return null
  const match = ISO_DATE.exec(value.value)
  if (!match) return null
  const year = Number(match[1])
  const month = Number(match[2])
  const day = Number(match[3])
  const date = new Date(Date.UTC(year, month - 1, day))
  return date.getUTCFullYear() === year && date.getUTCMonth() === month - 1 && date.getUTCDate() === day
    ? date
    : null
}

function finiteNumber(value: TypedValue): number | null {
  if (value.type === 'float64') {
    if (!Number.isFinite(value.value)) throw new ChartModelError('unsafe-number', 'Chart values must be finite numbers.')
    return value.value
  }
  if (value.type === 'int64') {
    const number = Number(value.value)
    if (!Number.isSafeInteger(number) || BigInt(value.value) !== BigInt(number)) {
      throw new ChartModelError('unsafe-number', `Integer ${value.value} cannot be represented safely in a chart.`)
    }
    return number
  }
  return null
}

function isSafeNumeric(value: TypedValue): boolean {
  try {
    return finiteNumber(value) !== null
  } catch {
    return false
  }
}

function typedKey(value: TypedValue | null): string {
  return value ? JSON.stringify(value) : 'default'
}

function display(value: TypedValue | null): string {
  if (!value) return 'Series'
  if ('value' in value && ['string', 'date', 'timestamp', 'int64', 'float64', 'boolean', 'unique-id'].includes(value.type)) {
    return String(value.value)
  }
  if (value.type === 'null') return 'Null'
  return JSON.stringify(value)
}

function profileColumn(table: QueryTable, index: number): ChartColumnProfile {
  const types = new Set<string>()
  let present = 0
  let missing = 0
  let truncated = 0
  let numeric = true
  let dates = true
  let list = true
  for (let row = 0; row < table.bound.returned; row += 1) {
    const cell = table.cells[index]?.[row]
    if (!cell || cell.state === 'missing' || cell.state === 'null') {
      missing += 1
      continue
    }
    if (cell.state === 'truncated') {
      truncated += 1
      continue
    }
    if (cell.state !== 'value') {
      missing += 1
      continue
    }
    present += 1
    types.add(cell.value.type)
    numeric &&= isSafeNumeric(cell.value)
    dates &&= strictDate(cell.value) !== null
    list &&= cell.value.type === 'list'
  }
  return { name: table.columns[index] ?? '', present, missing, truncated, types: [...types].sort(), numeric: present > 0 && numeric, strictDates: present > 0 && dates, list: present > 0 && list }
}

function pointMapSuggestion(table: QueryTable, columns: ChartColumnProfile[]): ChartAnalysis['suggestions'][number] | null {
  const listColumn = columns.find((column) => column.list && column.truncated === 0)
  if (!listColumn) return null
  const listIndex = table.columns.indexOf(listColumn.name)
  const maps = (table.cells[listIndex] ?? []).flatMap((cell) =>
    cell.state === 'value' && cell.value.type === 'list' ? cell.value.value : [],
  )
  if (maps.length === 0 || maps.some((value) => value.type !== 'map')) return null
  const keys = new Map<string, TypedValue[]>()
  for (const value of maps) {
    if (value.type !== 'map') return null
    for (const [key, item] of value.value) keys.set(key, [...(keys.get(key) ?? []), item])
  }
  const x = [...keys].find(([, values]) => values.length === maps.length && values.every((value) => strictDate(value)))?.[0]
  const y = [...keys].find(([, values]) => values.length === maps.length && values.every(isSafeNumeric))?.[0]
  if (!x || !y) return null
  const series = columns.find((column) => column.name !== listColumn.name && !column.list && column.present > 0)?.name
  return {
    kind: 'line',
    mapping: { kind: 'line', shape: 'point-map-array', points: listColumn.name, x, y, ...(series ? { series } : {}) },
    reason: 'A complete list of consistently shaped date/value points supports a time series.',
  }
}

export function analyzeQueryResult(table: QueryTable): ChartAnalysis {
  const columns = table.columns.map((_, index) => profileColumn(table, index))
  const suggestions: ChartAnalysis['suggestions'] = []
  const numeric = columns.filter((column) => column.numeric && column.truncated === 0)
  const temporal = columns.filter((column) => column.strictDates && column.truncated === 0)
  const categorical = columns.filter((column) => column.present > 0 && !column.numeric && !column.strictDates && !column.list && column.truncated === 0)
  const y = numeric[0]
  const date = temporal[0]
  if (date && y && date.name !== y.name) suggestions.push({ kind: 'line', mapping: { kind: 'line', shape: 'rows', x: date.name, y: y.name, ...(categorical[0] ? { series: categorical[0].name } : {}) }, reason: 'A complete date column and numeric value support a time series.' })
  if (numeric.length >= 2) suggestions.push({ kind: 'scatter', mapping: { kind: 'scatter', shape: 'rows', x: numeric[0]!.name, y: numeric[1]!.name }, reason: 'Two numeric columns support a scatter plot.' })
  if (categorical[0] && y) suggestions.push({ kind: 'bar', mapping: { kind: 'bar', shape: 'rows', x: categorical[0].name, y: y.name }, reason: 'A category and numeric value support bars.' })
  const nested = pointMapSuggestion(table, columns)
  if (nested) suggestions.push(nested)
  return {
    rows: table.bound.returned,
    truncated: table.bound.truncated,
    columns,
    suggestions: suggestions.filter((suggestion) => {
      try {
        buildChart(table, suggestion.mapping)
        return true
      } catch {
        return false
      }
    }),
  }
}

function xValue(value: TypedValue, categories: Map<string, number>): { kind: ChartXKind; x: number; label?: string } {
  const date = strictDate(value)
  if (date) return { kind: 'date', x: date.getTime(), label: value.type === 'date' || value.type === 'string' ? value.value : undefined }
  const number = finiteNumber(value)
  if (number !== null) return { kind: 'number', x: number }
  if (value.type === 'null') throw new ChartModelError('missing-x', 'Every chart point needs an x value.')
  if (!['string', 'boolean', 'unique-id'].includes(value.type)) throw new ChartModelError('invalid-x', `Values of type ${value.type} cannot be used on this axis.`)
  const key = typedKey(value)
  let index = categories.get(key)
  if (index === undefined) {
    index = categories.size
    categories.set(key, index)
  }
  return { kind: 'category', x: index, label: display(value) }
}

function calendarMonth(timestamp: number): string {
  const date = new Date(timestamp)
  return `${date.getUTCFullYear()}-${String(date.getUTCMonth() + 1).padStart(2, '0')}`
}

function transformMonthly(series: ChartSeries[], options: ChartBuildOptions): number {
  const transform = options.transform
  if (!transform) return 0
  if (!transform.confirmedMonthly) throw new ChartModelError('monthly-confirmation', 'Confirm that each value is a monthly total before converting it to a daily rate.')
  if (!Number.isFinite(transform.scale)) throw new ChartModelError('invalid-scale', 'Monthly conversion scale must be finite.')
  let inserted = 0
  let modeled = 0
  for (const item of series) {
    const byMonth = new Map<string, ChartPoint>()
    for (const point of item.points) {
      const date = new Date(point.x)
      if (date.getUTCDate() !== 1) throw new ChartModelError('invalid-month', 'Monthly conversion requires calendar month timestamps on day 1.')
      const month = calendarMonth(point.x)
      if (byMonth.has(month)) throw new ChartModelError('duplicate-month', `Series “${item.label}” has more than one observation for ${month}.`)
      if (point.y !== null) {
        point.y = (point.y * transform.scale) / new Date(Date.UTC(date.getUTCFullYear(), date.getUTCMonth() + 1, 0)).getUTCDate()
        if (!Number.isFinite(point.y)) throw new ChartModelError('unsafe-number', 'Monthly conversion produced a non-finite value.')
      }
      byMonth.set(month, point)
    }
    if (item.points.length === 0) continue
    const first = new Date(Math.min(...item.points.map((point) => point.x)))
    const last = new Date(Math.max(...item.points.map((point) => point.x)))
    const filled: ChartPoint[] = []
    for (let cursor = new Date(Date.UTC(first.getUTCFullYear(), first.getUTCMonth(), 1)); cursor <= last; cursor = new Date(Date.UTC(cursor.getUTCFullYear(), cursor.getUTCMonth() + 1, 1))) {
      const existing = byMonth.get(calendarMonth(cursor.getTime()))
      filled.push(existing ?? { x: cursor.getTime(), y: null, sourceRow: -1 })
      if (!existing) inserted += 1
      modeled += 1
      if (modeled > MAX_CHART_POINTS) throw new ChartModelError('point-bound', `Calendar expansion exceeds the ${MAX_CHART_POINTS}-point limit.`)
    }
    item.points = filled
  }
  return inserted
}

export function buildChart(table: QueryTable, mapping: ChartMapping, options: ChartBuildOptions = {}): ChartModel {
  if (table.bound.truncated) throw new ChartModelError('truncated-result', `Query returned ${table.bound.returned} of ${table.bound.total} rows; narrow or aggregate it before charting.`)
  const expanded = expandChartValues(table, mapping)
  if (expanded.length > MAX_CHART_POINTS) throw new ChartModelError('point-bound', `Chart expands to ${expanded.length} points; the limit is ${MAX_CHART_POINTS}.`)
  const categories = new Map<string, number>()
  const bySeries = new Map<string, ChartSeries>()
  let xKind: ChartXKind | null = null
  let missingY = 0
  for (const value of expanded) {
    const normalizedX = xValue(value.x, categories)
    if (xKind && xKind !== normalizedX.kind) throw new ChartModelError('mixed-x', 'The chosen x column mixes incompatible value types.')
    xKind = normalizedX.kind
    const y = value.y.type === 'null' ? null : finiteNumber(value.y)
    if (value.y.type !== 'null' && y === null) throw new ChartModelError('invalid-y', 'The chosen y column must contain only numeric values or gaps.')
    if (y === null) missingY += 1
    const key = typedKey(value.series)
    let series = bySeries.get(key)
    if (!series) {
      if (bySeries.size >= MAX_CHART_SERIES) throw new ChartModelError('series-bound', `Charts support at most ${MAX_CHART_SERIES} series.`)
      series = { key, label: display(value.series), points: [] }
      bySeries.set(key, series)
    }
    if (mapping.kind !== 'scatter' && !options.transform && series.points.some((point) => point.x === normalizedX.x)) {
      throw new ChartModelError('duplicate-x', `Series “${series.label}” contains duplicate x values; aggregate or add a series mapping.`)
    }
    series.points.push({ x: normalizedX.x, y, sourceRow: value.sourceRow, sourceIndex: value.sourceIndex })
  }
  if (mapping.kind === 'bar') {
    const distinctX = new Set([...bySeries.values()].flatMap((item) => item.points.map((point) => point.x)))
    if (distinctX.size > MAX_BAR_CATEGORIES) throw new ChartModelError('category-bound', `Bar charts support at most ${MAX_BAR_CATEGORIES} categories.`)
  }
  if (options.transform && xKind !== 'date') throw new ChartModelError('monthly-x', 'Monthly conversion requires strict calendar dates on the x axis.')
  const series = [...bySeries.values()]
  const insertedGaps = transformMonthly(series, options)
  const modeledPoints = series.reduce((count, item) => count + item.points.length, 0)
  if (modeledPoints > MAX_CHART_POINTS) throw new ChartModelError('point-bound', `Chart contains ${modeledPoints} points after inserting calendar gaps; the limit is ${MAX_CHART_POINTS}.`)
  const plottedPoints = series.reduce((count, item) => count + item.points.filter((point) => point.y !== null).length, 0)
  return {
    kind: mapping.kind,
    mapping: {...mapping},
    xKind: xKind ?? 'number',
    xLabels: xKind === 'category' ? [...categories.keys()].map((key) => display(JSON.parse(key) as TypedValue)) : null,
    series,
    coverage: { sourceRows: table.bound.returned, expandedPoints: expanded.length, plottedPoints, missingY, insertedGaps, rejectedPoints: 0 },
    provenance: { tableStamp: table.stamp, resultRowsReturned: table.bound.returned, resultRowsTotal: table.bound.total, resultTruncated: table.bound.truncated },
    xLabel: options.xLabel ?? mapping.x,
    yLabel: options.yLabel ?? mapping.y,
    unit: options.unit,
    transform: options.transform ? {...options.transform} : undefined,
  }
}
