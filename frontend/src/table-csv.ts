import type { RecordCell } from './generated/RecordCell'
import { recordText } from './cells'
import { downloadBlob, MAX_DOWNLOAD_BYTES } from './download'

/** CSV is a typed display export: explicit cell states and previews remain explicit. */
export function tableCsv(headers: string[], rows: Iterable<ReadonlyArray<RecordCell>>, maxBytes = MAX_DOWNLOAD_BYTES): Blob {
  const encoder = new TextEncoder(); const chunks: Uint8Array<ArrayBuffer>[] = []; let bytes = 0
  const line = (values: string[]): void => {
    const chunk = encoder.encode(values.map(value => `"${value.replaceAll('"', '""')}"`).join(',') + '\r\n')
    bytes += chunk.byteLength
    if (bytes > maxBytes) throw new Error('CSV exceeds the 16 MiB browser limit. Choose fewer records or fields.')
    chunks.push(chunk)
  }
  const used = new Set<string>()
  const unique = (name: string): string => { let candidate = name; let suffix = 2; while (used.has(candidate)) candidate = `${name} (${suffix++})`; used.add(candidate); return candidate }
  const values = headers.map(unique)
  const columns = values.flatMap(name => [name, unique(`${name} [cell state]`), unique(`${name} [value type]`), unique(`${name} [detail]`)])
  line(columns)
  for (const row of rows) line(headers.flatMap((_, index) => {
    const cell = row[index] ?? {state: 'missing' as const}
    return [recordText(cell), cell.state, cell.state === 'value' ? cell.value.type : '', cell.state === 'truncated' || cell.state === 'unavailable' ? cell.reason : '']
  }))
  return new Blob(chunks, {type: 'text/csv;charset=utf-8'})
}
export function saveTableCsv(headers: string[], rows: Iterable<ReadonlyArray<RecordCell>>, filename: string): void { downloadBlob(tableCsv(headers, rows), filename) }
