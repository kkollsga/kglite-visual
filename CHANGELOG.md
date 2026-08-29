# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This file records what a **user** can see change: the viewer, the wheel and the
CLI. Internal refactors, CI plumbing, test-only work and formatting do not
appear here (CLAUDE.md → "Commits & releases"). `/release` promotes
`[Unreleased]` to a version section; nothing else edits the promoted sections.

## [Unreleased]

## [0.1.0] - 2026-08-29

Everything the project does, listed once: this is the first release, and it
ships the whole build-out.

### Added

- **`kglite-visual <graph.kgl>` — a localhost viewer for `.kgl` knowledge
  graphs.** One binary, no install step: it starts an HTTP server on a free
  port, prints its launch details as a single JSON line on stdout, opens a
  browser, and serves a WebGL frontend that is embedded in the executable
  itself. `--no-open` suppresses the browser, `--port` pins the port.
- **The type-level meta-graph as the entry screen.** A `.kgl` file can hold
  more nodes than any browser will render, so the first view is never the whole
  graph: it is the labels and relationship types with their counts and the
  connections between them, which stays small whatever the graph underneath.
- **Drill-down by Cypher and by bounded neighborhood expansion.** Run a query
  from the panel, or expand outward from a selection. The size of any slice
  that crosses the wire is decided and enforced by the server, and a truncated
  result says so in the UI rather than silently showing part of an answer.
- **Node and relationship inspection** — properties, per-type property
  statistics, and server-side search over the loaded graph.
- **`pip install kglite-visual` — the same viewer as a Python wheel.**
  `kglite_visual.show(path_or_graph)` starts the server in-process and returns
  a live view object carrying the URL and port; `.close()` shuts it down, and
  it also closes at interpreter exit. An in-memory `kglite` graph is handed
  over through `to_bytes()` without touching the disk.
- **Jupyter rendering.** A view returned by `show()` renders itself as an
  iframe in a notebook, including a check for the remote-kernel case where the
  browser cannot reach the kernel's localhost.
- **The `kglite-visual` console script ships in the wheel**, re-entering the
  same code path as the standalone binary, so the pip install and the cargo
  install are the same program rather than two that drift.
- **One abi3 wheel per platform**, serving every CPython from 3.10 up, with no
  required Python runtime dependencies: the engine, the server and the frontend
  bundle are all inside the compiled extension.
- **A source distribution that carries a prebuilt frontend.** Installing from
  source on a platform with no matching wheel needs a Rust toolchain — and
  deliberately not Node, npm or a network round trip to a JavaScript registry.
- **`kglite-visual render <graph.kgl>` — an image of a graph, without a
  browser.** Draw the type-level meta-graph (`--meta`), the graph a Cypher
  query returns (`--cypher "…"`), or a bounded neighbourhood expansion
  (`--expand type=T rel=R dir=out`), as SVG or PNG. The picture uses the same
  visual encoding as the interactive viewer — the same size ramp, link widths,
  capability badges and colours — so the exported image and the app show the
  same graph. The layout is seeded and deterministic: the same request produces
  the same bytes every time, and `--seed` picks a different arrangement of the
  same data. `--theme light` renders for a white page. The command writes the
  file and prints one JSON line describing it.
- **The image's layout is chosen from the graph's own structure, not fixed.** A
  force layout is the right tool for a graph with no discoverable shape and the
  wrong one for a star, a bipartite result or a schema with disconnected
  families — and those are most of what a real graph hands it. So a
  neighbourhood or expansion is drawn as **hop rings** around its centre, with
  same-type siblings grouped into contiguous arcs so a branch reads as a
  branch; a graph with community structure is drawn as **islands**, each laid
  out on its own and packed with a quiet boundary round it; a community that is
  two kinds of thing joined only to each other is drawn as **two concentric
  shells**; and unattached nodes are gathered into one labelled grid rather than
  scattered. The generic force layout remains the fallback for input with no
  shape to find. Names radiate outward from a ring rather than sitting under
  every circle, which the app has no equivalent of because the app's layout has
  no centre to radiate from.
- **A fan too big to read says how big it is.** When more than a couple of dozen
  same-type leaves hang off one node *and* the canvas has no room for them, they
  are drawn as one wedge reading `Type × N (showing none)`, moored outside the
  fan it belongs to; the image's status block says how many nodes were folded
  this way. On a canvas too small to name every node, the picture keeps the
  names a reader can use and says how many it dropped.
- **A canvas too small for the whole schema draws the largest types and says
  so.** A meta-graph render sized for a chat message cannot hold a hundred type
  names legibly, and the honest answer is not a hundred unreadable ones: the
  picture draws the types the canvas can hold, largest first, and its status
  block reads `top 24 of 98 types shown — render larger for all`. The CLI's
  JSON line and the MCP render result carry the same two numbers, because an
  image travels without its response. A canvas that fits the schema is
  unaffected and says nothing.
