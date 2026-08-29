/**
 * The label overlay (plan D7).
 *
 * cosmos.gl draws no text, so labels are HTML positioned over the canvas. Two
 * rules carried over from the comparable-repo study:
 *
 * - **Sample, don't iterate.** `getSampledPoints()` returns only what is
 *   on screen, already thinned by the renderer's own sampling distance. A
 *   layer that walks every point per frame is the O(V) freeze gephi-lite hits
 *   at 5k nodes.
 * - **Deterministic collision resolution.** sigma.js's LabelGrid keeps one
 *   label per screen cell; when two candidates tie, the winner must be chosen
 *   by a stable key or the overlay flickers between frames that are otherwise
 *   identical. Here the key is the slot id — the one identifier that is stable
 *   across zooms, expansions and reconnects.
 */

/** One candidate label. */
export type LabelSpec = {
  slot: number
  text: string
  /** Capability flags (`ts`/`geo`/`loc`/`vec`) rendered as small badges. */
  badges: string[]
  /** Bigger wins a screen cell. Node count, in practice. */
  weight: number
  /** A supporting type's name is drawn quieter, matching its circle. */
  dimmed?: boolean
}

/** What the overlay needs from the renderer, so it can be tested without one. */
export type ScreenSource = {
  sampledPoints(): { indices: number[]; positions: number[] }
  toScreen(position: [number, number]): [number, number]
  /** On-screen radius of a point, so a label can clear the circle it names. */
  radius(index: number): number
}

/** Screen-space cell size, in CSS pixels. Roughly one label's footprint. */
const CELL_WIDTH = 130
const CELL_HEIGHT = 30

type Placed = { slot: number; x: number; y: number }

/**
 * Cells a displaced label will try, in order, before it gives up and overlaps.
 *
 * Vertical first and only ±2 rows out, because a label has to stay recognisably
 * attached to the circle it names: 30 px down is still "that one", 260 px
 * across is a different node's name sitting next to it. Fixed order, so the
 * choice is a function of the input and nothing else.
 */
const NUDGES: readonly [number, number][] = [
  [0, -1],
  [0, 1],
  [0, -2],
  [0, 2],
  [-1, 0],
  [1, 0],
  [-1, -1],
  [1, -1],
  [-1, 1],
  [1, 1],
]

/**
 * Place labels, at most one per screen cell.
 *
 * Exported for its own test: the tie-break is the part that silently degrades
 * into flicker, and a flicker is not something a screenshot assert can catch.
 *
 * `placeAll` is the meta-graph's mode, and the meta-graph IS its labels — a
 * type node with no name on it is a dot, which is precisely the entry screen a
 * user reported as useless. So there, a label that loses its cell is *nudged*
 * into a free neighbour rather than dropped, and if nothing near it is free it
 * is drawn overlapping: a hundred type names crowding each other is a legible
 * schema, and ninety-eight dots with sixty names is not. An instance slice
 * keeps dropping, because at the 5 000-node response bound "every label" is
 * not a picture at any density.
 */
export function chooseLabels(
  candidates: { slot: number; x: number; y: number; weight: number }[],
  placeAll = false,
): Placed[] {
  // Sorted rather than compared in place: the winner of a cell must not depend
  // on the sampler's output order, which changes on every zoom. Heaviest
  // first, and an exact tie goes to the lower slot — the one identifier that
  // is stable across zooms, expansions and reconnects.
  const ordered = [...candidates].sort(
    (a, b) => b.weight - a.weight || a.slot - b.slot,
  )
  const taken = new Set<string>()
  const placed: Placed[] = []
  for (const candidate of ordered) {
    const column = Math.floor(candidate.x / CELL_WIDTH)
    const row = Math.floor(candidate.y / CELL_HEIGHT)
    if (!taken.has(`${column}:${row}`)) {
      taken.add(`${column}:${row}`)
      placed.push({ slot: candidate.slot, x: candidate.x, y: candidate.y })
      continue
    }
    if (!placeAll) continue
    const nudge = NUDGES.find(
      ([dx, dy]) => !taken.has(`${column + dx}:${row + dy}`),
    )
    if (nudge === undefined) {
      // Every neighbour is spoken for. Drawing it anyway is the deliberate
      // choice: an unnamed type node carries no information at all.
      placed.push({ slot: candidate.slot, x: candidate.x, y: candidate.y })
      continue
    }
    const [dx, dy] = nudge
    taken.add(`${column + dx}:${row + dy}`)
    placed.push({
      slot: candidate.slot,
      x: candidate.x + dx * CELL_WIDTH,
      y: candidate.y + dy * CELL_HEIGHT,
    })
  }
  return placed.sort((a, b) => a.slot - b.slot)
}

