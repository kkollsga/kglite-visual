import { expect, test } from '@playwright/test'

import { analyzeQueryResult, buildChart } from '../../src/charts/model'
import { ChartModelError, MAX_CHART_POINTS } from '../../src/charts/types'
import type { QueryTable } from '../../src/generated/QueryTable'
import type { RecordCell } from '../../src/generated/RecordCell'
import type { TypedValue } from '../../src/generated/TypedValue'

const tv = {
  string: (value: string): TypedValue => ({ type: 'string', value }),
  date: (value: string): TypedValue => ({ type: 'date', value }),
  int: (value: string): TypedValue => ({ type: 'int64', value }),
  float: (value: number): TypedValue => ({ type: 'float64', value }),
  null: (): TypedValue => ({ type: 'null' }),
  list: (value: TypedValue[]): TypedValue => ({ type: 'list', value }),
  map: (value: Array<[string, TypedValue]>): TypedValue => ({ type: 'map', value }),
}

function table(columns: string[], rows: TypedValue[][], truncated = false, total = rows.length): QueryTable {
  const cells = columns.map((_, column) => rows.map((row): RecordCell => {
    const value = row[column] ?? tv.null()
    return value.type === 'null' ? { state: 'null' } : { state: 'value', value }
  }))
  return {
    cells,
    columns,
    data: columns.map(() => []),
    bound: { returned: rows.length, total, truncated },
    stamp: null,
    row_references: [],
    graph_references_truncated: false,
    protocol_version: 10,
    elapsed_ms: 0,
    node_ids: [],
    relationships: [],
    explain: false,
    warnings: [],
    profile: null,
  }
}

function failure(run: () => unknown): ChartModelError {
  try {
    run()
  } catch (error) {
    expect(error).toBeInstanceOf(ChartModelError)
    return error as ChartModelError
  }
  throw new Error('expected chart construction to fail')
}

test('flat rows preserve typed series identity, category indexes and missing y gaps', () => {
  const result = table(['category', 'value', 'series'], [
    [tv.string('A'), tv.float(2), tv.int('7')],
    [tv.string('B'), tv.null(), tv.int('7')],
    [tv.string('A'), tv.float(3), tv.string('7')],
  ])
  const chart = buildChart(result, { kind: 'bar', shape: 'rows', x: 'category', y: 'value', series: 'series' })
  expect(chart.series.map((series) => series.key)).toEqual(['{"type":"int64","value":"7"}', '{"type":"string","value":"7"}'])
  expect(chart.xLabels).toEqual(['A', 'B'])
  expect(chart.series[0]?.points[1]?.y).toBeNull()
  expect(chart.coverage.missingY).toBe(1)
})

test('complete point maps inherit their parent scalar series and paired arrays never form products', () => {
  const points = table(['field', 'points'], [[tv.string('FIELD A'), tv.list([
    tv.map([['time', tv.date('2024-01-01')], ['value', tv.float(1)]]),
    tv.map([['time', tv.date('2024-02-01')], ['value', tv.float(2)]]),
  ])]])
  const mapped = buildChart(points, { kind: 'line', shape: 'point-map-array', points: 'points', x: 'time', y: 'value', series: 'field' })
  expect(mapped.coverage.expandedPoints).toBe(2)
  expect(mapped.series[0]?.label).toBe('FIELD A')

  const paired = table(['months', 'values'], [[tv.list([tv.date('2024-01-01'), tv.date('2024-02-01')]), tv.list([tv.float(1), tv.float(2)])]])
  expect(buildChart(paired, { kind: 'line', shape: 'paired-arrays', x: 'months', y: 'values' }).series[0]?.points).toHaveLength(2)
  expect(failure(() => buildChart(table(['x', 'y'], [[tv.list([tv.int('1')]), tv.list([])]]), { kind: 'line', shape: 'paired-arrays', x: 'x', y: 'y' })).code).toBe('array-length')
})

