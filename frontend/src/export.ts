/**
 * Taking the view out of the viewer (plan E8).
 *
 * **Anchors, not fetches.** Each format is an `<a href download>` pointing at
 * `GET /api/export`, so the browser streams the file straight to disk under the
 * name the server's `Content-Disposition` gives it. Fetching the bytes into
 * JavaScript to build a blob URL would mean holding a whole graph in the tab's
 * memory to hand it to the same download the browser can do on its own — and it
 * would lose the server's filename, which is the half that carries a Norwegian
 * graph's letters.
 *
 * **It sits beside the legend and says the same kind of thing.** The legend
 * explains what the picture means; this says what leaves with it — how many
 * nodes, and the one way the file differs from the canvas. Both are cards over
 * the canvas rather than sidebar sections because both are about *this view*.
 */

import { apiUrl } from './urls'

/** The formats offered, in the order a user reads them. */
const FORMATS: readonly (readonly [string, string, string])[] = [
  ['graphml', 'GraphML', 'Gephi, yEd, Cytoscape'],
  ['gexf', 'GEXF', "Gephi's own"],
  ['csv', 'CSV nodes', 'id, type, title'],
  ['csv-edges', 'CSV edges', 'source, target, type'],
  ['json', 'JSON', "D3's nodes and links"],
]

/**
 * The caveat the server also puts in `x-kglv-note`, said before the click.
 *
 * A user who imports the file into Gephi and counts more edges than they saw
 * has found a bug in this app, as far as they can tell. Saying it here is the
 * difference between a property of the export and a surprise.
 */
const EDGE_NOTE =
  'the file carries every edge between the exported nodes — which can be more than the canvas ' +
  'drew, because a link the byte budget refused is still an edge in the graph'

export class ExportCard {
  private readonly root: HTMLDivElement
  private readonly toggle: HTMLButtonElement
  private readonly body: HTMLDivElement
  private readonly note: HTMLDivElement
  private readonly links: HTMLAnchorElement[] = []
  private open = false

  constructor(container: HTMLElement) {
    this.root = document.createElement('div')
    this.root.className = 'kglv-export'
    this.root.setAttribute('data-testid', 'export')

    this.toggle = document.createElement('button')
    this.toggle.className = 'kglv-legend-toggle'
    this.toggle.setAttribute('data-testid', 'export-toggle')
    this.toggle.addEventListener('click', () => {
      this.open = !this.open
      this.applyOpenState()
    })

    this.body = document.createElement('div')
    this.body.className = 'kglv-export-body'
    this.body.setAttribute('data-testid', 'export-body')

    for (const [format, label, hint] of FORMATS) {
      const row = document.createElement('a')
      row.className = 'kglv-export-link'
      row.setAttribute('data-testid', `export-${format}`)
      // Relative to whatever prefix served this page (`urls.ts`): the same
      // bundle is served at `/` by the CLI and under `/proxy/8731/` by
      // jupyter-server-proxy, and an absolute path 404s in the third case
      // while the page around it still works.
      row.href = apiUrl(`api/export?format=${format}&source=live-view`)
      // The server's Content-Disposition names the file; this only asks the
      // browser to save rather than navigate.
      row.setAttribute('download', '')
      const name = document.createElement('span')
      name.className = 'kglv-export-name'
      name.textContent = label
      const aside = document.createElement('span')
      aside.className = 'kglv-export-hint'
      aside.textContent = hint
      row.append(name, aside)
      this.links.push(row)
      this.body.appendChild(row)
    }

    this.note = document.createElement('div')
    this.note.className = 'kglv-legend-note'
    this.note.setAttribute('data-testid', 'export-note')
    this.body.appendChild(this.note)

    this.root.append(this.toggle, this.body)
    container.appendChild(this.root)
    this.setLoaded(0)
    this.applyOpenState()
  }

  /**
   * Say what a click would produce, and refuse the click when the answer is
   * nothing.
   *
   * The server refuses an empty view by name, so a link left live would hand
   * the user a JSON error where they expected a file. Disabling the anchors and
   * saying why keeps the refusal in the place the user is looking.
   */
  setLoaded(instances: number): void {
    const empty = instances === 0
    for (const link of this.links) {
      link.classList.toggle('kglv-export-disabled', empty)
      // `aria-disabled` plus a removed href: an `<a>` has no disabled
      // attribute, and a click handler that swallows the event would still let
      // a middle-click through to the failing URL.
      link.setAttribute('aria-disabled', String(empty))
      if (empty) {
        link.removeAttribute('href')
      } else {
        const format = link.getAttribute('data-testid')?.replace('export-', '') ?? 'graphml'
        link.href = apiUrl(`api/export?format=${format}&source=live-view`)
      }
    }
    this.note.textContent = empty
      ? 'nothing to export yet — expand a type, or run a query with "show in graph"'
      : `${instances.toLocaleString('en-US')} node${instances === 1 ? '' : 's'}; ${EDGE_NOTE}`
    this.count = instances
    this.applyOpenState()
  }

  /** Instance nodes the next export would carry — what `__kglv` reports. */
  count = 0

  private applyOpenState(): void {
    this.body.style.display = this.open ? '' : 'none'
    this.toggle.textContent = this.open ? 'export ▾' : 'export ▸'
    this.toggle.setAttribute('aria-expanded', String(this.open))
  }
}