/**
 * What a label does when it is clicked or pointed at.
 *
 * Labels are the app's only *addressable* handle on a node: a canvas point can
 * only be reached by guessing at pixels, which is what the test plan's
 * agent-operability principle rules out. Making the label itself select and
 * hover is therefore a product decision first — a name is a bigger target than
 * a five-pixel circle — and the thing that makes the drill-in drivable second.
 */
export type LabelHandlers = {
  onSelect(slot: number): void
  onHover(slot: number | null): void
}

export class LabelOverlay {
  private readonly root: HTMLDivElement
  private specs = new Map<number, LabelSpec>()
  private readonly elements = new Map<number, HTMLDivElement>()

  constructor(
    container: HTMLElement,
    private readonly handlers: LabelHandlers | null = null,
  ) {
    this.root = document.createElement('div')
    this.root.className = 'kglv-labels'
    container.appendChild(this.root)
  }

  /**
   * Replace the candidate set. **View path only, never a camera event.**
   *
   * This is the O(live slots) half of the overlay — it walks every live slot
   * and drops every placed element — so it runs when the *view* changed and
   * nothing else. Wiring it to `onZoom` (which fires continuously through a
   * wheel gesture) is exactly the per-frame walk the module doc above says the
   * overlay avoids, and it was measured: 33.3 ms p95 frame period against
   * 18.0 ms for the same slice with the rebuild removed, at the 5 000-node
   * response bound on a real GPU.
   */
  setLabels(specs: LabelSpec[]): void {
    this.specs = new Map(specs.map((spec) => [spec.slot, spec]))
    for (const element of this.elements.values()) element.remove()
    this.elements.clear()
  }

  /**
   * Reposition every visible label. Call after a render and on every zoom.
   *
   * The sampled-points pass, and the only one a camera event runs: it touches
   * what the renderer says is on screen, not what the view holds.
   */
  update(source: ScreenSource, placeAll = false): void {
    const sampled = source.sampledPoints()
    const candidates: { slot: number; x: number; y: number; weight: number }[] = []
    for (let i = 0; i < sampled.indices.length; i += 1) {
      const slot = sampled.indices[i]
      const spaceX = sampled.positions[i * 2]
      const spaceY = sampled.positions[i * 2 + 1]
      if (slot === undefined || spaceX === undefined || spaceY === undefined) continue
      const spec = this.specs.get(slot)
      if (spec === undefined) continue
      const [x, y] = source.toScreen([spaceX, spaceY])
      // Below the circle, not on top of it: a label centred on its point hides
      // the size that encodes the type's count.
      candidates.push({ slot, x, y: y + source.radius(slot) + 4, weight: spec.weight })
    }

    const placed = chooseLabels(candidates, placeAll)
    const live = new Set(placed.map((p) => p.slot))
    for (const [slot, element] of this.elements) {
      if (!live.has(slot)) {
        element.remove()
        this.elements.delete(slot)
      }
    }
    for (const { slot, x, y } of placed) {
      const spec = this.specs.get(slot)
      if (spec === undefined) continue
      let element = this.elements.get(slot)
      if (element === undefined) {
        element = this.render(spec)
        this.root.appendChild(element)
        this.elements.set(slot, element)
      }
      element.style.transform = `translate(-50%, 0) translate(${x}px, ${y}px)`
    }
  }

  /** Labels currently on screen, lowest slot first — what `__kglv` reports. */
  visibleSlots(): number[] {
    return [...this.elements.keys()].sort((a, b) => a - b)
  }

  private render(spec: LabelSpec): HTMLDivElement {
    const element = document.createElement('div')
    element.className = spec.dimmed === true ? 'kglv-label kglv-label-dim' : 'kglv-label'
    element.dataset['slot'] = String(spec.slot)

    const name = document.createElement('span')
    name.className = 'kglv-label-name'
    name.textContent = spec.text
    element.appendChild(name)

    const count = document.createElement('span')
    count.className = 'kglv-label-count'
    count.textContent = spec.weight.toLocaleString('en-US')
    element.appendChild(count)

    if (this.handlers !== null) {
      const handlers = this.handlers
      element.addEventListener('click', () => handlers.onSelect(spec.slot))
      element.addEventListener('mouseenter', () => handlers.onHover(spec.slot))
      element.addEventListener('mouseleave', () => handlers.onHover(null))
    }

    for (const badge of spec.badges) {
      const chip = document.createElement('span')
      chip.className = `kglv-badge kglv-badge-${badge}`
      chip.textContent = badge
      chip.title = BADGE_TITLES[badge] ?? badge
      element.appendChild(chip)
    }
    return element
  }
}

/** kglite's own four capability flags, spelled out for a human reader. */
const BADGE_TITLES: Record<string, string> = {
  ts: 'has timeseries data',
  geo: 'has WKT geometry',
  loc: 'has lat/lon locations',
  vec: 'has embedding vectors',
}
