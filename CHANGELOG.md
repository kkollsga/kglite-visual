# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This file records what a **user** can see change: the viewer, the wheel and the
CLI. Internal refactors, CI plumbing, test-only work and formatting do not
appear here (CLAUDE.md → "Commits & releases"). `/release` promotes
`[Unreleased]` to a version section; nothing else edits the promoted sections.

## [Unreleased]

### Changed

- **The engine is `kglite` 0.16.17**, exactly pinned (was 0.16.15). It carries
  the fixes KGLite cut from this project's six findings, and every workaround
  that existed for them is deleted rather than left standing.
- **A runaway query no longer takes the server with it.** kglite polled a
  query's deadline inside its pattern matcher but not in the row-building layer
  above it, so a wide path query ran to completion however long ago the clock
  had expired — measured here on a 546,850-node graph as more than 120 seconds
  past a 30-second ceiling, 7.3 GB of RSS, and an OS kill. It now stops at the
  deadline: three runs under a 5-second ceiling returned *"Query timed out"* at
  5.4–5.6 seconds, the extra half second being cancellation plus dropping the
  work already built, and the server answered the next request normally every
  time. The three-hop path that previews at 1,941,015 rows now answers in about
  a second, truncated to the row ceiling. The path builder's `count(*)` preview
  stays — a query one hop wider is still refused, and still costs memory before
  it is.
- **A GraphML export names its nodes.** kglite writes a Gephi-readable
  `attr.name="label"` key as of 0.16.16 — the node's title, and the connection
  type on edges — so an import into Gephi, yEd or Cytoscape shows real names
  instead of `n0`, `n1`, …. The export's second caveat note is gone with it;
  the edge-superset note stays. The `title`, `id`, `type`, `connection_type`
  and `properties` keys are unchanged.
- **`LOC` and `GEO` badges are independent.** A type declaring both a lat/lon
  pair and a WKT geometry showed only `GEO`, because kglite suppressed one
  under the other. On a real dataset 37 of 38 spatial types declare both, so
  the badge was hiding the cheap coordinates sitting next to the geometry. Both
  badges now appear. Which layouts a view offers is unchanged — that has always
  tested for either flag.

### Removed

- **`QueryTable.timed_out`.** The flag could never be `true`: a query that hits
  its deadline errors and is answered `422` with the engine's own message, so
  it never produces a result table at all. kglite deleted the diagnostic behind
  it in 0.16.16 and this follows. Nothing else on the wire moved, and the
  protocol version is unchanged — a client that still reads the key gets
  `undefined`, which behaves exactly as the constant `false` it replaced.

## [0.1.2] - 2026-08-31

### Added

