import type { AppearanceMapping } from './generated/AppearanceMapping'
import type { AppearanceNode } from './generated/AppearanceNode'
import type { NodeHandle } from './generated/NodeHandle'
import { handleKey } from './data'

/** Source identity survives slot compaction; null channels retain structural encoding. */
export class AppearanceMappingIndex {
  private nodes = new Map<string, AppearanceNode>()
  set(mapping: AppearanceMapping): void { this.nodes = new Map(mapping.nodes.map(node => [handleKey(node.handle), node])) }
  get(handle: NodeHandle | null | undefined): AppearanceNode | undefined { return handle == null ? undefined : this.nodes.get(handleKey(handle)) }
}
