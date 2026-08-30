# kglite-visual

Interactive, high-performance visualization for `.kgl` knowledge-graph files
produced by [KGLite](https://github.com/kkollsga/kglite) — a Rust workspace and
a WebGL frontend, shipped as a localhost CLI and a Python wheel.

> **Not yet published.** No release has been cut, so the `pip install` line
> below does not resolve yet; build from the repo (`make wheel`) until it does.
> The first release deletes this paragraph.

## What it does

`.kgl` graphs reach 100M+ nodes and no browser renders that, so the entry
screen is never the whole graph. It is the **type-level meta-graph** — the
labels and relationship types with their counts, always small whatever sits
underneath — and you drill in from there with Cypher and bounded neighborhood
expansion. The server decides what crosses the wire; the bound is enforced
server-side, so a slice cannot grow past it on the way to the renderer.

Rendering is [cosmos.gl](https://cosmosgl.dev) (MIT, OpenJS Foundation): a
WebGL GPU force layout, fed over a binary protocol (typed-array buffers for
topology and positions, JSON for metadata).

## Install

```bash
pip install kglite-visual
```

The wheel carries the engine, the server and the frontend bundle inside one
compiled extension — no Node, no separate server process, no runtime Python
dependencies.

## Use it

From the shell:

```bash
kglite-visual path/to/graph.kgl        # opens a browser on localhost
kglite-visual path/to/graph.kgl --no-open --port 8080

# no server, no browser: one image, or one file for somebody else's tool
kglite-visual render path/to/graph.kgl --meta
kglite-visual export path/to/graph.kgl --format gexf -o graph.gexf
kglite-visual export path/to/graph.kgl --format csv --cypher "MATCH (n:Field) RETURN n"
```

From Python, including inside Jupyter:

```python
import kglite_visual

view = kglite_visual.show("path/to/graph.kgl")   # a live URL, served in-process
view.url
view.close()
```

`show()` also takes an in-memory `kglite` graph, handed over through
`to_bytes()` without touching the disk. In a notebook the returned view renders
itself as an iframe.

## Requirements

- Python 3.10+ (one abi3 wheel serves every version from 3.10 up)
- A `.kgl` file written by a matching KGLite release

Building from source additionally needs a Rust toolchain and Node; the
published wheels and the source distribution both carry a prebuilt frontend, so
neither needs Node at install time.

## License

MIT — see [LICENSE](LICENSE).
