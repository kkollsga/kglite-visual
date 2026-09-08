import type { QueryTable } from '../generated/QueryTable'
import './panel.css'
import type { QueryProvenance } from './provenance'
export type { QueryProvenance } from './provenance'
import { downloadChartPng, downloadChartSvg } from './export'
import { analyzeQueryResult, buildChart } from './model'
import { inspectChartValues, renderChartSvg } from './render'
import type { ChartAnalysis, ChartBuildOptions, ChartKind, ChartMapping, ChartModel, ChartShape } from './types'

type Handlers = { viewChanged(): void }

function el<K extends keyof HTMLElementTagNameMap>(tag: K, text = ''): HTMLElementTagNameMap[K] {
  const result = document.createElement(tag); result.textContent = text; return result
}

function button(text: string, testid: string, action: () => void): HTMLButtonElement {
  const result = el('button', text); result.type = 'button'; result.className = 'kglv-button'
  result.dataset['testid'] = testid; result.onclick = action; return result
}

function labelled(label: string, control: HTMLElement): HTMLLabelElement {
  const row = el('label'); row.className = 'kglv-chart-field'
  row.append(el('span', label), control); return row
}

function select(testid: string): HTMLSelectElement {
  const result = el('select'); result.className = 'kglv-select'; result.dataset['testid'] = testid
  return result
}

/** Chart controls and result tied to exactly one bounded query table. */
export class QueryChartPanel {
  readonly root = el('section')
  private readonly tableTab: HTMLButtonElement
  private readonly chartTab: HTMLButtonElement
  private readonly editButton: HTMLButtonElement
  private readonly setup = el('div')
  private readonly suggestions = el('div')
  private readonly status = el('p')
  private readonly picture = el('div')
  private readonly seriesControls = el('div')
  private readonly sourceDetails = el('details')
  private readonly values = el('div')
  private readonly kind = select('chart-kind')
  private readonly shape = select('chart-shape')
  private readonly x = select('chart-x')
  private readonly y = select('chart-y')
  private readonly series = select('chart-series')
  private readonly points = select('chart-points')
  private readonly xKey = el('input')
  private readonly yKey = el('input')
  private readonly title = el('input')
  private readonly xLabel = el('input')
  private readonly yLabel = el('input')
  private readonly unit = el('input')
  private readonly monthly = el('input')
  private readonly monthlyConfirm = el('input')
  private readonly scale = el('input')
  private table: QueryTable | null = null
  private source: QueryProvenance | null = null
  private analysis: ChartAnalysis | null = null
  private model: ChartModel | null = null
  private hiddenSeries = new Set<string>()
  private chartView = false
  private renderTitle: string | undefined
  private valuePage = 0
  private valuesOpen = false

