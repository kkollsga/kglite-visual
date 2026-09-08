import { downloadBlob } from '../download'
import type { ChartModel } from './types'
import { chartDimensions, renderChartSvg, type ChartRenderOptions } from './render'

export function safeChartFilename(title: string | undefined, extension: 'svg' | 'png'): string {
  const base = (title ?? 'query-chart').normalize('NFKD').replace(/[^a-zA-Z0-9]+/g, '-').replace(/^-|-$/g, '').toLowerCase().slice(0, 80) || 'query-chart'
  return `${base}.${extension}`
}

export function chartSvgBlob(model: ChartModel, options: ChartRenderOptions = {}): Blob {
  return new Blob([renderChartSvg(model, options)], {type: 'image/svg+xml;charset=utf-8'})
}

export async function chartPngBlob(model: ChartModel, options: ChartRenderOptions = {}): Promise<Blob> {
  const svg = chartSvgBlob(model, options); const url = URL.createObjectURL(svg)
  try {
    const image = new Image(); image.decoding = 'async'
    await new Promise<void>((resolve, reject) => { image.onload = () => resolve(); image.onerror = () => reject(new Error('The chart SVG could not be decoded for PNG export.')); image.src = url })
    const {width, height} = chartDimensions(model, options)
    const canvas = document.createElement('canvas'); canvas.width = width; canvas.height = height
    const context = canvas.getContext('2d'); if (!context) throw new Error('PNG export is unavailable because no 2D canvas context exists.')
    context.drawImage(image, 0, 0, width, height)
    return await new Promise<Blob>((resolve, reject) => canvas.toBlob(blob => blob ? resolve(blob) : reject(new Error('The browser could not encode the chart PNG.')), 'image/png'))
  } finally { URL.revokeObjectURL(url) }
}

export function downloadChartSvg(model: ChartModel, options: ChartRenderOptions = {}): void {
  downloadBlob(chartSvgBlob(model, options), safeChartFilename(options.title, 'svg'))
}

export async function downloadChartPng(model: ChartModel, options: ChartRenderOptions = {}): Promise<void> {
  downloadBlob(await chartPngBlob(model, options), safeChartFilename(options.title, 'png'))
}