test('bounds, truncation, unsafe values and line duplicates refuse misleading charts', () => {
  expect(failure(() => buildChart(table(['x', 'y'], [[tv.int('1'), tv.float(2)]], true, 3), { kind: 'line', shape: 'rows', x: 'x', y: 'y' })).code).toBe('truncated-result')
  const huge = table(['x', 'y'], [[tv.list(Array.from({ length: MAX_CHART_POINTS + 1 }, (_, index) => tv.int(String(index)))), tv.list(Array.from({ length: MAX_CHART_POINTS + 1 }, () => tv.float(1)))]])
  expect(failure(() => buildChart(huge, { kind: 'line', shape: 'paired-arrays', x: 'x', y: 'y' })).code).toBe('point-bound')
  expect(failure(() => buildChart(table(['x', 'y'], [[tv.int('9007199254740993'), tv.float(1)]]), { kind: 'scatter', shape: 'rows', x: 'x', y: 'y' })).code).toBe('unsafe-number')
  const duplicate = table(['x', 'y'], [[tv.int('1'), tv.float(2)], [tv.int('1'), tv.float(3)]])
  expect(failure(() => buildChart(duplicate, { kind: 'line', shape: 'rows', x: 'x', y: 'y' })).code).toBe('duplicate-x')
  expect(buildChart(duplicate, { kind: 'scatter', shape: 'rows', x: 'x', y: 'y' }).series[0]?.points).toHaveLength(2)
})

test('monthly conversion uses leap calendar days and inserts absent months as gaps', () => {
  const result = table(['month', 'volume'], [
    [tv.date('2024-01-01'), tv.float(31)],
    [tv.date('2024-03-01'), tv.float(31)],
  ])
  const chart = buildChart(
    result,
    { kind: 'line', shape: 'rows', x: 'month', y: 'volume' },
    { transform: { kind: 'monthly-average-calendar-day-rate', confirmedMonthly: true, scale: 1 } },
  )
  expect(chart.series[0]?.points.map((point) => point.y)).toEqual([1, null, 1])
  expect(chart.coverage.insertedGaps).toBe(1)
  expect(new Date(chart.series[0]?.points[1]?.x ?? 0).toISOString().slice(0, 10)).toBe('2024-02-01')

  const february = buildChart(table(['month', 'volume'], [[tv.date('2024-02-01'), tv.float(29)]]), { kind: 'line', shape: 'rows', x: 'month', y: 'volume' }, { transform: { kind: 'monthly-average-calendar-day-rate', confirmedMonthly: true, scale: 1 } })
  expect(february.series[0]?.points[0]?.y).toBe(1)
  expect(failure(() => buildChart(result, { kind: 'line', shape: 'rows', x: 'month', y: 'volume' }, { transform: { kind: 'monthly-average-calendar-day-rate', confirmedMonthly: false, scale: 1 } })).code).toBe('monthly-confirmation')
  expect(failure(() => buildChart(table(['month', 'volume'], [[tv.date('2024-02-30'), tv.float(1)]]), { kind: 'line', shape: 'rows', x: 'month', y: 'volume' }, { transform: { kind: 'monthly-average-calendar-day-rate', confirmedMonthly: true, scale: 1 } })).code).toBe('invalid-x')
})

test('line suggestions include a compatible scalar group and coverage counts visible values', () => {
  const production = table(['field', 'month', 'oil'], [
    [tv.string('A'), tv.date('2024-01-01'), tv.float(1)],
    [tv.string('B'), tv.date('2024-01-01'), tv.float(2)],
    [tv.string('A'), tv.date('2024-02-01'), tv.null()],
  ])
  const line = analyzeQueryResult(production).suggestions.find((suggestion) => suggestion.kind === 'line')
  expect(line?.mapping.series).toBe('field')
  const chart = buildChart(production, line!.mapping)
  expect(chart.coverage).toMatchObject({ expandedPoints: 3, plottedPoints: 2, missingY: 1 })
})

test('calendar expansion and transformed overflow fail before creating an unbounded model', () => {
  const distant = table(['month', 'volume'], [
    [tv.date('1000-01-01'), tv.float(1)],
    [tv.date('9999-01-01'), tv.float(1)],
  ])
  expect(failure(() => buildChart(distant, { kind: 'line', shape: 'rows', x: 'month', y: 'volume' }, { transform: { kind: 'monthly-average-calendar-day-rate', confirmedMonthly: true, scale: 1 } })).code).toBe('point-bound')
  expect(failure(() => buildChart(table(['month', 'volume'], [[tv.date('2024-01-01'), tv.float(Number.MAX_VALUE)]]), { kind: 'line', shape: 'rows', x: 'month', y: 'volume' }, { transform: { kind: 'monthly-average-calendar-day-rate', confirmedMonthly: true, scale: Number.MAX_VALUE } })).code).toBe('unsafe-number')
})
