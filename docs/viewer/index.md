# The viewer

`kglite-visual graph.kgl` opens one page: a WebGL canvas on the left and a
column of panels on the right. This page is the tour. Four subjects are big
enough to have their own:

- **[The honesty model](honesty.md)** — bounds, truncation, and every place the
  app tells you what it is *not* showing. This is the product philosophy, not a
  feature list.
- **[Layouts](layouts.md)** — the live GPU force simulation, the static kernels
  the server computes, and the geographic map.
- **[Query surfaces](queries.md)** — the Cypher editor, saved queries, the
  generated table, the path builder, `PROFILE` and `EXPLAIN`.
- **[Appearance](appearance.md)** — colour, size, captions, the legend and the
  client-side filter.

```{toctree}
:maxdepth: 1
:hidden:

honesty
layouts
queries
appearance
```

## The entry screen is the meta-graph

A `.kgl` file can hold more nodes than any browser will render, so the first
view is never the whole graph. It is the **type-level meta-graph**: one node
per node label, one link per relationship type, each carrying its count. That
picture stays small whatever the graph underneath — 98 nodes for a graph of
546,850 — and it is the only entry screen that works at every scale kglite
supports.

The encoding is the same one the [renderer](../render.md) uses, so an exported
image and the app agree:

| Channel | Meaning |
|---|---|
| Circle area | Member count, log-scaled |
| Link width | Number of edges the relationship type stands for |
| Muted styling | A *supporting* type — one that hangs off another in the graph's own type hierarchy |
| `GEO` / `TS` badge | The type declares coordinates or a timeseries |

### Detail tiers

The server asks kglite for a schema sized for the graph it has, and reports
which tier it used on stderr and in `GET /api/session`:

```json
{"protocol_version":4,"tier":"compact","slot_count":98,
 "stats":{"node_count":546850,"edge_count":765373,"node_type_count":98,
          "relationship_type_count":54,"core_type_count":35}}
```

`core_type_count` is the number of types the graph is *about*, as opposed to
the supporting types hanging off them — 35 of 98 in the example. The tier
decides how much per-type detail the schema carries, not what the picture
draws.

## Selection, preview, expansion

Clicking a node does not load anything. It fills the **Selection** panel with
what expanding it *would* add — every relationship that type actually has, in
each direction, with a count:

```console
$ curl -s -XPOST $B/api/preview -H 'content-type: application/json' -d '{"slot":0}'
{"protocol_version":4,"slot":0,"scope":"type","node_type":"Person","title":"",
 "relationships":[
   {"name":"HAS_SKILL","direction":"out","other_type":"Skill","count":180},
   {"name":"KNOWS","direction":"out","other_type":"Person","count":180},
   {"name":"KNOWS","direction":"in","other_type":"Person","count":180},
   {"name":"CONTRIBUTES_TO","direction":"out","other_type":"Project","count":93},
   {"name":"WORKS_AT","direction":"out","other_type":"Company","count":60}],
 "total_edges":693,"max_nodes":5000}
```

Preview before expansion is the whole progressive-disclosure idea in one
interaction: you learn the size of the answer before it costs anything.

Then expand. A type slot loads instances of that type; an instance slot loads
what it is connected to. Naming a relationship is the cheap case; walking every
relationship a type has is the expensive one. `max nodes` in the panel is a
*request* — the ceiling is enforced in
[core](../concepts/bounds-in-core.md), and the answer reports what it cut.

**Collapse** removes an expansion again. Collapsing a selection also clears it,
so the counts the viewer reports never describe nodes that have left the
screen. Slot numbers are not reissued, so anything holding a slot stays valid
unless the answer carries a compaction — which renumbers everything, and says
so.

## Search

Search runs **server-side over the whole loaded graph**, not over what is on
screen. Give it a string, optionally a type and a property:

```bash
curl -s -XPOST $B/api/search -H 'content-type: application/json' \
  -d '{"query":"ada","node_type":"Person","property":"title"}'
```

Every hit comes back with a `slot` — the slot it occupies in the view, or
`null` when that node is not loaded:

```json
{"query":"Person_1","property":"title","node_type":"Person","mode":"contains",
 "hits":[{"node_id":59,"node_type":"Person","label":"Person_1","slot":null},
         {"node_id":68,"node_type":"Person","label":"Person_10","slot":null}]}
```

So a hit that is on screen can be highlighted and one that is not can be
loaded, and neither case is presented as the other. Use **Search** to bring
nodes in and the
{ref}`filter <filter>` to hide ones already loaded — they are opposite
tools and the app refuses to pretend otherwise.

## Inspecting a node

Selecting an instance node fills the panel with its properties, and the type
panel offers **property statistics** for the type as a whole: per property, the
value type, how many nodes carry it, how many distinct values there are, and
either the full value set or a sample.

```json
{"node_type":"Person","node_count":60,"sampled":false,"exact_scan_ceiling":200000,
 "properties":[
   {"name":"city","value_type":"String","non_null":60,"unique":10,
    "values":["City_0","City_1","…"],"approx":false,"role":"categorical"},
   {"name":"age","value_type":"Int64","non_null":60,"unique":33,
    "values":[],"sample":45,"approx":true,"role":"numeric"}]}
```

`approx` and `sampled` are not decoration. Above the exact-scan ceiling the
statistics are estimated, and the app says which numbers are estimates —
the same rule as everywhere else in this program.

Those statistics are also what drives the [appearance](appearance.md) pickers:
a property with ten distinct values over full coverage is a good colour
channel, and one with sixty unique strings is not.

## Read-only, always

The viewer runs queries read-only. A statement that writes is refused before it
runs — `POST /api/validate` reports it as an `error` with the sentence *"this
viewer runs queries read-only — the engine will refuse a statement that
writes"*, and the editor shows it under the query box while you are still
typing.
