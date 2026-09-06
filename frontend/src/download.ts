/** Downloads are bounded before creating a browser-owned Blob URL. */
export const MAX_DOWNLOAD_BYTES = 16 * 1024 * 1024
export function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a'); link.href = url; link.download = filename
  document.body.append(link); link.click(); link.remove()
  setTimeout(() => URL.revokeObjectURL(url), 30_000)
}
export function downloadFilename(header: string | null, fallback: string): string {
  const encoded = header?.match(/filename\*=UTF-8''([^;]+)/i)?.[1]
  if (encoded) { try { return decodeURIComponent(encoded) } catch { /* Fall through to quoted filename. */ } }
  return header?.match(/filename="([^"]+)"/i)?.[1] ?? fallback
}
export async function boundedBlob(response: Response): Promise<Blob> {
  const reader = response.body?.getReader()
  if (!reader) throw new Error('The download has no body.')
  const chunks: Uint8Array<ArrayBuffer>[] = []; let bytes = 0
  try {
    while (true) {
      const part = await reader.read(); if (part.done) break
      bytes += part.value.byteLength
      if (bytes > MAX_DOWNLOAD_BYTES) throw new Error('Download exceeds the 16 MiB browser limit. Narrow the scope or fields.')
      chunks.push(new Uint8Array(part.value))
    }
  } catch (error) { await reader.cancel(); throw error }
  return new Blob(chunks, {type: response.headers.get('content-type') ?? 'application/octet-stream'})
}
