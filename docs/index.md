# kglite-visual

A viewer, a renderer and an agent interface for `.kgl` knowledge-graph files.
One command opens a browser on a localhost server; the same binary draws an
image without one; and while the server runs it speaks the Model Context
Protocol, so an agent can drive the window a person is watching.

```bash
pip install kglite-visual
kglite-visual graph.kgl
```

`.kgl` files reach 100M+ nodes and no browser renders that, so the entry screen
is never the whole graph. It is the **type-level meta-graph** — the labels and
relationship types with their counts — and you drill in from there with Cypher
and bounded neighbourhood expansion. The bound is enforced by the server, in
core: a slice cannot grow past it on the way to the renderer.

![The entry screen: sodir's 98 node types, drawn in proportion](_static/sodir-meta-graph.png)

Rendering is [cosmos.gl](https://cosmosgl.dev) (MIT, OpenJS Foundation) — a
WebGL GPU force layout fed over a binary protocol. The engine is
[kglite](https://kglite.readthedocs.io), embedded, so there is no database
service in the picture.

## Start here

1. `pip install kglite-visual`. One abi3 wheel per platform, no required Python
   runtime dependencies — the engine, the server and the frontend bundle all
   live inside one compiled extension.
2. `kglite-visual graph.kgl` and read the meta-graph: which types the graph has,
   how many of each, what connects to what.
3. Click a type to see what expanding it would cost, then expand it — or write
   Cypher in the panel and put the answer on screen.
4. From Python or a notebook, `kglite_visual.show(path_or_graph)` does the same
   thing in-process and hands you back the URL.
5. Point an agent at the `mcp` URL the launch line printed, and it drives the
   same view you are looking at.

**[Getting started](getting-started.md)** ·
**[The viewer](viewer/index.md)** ·
**[Agents and MCP](agents.md)** ·
**[Python API](python.md)** ·
**[CLI reference](cli.md)**

```{rubric} What it is
```

| | |
|---|---|
| Progressive disclosure | The type-level meta-graph first, drill-down after — the only entry screen that works at 100M nodes |
| Bounds in core | The response bound lives in `kglite-visual-core`, not in the UI; every bounded answer carries `{returned, total, truncated}` |
| Truncation drawn in | A clipped picture says so *in the picture*, because an image travels without its response |
| Agent-native | MCP at `/mcp` on the running server: thirteen tools over one shared, last-writer-wins view |
| Structure-chosen layouts | Hop rings, packed islands, a seeded force pass — and a real-coastline map for graphs whose nodes have coordinates |
| One binary | The frontend bundle is compiled into the executable; `pip install` and `cargo install` are the same program |

```{rubric} Pick your track
```

- **[Getting started](getting-started.md)** — install, first launch, the entry
  screen, the first drill-in, `show()` in a notebook.
- **[The viewer](viewer/index.md)** — everything the app does: expansion,
  search, filtering, appearance, the [honesty model](viewer/honesty.md), the
  [layouts](viewer/layouts.md), and the
  [query surfaces](viewer/queries.md) — saved queries, generated tables, the
  path builder, `PROFILE` and `EXPLAIN`.
- **[Agents and MCP](agents.md)** — the flagship track. The launch contract,
  the JSON twin, the thirteen MCP tools, `window.__kglv`, and the rules about
  what an agent may and may not claim about a screen it cannot see.
- **[Render](render.md)** and **[Export](export.md)** — an image, or a file for
  somebody else's tool, with no browser in the loop.
- **[Python API](python.md)** — `show()`, `launch_info`, `close()`, Jupyter,
  and honest memory numbers.
- **[CLI reference](cli.md)** — every flag of `serve`, `render`, `export` and
  `queries`.
- **[Concepts](concepts/index.md)** — why the bound lives in core, what the
  protocol version means, where saved queries are kept, what the process costs.

```{toctree}
:maxdepth: 1
:hidden:

getting-started
```

```{toctree}
:maxdepth: 2
:caption: The viewer
:hidden:

viewer/index
```

```{toctree}
:maxdepth: 1
:caption: Interfaces
:hidden:

agents
python
cli
render
export
```

```{toctree}
:maxdepth: 1
:caption: Concepts
:hidden:

concepts/index
```

```{toctree}
:maxdepth: 1
:caption: Project
:hidden:

contributing
changelog
```