  constructor(private readonly handlers: Handlers) {
    this.root.className = 'kglv-chart-panel'
    this.root.dataset['testid'] = 'query-chart-panel'
    this.tableTab = button('Table', 'chart-table-view', () => this.setView(false))
    this.chartTab = button('Chart', 'chart-chart-view', () => this.setView(true))
    const tabs = el('div'); tabs.className = 'kglv-data-lanes'; tabs.setAttribute('role', 'tablist')
    for (const tab of [this.tableTab, this.chartTab]) tab.setAttribute('role', 'tab')
    this.editButton = button('Visualize result', 'chart-open', () => { this.setup.hidden = false; this.paintTabs() })
    tabs.append(this.tableTab, this.chartTab, this.editButton)

    for (const [value, label] of [['line', 'Line'], ['bar', 'Bar'], ['scatter', 'Scatter']]) this.kind.append(new Option(label, value))
    for (const [value, label] of [['rows', 'Scalar rows'], ['point-map-array', 'Point-map array'], ['paired-arrays', 'Paired arrays']]) this.shape.append(new Option(label, value))
    this.series.append(new Option('One series', ''))
    this.monthly.type = 'checkbox'; this.monthly.dataset['testid'] = 'chart-monthly'
    this.monthlyConfirm.type = 'checkbox'; this.monthlyConfirm.dataset['testid'] = 'chart-monthly-confirm'
    this.scale.type = 'number'; this.scale.step = 'any'; this.scale.value = '1'; this.scale.className = 'kglv-input'; this.scale.dataset['testid'] = 'chart-scale'
    for (const [input, testid, placeholder] of [[this.xKey, 'chart-x-key', 'time'], [this.yKey, 'chart-y-key', 'value'], [this.title, 'chart-title', 'Chart title'], [this.xLabel, 'chart-x-label', 'x axis'], [this.yLabel, 'chart-y-label', 'y axis'], [this.unit, 'chart-unit', 'unit']] as const) {
      input.className = 'kglv-input'; input.dataset['testid'] = testid; input.placeholder = placeholder
    }
    const mappings = el('div'); mappings.className = 'kglv-chart-fields'
    mappings.append(labelled('chart', this.kind), labelled('source shape', this.shape), labelled('x column', this.x), labelled('y column', this.y), labelled('series / group', this.series), labelled('array column', this.points), labelled('point x key', this.xKey), labelled('point y key', this.yKey))
    const labels = el('div'); labels.className = 'kglv-chart-fields'
    labels.append(labelled('title', this.title), labelled('x label', this.xLabel), labelled('y label', this.yLabel), labelled('unit', this.unit))
    const transform = el('div'); transform.className = 'kglv-chart-transform'
    const monthlyLabel = el('label'); monthlyLabel.append(this.monthly, ' monthly total → calendar-day average')
    const confirmLabel = el('label'); confirmLabel.append(this.monthlyConfirm, ' I confirm each source value is one calendar-month total')
    transform.append(monthlyLabel, confirmLabel, labelled('scale before division', this.scale))
    this.status.className = 'kglv-hint'; this.status.setAttribute('role', 'status'); this.status.dataset['testid'] = 'chart-status'
    this.suggestions.className = 'kglv-chart-suggestions'
    this.picture.className = 'kglv-chart-picture'; this.picture.dataset['testid'] = 'chart-picture'
    this.seriesControls.className = 'kglv-chart-series'
    const build = button('Build chart', 'chart-build', () => this.build())
    this.setup.className = 'kglv-chart-setup'; this.setup.hidden = true
    this.setup.append(el('p', 'Suggestions come from every returned typed cell. Confirm or change the mapping; the source table stays available.'), this.suggestions, mappings, labels, transform, build)
    this.shape.onchange = () => this.paintShape()
    this.values.className = 'kglv-chart-values'
    this.root.append(tabs, this.sourceDetails, this.setup, this.status, this.seriesControls, this.picture, this.values)
    this.paintTabs()
  }

  showingChart(): boolean { return this.chartView }

  setResult(table: QueryTable, source: QueryProvenance | null): void {
    this.table = table; this.source = source; this.model = null; this.hiddenSeries.clear(); this.chartView = false
    this.renderTitle = undefined; this.valuePage = 0; this.valuesOpen = false; this.editButton.textContent = 'Visualize result'
    this.picture.replaceChildren(); this.seriesControls.replaceChildren(); this.values.replaceChildren(); this.setup.hidden = true
    this.resetControls(); this.paintSource()
    try {
      this.analysis = analyzeQueryResult(table)
      this.populateColumns(table.columns)
      this.paintSuggestions()
      this.status.className = table.bound.truncated ? 'kglv-hint kglv-warn' : 'kglv-hint'
      this.status.textContent = table.bound.truncated
        ? `Visualization refused: this result contains ${table.bound.returned} of ${table.bound.total} rows. Narrow or aggregate the query.`
        : `${table.bound.returned} complete source rows profiled. Chart settings live only until another result replaces this one.`
    } catch (error) {
      this.analysis = null; this.status.className = 'kglv-hint kglv-error'
      this.status.textContent = error instanceof Error ? error.message : String(error)
    }
    this.paintTabs(); this.handlers.viewChanged()
  }

  clear(): void {
    this.table = null; this.source = null; this.analysis = null; this.model = null; this.chartView = false
    this.picture.replaceChildren(); this.seriesControls.replaceChildren(); this.values.replaceChildren(); this.sourceDetails.replaceChildren(); this.paintTabs()
  }

  private resetControls(): void {
    this.kind.value = 'line'; this.shape.value = 'rows'; this.title.value = ''; this.xLabel.value = ''; this.yLabel.value = ''; this.unit.value = ''
    this.xKey.value = 'time'; this.yKey.value = 'value'; this.monthly.checked = false; this.monthlyConfirm.checked = false; this.scale.value = '1'
  }