- **Names that would land on top of each other are dropped, and counted.** A
  label whose own cell and every cell around it is taken keeps its circle and
  loses its name, and the status block says `56 of 98 names shown`. Which names
  go is decided by size and connectedness, so a hub type keeps its name and the
  small fry thin out. A folded fan's count and the node a picture is centred on
  are never thinned.
- **`kglite-visual` stops cleanly on SIGTERM and Ctrl-C**, which means the
  temporary working copy the engine spills for a large graph — 370 MB for a
  half-million-node file — is removed instead of left on disk. A `kill -9`
  still leaks it; nothing in a process can prevent that.
- **`POST /api/render` on the running server** answers with the image bytes and
  the right content type, so an agent attached to a live session can ask for a
  picture without a browser in the loop. It renders against a private view of
  the same graph, so asking for an image never moves what the user is looking
  at.
- **A truncated image says so, in the image.** The same banner the viewer
  shows — "showing 400 of 11,292 nodes and 748 of up to 25,160 links" — is
  drawn into the picture, because an image travels without the response that
  produced it.
- **An agent can drive the window you are looking at.** The running server now
  speaks the Model Context Protocol at `/mcp` — the URL is printed on the same
  stdout line as everything else, so attaching an agent is pointing it at a
  URL, with no second process to start and nothing to install. Nine tools: read
  what is on screen, put a Cypher result into it, expand or collapse a
  selection, highlight things, zoom to them, change how they are coloured and
  sized, reset, and draw a picture of the current view. The tools are for
  *navigating* a graph together; querying one is still the graph's own MCP
  server's job.
- **The picture follows, live.** Whoever changes the view — you, an agent, a
  `curl` — every connected window sees the change immediately. Watching an
  agent expand a type and zoom to what it found is the point of the feature,
  not a side effect of it.
- **An agent knows what it cannot see.** The layout runs on your GPU and the
  server never learns where the points ended up, so a rendered image of your
  view has the same nodes and links in a different arrangement. That caveat is
  written into the tool descriptions and returned beside every render, so an
  agent describes what is in your view rather than where it is on your screen.

### Changed

- **The entry screen lays itself out.** The meta-graph used to be drawn on a
  fixed lattice, which on a real graph is a field of evenly spaced dots that
  says nothing about which types connect to which. The server's positions are
  now a starting point and a force layout takes over from there, so connected
  types settle next to each other and the schema is visible as a shape. Nodes
  can be dragged. `?deterministic=1` on the viewer URL restores the fixed
  layout, which is what the test suites use.
- **The meta-graph is drawn in proportion.** Type circles are sized on a log
  scale by member count, so a type with three members and one with a hundred
  thousand are both visible and clearly different; links are drawn in
  proportion to the edges they stand for; and a *supporting* type — one that
  hangs off another in the graph's own type hierarchy — is drawn quieter than
  the types the graph is about. Every type is now labelled, rather than only
  those that won their patch of screen.
- **The graph no longer draws underneath the side panels**, where it could not
  be seen, clicked or labelled.
- **The launch line carries one more key.** The single JSON line on stdout —
  and the dict `show()` returns — now includes `mcp`, the MCP endpoint's URL,
  alongside `url`, `port`, `pid` and `graph`. Anything reading the four
  existing keys by name is unaffected.
- **A `curl` against the JSON API no longer moves the view in secret.** Those
  endpoints have always changed what the server is showing; the browser was
  never told, and drew a stale picture until something else happened to
  refresh it.

- **The engine is `kglite` 0.16.14**, exactly pinned. That release fixes two
  defects this project found and reported while building against 0.16.13, and
  both are visible from here: a `.kgl` whose stored relationship counts were
  fabricated as zero is repaired when it is opened, and saving the same graph
  twice now produces identical bytes.

### Fixed

- **Relationship counts on a `.kgl` saved without a cardinality cache.** Such a
  file loaded with every relationship count fabricated as zero, which made the
  meta-graph claim a graph with three quarters of a million edges had none and
  left the expansion preview empty. Fixed in the engine and verified on a
  546 850-node file that had the defect: the counts now arrive correct, and
  quietly, with no repair note to explain them. This project shipped a
  load-time repair for it first; that repair is gone, because the file no
  longer arrives broken.
- **An expansion that matched nothing said the wrong thing.** Expanding a type
  over a relationship none of its nodes has reported "showing 0 of 144 nodes" —
  the wording of a size limit being hit — when in truth the walk found nothing
  at all. The two have opposite remedies: one asks you to raise a limit, the
  other to fix a relationship name. It now reports an empty answer as empty.
- **Collapsing a selection clears it.** Nodes removed from the view stayed in
  its highlight and selection sets, so the counts the viewer reports described
  nodes that were no longer on screen.

[Unreleased]: https://github.com/kkollsga/kglite-visual/commits/main
