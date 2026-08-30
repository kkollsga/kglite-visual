/**
 * What the editor knows about this graph's schema, and where it got it.
 *
 * **From `property-stats` and the meta-graph, not from `/api/describe`** (plan
 * E2). `session.rs` records the decision that `describe` is deliberately not a
 * frontend surface, and quietly reversing a written decision because it would
 * have been convenient is worse than the extra call: the meta-graph is already
 * in hand — it *is* the entry screen — and per-type properties come from the
 * same `property-stats` endpoint the appearance menus use.
 *
 * **Lazily, per type, once.** A graph with 98 node types would be 98 requests
 * to fill a completion list nobody asked for, and `property-stats` walks a type
 * rather than a row. So a type's properties are fetched the first time the
 * editor needs them and kept; the caller gets whatever is cached *now* and a
 * callback when a fetch lands, because a completion source must answer
 * synchronously and a list that arrived 40 ms late is a list nobody sees.
 *
 * **Over HTTP, not the WebSocket.** A `property-stats` request on the socket
 * lands in the same handler that fills the appearance menus, so a completion
 * would silently repaint the colour-by dropdown for a type the user is not
 * looking at. The JSON twin answers the same struct with no such side effect.
 */

import type { MetaGraphMeta } from './generated/MetaGraphMeta'
import type { PropertyStatsResponse } from './generated/PropertyStatsResponse'
import { apiUrl } from './urls'

export class SchemaCache {
  private labelNames: string[] = []
  private relationshipNames: string[] = []
  private readonly properties = new Map<string, string[]>()
  private readonly inflight = new Set<string>()
  private listener: (() => void) | null = null

  /** Node labels and relationship types, from the entry screen's own payload. */
  setMetaGraph(meta: MetaGraphMeta): void {
    this.labelNames = meta.nodes.map((node) => node.name)
    // A relationship type appears once per type pair it connects, so the
    // meta-graph's edge list holds duplicates the completion list must not.
    this.relationshipNames = [...new Set(meta.edges.map((edge) => edge.name))].sort()
    this.listener?.()
  }

  /** Called when a lazy fetch lands, so an open completion list can re-query. */
  onChange(listener: () => void): void {
    this.listener = listener
  }

  labels(): readonly string[] {
    return this.labelNames
  }

  relationshipTypes(): readonly string[] {
    return this.relationshipNames
  }

  /**
   * The property names of one node label.
   *
   * An empty array means either "no properties" or "not fetched yet"; the
   * caller cannot tell, and deliberately does not need to — either way it
   * offers what it has, and `onChange` re-runs the query when more arrives.
   */
  propertiesFor(label: string): readonly string[] {
    const known = this.properties.get(label)
    if (known !== undefined) return known
    void this.fetchProperties(label)
    return []
  }

  private async fetchProperties(label: string): Promise<void> {
    if (this.inflight.has(label)) return
    this.inflight.add(label)
    try {
      const response = await fetch(apiUrl('api/property-stats'), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ node_type: label }),
      })
      if (!response.ok) {
        // A label the graph does not have answers 400. Caching the empty
        // result is the point: without it every keystroke re-asks a question
        // whose answer will not change.
        this.properties.set(label, [])
        return
      }
      const stats = (await response.json()) as PropertyStatsResponse
      this.properties.set(
        label,
        stats.properties.map((stat) => stat.name),
      )
      this.listener?.()
    } catch {
      // Offline, or the server went away. Not cached, so a later keystroke
      // retries rather than remembering a network failure as a schema fact.
    } finally {
      this.inflight.delete(label)
    }
  }
}
