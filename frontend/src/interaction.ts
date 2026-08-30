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
   * Slots a client-side filter is currently hiding (plan E7).
   *
   * **A view over the four sets, not an edit of them.** Filtering is reversible
   * — the nodes are still loaded, still in the slot space, still the server's
   * idea of what is on screen — so clearing the box has to give the selection
   * back. `dropSlots` is the other operation and stays destructive, because a
   * tombstone is not coming back.
   *
   * Applied in one place, {@link visible}, which every getter and the renderer
   * projection go through: the counts `window.__kglv` publishes are what an
   * agent and every e2e assertion read, and a selection count that kept
   * describing a node the filter had hidden would be the same class of lie
   * `dropSlots` exists to prevent.
   */
  private hidden: ReadonlySet<number> = new Set()

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

  /** Replace the hidden set. See {@link hidden}. */
  setHidden(slots: ReadonlySet<number>): void {
    this.hidden = slots
  }

  private visible(list: number[]): number[] {
    if (this.hidden.size === 0) return [...list]
    return list.filter((slot) => !this.hidden.has(slot))
  }

  hoveredSlot(): number | null {
    if (this.hovered !== null && this.hidden.has(this.hovered)) return null
    return this.hovered
  }

  emphasizedSlots(): number[] {
    return this.visible(this.emphasized)
  }

  highlightedSlots(): number[] {
    return this.visible(this.highlighted)
  }

  selectedSlots(): number[] {
    return this.visible(this.selected)
  }

  /**
   * Project the four sets onto cosmos.gl's channels.
   *
   * Pure, and separate from `apply`, so the mapping has a unit test that needs
   * no WebGL context: what these four sets become is a decision, and a decision
   * that only a browser can check is a decision nothing checks.
   */
  toConfig(): InteractionConfig {
    const emphasized = this.emphasizedSlots()
    const selected = this.selectedSlots()
    return {
      focusedPointIndex: this.hoveredSlot() ?? undefined,
      // `undefined`, not `[]`: an empty array means "highlight nothing", which
      // greys out the entire graph. Not hovering must leave the view alone —
      // and so must a filter that hid everything the hover was emphasising.
      highlightedPointIndices: emphasized.length > 0 ? emphasized : undefined,
      highlightedLinkIndices: this.emphasizedLinks.length > 0 ? this.emphasizedLinks : undefined,
      outlinedPointIndices: selected.length > 0 ? selected : undefined,
    }
  }

  /** One `setConfigPartial` call — never one per item. */
  apply(target: InteractionTarget): void {
    target.setConfigPartial(this.toConfig() as unknown as Record<string, unknown>)
  }
}
