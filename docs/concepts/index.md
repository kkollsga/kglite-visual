# Concepts

Why the program is shaped the way it is. Four subjects, none of which you need
to read to use the viewer, and all of which explain a decision that looks odd
until you know the reason.

- **[Bounds in core](bounds-in-core.md)** — why the response bound lives in the
  engine crate rather than in the UI, and what that costs.
- **[The protocol](protocol.md)** — a binary wire format, a version number, and
  why an embedded frontend means there is no version skew to manage.
- **[Storage and the config directory](storage.md)** — where saved queries
  live, why not in the browser, and why nothing sweeps them.
- **[Memory](memory.md)** — what a graph costs, where it spills, and what a
  clean shutdown is for.

## The three crates

`kglite-visual-core`
: Embeds the `kglite` crate. Sessions, Cypher, snapshots, the type-level
  meta-graph, bounded neighbourhood expansion, the layout kernels and a final
  separation pass, and a **transport-agnostic** binary protocol.
  Transport-agnostic is a rule, not a description: nothing in this crate may
  know it is talking to a WebSocket.

`kglite-visual-cli`
: An axum server on localhost serving the frontend bundle out of the binary via
  `rust-embed`, plus the JSON twin, the WebSocket and the MCP endpoint. Also
  the `render`, `export` and `queries` subcommands. The tensorboard / marimo
  pattern: one binary, no install step.

`kglite-visual-py`
: PyO3 + maturin. `kglite_visual.show()`, the notebook rendering, and the
  console script that re-enters the CLI crate's own parser.

The frontend is TypeScript + Vite + [cosmos.gl](https://cosmosgl.dev)
(`@cosmos.gl/graph`, MIT, OpenJS Foundation), built to static assets and
embedded. It is not published to npm.

```{toctree}
:maxdepth: 1
:hidden:

bounds-in-core
protocol
storage
memory
```
