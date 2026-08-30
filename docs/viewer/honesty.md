# The honesty model

A graph viewer is a machine for showing you less than there is. Every screen in
this app is a subset, and the design position is that **the subset must name
itself**. A picture that quietly shows part of an answer is worse than no
picture, because it is indistinguishable from a complete one.

That position has consequences in code, and they are the reason several
features look the way they do.

## The bound is in core, not in the UI

The response bound — how many nodes and links may cross the wire for one
request — lives in `kglite-visual-core`, at
`core::expand::effective_bound`. Nothing reaches the renderer around it.

A guarantee the client implements is not a guarantee: a `curl`, an agent over
MCP, a second browser tab and the app itself all go through the same choke
point and get the same ceiling. The `max nodes` box in the panel is a *request*
that the server clamps. See [bounds in core](../concepts/bounds-in-core.md) for
why this is architecture rather than validation.

## Every bounded answer carries three numbers

`{returned, total, truncated}` rides with every bounded response. Not a
boolean, not a "…" in the corner: the count you got, the count there was, and
whether a ceiling was the reason for the difference.

A **graph slice carries two of these triples**, and that is deliberate:
`meta.bound` for its nodes and `meta.link_bound` for its links. Nodes and links
share one byte budget in core, so the node list can be complete while the link
list is not. A slice that reported only the first would let a partial
neighbourhood read as a whole one.

In the browser this becomes the truncation banner:

```text
showing 400 of 11,292 nodes and 748 of up to 25,160 links
```

`up to` on the link count, because the link total is itself an upper bound
rather than a count of edges that exist between the returned nodes.

## Empty is not the same as truncated

Expanding a type over a relationship none of its nodes has used to report
*"showing 0 of 144 nodes"* — the wording of a size limit being hit, when in
truth the walk found nothing at all. The two have opposite remedies: one asks
you to raise a limit, the other to fix a relationship name. An empty answer is
now reported as empty.

The same distinction runs through the rest of the app. `/api/export` on an
empty view is a **400 that names the problem**, not an empty file:

```json
{"error":"there is nothing to export: no instance nodes are loaded. Expand a type or run a query with 'show in graph' first."}
```

## Ceilings are refusals, not truncations

Where a limit protects a durable store rather than a response, it refuses by
name and quotes its own number. The [saved-query store](../concepts/storage.md)
holds 64 queries per graph, 64 KB per query, 256 KB per file and 512 graphs;
exceeding one of those is a `400` saying which, never a silent drop of the
oldest entry. Nothing in that store is ever deleted on your behalf.

## Truncation is drawn into the picture

An image travels without the response that produced it. Somebody pastes a PNG
into a chat window and the JSON that said "this is 400 of 11,292" is gone. So
the same banner the viewer shows is **drawn into the image**, and the
[render](../render.md) summary carries the numbers a second time for anything
reading the line instead of looking at the picture.

Three more honesty fields exist for exactly this reason:

```console
$ kglite-visual render graph.kgl --meta --width 300 --height 200 -o small.svg
{"out":"small.svg","format":"svg","width":300,"height":200,"nodes":1,"links":1,
 "folded":0,"layout_ms":0.0,"layout_kernel":"force",
 "types_shown":1,"types_total":5,"truncated":false,"banners":[],"bytes":1535}
```

`types_shown` / `types_total`
: A meta-graph render on a canvas too small for the schema draws the largest
  types it can hold, largest first, and its status block reads `top 1 of 5
  types shown — render larger for all`. On a real schema that is `top 24 of 98`.
  The honest answer to "a hundred type names will not fit" is not a hundred
  unreadable ones. A canvas that fits the schema is unaffected and says
  nothing — both keys are absent from the line above 1600×1000, which holds all
  98 of them.

`names_shown`
: A label whose own cell and every cell around it is taken keeps its circle and
  loses its name. Which names go is decided by size and connectedness, so hubs
  keep their names and the small fry thin out — and the count of what was
  dropped is drawn in the status block.

`folded`
: When more than a couple of dozen same-type leaves hang off one node *and* the
  canvas has no room for them, they are drawn as a single wedge reading
  `Type × N (showing none)`, moored outside the fan it belongs to. `nodes -
  folded` is what a reader can actually count in the picture.

All three are absent from the JSON line when they have nothing to say, so a key
that is present always carries a number.

## The filter says how much it is hiding

The client-side {ref}`filter <filter>` hides what is already loaded. It
never fetches, and it never pretends to: the panel carries an **n of m drawn**
line, and a term it cannot answer without a fetch — a property no slice in the
view has ever carried — is refused by name and points at Search.

The same pair shows up in the debug hook: `window.__kglv.pointCount` is *live
points excluding whatever the filter is hiding*, and `filteredOut` is that
count. Neither number is honest alone.

## An agent is told what it cannot see

The layout runs on the viewer's GPU, and the server never learns where the
points ended up. So a rendered image of a live view has the same nodes and the
same links in a **different arrangement**, and every MCP surface says so:

> The live layout runs on the viewer's GPU and the server does not know where
> the points ended up (`layout_kernel` is `simulation`). A render of this view
> is content-identical and geometry-different: same nodes, same links, same
> truncation, a different arrangement. Describe what is in the view, never
> where it is on the user's screen — or ask for a static layout, after which
> the arrangement is this server's own and can be described.

That caveat is **conditional**, which is the part that took work. Under a
static layout the server computed, the sentence above is false — and an
unconditional caveat is one agents learn to ignore. `view_state.layout_kernel`
decides which of two wordings applies, and core owns both.
See [agents](../agents.md#what-an-agent-may-claim).

## The export names what the file cannot

Two facts about an exported view are true of the file but not visible in it, so
they ride in a response header and in the MCP reply's `notes`:

```text
x-kglv-note: the nodes are exactly the ones selected; the edges are every edge
  this graph holds between them, which can be MORE than you saw … | kglite
  writes the node title under attr.name="title"; Gephi reads
  attr.name="label", so a GraphML import shows n0, n1, … as the node names.
```

Reporting a caveat beats discovering it in Gephi. See [export](../export.md).

## Advisories reach the person who typed the query

kglite raises non-fatal warnings — an unknown label, an unknown relationship
type, an absent property, each with a *"did you mean?"* hint. Every one of them
used to go to the **server's** stderr while the browser showed "0 rows".

`MATCH (n:NoSuchLabel) RETURN n` against a 546,850-node graph answers `200`
with an empty table, which reads as *"the graph has no such nodes"* rather than
*"you mistyped a label"*. The advisories now ride the result table and are
drawn above it, in kglite's own words. The one warning still filtered out is
the row-limit truncation notice, because the truncation banner already says
that in this app's wording.

`QueryTable` also carries a `timed_out` flag beside them, so a future engine
that cancels a query at its deadline and returns partial rows cannot have them
read as a complete answer.
