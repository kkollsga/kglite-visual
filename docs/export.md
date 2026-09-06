# Export a scoped graph or image

Open **Export** to choose the data scope and format, inspect a preview, then
download. The preview identifies its graph revision, node and relationship
counts, and format limitations.

## Choose the scope

- **Visible instances** exports the nodes that pass the current filters and
  exactly their retained visible relationships. Parallel relationships and
  self-loops remain separate records.
- **Loaded instances with source-induced relationships** includes hidden loaded
  nodes and every source relationship between them. It can contain relationships
  the exploration never loaded.
- **Deterministic server image** renders the chosen scope as SVG or PNG with
  explicit dimensions. Its layout is computed for the image; it is not a
  screenshot of the browser camera or live force simulation.

Changing the shared view or export settings after a preview requires refreshing
it. Download refuses a stale preview instead of silently exporting a different
result. Captured output remains private: preview and download do not alter the
shared exploration.

## Graph formats

| Format | Contents and limitations |
|---|---|
| `graphml` | XML with readable node/relationship labels and JSON-valued property data. |
| `gexf` | XML with node type/title and relationship labels; arbitrary source properties are omitted. |
| `csv` | Structural node columns: `id,type,title`. |
| `csv-edges` | Structural relationship columns: `source,target,type`. |
| `json` | D3 nodes/links. Large integers require a lossless JSON parser. |

Structural CSV exports are separate node and relationship files. The Data
workspace also offers table CSV with the exported lane and scope named: query
results, selected records or visible records. A partial fetched result remains
labelled partial; downloading it does not fetch the whole source graph.

Scoped graph files use export-local IDs. These are not durable source keys.
Machine callers can request an identity mapping in preview metadata; source
handles are valid only in their session generation. No format is advertised as
a lossless typed round trip. D3 JSON refuses relationship properties that
collide with its reserved topology fields; GraphML preserves those properties.
D3 JSON also omits a source node property named `type`, which is reserved for
the node type. Choose GraphML when that property must be retained.

## Images

Choose SVG or PNG, dimensions, theme and a static layout kernel. The image and
its metadata identify scope, revision, omissions and rendering limitations.
Dense labels can be omitted or nodes folded to fit; these outcomes are reported
in the image. A self-loop retained in data may be reported as not drawn by the
image renderer. Preview and download use the same settings. Label priority and highlighting use
shared selection/highlights; ordinary browser-local selection and hover are not
captured in the server image.

Scoped output preparation admits at most the existing loaded-member limits,
8 MiB of copied properties and a conservative 16 MiB encoded artifact bound.
Oversize output is refused as a whole. It never silently drops properties or
relationships to fit an export. Image dimensions retain their own limits. KGLite may materialize selected disk
values internally before the viewer can inspect their size, so these limits
are not a total process-memory ceiling. Traversal checks a ten-second work
budget; individual layout/format library calls are checked after returning and
are not forcibly interrupted at that deadline.

## HTTP and MCP

Read the current `stamp` and `subset_revision` from `GET /api/view-state`, then
submit the explicit scope and format to `POST /api/export/preview`:

```json
{"scope":"visible","format":"gexf",
 "expected":{"generation":"<session generation>","revision":"<revision>"},
 "subset_revision":"<subset revision>"}
```

Send the same fields plus the returned `preview_digest` to
`POST /api/export/download`. A stale stamp or changed settings returns `409`.
`include_identity:true` adds the bounded source-handle mapping to export preview
metadata. It does not put that mapping into HTTP headers.

Images use `POST /api/render/preview` and `POST /api/render/download`, with
format, width, height, seed, theme and static kernel settings. Image preview
returns bounded metadata and base64 image data.

MCP `export_view` and `render` accept explicit captured scopes and revision
fields. Their descriptions distinguish preview from download/final output.
Omitting the new scope preserves their existing behavior.

The compatibility route remains:

```bash
curl -sD- "$B/api/export?format=graphml&source=live-view" -o view.graphml
```

Its scope is loaded instances and source-induced relationships, including hidden
nodes. It does not export the whole graph. An empty loaded view is refused.
The `x-kglv-note` header describes its relationship-scope caveat.

## CLI whole-source export

```bash
kglite-visual export graph.kgl --format gexf -o graph.gexf
kglite-visual export graph.kgl --format csv --cypher "MATCH (n:Field) RETURN n"
```

The standalone export command can export an entire source file. With `--cypher`,
it exports the query's bounded node selection and the source-induced
relationships between those nodes. It writes the file, then prints one JSON
line with filename, format, counts, bytes and notes. This is separate from a
running viewer's captured visible scope.

(the-graphml-label-note)=
## GraphML labels

GraphML declares `attr.name="label"` keys: `node_label` holds the node title and
`edge_label` the relationship type. These give external consumers readable
names while preserving the separate `id`, `type` and property data.
