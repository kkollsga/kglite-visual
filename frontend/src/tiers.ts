/**
 * The client half of the describe()-driven tiering (plan D12).
 *
 * The *choice* of tier is the server's — it is the only side that can see how
 * many types exist without sending them. All the client decides is which of
 * its two entry screens the chosen tier maps to.
 */

import type { DetailTier } from './generated/DetailTier'

/**
 * True when the payload is small enough to be a picture.
 *
 * `top-types` renders too: it is a real meta-graph, clipped to the largest 50,
 * and its truncation metadata is what tells the user so. Only `summary` — a
 * graph with thousands of node types — has nothing drawable, and five thousand
 * labelled circles is not a picture of anything.
 */
export function rendersGraph(tier: DetailTier): boolean {
  return tier !== 'summary'
}
