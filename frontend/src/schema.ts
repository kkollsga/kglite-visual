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

import type { HopDirection } from './generate'
import type { MetaGraphMeta } from './generated/MetaGraphMeta'
import type { PropertyStatsResponse } from './generated/PropertyStatsResponse'
import { apiUrl } from './urls'

/**
 * One relationship a type can be left by, and where it lands.
 *
 * The path builder's whole option list, and the reason it can only offer hops
 * that exist: the meta-graph already says which relationship types join which
 * node types, so a builder reading it never proposes a pattern that matches
 * nothing. `count` is the graph-wide edge total for the pair, which is the
 * cheapest honest hint about how expensive a hop will be — the exact answer
 * comes from the count probe once the path is assembled.
 */
export type Hop = {
  name: string
  direction: HopDirection
  otherType: string
  count: number
}

export class SchemaCache {
  private labelNames: string[] = []
  private relationshipNames: string[] = []
  /** Hops out of each node type, derived once from the meta-graph. */
  private hops = new Map<string, Hop[]>()
  private readonly properties = new Map<string, string[]>()
  private readonly inflight = new Set<string>()
  /**
   * Everyone waiting for a lazy fetch to land.
   *
   * A list rather than one slot: the editor's completions and the path
   * builder's filter pickers both feed from this cache, and a single-listener
   * hook silently replaced whichever registered first.
   */
  private readonly listeners: (() => void)[] = []

  /** Node labels and relationship types, from the entry screen's own payload. */
  setMetaGraph(meta: MetaGraphMeta): void {
    this.labelNames = meta.nodes.map((node) => node.name)
    // A relationship type appears once per type pair it connects, so the
    // meta-graph's edge list holds duplicates the completion list must not.
    this.relationshipNames = [...new Set(meta.edges.map((edge) => edge.name))].sort()
    this.hops = buildHops(meta)
    this.notify()
  }

  /**
   * The relationships that leave `label`, both ways, most-travelled first.
   *
   * Empty for a type nothing connects — which the builder shows as "no
   * relationships from here" rather than as an empty dropdown, because those
   * read identically and mean different things.
   */
  hopsFrom(label: string): readonly Hop[] {
    return this.hops.get(label) ?? []
  }

  /** Called when a lazy fetch lands, so an open completion list can re-query. */
  onChange(listener: () => void): void {
    this.listeners.push(listener)
  }

  private notify(): void {
    for (const listener of this.listeners) listener()
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
      this.notify()
    } catch {
      // Offline, or the server went away. Not cached, so a later keystroke
      // retries rather than remembering a network failure as a schema fact.
    } finally {
      this.inflight.delete(label)
    }
  }
}

/**
 * Fold the meta-graph's edge list into per-type hop lists.
 *
 * Each meta edge is *two* hops — one from each end — because a path can be
 * walked from either side, and a builder that only knew the source direction
 * would refuse half the paths a graph actually has.
 *
 * A self-referencing edge (`Person -[:KNOWS]-> Person`) is the exception, and
 * adding it twice was a real defect rather than a tidiness one: the two entries
 * collapse into one `both` hop whose count is then the same 180 edges added to
 * themselves, and the picker would have offered "KNOWS ↔ Person (360)" on a
 * graph with 180 of them. One entry, `both` from the start, because a loop
 * walked "out" and a loop walked "in" are the same edges.
 */
function buildHops(meta: MetaGraphMeta): Map<string, Hop[]> {
  const nameOf = new Map(meta.nodes.map((node) => [node.slot, node.name]))
  const hops = new Map<string, Hop[]>()
  const add = (from: string | undefined, hop: Hop | null): void => {
    if (from === undefined || hop === null) return
    const list = hops.get(from) ?? []
    const existing = list.find(
      (other) => other.name === hop.name && other.otherType === hop.otherType,
    )
    if (existing === undefined) {
      list.push(hop)
    } else if (existing.direction !== hop.direction) {
      // The same relationship type joins this pair in both directions, so the
      // honest single entry is one that matches either.
      existing.direction = 'both'
      existing.count += hop.count
    }
    hops.set(from, list)
  }

  for (const edge of meta.edges) {
    const source = nameOf.get(edge.source_slot)
    const target = nameOf.get(edge.target_slot)
    if (source === undefined || target === undefined) continue
    if (source === target) {
      add(source, { name: edge.name, direction: 'both', otherType: target, count: edge.count })
      continue
    }
    add(source, { name: edge.name, direction: 'out', otherType: target, count: edge.count })
    add(target, { name: edge.name, direction: 'in', otherType: source, count: edge.count })
  }
  for (const list of hops.values()) {
    list.sort((a, b) => b.count - a.count || a.name.localeCompare(b.name))
  }
  return hops
}