  private paintSource(): void {
    const summary = el('summary', 'Chart source and lifetime')
    const body = el('pre')
    body.textContent = this.source === null
      ? 'Source query unavailable: this result was not requested by this browser tab.\nChart settings live only until another query result replaces them.'
      : `${this.source.query}\n\nparameters: ${JSON.stringify(this.source.params, null, 2)}\nrequest: ${this.source.requestId}\nrequested: ${new Date(this.source.requestedAtMs).toISOString()}\nChart settings live only until another query result replaces them.`
    this.sourceDetails.replaceChildren(summary, body)
  }

  private populateColumns(columns: string[]): void {
    for (const control of [this.x, this.y, this.points]) control.replaceChildren(...columns.map(name => new Option(name, name)))
    this.series.replaceChildren(new Option('One series', ''), ...columns.map(name => new Option(name, name)))
    const first = this.analysis?.suggestions[0]?.mapping
    if (first) this.applyMapping(first)
    this.paintShape()
  }

  private paintSuggestions(): void {
    const suggestions = this.analysis?.suggestions ?? []
    if (suggestions.length === 0) {
      this.suggestions.replaceChildren(el('p', 'No automatic chart suggestion fits this result. Choose a mapping, or return complete date/category and numeric values.'))
      return
    }
    this.suggestions.replaceChildren(el('p', `${suggestions.length} compatible visualization${suggestions.length === 1 ? '' : 's'} suggested:`), ...suggestions.map((suggestion, index) => {
      const choose = button(`${suggestion.kind}: ${suggestion.reason}`, `chart-suggestion-${index}`, () => this.applyMapping(suggestion.mapping))
      choose.className += ' kglv-chart-suggestion'; return choose
    }))
  }

  private applyMapping(mapping: ChartMapping): void {
    this.kind.value = mapping.kind; this.shape.value = mapping.shape
    if (mapping.shape === 'point-map-array') {
      this.points.value = mapping.points ?? ''; this.xKey.value = mapping.x; this.yKey.value = mapping.y
    } else { this.x.value = mapping.x; this.y.value = mapping.y }
    this.series.value = mapping.series ?? ''; this.paintShape()
  }

  private paintShape(): void {
    const points = this.shape.value === 'point-map-array'
    this.points.closest('label')!.hidden = !points; this.xKey.closest('label')!.hidden = !points; this.yKey.closest('label')!.hidden = !points
    this.x.closest('label')!.hidden = points; this.y.closest('label')!.hidden = points
  }

  private mapping(): ChartMapping {
    const shape = this.shape.value as ChartShape
    return {
      kind: this.kind.value as ChartKind, shape,
      x: shape === 'point-map-array' ? this.xKey.value.trim() : this.x.value,
      y: shape === 'point-map-array' ? this.yKey.value.trim() : this.y.value,
      series: this.series.value || undefined,
      points: shape === 'point-map-array' ? this.points.value : undefined,
    }
  }

  private options(): ChartBuildOptions {
    return {
      xLabel: this.xLabel.value.trim() || undefined,
      yLabel: this.yLabel.value.trim() || undefined,
      unit: this.unit.value.trim() || undefined,
      transform: this.monthly.checked ? {kind: 'monthly-average-calendar-day-rate', confirmedMonthly: this.monthlyConfirm.checked, scale: Number(this.scale.value)} : undefined,
    }
  }

  private build(): void {
    if (this.table === null) return
    try {
      this.model = buildChart(this.table, this.mapping(), this.options()); this.hiddenSeries.clear(); this.renderTitle = this.title.value.trim() || undefined; this.valuePage = 0
      this.chartView = true; this.status.className = 'kglv-hint'
      this.status.textContent = `${this.model.coverage.plottedPoints} plotted points; ${this.model.coverage.missingY} missing y values; ${this.model.coverage.insertedGaps} calendar gaps inserted.`
      this.setup.hidden = true; this.editButton.textContent = 'Edit chart'
      this.paintChart(); this.paintTabs(); this.handlers.viewChanged()
      requestAnimationFrame(() => this.picture.scrollIntoView({block: 'start', behavior: 'smooth'}))
    } catch (error) {
      this.status.className = 'kglv-hint kglv-error'; this.status.textContent = error instanceof Error ? error.message : String(error)
    }
  }

