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
 * Choose at most one label per screen cell.
 *
 * Exported for its own test: the tie-break is the part that silently degrades
 * into flicker, and a flicker is not something a screenshot assert can catch.
 */
export function chooseLabels(
  candidates: { slot: number; x: number; y: number; weight: number }[],
): Placed[] {
  const winners = new Map<string, { slot: number; x: number; y: number; weight: number }>()
  for (const candidate of candidates) {
    const cell = `${Math.floor(candidate.x / CELL_WIDTH)}:${Math.floor(candidate.y / CELL_HEIGHT)}`
    const held = winners.get(cell)
    if (
      held === undefined ||
      candidate.weight > held.weight ||
      // Equal weight: the lower slot wins, always. Anything derived from
      // iteration order would reshuffle whenever the sampler's output order
      // changed, which it does on every zoom.
      (candidate.weight === held.weight && candidate.slot < held.slot)
    ) {
      winners.set(cell, candidate)
    }
  }
  return [...winners.values()]
    .map(({ slot, x, y }) => ({ slot, x, y }))
    .sort((a, b) => a.slot - b.slot)
}

export class LabelOverlay {
  private readonly root: HTMLDivElement
  private specs = new Map<number, LabelSpec>()
  private readonly elements = new Map<number, HTMLDivElement>()

  constructor(container: HTMLElement) {
    this.root = document.createElement('div')
    this.root.className = 'kglv-labels'
    container.appendChild(this.root)
  }

  setLabels(specs: LabelSpec[]): void {
    this.specs = new Map(specs.map((spec) => [spec.slot, spec]))
    for (const element of this.elements.values()) element.remove()
    this.elements.clear()
  }

  /** Reposition every visible label. Call after a render and on every zoom. */
  update(source: ScreenSource): void {
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

    const placed = chooseLabels(candidates)
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
    element.className = 'kglv-label'
    element.dataset['slot'] = String(spec.slot)

    const name = document.createElement('span')
    name.className = 'kglv-label-name'
    name.textContent = spec.text
    element.appendChild(name)

    const count = document.createElement('span')
    count.className = 'kglv-label-count'
    count.textContent = spec.weight.toLocaleString('en-US')
    element.appendChild(count)

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