- **Documentation, at
  [kglite-visual.readthedocs.io](https://kglite-visual.readthedocs.io).** A
  Sphinx + MyST site under `docs/`, built by Read the Docs from
  `.readthedocs.yaml`: getting started, the viewer in five pages — the tour,
  the **honesty model**, the layouts, the query surfaces, appearance — then
  render, export, the Python API, the CLI's full flag reference, four concept
  pages, and an agents track covering the launch contract, the JSON twin, all
  thirteen MCP tools and what an agent may claim about a screen it cannot see.
  Every claim in it was checked against the running program rather than
  against the plan: the endpoint payloads, the refusal messages, the `--help`
  text, the JSON summary lines and the MCP schemas quoted on those pages are
  transcripts. Four screenshots ship with it (432 KB total), including a `geo`
  render of a real shelf. There is deliberately **no sphinx-autoapi**: this
  package's whole Python surface is `show()` and the handle it returns, and a
  generated page over two private modules would be longer than the surface it
  describes.
- **`make docs`** builds the site with `-W --keep-going` into a purged
  `target/docs`, in its own `target/docs-venv` — not `.venv`, which belongs to
  the wheel's test loop and has no business growing Sphinx. It is **not** in
  `make gate`: gate membership is earned by a record of catching a CI failure
  and this check has none yet. CI owns it, as a sixth job on the `ci-success`
  aggregate — four pure-Python packages and a Markdown tree, no Rust
  toolchain and no Node.

### Changed

- **The README is rewritten** around what the program is for: the agent-driven
  shared view, the honesty model, and the structure-chosen layouts including
  the map. Its image links are absolute `raw.githubusercontent.com` URLs
  because a relative one breaks on PyPI, where this file is the wheel's long
  description; verified through `readme_renderer` rather than assumed. The
  "not yet published" note is gone — it should have gone with 0.1.0, which is
  what it said it would do.

## [0.1.1] - 2026-08-30

### Fixed

- **The table of a type's nodes asked for the wrong nodes.** kglite's Cypher
  `id(n)` reads the node's `id` **field** — whatever the source data called its
  key — and the generated table query was filling `$ids` with this app's
  internal node indices instead. On sodir that answered **0 rows** for
  `FieldReserves` (keys 1–2 329 against indices near 41 000) and, worse, the
  **wrong 120 rows** for `Wellbore`, whose key range overlaps its index range
  so the count came out right and the table looked correct. A slice now carries
  each node's `id` field beside its index, the query names nodes by the field,
  and a node with no `id` field is reported rather than silently missing.

- **A generated table's caveat stayed on screen under someone else's rows.**
  "12 of 18 properties, the ones most FieldReserves nodes carry" describes one
  generated table; it was left standing over every query run afterwards, so a
  `PROFILE` of `Field` was rendered under a note about `FieldReserves`. Running
  your own query now clears it.

- **A query that drew a graph left the previous query's row count on screen.**
  "Show in graph" answers with a slice rather than a table, so the results card
  kept describing whatever ran before it — a path run after a table read as
  "120 rows in 10 ms". It now says how many nodes the result named and how many
  of them were new.
- **A self-referencing relationship was counted twice.** `Person -[:KNOWS]->
  Person` contributed a hop from each end of the same edge list, so the path
  builder's picker would have offered "KNOWS ↔ Person (360)" on a graph with
  180 of them.
- **A sidebar row that hid itself did not.** `.kglv-field` set `display: flex`
  with the same specificity as the browser's own `[hidden]` rule and, being an
  author rule, won — so every row `panels.ts` hid stayed on screen. The visible
  symptom was a "caption by" picker offered on types with no string property to
  caption them with.
- **A browser that joins a session already in progress now sees the session.**
  Every client used to be greeted with the entry screen — the type-level
  meta-graph, slots `0..n` — whatever the shared view had been drilled into
  since. Attach a second tab to a view an agent had expanded and the next
  change arrived indexing slots that browser had never been told about: the
  points appeared with no label, no id and nothing to click (144 of them,
  measured on the sodir graph). The greeting now carries the whole current
  view — every live node named, holes marked, positions from slot zero — and
  the static arrangement in force, if there is one, so the newcomer lands on
  the same picture everyone else is looking at.

### Added

- **A path builder: a multi-hop question from dropdowns, shown as Cypher.**
  Pick a start type, add up to three hops — the relationship pickers offer only
  the ones the graph actually has, read out of the meta-graph, with direction in
  the label and the edge count beside it — and narrow any node with a
  `property is/contains/>/< value` filter. **The generated Cypher is on screen
  the whole time**, in a read-only strip, and one button copies it into the
  editor. Each hop carries a `count(*)` preview so the size of the answer is
  known before anything is drawn, and Run sends exactly the query on screen
  down the ordinary bounded path — no per-hop bound, one row ceiling, the same
  banner. When the last hop's preview is past what the server will return, the
  card says so before the click rather than after the wait. Values are always bound as parameters; labels, relationship types and
  property names are validated as identifiers and refused rather than quoted.
- **A table of a type's nodes, generated and shown.** The type panel offers
  "table of the N on screen"; the app writes
  `MATCH (n:Type) WHERE id(n) IN $ids RETURN …` over the twelve properties most
  of that type's nodes carry, **puts it in the Cypher box where the user can
  read and edit it**, and runs it down the ordinary bounded path. The columns
  sort by clicking a header — stably, and by type: a numeric column compares as
  numbers, so a column of ids does not put 100 before 58. The panel says when
  the twelve-column cap dropped something, and when a node on screen carries no
  `id` field for a query to name it by.
- **Export: the view, as a file somebody else's tool can open.** GraphML,
  GEXF, node CSV, edge CSV or D3 JSON, from an Export card beside the legend,
  from `GET /api/export?format=…&source=live-view`, and from the MCP
  `export_view` tool. **The scope is the view** — exactly the instance nodes
  on screen, never the whole graph: this is a viewer built around a response
  bound, and an export that answered "everything" would walk straight around
  it. An empty view is refused by name rather than answered with an empty
  file. The download is named from the graph, in UTF-8, so a Norwegian graph
  keeps its letters; the response says in a header what the file cannot — that
  the edge set can be a superset of what the canvas drew, and that kglite's
  GraphML carries no Gephi `label` key (export GEXF, or map the `title`
  column). `kglite-visual export <file>` is the CLI half, and the one place a
  whole-graph dump is on offer, because there the user typed it.
- **A geographic layout: `--layout geo`, `kernel: "geo"`.** Every node whose
  type declares a lat/lon location or a WKT geometry is drawn where it
  actually is, on an equirectangular projection whose longitudes are
  corrected by the cosine of the data's mid-latitude — so a shelf at 68°N
  comes out its own shape rather than 2.7× too wide. Mercator is deliberately
  not used: over 56–82°N it stretches one end of the same picture three times
  as much as the other. Nodes sharing a coordinate exactly (a drilling pad
  reported once per bore) are spread deterministically; nodes with no
  coordinate go into a labelled tray at the foot with a count in the status
  block, never dropped. The static render draws the world's coastline and a
  graticule under the graph, from vendored TopoJSON at **three scales, chosen
  by how much of the world the frame covers** — 1:110M for a hemisphere or
  more, 1:50M for a shelf, 1:10M for anything tighter than 25°, so a North Sea
  crop gets the fjords and a world map does not carry 400 000 points nothing
  can resolve. Each ring is cut to the segments the frame can see. No network,
  no tiles. The live view gets the positions only, and the picker says so. The
  picker offers the map exactly while the view holds nodes that are somewhere:
  a *type* is not anywhere, so the entry screen never offers it.
- **Protocol v4: a `layout` message and request.** The server computes a
  static arrangement for the live view with the same structure-chosen
  kernels the headless render uses — hop rings, packed islands or a
  held-still force pass — and broadcasts it to every attached client, which
  stops its simulation and holds the picture still. `POST /api/layout`, the
  MCP `set_layout` tool, and a picker in the sidebar.
- **The geometry caveat is conditional.** Under a static kernel the server
  knows the arrangement it sent, so `view_state` reports `layout_kernel`
  and the caveat that goes with it instead of an absolute claim that is no
  longer true.
- **A legend card** over the colour, size and link encodings, built from the
  same state the renderer's arrays are filled from.
- **A client-side filter that hides what is already loaded** — fuzzy text,
  `type:` and any property the view has fetched — with an "n of m drawn"
  honesty line. A term it cannot answer without a fetch is refused by name
  and points at Search.
- **Auto-caption:** where a type's title names nothing (few distinct values,
  or poor coverage), the server suggests the property its nodes read best
  under and the client draws it on the labels. Overridable per type; no
  slice is re-sent.

