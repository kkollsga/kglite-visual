import type { QueryTable } from '../generated/QueryTable'
import type { TypedValue } from '../generated/TypedValue'
import { ChartModelError, type ChartMapping } from './types'

export interface ExpandedChartValue {
  x: TypedValue
  y: TypedValue
  series: TypedValue | null
  sourceRow: number
  sourceIndex?: number
}

function columnIndex(table: QueryTable, name: string): number {
  const index = table.columns.indexOf(name)
  if (index < 0) throw new ChartModelError('unknown-column', `Query result has no “${name}” column.`)
  return index
}

function cellAt(table: QueryTable, column: string, row: number): TypedValue {
  const cell = table.cells[columnIndex(table, column)]?.[row]
  if (!cell) throw new ChartModelError('missing-cell', `Column “${column}” has no value at row ${row + 1}.`)
  if (cell.state === 'truncated') {
    throw new ChartModelError('truncated-cell', `Column “${column}” is truncated; return complete rows with UNWIND.`)
  }
  if (cell.state === 'unavailable') {
    throw new ChartModelError('unavailable-cell', `Column “${column}” is unavailable: ${cell.reason}`)
  }
  if (cell.state === 'missing' || cell.state === 'null') return { type: 'null' }
  return cell.value
}

function mapValue(value: TypedValue, key: string, row: number, index: number): TypedValue {
  if (value.type !== 'map') {
    throw new ChartModelError('array-shape', `Point ${index + 1} at row ${row + 1} is not a map.`)
  }
  const found = value.value.find(([name]) => name === key)
  if (!found) throw new ChartModelError('array-key', `Point ${index + 1} has no “${key}” value.`)
  return found[1]
}

function scalarSeries(table: QueryTable, mapping: ChartMapping, row: number): TypedValue | null {
  return mapping.series ? cellAt(table, mapping.series, row) : null
}

export function expandChartValues(table: QueryTable, mapping: ChartMapping): ExpandedChartValue[] {
  const values: ExpandedChartValue[] = []
  for (let row = 0; row < table.bound.returned; row += 1) {
    if (mapping.shape === 'rows') {
      values.push({
        x: cellAt(table, mapping.x, row),
        y: cellAt(table, mapping.y, row),
        series: scalarSeries(table, mapping, row),
        sourceRow: row,
      })
      continue
    }
    if (mapping.shape === 'point-map-array') {
      if (!mapping.points) throw new ChartModelError('array-column', 'Point-map arrays require a containing list column.')
      const list = cellAt(table, mapping.points, row)
      if (list.type !== 'list') throw new ChartModelError('array-shape', `Column “${mapping.points}” is not a list.`)
      list.value.forEach((point, sourceIndex) => {
        values.push({
          x: mapValue(point, mapping.x, row, sourceIndex),
          y: mapValue(point, mapping.y, row, sourceIndex),
          series: scalarSeries(table, mapping, row),
          sourceRow: row,
          sourceIndex,
        })
      })
      continue
    }
    const xs = cellAt(table, mapping.x, row)
    const ys = cellAt(table, mapping.y, row)
    if (xs.type !== 'list' || ys.type !== 'list') {
      throw new ChartModelError('array-shape', 'Paired array mappings require two list columns.')
    }
    if (xs.value.length !== ys.value.length) {
      throw new ChartModelError('array-length', 'Paired arrays must have equal lengths; alignment is never guessed.')
    }
    xs.value.forEach((x, sourceIndex) => {
      const y = ys.value[sourceIndex]
      if (!y) throw new ChartModelError('array-length', 'Paired arrays must have equal lengths.')
      values.push({ x, y, series: scalarSeries(table, mapping, row), sourceRow: row, sourceIndex })
    })
  }
  return values
}
