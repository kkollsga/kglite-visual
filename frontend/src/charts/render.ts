import type { ChartKind, ChartModel, ChartPoint, ChartSeries } from './types'

const PALETTE = ['#2563eb', '#dc2626', '#059669', '#7c3aed', '#d97706', '#0891b2', '#be185d', '#4f46e5', '#65a30d', '#9333ea', '#ea580c', '#0f766e', '#b91c1c', '#0369a1', '#6d28d9', '#15803d', '#c2410c', '#4338ca', '#a21caf', '#047857']

export interface ChartSource {
  query?: string
  params?: unknown
  label?: string
}

export interface ChartRenderOptions {
  width?: number
  height?: number
  title?: string
  background?: string
  hiddenSeries?: ReadonlySet<string>
  source?: ChartSource
}

export interface InspectedPoint {
  seriesKey: string
  seriesLabel: string
  x: number
  xLabel: string
  y: number | null
  sourceRow: number
  sourceIndex?: number
}

interface Domain { min: number; max: number }
interface Frame { left: number; right: number; top: number; bottom: number; width: number; height: number }

export function chartDimensions(model: ChartModel, options: ChartRenderOptions = {}): {width: number; height: number} {
  const width = Math.max(480, Math.round(options.width ?? 960))
  const count = visibleChartSeries(model, options.hiddenSeries).length
  const columns = Math.max(1, Math.min(4, Math.floor((width - 112) / 180)))
  const rows = Math.max(1, Math.ceil(count / columns))
  // Title/legend + a useful plot + axes/footers. Grow an undersized export;
  // squeezing this stack used to cross the y-axis top and bottom at 20 series.
  const minimumHeight = 58 + rows * 18 + 120 + 125
  return {width, height: Math.max(minimumHeight, Math.round(options.height ?? 600))}
}

