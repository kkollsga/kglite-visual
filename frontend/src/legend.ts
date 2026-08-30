/**
 * What the colours and sizes on screen mean (plan E11).
 *
 * **A reader of this app's own state, not a second source of truth.** Every
 * entry is built from the values the appearance layer is *currently* filling
 * its typed arrays with — `categoricalLegend` returns the same value→colour
 * pairs `compileCategoricalColor` assigns, and the structural swatches are the
 * literals `baseColor` returns. A legend that computed its own colours would be
 * a second opinion about the picture, and the swatch is the half a reader
 * trusts, so it is the half that would be wrong.
 *
 * **No wire.** Nothing here asks the server anything: the encoding is entirely
 * a client decision (which is why `control::Appearance` had to exist at all),
 * so a legend over it is DOM and nothing else.
 *
 * It is collapsible and starts collapsed on a view whose encoding is purely
 * structural, because "big circle = many members" is not worth a card until
 * something less obvious is driving the colours.
 */

import type { Rgba } from './appearance'

/** One row: a swatch and what it means. */
export type LegendEntry = {
  color: Rgba
  label: string
  /** Drawn as a line rather than a dot — the link-width channel. */
  line?: boolean
}

/** A titled group of rows. */
export type LegendSection = {
  title: string
  entries: LegendEntry[]
  /** Shown under the rows, in the hint voice — a caveat, never a row. */
  note?: string
}

function rgbaCss(color: Rgba): string {
  const channel = (v: number): number => Math.round(v * 255)
  return `rgba(${channel(color[0])}, ${channel(color[1])}, ${channel(color[2])}, ${color[3]})`
}

export class Legend {
  private readonly root: HTMLDivElement
  private readonly body: HTMLDivElement
  private readonly toggle: HTMLButtonElement
  private open_: boolean

  constructor(container: HTMLElement, startOpen = false) {
    this.open_ = startOpen
    this.root = document.createElement('div')
    this.root.className = 'kglv-legend'
    this.root.setAttribute('data-testid', 'legend')

    this.toggle = document.createElement('button')
    this.toggle.className = 'kglv-legend-toggle'
    this.toggle.setAttribute('data-testid', 'legend-toggle')
    this.toggle.addEventListener('click', () => {
      this.open_ = !this.open_
      this.applyOpenState()
    })

    this.body = document.createElement('div')
    this.body.className = 'kglv-legend-body'
    this.body.setAttribute('data-testid', 'legend-body')

    this.root.append(this.toggle, this.body)
    container.appendChild(this.root)
    this.applyOpenState()
  }

  /**
   * Redraw from the encoding in force.
   *
   * Called wherever the appearance changes — a colour-by choice from either
   * driver, a slice that brought a new type on screen — because a legend that
   * describes the previous encoding is worse than none: it is a confident
   * caption on the wrong picture.
   */
  update(sections: LegendSection[]): void {
    this.body.replaceChildren()
    for (const section of sections) {
      if (section.entries.length === 0 && section.note === undefined) continue
      const group = document.createElement('div')
      group.className = 'kglv-legend-group'
      const title = document.createElement('div')
      title.className = 'kglv-legend-title'
      title.textContent = section.title
      group.appendChild(title)
      for (const entry of section.entries) {
        const row = document.createElement('div')
        row.className = 'kglv-legend-row'
        const swatch = document.createElement('span')
        swatch.className = entry.line === true ? 'kglv-legend-line' : 'kglv-legend-swatch'
        swatch.style.background = rgbaCss(entry.color)
        const label = document.createElement('span')
        label.className = 'kglv-legend-label'
        label.textContent = entry.label
        row.append(swatch, label)
        group.appendChild(row)
      }
      if (section.note !== undefined) {
        const note = document.createElement('div')
        note.className = 'kglv-legend-note'
        note.textContent = section.note
        group.appendChild(note)
      }
      this.body.appendChild(group)
    }
    this.entryCount = sections.reduce((total, section) => total + section.entries.length, 0)
    this.applyOpenState()
  }

  /** Rows currently drawn — what `__kglv` reports and an e2e asserts. */
  entryCount = 0

  /**
   * Show the card without waiting to be clicked.
   *
   * Called when a colour-by choice lands, from either driver. A user who just
   * asked for a colouring nobody can decode from the picture should not also
   * have to find the control that explains it — and an agent that set the
   * channel remotely leaves the human in front of the screen with a graph that
   * changed colour for no visible reason.
   */
  open(): void {
    if (this.open_) return
    this.open_ = true
    this.applyOpenState()
  }

  private applyOpenState(): void {
    this.body.style.display = this.open_ ? '' : 'none'
    this.toggle.textContent = this.open_
      ? `legend (${this.entryCount}) ▾`
      : `legend (${this.entryCount}) ▸`
    this.toggle.setAttribute('aria-expanded', String(this.open_))
  }
}
