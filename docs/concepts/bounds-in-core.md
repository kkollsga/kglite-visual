# Bounds in core

**Progressive disclosure is the product, not a UI preference.**

kglite's disk mode reaches 100M+ nodes and no browser renders that. So the
entry screen is the type-level meta-graph — labels and relationship types with
counts, always small, whatever the graph underneath — and drill-down happens
through Cypher and bounded neighbourhood expansion.

That much is a design. This is the part that is architecture:

> **The server decides what crosses the wire, and the bound is enforced in
> core, not in the UI.**

## Why not in the UI

A guarantee the client implements is not a guarantee.

The browser is one of at least five callers. There is also a `curl` against the
JSON twin, an agent over MCP, a second browser tab sharing the same view, and
the headless renderer. Put the ceiling in the frontend and four of those get no
ceiling at all — and the one that has it can be edited in a devtools console.

So there is one choke point, `core::expand::effective_bound`, and **no input
reaches the renderer around it**. `max nodes` in the panel, `limit` in
`/api/expand`, `limit` on the MCP `expand` tool and `--limit` on
`kglite-visual render` are all *requests*. Core clamps them.

**A change that lets an unbounded result reach the renderer is a defect, not a
feature request.**

## What the bound buys

Three things, and the third is the one that is easy to miss.

**A browser that cannot be hung by a query.** The 5,000-row ceiling reaches
kglite's executor, so rows above it are never built — `MATCH (n) RETURN id(n)`
over a 546,850-node graph costs about 100 MB less at its peak than a
bound applied after the fact. Nothing about the answer changes: the query still
runs to completion, so an `ORDER BY` result is still the genuine top 5,000, an
aggregate still folds every row, and the count beside it is exact.

**A wire format with a budget.** Nodes and links share one byte budget in core.
That is why a slice carries *two* bound triples — `meta.bound` for nodes and
`meta.link_bound` for links — and why a complete node list can sit beside an
incomplete link list.

**An export that cannot be used to walk around it.** The server's
[export](../export.md) is scoped to the view: exactly the instance nodes on
screen, never the whole graph. An export that answered "everything" would be a
one-click bypass of the bound the rest of the program is built on. The CLI's
`export` subcommand *is* allowed a whole-graph dump, because there the question
is different — the caller named a file and a destination at a terminal, with no
view in existence and no browser to hang.

## What it costs

Honesty work, everywhere. Because every answer might be partial, every answer
has to say whether it is:

- `{returned, total, truncated}` on every bounded response.
- A truncation banner in the UI, in the words the user is reading.
- The same banner **drawn into** every image, because an image travels without
  its response.
- `types_shown` / `names_shown` / `folded` in the render summary, for the three
  other ways a picture can be less than its input.
- An empty answer reported as *empty* rather than as *truncated to zero*, since
  the two have opposite remedies.

That is the subject of [the honesty model](../viewer/honesty.md), and it is
downstream of this decision rather than parallel to it. A program that draws
subsets has to be able to say so, and it can only say so reliably if one place
decides what the subset is.
