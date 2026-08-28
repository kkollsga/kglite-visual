/**
 * Module-level atoms — the app's only state container, deliberately.
 *
 * No state library, and no graph data in a component tree (plan D7). The
 * comparable-repo study found the same failure in every slow graph UI: putting
 * nodes and edges in framework state turns one click into an O(V+E) re-render.
 * Graph payloads live in typed arrays handed straight to the renderer; only
 * small scalars — the ones below — are ever observed.
 *
 * `subscribe` returns its own unsubscriber so a caller never has to keep the
 * listener reference around to detach it.
 */

export type Atom<T> = {
  get(): T
  set(next: T): void
  subscribe(listener: (value: T) => void): () => void
}

export function atom<T>(initial: T): Atom<T> {
  let value = initial
  const listeners = new Set<(value: T) => void>()
  return {
    get: () => value,
    set(next: T) {
      // Reference/identity equality only. Atoms hold scalars and immutable
      // snapshots; a deep compare here would quietly invite callers to store
      // the large mutable payloads this module exists to keep out.
      if (Object.is(value, next)) return
      value = next
      for (const listener of listeners) listener(value)
    },
    subscribe(listener) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
  }
}

/** Which view the app is showing. The meta-graph is always the entry point. */
export const viewAtom = atom<'meta-graph' | 'neighborhood' | 'query'>('meta-graph')

/** Connection state, so the UI can say "disconnected" instead of going stale. */
export const connectedAtom = atom(false)
