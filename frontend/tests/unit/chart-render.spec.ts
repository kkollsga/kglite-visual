import { expect, test } from '@playwright/test'
import type { ChartModel } from '../../src/charts/types'
import { escapeSvg, inspectChartValues, renderChartSvg } from '../../src/charts/render'
import { chartPngBlob, safeChartFilename } from '../../src/charts/export'

function model(kind: ChartModel['kind'] = 'line'): ChartModel {
  return {
    kind, xKind: 'number', xLabels: null,
    mapping: {kind, shape:'rows', x:'elapsed', y:'rate', series:'field'},
    series: [
      {key: 'a', label: 'Alpha & <one>', points: [{x: -2, y: -3, sourceRow: 0}, {x: 0, y: null, sourceRow: 1}, {x: 2, y: 4, sourceRow: 2}]},
      {key: 'b', label: 'Beta', points: [{x: -2, y: 2, sourceRow: 3}, {x: 2, y: 2, sourceRow: 4}]},
    ],
    coverage: {sourceRows: 5, expandedPoints: 5, plottedPoints: 4, missingY: 1, insertedGaps: 0, rejectedPoints: 0},
    provenance: {tableStamp: {generation: 'g', revision: '7'}, resultRowsReturned: 5, resultRowsTotal: 5, resultTruncated: false},
    xLabel: 'Elapsed <days>', yLabel: 'Net "rate"', unit: 'm³/day',
  }
}

test('standalone SVG escapes labels, retains provenance and splits line gaps without dropping values', () => {
  const svg = renderChartSvg(model(), {title: 'A < B', source: {query: 'MATCH (n) WHERE n.x < $x RETURN n', params: {x: 4}}})
  expect(svg).toContain('xmlns="http://www.w3.org/2000/svg"')
  expect(svg).toContain('role="img"')
  expect(svg).toContain('A &lt; B')
  expect(svg).not.toContain('Alpha & <one>')
  expect(svg).toContain('Alpha &amp; &lt;one&gt;')
  expect(svg).toContain('&quot;rate&quot;')
  expect(svg).toContain('&quot;x&quot;:4')
  expect(svg).toContain('&quot;mapping&quot;:{&quot;kind&quot;:&quot;line&quot;,&quot;shape&quot;:&quot;rows&quot;,&quot;x&quot;:&quot;elapsed&quot;,&quot;y&quot;:&quot;rate&quot;,&quot;series&quot;:&quot;field&quot;}')
  expect(svg).toContain('&quot;presentation&quot;:{&quot;xLabel&quot;:&quot;Elapsed \\u003cdays&gt;&quot;,&quot;yLabel&quot;:&quot;Net \\&quot;rate\\&quot;&quot;,&quot;unit&quot;:&quot;m³/day&quot;}')
  expect((svg.match(/class="series-line"/g) ?? []).length).toBe(1)
  expect((svg.match(/class="series-single"/g) ?? []).length).toBe(2)
  expect(inspectChartValues(model())).toHaveLength(5)
})

test('visibility applies identically to rendered legend, marks and inspected export values', () => {
  const complete = renderChartSvg(model('scatter')); const hidden = new Set(['a']); const svg = renderChartSvg(model('scatter'), {hiddenSeries: hidden})
  expect(svg).not.toContain('Alpha')
  expect(svg).toContain('Beta')
  expect((svg.match(/class="point"/g) ?? []).length).toBe(2)
  expect(inspectChartValues(model(), hidden).map(point => point.seriesKey)).toEqual(['b', 'b'])
  const betaColor = complete.match(/<circle[^>]+fill="([^"]+)"[^>]*><title>Beta:/)?.[1]
  expect(svg).toContain(`fill="${betaColor}"`)
  expect(svg).toContain('2 visible plotted of 4 plotted points')
  expect(svg).toContain('&quot;plottedPoints&quot;:2')
})