export function escapeSvg(value: string): string {
  return value.replace(/[&<>"']/g, char => ({'&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'}[char]!))
}

function finiteDomain(values: number[], includeZero = false): Domain {
  if (values.length === 0) return {min: 0, max: 1}
  let min = Math.min(...values); let max = Math.max(...values)
  if (includeZero) { min = Math.min(0, min); max = Math.max(0, max) }
  if (min === max) {
    const pad = Math.abs(min) * 0.05 || 1
    min -= pad; max += pad
    if (includeZero) { min = Math.min(0, min); max = Math.max(0, max) }
  }
  return {min, max}
}

function scale(value: number, domain: Domain, start: number, end: number): number {
  return start + ((value - domain.min) / (domain.max - domain.min)) * (end - start)
}

function fmt(value: number): string {
  if (Math.abs(value) >= 1_000_000 || (Math.abs(value) > 0 && Math.abs(value) < 0.001)) return value.toExponential(2)
  return new Intl.NumberFormat('en', {maximumFractionDigits: 3}).format(value)
}

function xText(model: ChartModel, point: ChartPoint): string {
  if (model.xKind === 'category') return model.xLabels?.[point.x] ?? String(point.x)
  if (model.xKind === 'date') return new Date(point.x).toISOString().slice(0, 10)
  return fmt(point.x)
}

function ticks(domain: Domain, count = 5): number[] {
  return Array.from({length: count}, (_, i) => domain.min + (domain.max - domain.min) * i / (count - 1))
}

export function visibleChartSeries(model: ChartModel, hidden: ReadonlySet<string> = new Set()): ChartSeries[] {
  return model.series.filter(series => !hidden.has(series.key))
}

export function inspectChartValues(model: ChartModel, hidden: ReadonlySet<string> = new Set()): InspectedPoint[] {
  return visibleChartSeries(model, hidden).flatMap(series => series.points.map(point => ({
    seriesKey: series.key, seriesLabel: series.label, x: point.x, xLabel: xText(model, point),
    y: point.y, sourceRow: point.sourceRow, sourceIndex: point.sourceIndex,
  })))
}

function provenance(model: ChartModel, source: ChartSource | undefined, series: ChartSeries[]): string {
  const points = series.flatMap(item => item.points)
  return JSON.stringify({
    query: source?.query ?? null, params: source?.params ?? null, source: source?.label ?? null,
    result: model.provenance, mapping: model.mapping,
    presentation: {xLabel: model.xLabel ?? null, yLabel: model.yLabel ?? null, unit: model.unit ?? null},
    coverage: model.coverage, transform: model.transform ?? null,
    visible: {series: series.map(item => item.key), plottedPoints: points.filter(point => point.y !== null).length, missingY: points.filter(point => point.y === null).length},
  }).replace(/</g, '\\u003c')
}

function seriesColor(model: ChartModel, series: ChartSeries): string { return PALETTE[Math.max(0, model.series.indexOf(series)) % PALETTE.length]! }

function marks(kind: ChartKind, model: ChartModel, series: ChartSeries[], frame: Frame, xd: Domain, yd: Domain): string {
  const sx = (x: number) => scale(x, xd, frame.left, frame.right)
  const sy = (y: number) => scale(y, yd, frame.bottom, frame.top)
  if (kind === 'line') return series.map(s => {
    const runs: ChartPoint[][] = []; let run: ChartPoint[] = []
    for (const p of s.points) { if (p.y === null) { if (run.length) runs.push(run); run = [] } else run.push(p) }
    if (run.length) runs.push(run)
    return runs.map(points => points.length === 1
      ? `<circle class="series-single" data-series="${escapeSvg(s.key)}" cx="${sx(points[0]!.x).toFixed(2)}" cy="${sy(points[0]!.y!).toFixed(2)}" r="3.5" fill="${seriesColor(model,s)}"><title>${escapeSvg(`${s.label}: ${xText(model, points[0]!)}, ${fmt(points[0]!.y!)}`)}</title></circle>`
      : `<path class="series-line" data-series="${escapeSvg(s.key)}" d="${points.map((p, i) => `${i ? 'L' : 'M'}${sx(p.x).toFixed(2)},${sy(p.y!).toFixed(2)}`).join(' ')}" stroke="${seriesColor(model,s)}"/>`).join('')
  }).join('')
  if (kind === 'scatter') return series.map(s => s.points.filter(p => p.y !== null).map(p => `<circle class="point" cx="${sx(p.x).toFixed(2)}" cy="${sy(p.y!).toFixed(2)}" r="3" fill="${seriesColor(model,s)}"><title>${escapeSvg(`${s.label}: ${xText(model, p)}, ${fmt(p.y!)}`)}</title></circle>`).join('')).join('')
  const all = series.flatMap((s, si) => s.points.filter(p => p.y !== null).map(p => ({s, si, p})))
  const categories = [...new Set(all.map(v => v.p.x))].sort((a, b) => a - b)
  const band = frame.width / Math.max(1, categories.length); const bar = Math.max(1, band * 0.8 / Math.max(1, series.length)); const zero = sy(0)
  return all.map(({s, si, p}) => { const ci = categories.indexOf(p.x); const x = frame.left + ci * band + band * 0.1 + si * bar; const y = sy(p.y!); return `<rect class="bar" x="${x.toFixed(2)}" y="${Math.min(y, zero).toFixed(2)}" width="${bar.toFixed(2)}" height="${Math.max(1, Math.abs(zero-y)).toFixed(2)}" fill="${seriesColor(model,s)}"><title>${escapeSvg(`${s.label}: ${xText(model, p)}, ${fmt(p.y!)}`)}</title></rect>` }).join('')
}

export function renderChartSvg(model: ChartModel, options: ChartRenderOptions = {}): string {
  const {width, height} = chartDimensions(model, options)
  const title = options.title?.trim() || `${model.kind.charAt(0).toUpperCase()}${model.kind.slice(1)} chart`
  const background = options.background ?? '#ffffff'; const series = visibleChartSeries(model, options.hiddenSeries)
  const values = series.flatMap(s => s.points); const xs = values.map(p => p.x); const ys = values.flatMap(p => p.y === null ? [] : [p.y])
  const xd = finiteDomain(xs); const yd = finiteDomain(ys, model.kind === 'bar')
  const legendColumns = Math.max(1, Math.min(4, Math.floor((width - 112) / 180))); const legendRows = Math.max(1, Math.ceil(series.length / legendColumns))
  const frame: Frame = {left: 112, right: width - 30, top: 58 + legendRows * 18, bottom: height - 125, width: width - 142, height: height - 183 - legendRows * 18}
  const categories = [...new Set(xs)].sort((a,b) => a-b); const xTicks = model.kind === 'bar' ? categories.slice(0, 8) : model.xKind === 'category' ? categories.slice(0, 8) : ticks(xd)
  const yTicks = ticks(yd)
  const axis = [
    `<path d="M${frame.left},${frame.top}V${frame.bottom}H${frame.right}" fill="none" stroke="#334155"/>`,
    ...yTicks.map(v => `<g><path d="M${frame.left},${scale(v,yd,frame.bottom,frame.top).toFixed(2)}H${frame.right}" stroke="#e2e8f0"/><text x="${frame.left-10}" y="${(scale(v,yd,frame.bottom,frame.top)+4).toFixed(2)}" text-anchor="end">${escapeSvg(fmt(v))}</text></g>`),
    ...xTicks.map((v,index) => `<text x="${(model.kind === 'bar' ? frame.left + (categories.indexOf(v)+0.5)*frame.width/categories.length : scale(v,xd,frame.left,frame.right)).toFixed(2)}" y="${frame.bottom+24}" text-anchor="${model.kind !== 'bar' && index === 0 ? 'start' : model.kind !== 'bar' && index === xTicks.length-1 ? 'end' : 'middle'}">${escapeSvg(xText(model,{x:v,y:0,sourceRow:0}))}</text>`),
  ].join('')
  const legendWidth = frame.width / legendColumns
  const legend = series.map((s,i) => `<g transform="translate(${frame.left + (i%legendColumns)*legendWidth},${48+Math.floor(i/legendColumns)*18})"><rect width="12" height="12" fill="${seriesColor(model,s)}"/><text x="18" y="11">${escapeSvg(s.label.length > 24 ? `${s.label.slice(0,23)}…` : s.label)}</text></g>`).join('')
  const visiblePoints = series.flatMap(item => item.points); const visiblePlotted = visiblePoints.filter(point => point.y !== null).length; const visibleMissing = visiblePoints.length - visiblePlotted
  const coverage = `${visiblePlotted} visible plotted of ${model.coverage.plottedPoints} plotted points; ${visibleMissing} visible missing y; ${model.coverage.rejectedPoints} rejected.`
  const source = options.source?.query ? `Source query: ${options.source.query}` : (options.source?.label ?? 'Source query unavailable')
  const transform = model.transform ? `Transformation: monthly-average calendar-day rate; source scale ${fmt(model.transform.scale)} before division by actual month length.` : ''
  return `<svg xmlns="http://www.w3.org/2000/svg" role="img" aria-labelledby="chart-title chart-desc" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}"><title id="chart-title">${escapeSvg(title)}</title><desc id="chart-desc">${escapeSvg(`${model.kind} chart. ${coverage} ${transform} ${source}`)}</desc><metadata>${escapeSvg(provenance(model, options.source, series))}</metadata><style>text{font:12px system-ui,sans-serif;fill:#334155}.series-line{fill:none;stroke-width:2;stroke-linejoin:round;stroke-linecap:round}.axis-label{font-size:13px;font-weight:600}.chart-title{font-size:19px;font-weight:700}</style><rect width="100%" height="100%" fill="${escapeSvg(background)}"/><text class="chart-title" x="${frame.left}" y="28">${escapeSvg(title)}</text>${axis}${marks(model.kind,model,series,frame,xd,yd)}<text class="axis-label" x="${(frame.left+frame.right)/2}" y="${frame.bottom+48}" text-anchor="middle">${escapeSvg(model.xLabel ?? 'x')}</text><text class="axis-label" transform="translate(22 ${(frame.top+frame.bottom)/2}) rotate(-90)" text-anchor="middle">${escapeSvg(`${model.yLabel ?? 'y'}${model.unit ? ` (${model.unit})` : ''}`)}</text>${legend}${transform ? `<text x="${frame.left}" y="${height-48}">${escapeSvg(transform)}</text>` : ''}<text x="${frame.left}" y="${height-30}">${escapeSvg(coverage)}</text><text x="${frame.left}" y="${height-12}">${escapeSvg(source.length > 135 ? `${source.slice(0,132)}…` : source)}</text></svg>`
}