- **A real editor for the Cypher panel.** The query box highlights Cypher as
  you type — keywords, strings, numbers, comments, node labels (`:Wellbore`),
  relationship types (`[:DRILLED_IN]`), property reads (`.title`) and
  parameters (`$ids`), with node labels and relationship types in deliberately
  different colours because they are the two halves of the meta-graph. Undo,
  multi-line editing and Ctrl/Cmd+Enter all work as before; save, load and the
  recent list still put queries into it and read them back.

  It also completes from **your graph's own schema**: `:` inside a node pattern
  offers the node labels, `:` inside brackets offers the relationship types,
  and `alias.` offers the properties of whatever type that alias was bound to —
  on sodir, `MATCH (w:W` offers 15 Wellbore-ish types and `w.` offers 91
  Wellbore properties. Nothing is guessed: the labels and relationship types
  come from the meta-graph the entry screen already loaded, and a type's
  properties are fetched once, the first time you ask for them. An alias the
  editor cannot bind to a label offers nothing rather than every property of
  every type.

  And it marks mistakes **before you run anything**. A pause in typing sends the
  query to the engine's own parser — parsed, never executed — and what comes
  back is underlined where kglite put the caret and listed under the editor in
  kglite's own words: `ORDR BY` is a syntax error at line 3, column 1; a
  mistyped `:Wellbor` is a *warning* with "did you mean 'Wellbore'?", because a
  pattern that matches nothing is legal Cypher; and a `CREATE` says up front
  that this viewer runs read-only. It is a new `POST /api/validate` endpoint, so
  `curl` and an agent can ask the same question.

  It is CodeMirror 6 with hand-picked extensions rather than a stock setup, and
  it arrives in **its own chunk, fetched after the page is already usable** (102
  KB gzipped, no change to the main bundle). The plain text box is what you get
  until it lands, and what you keep if it never does — in which case the panel
  says so in one line rather than quietly handing you a worse editor.

