# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This file records what a **user** can see change: the viewer, the wheel and the
CLI. Internal refactors, CI plumbing, test-only work and formatting do not
appear here (CLAUDE.md → "Commits & releases"). `/release` promotes
`[Unreleased]` to a version section; nothing else edits the promoted sections.

## [Unreleased]

Nothing has been released yet, so everything the project does is listed once,
here. The first release promotes this block whole.

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

### Fixed

- **Relationship counts on a `.kgl` saved without a cardinality cache.** Such a
  file loads with every relationship count fabricated as zero, which made the
  meta-graph claim a graph with three quarters of a million edges had none and
  left the expansion preview empty. The viewer now detects that shape at load,
  recomputes the counts from the edges themselves (11 ms for 765 373 edges),
  and says on stderr that it did so — a recomputed number and a stored one are
  different provenance, and the difference is reported rather than laundered.

[Unreleased]: https://github.com/kkollsga/kglite-visual/commits/main
