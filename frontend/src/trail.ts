import type { Request } from './generated/Request'
import type { SharedWireMeta } from './generated/SharedWireMeta'

/** Local request provenance, recorded only after its matching accepted shared revision. */
export class ExplorationTrail {
  private readonly pending = new Map<string, string>()
  private readonly entries: string[] = []
  private readonly list = document.createElement('ol')
  private generation: string | null = null
  constructor(host: HTMLElement) {
    const details = document.createElement('details'); details.dataset['testid'] = 'exploration-trail'
    const summary = document.createElement('summary'); summary.textContent = 'This browser’s exploration trail'
    details.append(summary, this.list); host.append(details)
    this.paint()
  }
  begin(generation: string): void {
    this.pending.clear()
    if (this.generation !== generation) this.entries.length = 0
    this.generation = generation; this.paint()
  }
  request(id: string, request: Request, target: string | null): void {
    let title: string
    switch (request.type) {
      case 'expand': title = `Expand ${target ?? `slot ${request.slot}`} · ${request.direction} · ${request.relationship ?? 'all relationships'}`; break
      case 'collapse': title = `Collapse ${target ?? `slot ${request.slot}`}`; break
      case 'browse-type': title = `Browse ${request.node_type} instances`; break
      case 'load-nodes': title = `Load ${request.handles.length} source node references`; break
      case 'load-entities': title = `Load ${request.nodes.length} node and ${request.relationships.length} relationship references`; break
      case 'cypher': if (!request.as_graph) return; title = 'Query graph'; break
      case 'reset': title = 'Reset exploration'; break
      default: return
    }
    this.pending.set(id, title)
  }
  refuse(id: string | undefined): void { if (id !== undefined) this.pending.delete(id) }
  acknowledge(meta: SharedWireMeta, admitted: number, removed: number): void {
    if (meta.request_id === null) return
    const title = this.pending.get(meta.request_id)
    if (title === undefined) return
    this.pending.delete(meta.request_id)
    if (title === 'Reset exploration') this.entries.length = 0
    const nodes = meta.snapshot.last_slice?.bound
    const links = meta.snapshot.last_slice?.link_bound
    const omitted = nodes?.truncated ? Math.max(0, nodes.total - nodes.returned) : 0
    const omittedLinks = links?.truncated ? Math.max(0, links.total - links.returned) : 0
    this.entries.unshift(`${title} · ${admitted} admitted · ${removed} removed · ${nodes?.truncated ? 'at least ' : ''}${omitted} nodes omitted${omittedLinks > 0 ? ` · up to ${omittedLinks} relationships omitted` : ''}`)
    this.entries.length = Math.min(this.entries.length, 20)
    this.paint()
  }
  private paint(): void {
    this.list.replaceChildren(...this.entries.map(text => { const row = document.createElement('li'); row.textContent = text; return row }))
    this.list.dataset['empty'] = String(this.entries.length === 0)
  }
}
