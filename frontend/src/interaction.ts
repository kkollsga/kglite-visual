/**
 * The four interaction concepts (plan D7).
 *
 * They are **separate**, they are **index arrays**, and they reach the renderer
 * through `setConfigPartial` — never through a per-item callback. That is the
 * one finding the comparable-repo study was unambiguous about: gephi-lite's
 * per-node reducer turns one hover into an O(V+E) pass and freezes at 5k nodes,
 * while an index array is a uniform upload whatever the graph's size.
 *
 * - **hovered** — the point under the cursor. One index.
 * - **emphasized** — its 1-hop neighbourhood, from cosmos.gl's own
 *   `getNeighboringPointIndices` / `getConnectedLinkIndices`. **Client-side, no
 *   server round trip**: the renderer already holds the adjacency it drew, and
 *   asking the server would put a network hop inside a mouse-move.
 * - **highlighted** — search and query hits.
 * - **selected** — what the panel is describing.
 *
 * **cosmos.gl offers three index-addressed channels, not four**, so the mapping
 * below is a decision rather than a transcription (v3.4.1: `focusedPointIndex`,
 * `highlightedPointIndices`, `outlinedPointIndices`, plus
 * `highlightedLinkIndices`):
 *
 * | concept     | channel                                        |
 * |-------------|------------------------------------------------|
 * | hovered     | `focusedPointIndex` — the large focus ring      |
 * | emphasized  | `highlightedPointIndices` — everything else greys |
 * | selected    | `outlinedPointIndices` — a smaller ring         |
 * | highlighted | a colour-array fill (see `appearance.ts`)       |
 *
 * The fourth rides the colour array because two ring channels cannot carry two
 * different colours — `outlinedPointRingColor` is a single value. That is still
 * one typed-array fill driven by an index set, which is the rule; it is the
 * *channel* that differs, not the mechanism.
 */

/** What a renderer must offer for this module to drive it. */
export type InteractionTarget = {
  getNeighboringPointIndices(pointIndices: number | number[]): number[]
  getConnectedLinkIndices(pointIndices: number | number[]): number[]
  setConfigPartial(config: Record<string, unknown>): void
}

/** The four sets, as the renderer will be told about them. */
export type InteractionConfig = {
  focusedPointIndex: number | undefined
  highlightedPointIndices: number[] | undefined
  highlightedLinkIndices: number[] | undefined
  outlinedPointIndices: number[] | undefined
}

export class InteractionState {
  private hovered: number | null = null
  private emphasized: number[] = []
  private emphasizedLinks: number[] = []
  private highlighted: number[] = []
  private selected: number[] = []

  /**
   * Set the hover and recompute its 1-hop neighbourhood.
   *
   * Returns true when anything changed, so a mouse-move over the same point
   * does not re-upload three arrays sixty times a second.
   */
  hover(target: InteractionTarget, slot: number | null): boolean {
    if (this.hovered === slot) return false
    this.hovered = slot
    if (slot === null) {
      this.emphasized = []
      this.emphasizedLinks = []
      return true
    }
    // The hovered point is part of its own emphasis: `highlightedPointIndices`
    // greys out everything NOT in the array, so omitting the hovered point
    // would grey out the very thing under the cursor.
    this.emphasized = [slot, ...target.getNeighboringPointIndices(slot)]
    this.emphasizedLinks = target.getConnectedLinkIndices(slot)
    return true
  }

  /** Search and query hits. */
  setHighlighted(slots: number[]): void {
    this.highlighted = [...slots]
  }

  /** What the selection panel is describing. */
  setSelected(slots: number[]): void {
    this.selected = [...slots]
  }

  /**
   * Forget slots that no longer draw anything.
   *
   * A collapse tombstones slots without renumbering anything, so the sets here
   * keep naming indices that are now NaN positions: nothing wrong appears on
   * screen — a highlight colour at a NaN point draws no point — but the four
   * counts `window.__kglv` publishes go on describing nodes that are gone, and
   * those counts are the instrument every agent and every e2e assertion reads.
   * A compaction already forces a full clear (it renumbers everything); this is
   * the same duty for the cheaper operation that does not.
   */
  dropSlots(slots: number[]): void {
    if (slots.length === 0) return
    const gone = new Set(slots)
    const keep = (list: number[]): number[] => list.filter((slot) => !gone.has(slot))
    this.highlighted = keep(this.highlighted)
    this.selected = keep(this.selected)
    this.emphasized = keep(this.emphasized)
    if (this.hovered !== null && gone.has(this.hovered)) {
      this.hovered = null
      this.emphasized = []
      this.emphasizedLinks = []
    }
  }

  hoveredSlot(): number | null {
    return this.hovered
  }

  emphasizedSlots(): number[] {
    return [...this.emphasized]
  }

  highlightedSlots(): number[] {
    return [...this.highlighted]
  }

  selectedSlots(): number[] {
    return [...this.selected]
  }

  /**
   * Project the four sets onto cosmos.gl's channels.
   *
   * Pure, and separate from `apply`, so the mapping has a unit test that needs
   * no WebGL context: what these four sets become is a decision, and a decision
   * that only a browser can check is a decision nothing checks.
   */
  toConfig(): InteractionConfig {
    return {
      focusedPointIndex: this.hovered ?? undefined,
      // `undefined`, not `[]`: an empty array means "highlight nothing", which
      // greys out the entire graph. Not hovering must leave the view alone.
      highlightedPointIndices: this.emphasized.length > 0 ? this.emphasized : undefined,
      highlightedLinkIndices: this.emphasizedLinks.length > 0 ? this.emphasizedLinks : undefined,
      outlinedPointIndices: this.selected.length > 0 ? this.selected : undefined,
    }
  }

  /** One `setConfigPartial` call — never one per item. */
  apply(target: InteractionTarget): void {
    target.setConfigPartial(this.toConfig() as unknown as Record<string, unknown>)
  }
}