test('bars include a true zero baseline and constant/negative domains stay finite', () => {
  const chart = model('bar'); chart.series = [{key: 'negative', label: 'Negative', points: [{x: 1, y: -4, sourceRow: 0}, {x: 2, y: -4, sourceRow: 1}]}]
  const svg = renderChartSvg(chart)
  expect(svg).toContain('class="bar"')
  expect(svg).not.toContain('NaN')
  expect(svg).not.toContain('Infinity')
  for (const height of [...svg.matchAll(/class="bar"[^>]+height="([^"]+)"/g)].map(match => Number(match[1]))) expect(height).toBeGreaterThan(0)
})

test('category ticks are centered under bars and footer rows remain separate', () => {
  const chart = model('bar'); chart.xKind = 'category'; chart.xLabels = ['First', 'Second']; chart.series = [{key:'a',label:'A',points:[{x:0,y:1,sourceRow:0},{x:1,y:2,sourceRow:1}]}]
  const svg = renderChartSvg(chart, {width: 480, height: 400, source: {query: 'RETURN a reasonably long source query for the readable footer'}})
  const ticks = [...svg.matchAll(/x="([\d.]+)" y="[\d.]+" text-anchor="middle">(First|Second)/g)].map(match => Number(match[1]))
  expect(ticks).toEqual([196.5, 365.5])
  expect(svg).toContain('y="370">2 visible plotted of 4 plotted points')
  expect(svg).toContain('y="388">Source query:')
})

test('numeric bars use equal bands and put each actual x label under its mark', () => {
  const chart = model('bar'); chart.xKind = 'number'; chart.xLabels = null; chart.series = [{key:'a',label:'A',points:[{x:1,y:1,sourceRow:0},{x:2,y:2,sourceRow:1},{x:100,y:3,sourceRow:2}]}]
  const svg = renderChartSvg(chart, {width: 600})
  const ticks = [...svg.matchAll(/x="([\d.]+)" y="[\d.]+" text-anchor="middle">(1|2|100)<\/text>/g)].map(match => Number(match[1]))
  expect(ticks).toEqual([188.33, 341, 493.67])
})

test('twenty-series compact exports grow enough to keep legend and plot separated', () => {
  const chart = model(); chart.series = Array.from({length:20},(_,index)=>({key:`s${index}`,label:`Series ${index}`,points:[{x:0,y:index,sourceRow:index},{x:1,y:index+1,sourceRow:index}]})); chart.coverage = {sourceRows:40,expandedPoints:40,plottedPoints:40,missingY:0,insertedGaps:0,rejectedPoints:0}
  const svg = renderChartSvg(chart,{width:480,height:320})
  expect(svg).toContain('width="480" height="483"')
  const axis = svg.match(/M112,([\d.]+)V([\d.]+)H450/)
  expect(Number(axis?.[1])).toBeLessThan(Number(axis?.[2]))
  expect((svg.match(/<g transform="translate\([^,]+,(?:48|66|84|102|120|138|156|174|192|210)\)"/g) ?? []).length).toBe(20)
})

test('monthly conversion is visibly named with its scale independent of user labels', () => {
  const chart = model(); chart.transform = {kind:'monthly-average-calendar-day-rate', confirmedMonthly:true, scale:1_000_000}; chart.yLabel = 'Oil'; chart.unit = 'Sm³/day'
  const svg = renderChartSvg(chart)
  expect(svg).toContain('Transformation: monthly-average calendar-day rate; source scale 1.00e+6 before division by actual month length.')
  expect(svg).toContain('Oil (Sm³/day)')
})

test('PNG is rasterized from the same explicit-size SVG and always revokes its URL', async () => {
  const prior = {Image: globalThis.Image, document: globalThis.document, create: URL.createObjectURL, revoke: URL.revokeObjectURL}
  let source = ''; let revoked = ''; let canvasSize = ''
  class LoadedImage { decoding = ''; onload: null | (() => void) = null; onerror: null | (() => void) = null; set src(_value: string) { queueMicrotask(() => this.onload?.()) } }
  URL.createObjectURL = value => { if (value instanceof Blob) void value.text().then((text: string) => { source = text }); return 'blob:chart' }
  URL.revokeObjectURL = value => { revoked = value }
  Object.assign(globalThis, {Image: LoadedImage, document: {createElement: () => {
    const canvas = {width: 0, height: 0, getContext: () => ({drawImage: () => { canvasSize = `${canvas.width}x${canvas.height}` }}), toBlob: (done: (blob: Blob) => void) => done(new Blob(['png'], {type: 'image/png'}))}
    return canvas
  }}})
  try {
    const png = await chartPngBlob(model(), {width: 701, height: 401, title: 'PNG source'})
    await Promise.resolve()
    expect(png.type).toBe('image/png'); expect(canvasSize).toBe('701x401')
    expect(source).toContain('width="701" height="401"'); expect(source).toContain('PNG source')
    expect(revoked).toBe('blob:chart')
    const compact = model(); compact.series = Array.from({length:20},(_,index)=>({key:`s${index}`,label:`Series ${index}`,points:[{x:0,y:index,sourceRow:index}]}))
    await chartPngBlob(compact, {width:480,height:320}); await Promise.resolve()
    expect(canvasSize).toBe('480x483'); expect(source).toContain('width="480" height="483"')
  } finally {
    Object.assign(globalThis, {Image: prior.Image, document: prior.document}); URL.createObjectURL = prior.create; URL.revokeObjectURL = prior.revoke
  }
  expect(safeChartFilename('../Oil & Gas 2024', 'svg')).toBe('oil-gas-2024.svg')
  expect(escapeSvg('<script>')).toBe('&lt;script&gt;')
})

test('PNG decode errors are actionable and still release the object URL', async () => {
  const priorImage = globalThis.Image; const priorCreate = URL.createObjectURL; const priorRevoke = URL.revokeObjectURL
  let revoked = ''
  class BrokenImage { decoding = ''; onload: null | (() => void) = null; onerror: null | (() => void) = null; set src(_value: string) { queueMicrotask(() => this.onerror?.()) } }
  Object.assign(globalThis, {Image: BrokenImage}); URL.createObjectURL = () => 'blob:broken'; URL.revokeObjectURL = value => { revoked = value }
  try {
    await expect(chartPngBlob(model())).rejects.toThrow('could not be decoded')
    expect(revoked).toBe('blob:broken')
  } finally { Object.assign(globalThis, {Image: priorImage}); URL.createObjectURL = priorCreate; URL.revokeObjectURL = priorRevoke }
})