  private renderOptions(theme: 'light' | 'dark' = 'light') {
    return {
      title: this.renderTitle,
      theme,
      hiddenSeries: this.hiddenSeries,
      source: this.source === null ? {label: 'Source query unavailable (external result)'} : {query: this.source.query, params: this.source.params, label: `request ${this.source.requestId}`},
    }
  }

  private paintChart(): void {
    const model = this.model
    if (model === null) { this.picture.replaceChildren(); this.seriesControls.replaceChildren(); return }
    this.picture.innerHTML = renderChartSvg(model, this.renderOptions('dark'))
    this.seriesControls.replaceChildren(...model.series.map(series => {
      const input = el('input'); input.type = 'checkbox'; input.checked = !this.hiddenSeries.has(series.key)
      input.onchange = () => { if (input.checked) this.hiddenSeries.delete(series.key); else this.hiddenSeries.add(series.key); this.paintChart() }
      const label = el('label'); label.append(input, ` ${series.label}`); return label
    }), button('Download SVG', 'chart-svg', () => downloadChartSvg(model, this.renderOptions())), button('Download PNG', 'chart-png', () => { void downloadChartPng(model, this.renderOptions()).catch(error => { this.status.className = 'kglv-hint kglv-error'; this.status.textContent = error instanceof Error ? error.message : String(error) }) }))
    this.paintValues()
  }

  private paintValues(): void {
    const model = this.model
    if (model === null || !this.chartView) { this.values.replaceChildren(); return }
    const rows = inspectChartValues(model, this.hiddenSeries)
    const pageSize = 100; const pages = Math.max(1, Math.ceil(rows.length / pageSize)); this.valuePage = Math.min(this.valuePage, pages - 1)
    const first = this.valuePage * pageSize; const shown = rows.slice(first, first + pageSize)
    const current = this.values.querySelector('details')
    if (current !== null) this.valuesOpen = current.open
    const details = el('details'); details.dataset['testid'] = 'chart-values'; details.open = this.valuesOpen
    details.ontoggle = () => { this.valuesOpen = details.open }
    details.append(el('summary', `Inspect plotted values · ${rows.length} visible`))
    const table = el('table'); table.className = 'kglv-table'
    const head = el('tr'); for (const name of ['Series', 'x', 'y', 'Source']) head.append(el('th', name)); table.append(head)
    for (const value of shown) {
      const row = el('tr'); row.append(el('td', value.seriesLabel), el('td', value.xLabel), el('td', value.y === null ? 'Missing — gap' : String(value.y)), el('td', `row ${value.sourceRow + 1}${value.sourceIndex === undefined ? '' : ` · item ${value.sourceIndex + 1}`}`)); table.append(row)
    }
    const pager = el('div'); pager.className = 'kglv-data-actions'
    const previous = button('Previous values', 'chart-values-previous', () => { this.valuesOpen = true; this.valuePage -= 1; this.paintValues() }); previous.disabled = this.valuePage === 0
    const next = button('Next values', 'chart-values-next', () => { this.valuesOpen = true; this.valuePage += 1; this.paintValues() }); next.disabled = this.valuePage + 1 >= pages
    pager.append(previous, el('span', `${rows.length === 0 ? 0 : first + 1}–${Math.min(first + pageSize, rows.length)} of ${rows.length}`), next)
    details.append(table, pager); this.values.replaceChildren(details)
  }

  private setView(chart: boolean): void {
    if (chart && this.model === null) { this.setup.hidden = false; this.status.textContent = 'Confirm a mapping and build the chart first.'; return }
    this.chartView = chart; this.paintTabs(); this.handlers.viewChanged()
  }

  private paintTabs(): void {
    this.tableTab.setAttribute('aria-selected', String(!this.chartView)); this.chartTab.setAttribute('aria-selected', String(this.chartView))
    this.chartTab.disabled = this.model === null
    this.picture.hidden = !this.chartView; this.seriesControls.hidden = !this.chartView
    this.values.hidden = !this.chartView
  }
}
