export type QueryProvenance = {
  requestId: string
  query: string
  params: Record<string, unknown>
  requestedAtMs: number
}

/** Capture the JSON values sent on the wire, detached from caller-owned objects. */
export function captureQueryProvenance(
  requestId: string,
  query: string,
  params: Record<string, unknown>,
  requestedAtMs = Date.now(),
): QueryProvenance {
  return {
    requestId,
    query,
    params: JSON.parse(JSON.stringify(params)) as Record<string, unknown>,
    requestedAtMs,
  }
}
