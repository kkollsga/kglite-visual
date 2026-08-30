# kglite-visual: see a knowledge graph, and let an agent drive it

[![PyPI version](https://img.shields.io/pypi/v/kglite-visual)](https://pypi.org/project/kglite-visual/)
[![Python versions](https://img.shields.io/pypi/pyversions/kglite-visual)](https://pypi.org/project/kglite-visual/)
[![License: MIT](https://img.shields.io/pypi/l/kglite-visual)](https://github.com/kkollsga/kglite-visual/blob/main/LICENSE)
[![Docs](https://img.shields.io/readthedocs/kglite-visual)](https://kglite-visual.readthedocs.io)

<!-- The pyversions badge reads `requires-python` (>=3.10). The extension is
     abi3-py310, so one wheel per platform serves every CPython from 3.10 up
     and there are deliberately no per-version classifiers to keep in step.
     The Docs badge stays red until the project is imported on readthedocs.org
     under the slug `kglite-visual`; .readthedocs.yaml already carries the
     whole build config. -->

kglite-visual is an interactive viewer, a headless renderer and an agent
interface for `.kgl` knowledge-graph files produced by
[KGLite](https://github.com/kkollsga/kglite). One command opens a browser on a
localhost server; the same binary draws an image without one; and while the
server runs it speaks the Model Context Protocol, so **an agent can drive the
window you are looking at**.

The Python wheel has no required runtime dependencies: the graph engine, the
HTTP server and the WebGL frontend bundle are all inside one compiled
extension. No Node, no separate server process, no database service.

## Quick Start

```bash
pip install kglite-visual
kglite-visual graph.kgl        # opens a browser on localhost
```

What you land on is **not** the graph. `.kgl` files reach 100M+ nodes and no
browser renders that, so the entry screen is the **type-level meta-graph** —
the labels and relationship types with their counts, always small whatever sits
underneath — and you drill in from there with Cypher and bounded neighbourhood
expansion.

<p align="center">
  <img src="https://raw.githubusercontent.com/kkollsga/kglite-visual/main/docs/_static/sodir-geo-fields.png"
       alt="144 fields drawn where they actually are, on a real coastline at the scale the frame can resolve"
       width="720">
</p>

*Above: `kglite-visual render graph.kgl --cypher "MATCH (f:Field) RETURN f"
--layout geo`. No tiles, no network — the coastline ships in the binary.*

Two more commands, no server and no browser in either:

```bash
kglite-visual render graph.kgl --meta -o schema.svg      # one image, one JSON line
kglite-visual export graph.kgl --format gexf -o out.gexf # one file for somebody else's tool
```

Then hand the running server to an agent. The MCP endpoint is on the same port,
and its URL is printed on the same stdout line as everything else:

```json
{"url":"http://127.0.0.1:54137/","port":54137,"pid":69850,"graph":"graph.kgl","mcp":"http://127.0.0.1:54137/mcp"}
```

**→ [Getting started](https://kglite-visual.readthedocs.io/en/latest/getting-started.html) ·
[Agents and MCP](https://kglite-visual.readthedocs.io/en/latest/agents.html).**

## What makes it different

Three things this does that a graph viewer normally does not.

**An agent drives the window you are watching.** The running server speaks MCP
at `/mcp` — no second process to start, no discovery file, nothing to install:
attaching an agent is pointing it at a URL. Thirteen tools act on **one shared
view**, last writer wins: read what is on screen, put a Cypher result into it,
expand or collapse, highlight, zoom, recolour, re-lay-out, export it, draw a
picture of it, and run the queries *you* saved under the names you chose.
Whoever changes the view — you, the agent, a `curl` — every connected window
sees the change immediately. Watching an agent expand a type and zoom to what
it found is the feature, not a side effect of it.
**→ [Agents and MCP](https://kglite-visual.readthedocs.io/en/latest/agents.html).**

**The honesty model: truncation is drawn into the picture.** A graph viewer is
a machine for showing you less than there is, and the design position here is
that the subset must name itself. The response bound lives in **core**, not in
the UI — a guarantee the client implements is not a guarantee — so a `curl`, an
agent and a second tab all hit the same ceiling. Every bounded answer carries
`{returned, total, truncated}`; a slice carries *two* of those, because nodes
and links share one byte budget and a complete node list can sit beside an
incomplete link list. And because an image travels without its response, the
banner is drawn **into** the image, beside three more counts for the other ways
a picture can be less than its input: `types_shown` (a canvas too small for the
schema draws the largest types and says `top 24 of 98`), `names_shown` (labels
that lost their cell keep their circle and lose their name), and `folded` (a
fan too big to read is one wedge saying how big it is).
**→ [The honesty model](https://kglite-visual.readthedocs.io/en/latest/viewer/honesty.html).**

**Layouts chosen from the graph's own structure — including a real map.** A
force layout is the right tool for a graph with no discoverable shape and the
wrong one for a star, a bipartite result or a schema with disconnected
families, which is most of what a real graph hands it. So a neighbourhood is
drawn as hop rings, a community-structured graph as packed islands with a quiet
boundary round each, and unattached nodes as one labelled grid. And where the
nodes carry coordinates there is `--layout geo`: an equirectangular projection
whose longitudes are corrected by the cosine of the data's mid-latitude — so a
shelf at 68°N comes out its own shape rather than 2.7× too wide — over the
world's real coastline at **three scales chosen by how much of the world the
frame covers**, so a North Sea crop gets the fjords and a world map does not
carry 400,000 points nothing can resolve. Nodes with no coordinate go in a
labelled tray with a count, never dropped.
**→ [Layouts](https://kglite-visual.readthedocs.io/en/latest/viewer/layouts.html).**

## From Python, and from a notebook

```python
import kglite_visual as kv

view = kv.show("graph.kgl")     # the same server, in-process
view.url                        # 'http://127.0.0.1:54137/'
view.launch_info                # {'url', 'port', 'pid', 'graph', 'mcp'}
view.close()                    # stops it, frees the port

# Or hand over an in-memory kglite graph — through to_bytes(), never the disk.
import kglite
view = kv.show(kglite.load("graph.kgl"))
```

In a notebook the returned object renders itself in the cell: a proxy-prefixed
iframe where `jupyter-server-proxy` can reach the port, and **no iframe at
all** — the URL plus an `ssh -N -L` hint — where the kernel looks remote,
because a localhost iframe from a remote kernel is a silently blank frame.

`show(path)` is the large-graph answer: handing over an in-memory graph costs
about 2× the graph's size at the moment of the call.
**→ [Python API](https://kglite-visual.readthedocs.io/en/latest/python.html).**

## Render and export

```bash
# an image: --meta, --cypher "…" or --expand type=T rel=R dir=out
kglite-visual render graph.kgl --meta -o schema.svg
kglite-visual render graph.kgl --cypher "MATCH (f:Field) RETURN f" --layout geo --format png

# a file: graphml | gexf | csv | csv-edges | json
kglite-visual export graph.kgl --format gexf -o graph.gexf
kglite-visual export graph.kgl --format csv --cypher "MATCH (n:Field) RETURN n"
```

The render's layout is **seeded and deterministic** — the same request produces
the same bytes, forever — so `--seed` is how you get a *different* arrangement
of the same data, and an exact golden baseline is possible at all. Each command
writes its file and prints one JSON line describing it; nothing else ever
touches stdout.
**→ [Render](https://kglite-visual.readthedocs.io/en/latest/render.html) ·
[Export](https://kglite-visual.readthedocs.io/en/latest/export.html).**

## Requirements

CPython 3.10+ (one abi3 wheel serves every version from 3.10 up) on macOS,
Linux and Windows, plus a `.kgl` file written by a matching KGLite release —
this version pins `kglite 0.16.15`. Building from source additionally needs a
Rust toolchain; the published wheels and the source distribution both carry a
prebuilt frontend, so neither needs Node at install time.

## Documentation

Full docs at **[kglite-visual.readthedocs.io](https://kglite-visual.readthedocs.io)**.

- **[Getting started](https://kglite-visual.readthedocs.io/en/latest/getting-started.html)** — install, first launch, the entry screen, the first drill-in, `show()` in a notebook.
- **[The viewer](https://kglite-visual.readthedocs.io/en/latest/viewer/index.html)** — the app in full:
  [the honesty model](https://kglite-visual.readthedocs.io/en/latest/viewer/honesty.html) ·
  [layouts](https://kglite-visual.readthedocs.io/en/latest/viewer/layouts.html) ·
  [query surfaces](https://kglite-visual.readthedocs.io/en/latest/viewer/queries.html) (editor, saved queries, generated tables, the path builder, `PROFILE`/`EXPLAIN`) ·
  [appearance and filtering](https://kglite-visual.readthedocs.io/en/latest/viewer/appearance.html).
- **[Agents and MCP](https://kglite-visual.readthedocs.io/en/latest/agents.html)** — the launch contract, the JSON twin, the thirteen tools, `window.__kglv`, and what an agent may claim about a screen it cannot see.
- **[Python API](https://kglite-visual.readthedocs.io/en/latest/python.html)** · **[CLI reference](https://kglite-visual.readthedocs.io/en/latest/cli.html)** · **[Render](https://kglite-visual.readthedocs.io/en/latest/render.html)** · **[Export](https://kglite-visual.readthedocs.io/en/latest/export.html)**.
- **[Concepts](https://kglite-visual.readthedocs.io/en/latest/concepts/index.html)** —
  [bounds in core](https://kglite-visual.readthedocs.io/en/latest/concepts/bounds-in-core.html) ·
  [the protocol](https://kglite-visual.readthedocs.io/en/latest/concepts/protocol.html) ·
  [storage](https://kglite-visual.readthedocs.io/en/latest/concepts/storage.html) ·
  [memory](https://kglite-visual.readthedocs.io/en/latest/concepts/memory.html).

Rendering is [cosmos.gl](https://cosmosgl.dev) (MIT, OpenJS Foundation): a
WebGL GPU force layout, fed over a binary protocol — typed-array buffers for
topology and positions, JSON for metadata.

## Stability

Alpha, pre-1.0. The launch contract (one JSON line: `url`, `port`, `pid`,
`graph`, `mcp`), the render and export summary lines, and the MCP tool **names**
are the surfaces to depend on; everything else may move.
[CHANGELOG.md](https://github.com/kkollsga/kglite-visual/blob/main/CHANGELOG.md)
records what a user can see change.

## License

MIT — see [LICENSE](https://github.com/kkollsga/kglite-visual/blob/main/LICENSE).
