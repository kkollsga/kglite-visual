# Render: an image, without a browser

```bash
kglite-visual render graph.kgl --meta -o schema.svg
```

One image, one JSON line on stdout, exit. No server, no browser, no display.

The picture uses the same visual encoding as the interactive viewer — the same
size ramp, link widths, capability badges and colours — so an exported image
and the app show the same graph.

## Sources

Mutually exclusive; naming two is a usage error rather than a silent pick.

`--meta`
: The type-level meta-graph — the app's entry screen. The default when nothing
  else is named.

`--cypher "…"`
: The graph a read-only query returns. The query must `RETURN` nodes,
  relationships or paths: a table has no picture, and this says so rather than
  emitting an empty canvas.

`--expand type=T [rel=R] [dir=out|in|both]`
: A bounded neighbourhood expansion. `rel` narrows to one relationship type;
  omitted, the walk follows every one, which is the expensive case. `dir`
  defaults to `both`.

```bash
kglite-visual render graph.kgl --cypher "MATCH (f:Field)-[r]->(c:Company) RETURN f,r,c" -o fields.png --format png
kglite-visual render graph.kgl --expand type=Wellbore rel=DRILLED_BY dir=out -o wells.svg
```

## Formats and framing

| Flag | Values | Default |
|---|---|---|
| `--format` | `svg`, `png` | `svg` |
| `--theme` | `dark`, `light` | `dark` — app parity; `light` is for a white page |
| `--width` / `--height` | pixels | 2000 × 1250 |
| `-o` / `--out` | path | a name derived from the graph and the source, in the current directory |

## Layout kernels

The arrangement is **chosen from the graph's own structure**, not fixed. A
force layout is the right tool for a graph with no discoverable shape and the
wrong one for a star, a bipartite result or a schema with disconnected
families — and those are most of what a real graph hands it.

So: a neighbourhood or expansion is drawn as **hop rings** around its centre,
with same-type siblings grouped into contiguous arcs; a graph with community
structure is drawn as **packed islands**; a community that is two kinds of
thing joined only to each other is drawn as **two concentric shells**; and
unattached nodes are gathered into one labelled grid rather than scattered. The
generic force layout is the fallback for input with no shape to find.

Names radiate outward from a ring rather than sitting under every circle. The
app has no equivalent, because the app's layout has no centre to radiate from.

`--layout` forces one instead: `auto` (the default), `radial`, `islands`,
`force`, `geo`. `simulation` is deliberately absent — it means "hand the
geometry back to the viewer's GPU", and a headless render has no viewer.

### `--layout geo`

```bash
kglite-visual render graph.kgl --cypher "MATCH (f:Field) RETURN f" --layout geo -o map.png --format png
```

![144 fields drawn where they are, on a three-scale coastline](_static/sodir-geo-fields.png)

Every node whose type declares a lat/lon location or a WKT geometry is drawn
where it actually is, on an equirectangular projection corrected by the cosine
of the data's mid-latitude. The static render draws the world's coastline and a
graticule underneath, from vendored TopoJSON at three scales chosen by how much
of the world the frame covers. No network, no tiles.

`geo` answers with an error rather than a picture when nothing in the slice has
a coordinate. Full detail: {ref}`layouts <geo>`.

## Determinism

**The layout is seeded and deterministic: the same request produces the same
bytes, every time, forever.** The force pass has no randomness at all — the
seed reaches the initial placement only.

That is what makes `make check-render-baseline` an *exact* baseline over
committed golden SVGs, and it is why `--seed N` exists: it is how you get a
*different* arrangement of the same data, not how you get a random one.

## The JSON summary line

stdout carries exactly one line and nothing else, ever. Diagnostics go to
stderr, and a failed render prints **nothing** on stdout — so a harness that
read a line got a render.

```console
$ kglite-visual render graph.kgl --meta -o m.svg
{"out":"m.svg","format":"svg","width":2000,"height":1250,"nodes":5,"links":7,
 "folded":0,"layout_ms":0.61,"layout_kernel":"force","truncated":false,
 "banners":[],"bytes":3867}
```

Field names are the contract an agent parses by name.

`out`, `format`, `width`, `height`, `bytes`
: What was written, and where.

`nodes`, `links`
: What is in the picture.

`layout_ms`
: How long the arrangement took, so a slow image says which half was slow.

`layout_kernel`
: The arrangement that actually ran — which can differ from `--layout`. `auto`
  reads the scene, and a forced kernel with nothing to work with falls back. A
  caller reading only this line still learns which picture it got.

`truncated`, `banners`
: Whether a bound clipped this answer, and the banner text saying what it cut.
  The banners are **also drawn into the image**, because an image travels
  without its response.

### The honesty fields

Three keys appear only when they have something to say, so a key that is
present always carries a number.

`folded`
: Nodes drawn as one aggregate wedge rather than individually. `nodes - folded`
  is what a reader can count in the picture. (Always present; `0` when nothing
  was folded.)

`types_shown` / `types_total`
: A meta-graph render on a canvas too small for the schema draws the largest
  types it can hold and says `top N of M types shown — render larger for all`.
  Absent when every type is on the picture, and absent for every non-meta
  source.

`names_shown`
: The number of names the label grid drew, when it drew fewer than there are
  nodes.

```console
$ kglite-visual render graph.kgl --meta --width 300 --height 200 -o small.svg
{"out":"small.svg","format":"svg","width":300,"height":200,"nodes":1,"links":1,
 "folded":0,"layout_ms":0.0,"layout_kernel":"force",
 "types_shown":1,"types_total":5,"truncated":false,"banners":[],"bytes":1535}
```

More on why these exist: [the honesty model](viewer/honesty.md).

## Against a running server

`POST /api/render` answers with the image bytes and the right content type, so
an agent attached to a live session can ask for a picture without a browser in
the loop:

```bash
curl -s -XPOST $B/api/render -H 'content-type: application/json' \
  -d '{"source":{"type":"meta"},"format":"png","width":1600,"height":1000}' -o m.png
```

```text
content-type: image/png
x-kglv-nodes: 98
x-kglv-links: 124
x-kglv-truncated: false
x-kglv-layout: islands
x-kglv-banner:
```

The request body takes `source` (`meta`, `cypher`, `expand`), `format`,
`width`, `height`, `seed`, `theme` and `kernel`.

**Prefer this over one-shot CLI renders when a server is already up.** Each CLI
render loads the whole graph fresh — about 627 MB resident for a 546,000-node
file — while this endpoint reuses the one already in memory.

**It does not move the live view.** `core::render` opens a private session over
the same read-only graph, so an image request is a question and never changes
what the user is looking at.

## What the image is not

Content-identical to the app; **geometry-different**. The browser's layout runs
on the user's GPU; this is one of the structure-chosen kernels in core. Same
nodes, same links, same truncation, a different arrangement.

That holds even when the live view is under a static kernel: the render pass
folds fans and separates circles for the page it draws, and the live layout
deliberately does neither. Never claim "your screen shows X at the top left".
