# Export: the graph as somebody else's file

Three faces write the same five formats: the **export** card beside the legend,
`GET /api/export` on the running server, and the `kglite-visual export`
subcommand. Agents get a fourth, the MCP `export_view` tool.

## Formats

| `--format` | What it is |
|---|---|
| `graphml` | XML that Gephi, yEd and Cytoscape all open. The default — and see the [label note](#the-graphml-label-note) |
| `gexf` | Gephi's own XML. The one whose labels Gephi reads directly |
| `csv` | `id,type,title`, one row per node |
| `csv-edges` | `source,target,type`, one row per edge — the other half of `csv` |
| `json` | D3's `{"nodes": [...], "links": [...]}` |

`csv` and `csv-edges` are two calls rather than one zip: a zip would be a new
dependency for two text files.

## From the CLI

```bash
kglite-visual export graph.kgl --format gexf -o graph.gexf
kglite-visual export graph.kgl --format csv --cypher "MATCH (n:Field) RETURN n"
```

This is the **one place a whole-graph dump is on offer**, and the reason is not
that the CLI is trusted — it is that the question is different. Nobody clicking
a button on a bounded, progressively-disclosed view asked for 546,850 nodes.
Here the caller named a `.kgl` file and a path to write it to, at a terminal,
with no view in existence and no browser to hang. "Dump this file" is exactly
what they typed.

`--cypher` is the narrower form and the one to reach for on a large graph: the
query's nodes are the selection, bounded by the same row and byte ceilings
every other query obeys.

One JSON line on stdout, after the file is on disk; the caveats go to stderr
too, because a person watching a shell will not parse the line:

```console
$ kglite-visual export graph.kgl --format gexf -o out.gexf
kglite-visual: the nodes are exactly the ones selected; the edges are every edge this graph holds between them, which can be MORE than you saw …
{"out":"out.gexf","format":"gexf","nodes":118,"bytes":105848,"notes":["…"]}
```

## From the running server

```bash
curl -sD- "$B/api/export?format=graphml&source=live-view" -o view.graphml
```

The one `GET` in the API vocabulary, because a download is an `<a href
download>`, an anchor issues a GET, and this route reads the view and mutates
nothing.

```text
content-type: application/xml; charset=utf-8
content-disposition: attachment; filename="graph-view.graphml"; filename*=UTF-8''graph-view.graphml
x-kglv-nodes: 144
x-kglv-format: graphml
x-kglv-note: …
```

The filename is derived from the graph, in UTF-8, so a Norwegian graph keeps
its letters.

**The scope is the view.** Exactly the instance nodes on screen, never the
whole graph: this is a viewer built around a response bound, and an export that
answered "everything" would walk straight around it. An export over the entry
screen is a `400` naming what to load first:

```json
{"error":"there is nothing to export: no instance nodes are loaded. Expand a type or run a query with 'show in graph' first."}
```

The filter does not change what is written: the server's export walks the slot
space, not the client's appearance arrays, so `window.__kglv.exportNodes` is
the honest count of what the card would write — filter or no filter.

## From an agent

The MCP `export_view` tool writes the same file and hands back the text, with
the counts first so a caller that stops reading at the summary still learns the
size of what follows:

```json
{"bytes":27033,"filename":"graph-view.gexf","format":"gexf","nodes":144,
 "notes":["the nodes are exactly the ones selected; the edges are every edge this graph holds between them, which can be MORE than you saw …"]}
```

Every one of these formats is UTF-8 text, an MCP reply has no file channel, and
base64 would be a decode step for something the agent can already read.

## The two caveats

Both are true of the file and invisible in it, so they ride in `x-kglv-note`,
in the CLI's `notes`, and in the MCP reply's `notes`. Report them; do not
discover them in Gephi.

### The edge set is a superset

> the nodes are exactly the ones selected; the edges are every edge this graph
> holds between them, which can be **MORE** than you saw — a link the view's
> byte budget refused, or one a query's rows never mentioned, is still an edge
> in this file

The node set is exact. The edge set is *every* edge the graph holds between
those nodes, which is generally more than the canvas drew. Nodes and links
share one byte budget in the live view, so a slice can hold a complete node
list and an incomplete link list — and a query that returned nodes without
relationships never mentioned any edges at all.

(the-graphml-label-note)=
### GraphML has no Gephi `label` key

> kglite writes the node title under `attr.name="title"`; Gephi reads
> `attr.name="label"`, so a GraphML import shows `n0`, `n1`, … as the node
> names. Map the `title` column after import, or export `gexf`, whose
> `<node label=…>` Gephi reads directly.

**If the destination is Gephi, export GEXF.** GraphML remains the default
because it is the format yEd and Cytoscape want, and because silently renaming
kglite's attribute to suit one tool would make the file disagree with the
engine that wrote it.
