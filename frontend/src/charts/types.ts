import type { QueryTable } from '../generated/QueryTable'
import type { TypedValue } from '../generated/TypedValue'

export const MAX_CHART_POINTS = 5_000
export const MAX_CHART_SERIES = 20
export const MAX_BAR_CATEGORIES = 100

export type ChartKind = 'line' | 'bar' | 'scatter'
export type ChartShape = 'rows' | 'point-map-array' | 'paired-arrays'
export type ChartXKind = 'number' | 'date' | 'category'

export interface ChartMapping {
  kind: ChartKind
  shape: ChartShape
  x: string
  y: string
  /** A scalar result column; for point-map arrays it applies to every point in its parent row. */
  series?: string
  /** The containing list column for point-map arrays. */
  points?: string
}

export interface MonthlyAverageTransform {
  kind: 'monthly-average-calendar-day-rate'
  confirmedMonthly: boolean
  scale: number
}

export interface ChartBuildOptions {
  transform?: MonthlyAverageTransform
  xLabel?: string
  yLabel?: string
  unit?: string
}

export interface ChartColumnProfile {
  name: string
  present: number
  missing: number
  truncated: number
  types: string[]
  numeric: boolean
  strictDates: boolean
  list: boolean
}

export interface ChartSuggestion {
  kind: ChartKind
  mapping: ChartMapping
  reason: string
}

export interface ChartAnalysis {
  rows: number
  truncated: boolean
  columns: ChartColumnProfile[]
  suggestions: ChartSuggestion[]
}

export interface ChartPoint {
  x: number
  y: number | null
  sourceRow: number
  sourceIndex?: number
}

export interface ChartSeries {
  /** Stable typed identity, distinct from its display label. */
  key: string
  label: string
  points: ChartPoint[]
}

export interface ChartCoverage {
  sourceRows: number
  expandedPoints: number
  plottedPoints: number
  missingY: number
  insertedGaps: number
  rejectedPoints: number
}

export interface ChartProvenance {
  tableStamp: QueryTable['stamp']
  resultRowsReturned: number
  resultRowsTotal: number
  resultTruncated: boolean
}

export interface ChartModel {
  kind: ChartKind
  xKind: ChartXKind
  xLabels: string[] | null
  series: ChartSeries[]
  coverage: ChartCoverage
  provenance: ChartProvenance
  xLabel?: string
  yLabel?: string
  unit?: string
  transform?: MonthlyAverageTransform
}

export type ChartScalar = Exclude<TypedValue, { type: 'list' | 'map' | 'point' | 'duration' }>

export class ChartModelError extends Error {
  constructor(
    public readonly code: string,
    message: string,
  ) {
    super(message)
    this.name = 'ChartModelError'
  }
}