- **Saved queries, kept by the server and shared by every face.** A query
  panel that saves what you wrote, a picker to load it back, and a recent list
  of the last 20 you ran. The store is a small JSON file per graph under your
  config directory (`$KGLITE_VISUAL_CONFIG_DIR` overrides it), keyed by the
  graph's absolute path, with one shared file for graphs handed over as bytes.
  Not the browser's storage, deliberately: an origin includes the port, and
  `--port 0` is the documented default, so `localStorage` would hand a
  different store to every launch. Because the store lives beside the session
  rather than in a handler, `kglite_visual.show()` gets the same one — and so
  does an agent: two new MCP tools, `list_saved_queries` and `run_saved_query`,
  read what you saved and run it through the ordinary Cypher path, so the
  result appears on the screen you are both looking at. Every ceiling is a
  refusal that names its number — 64 saved queries per graph, 64 KB per query,
  256 KB per file, 512 graphs — and nothing is ever deleted on your behalf:
  `kglite-visual queries list`, `… rm <file>` and `… prune` are the owner, and
  `prune` only offers the stores whose graph is gone from disk.

- **`PROFILE` reports what each clause of a query cost.** Prefix a query with
  `PROFILE` and the panel draws a row per clause above the results — the
  engine's own clause name, rows in → rows out, a bar scaled against the
  slowest clause, and microseconds. `PROFILE` is the whole interface on
  purpose: it is Cypher, it is what every other Cypher tool profiles with, and
  a checkbox that prepended the keyword behind your back would mean the query
  in the editor was not the query that ran. Unlike `EXPLAIN`, the query
  actually runs, so the results table is there beside the profile.

- **`EXPLAIN` results are drawn as a plan.** The rows have always arrived and
  the panel rendered them as three columns of data — a `step` column counting
  1..n beside an `operation` column is a numbered list wearing a grid, and
  `estimated_rows` read as a value rather than as the planner's guess. An
  `EXPLAIN` now gets its own monospace treatment: the step in the gutter, the
  operation indented, the estimate on the right where the planner produced one
  and blank where it did not. The status line says "not executed", because the
  query was planned rather than run.

- **The engine's query advisories now reach the person who typed the query.**
  kglite raises non-fatal warnings for an unknown label, an unknown
  relationship type or an absent property, each with a "did you mean?" hint —
  and until now every one of them went to the *server's* stderr while the
  browser showed "0 rows" and nothing else. `MATCH (n:NoSuchLabel) RETURN n`
  against a 546 850-node graph answered `200` with an empty table, which reads
  as "the graph has no such nodes" rather than "you mistyped a label". The
  advisories now ride the result table and are drawn above it, in kglite's own
  wording. The one warning still filtered out is the row-limit truncation
  notice, because the truncation banner already says that in this app's
  wording. `QueryTable` also carries a `timed_out` flag beside them, so a
  future engine that cancels a query at its deadline and returns the partial
  rows cannot have them read as a complete answer.

- **A load ceiling, so a graph too big for the machine is refused instead of
  swapped.** `kglite-visual --max-load-mb N`, `kglite-visual render
  --max-load-mb N` and `show(path, max_load_mb=N)` ask the engine what the
  `.kgl` will cost — read from the file's metadata head, with nothing
  decompressed — and refuse above the ceiling. The refusal is immediate
  (0.00–0.01 s against a 0.8 s load on a 546 000-node graph) and names the
  estimate, the ceiling, the terms it is made of and the two ways out. The
  wheel raises `MemoryError` for it, not `ValueError`: nothing is wrong with
  the file. Unset, kglite's own `KGLITE_MAX_LOAD_MB` still applies. The
  estimate is deliberately conservative and can refuse a graph that would have
  fitted, so it is a guard rather than a budget.

### Changed

- **The engine is `kglite` 0.16.15**, exactly pinned (was 0.16.14).
- **A truncated query result is now bounded inside the engine rather than
  after it.** The 5 000-row ceiling reaches kglite's executor, so the rows
  above it are never built: `MATCH (n) RETURN id(n)` over a 546 850-node graph
  costs about 100 MB less at its peak. Nothing about the answer changes — the
  query still runs to completion, so an `ORDER BY` result is still the genuine
  top 5 000 and an aggregate still folds every row — and the count beside it is
  still exact: the panel says *showing 5 000 of 546 850*, not an estimate.
- **A `SIGTERM`ed server still removes the engine's temporary spill**, now
  verified against a graph that actually produces one. 0.16.15 stopped
  creating the spill directory for small files, which had quietly made the
  existing check vacuous.

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
  URL, with no second process to start and nothing to install. Nine tools for
  the view: read what is on screen, put a Cypher result into it, expand or
  collapse a selection, highlight things, zoom to them, change how they are
  coloured and sized, reset, and draw a picture of the current view. The
  tools are for
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
