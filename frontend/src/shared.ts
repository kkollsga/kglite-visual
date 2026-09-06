import type { RevisionStamp } from './generated/RevisionStamp'
import type { SharedSnapshotMeta } from './generated/SharedSnapshotMeta'

/** A gap loses transient actions; reconnect before accepting a fresh snapshot baseline. */
export class SharedState {
  generation: string | null = null
  snapshot: SharedSnapshotMeta | null = null
  gaps = 0
  needsResync = false

  begin(generation: string): void {
    this.snapshot = null
    this.needsResync = false
    this.generation = generation
  }

  accept(next: SharedSnapshotMeta): boolean {
    if (this.needsResync) return false
    if (next.stamp.generation !== this.generation || !/^\d+$/.test(next.stamp.revision)) return false
    const previous = this.snapshot
    if (previous !== null) {
      const revision = BigInt(next.stamp.revision)
      const current = BigInt(previous.stamp.revision)
      if (revision <= current) return false
      if (revision > current + 1n) { this.gaps += 1; this.needsResync = true; return false }
    }
    this.snapshot = next
    return true
  }

  get stamp(): RevisionStamp | null { return this.snapshot?.stamp ?? null }

  matches(stamp: RevisionStamp): boolean {
    return stamp.generation === this.stamp?.generation && stamp.revision === this.stamp.revision
  }
}
