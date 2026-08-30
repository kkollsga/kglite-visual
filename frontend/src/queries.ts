/**
 * The saved-query store, from the browser's side.
 *
 * **Plain HTTP, not the binary protocol.** A saved query is a handful of
 * strings on the server's filesystem — it is not graph data, it does not move
 * the slot space, and putting it on the wire that carries typed arrays would
 * mean a message type, a protocol bump and a decoder for three strings. The
 * WebSocket carries what the renderer needs; everything else is the JSON twin,
 * which is also what makes `curl` able to drive this (see `api.rs`).
 *
 * **The store is the server's, and it is shared.** Another tab, a `curl`, or an
 * agent's `run_saved_query` all write the same file, so this module never keeps
 * a local copy authoritative: every mutation is followed by a re-read, and the
 * panel renders whatever came back.
 *
 * **There is no `run` here.** Running a saved query means putting its text in
 * the editor and pressing the same button — one execution path, and the user
 * sees what is about to run before it runs.
 */

import { apiUrl } from './urls'

export type SavedQuery = {
  name: string
  query: string
  saved_at: number
}

export type HistoryEntry = {
  query: string
  ran_at: number
}

export type SavedQueries = {
  /** Where the store lives, or `null` on a machine that offers no config dir. */
  store: string | null
  graph_path: string | null
  graph_label: string
  saved: SavedQuery[]
  history: HistoryEntry[]
  max_saved: number
  max_history: number
}

/**
 * The server's message, or a thrown `Error` carrying it.
 *
 * A store refusal is a `400` naming the ceiling it hit, and that sentence is
 * the entire value of the response — a generic "could not save" would delete
 * the number the user needs.
 */
async function json<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(
    apiUrl(path),
    body === undefined
      ? {}
      : {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify(body),
        },
  )
  const payload: unknown = await response.json().catch(() => null)
  if (!response.ok) {
    const message =
      payload !== null && typeof payload === 'object' && 'error' in payload
        ? String((payload as { error: unknown }).error)
        : `the saved-query store answered ${response.status}`
    throw new Error(message)
  }
  return payload as T
}

export function listQueries(): Promise<SavedQueries> {
  return json<SavedQueries>('api/queries')
}

export function saveQuery(name: string, query: string): Promise<SavedQuery> {
  return json<SavedQuery>('api/queries/save', { name, query })
}

export function deleteQuery(name: string): Promise<{ removed: boolean }> {
  return json<{ removed: boolean }>('api/queries/delete', { name })
}

/**
 * Record a query in the recent list.
 *
 * Called from the Run button and nowhere else on this side: the app runs Cypher
 * the user never typed — the per-node values behind a colour-by choice, the id
 * list behind "load into view" — and recording those would fill a list a human
 * reads with machine noise. The server's `record` documents the same boundary
 * from its end.
 */
export function recordQuery(query: string): Promise<{ recorded: boolean }> {
  return json<{ recorded: boolean }>('api/queries/history', { query })
}
