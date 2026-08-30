# Query surfaces

Everything on this page ends in the same place: a read-only Cypher query,
executed down the one bounded path, with the text that ran visible on screen.
Two of these surfaces *write* the Cypher for you, and both of them show you
what they wrote before running it. That is the rule — **what is on screen is
what runs** — and it is why there is no "smart mode" checkbox anywhere in this
app.

## The Cypher panel

Write a query, press **Run** or Ctrl/Cmd+Enter. Leave **show in graph**
unticked and the answer is a table; tick it and the nodes and relationships the
query returns are added to the view.

Both go through `POST /api/cypher`, so a `curl` and the panel are the same
request:

```bash
curl -s -XPOST $B/api/cypher -H 'content-type: application/json' \
  -d '{"query":"MATCH (p:Person) RETURN p.title LIMIT 5","params":{},"as_graph":false}'
```

A graph result carries the slice; a table result carries columns, rows,
`{returned, total, truncated}`, `elapsed_ms`, `warnings` and `timed_out`.

The 5,000-row ceiling reaches kglite's executor, so rows above it are never
built — `MATCH (n) RETURN id(n)` over a 546,850-node graph costs about 100 MB
less at its peak than it used to. Nothing about the answer changes: the query
still runs to completion, so an `ORDER BY` result is still the genuine top
5,000 and an aggregate still folds every row. The count beside it is exact —
*showing 5,000 of 546,850*, not an estimate.

`--query-timeout-secs N` (default 30) is the wall-clock ceiling for one query.
A viewer is interactive, so an unbounded query is a hung tab.

### The editor

The query box is CodeMirror 6 with hand-picked extensions. It highlights
keywords, strings, numbers, comments, node labels (`:Wellbore`), relationship
types (`[:DRILLED_IN]`), property reads (`.title`) and parameters (`$ids`) —
with node labels and relationship types in deliberately different colours,
because they are the two halves of the meta-graph.

It **completes from your graph's own schema**. `:` inside a node pattern offers
the node labels; `:` inside brackets offers the relationship types; `alias.`
offers the properties of whatever type that alias was bound to. Nothing is
guessed: labels and relationship types come from the meta-graph the entry
screen already loaded, and a type's properties are fetched once, the first time
you ask for them. An alias the editor cannot bind to a label offers nothing
rather than every property of every type.

And it marks mistakes **before you run anything**.

![A syntax error underlined and named while the query is still being typed](../_static/app-editor.png)

A pause in typing sends the query to kglite's own parser — parsed, never
executed — and what comes back is underlined at the caret kglite reported and
listed under the editor in kglite's own words. It is a real endpoint, so a
`curl` and an agent can ask the same question:

```console
$ curl -s -XPOST $B/api/validate -H "$C" -d '{"query":"MATCH (w:Wellbor) RETURN w"}'
{"protocol_version":4,"diagnostics":[{"severity":"warning","message":"MATCH references unknown node label 'Wellbor' — the graph has no such type, so this pattern returns no rows. Did you mean 'Wellbore'?","line":null,"col":null}]}

$ curl -s -XPOST $B/api/validate -H "$C" -d '{"query":"CREATE (n:Person) RETURN n"}'
{"protocol_version":4,"diagnostics":[{"severity":"error","message":"this viewer runs queries read-only — the engine will refuse a statement that writes","line":null,"col":null}]}
```

`severity` is `error` (it cannot run: a syntax error, or a write this read-only
viewer refuses) or `warning` (it runs and may answer nothing — a mistyped label
is legal Cypher). `line` and `col` are 1-indexed, and `null` when the finding is
about the whole query.

The editor arrives in **its own chunk, fetched after the page is already
usable** (102 KB gzipped, no change to the main bundle). A plain text box is
what you get until it lands, and what you keep if it never does — in which case
the panel says so in one line rather than quietly handing you a worse editor.

### Engine advisories

kglite's non-fatal warnings ride the result and are drawn above the table:

```console
$ curl -s -XPOST $B/api/cypher -H "$C" -d '{"query":"MATCH (n:NoSuchLabel) RETURN n","params":{},"as_graph":false}'
… "bound":{"returned":0,"total":0,"truncated":false},
   "warnings":["MATCH references unknown node label 'NoSuchLabel' — the graph has no such type, so this pattern returns no rows."]
```

Without that line, a `200` with an empty table reads as *"the graph has no such
nodes"* rather than *"you mistyped a label"*.

## Saved queries

**Save** keeps the query under a name; the picker loads it back; a **recent**
list holds the last 20 you ran.

The store is a small JSON file per graph under your config directory, keyed by
the graph's absolute path, with one shared file for graphs handed over as
bytes. Deliberately **not** the browser's storage: an origin includes the port,
and `--port 0` is the documented default, so `localStorage` would hand a
different store to every launch.

Because the store lives beside the session rather than in a handler, everything
sees the same one — `kglite_visual.show()` included, and so does an agent. Two
MCP tools read it: `list_saved_queries` and `run_saved_query`. A saved query is
what the person you are working with already decided was worth keeping, and it
uses their names for things.

```bash
curl -s $B/api/queries
curl -s -XPOST $B/api/queries/save   -H "$C" -d '{"name":"wells","query":"MATCH (w:Wellbore) RETURN w LIMIT 5"}'
curl -s -XPOST $B/api/queries/delete -H "$C" -d '{"name":"wells"}'
curl -s -XPOST $B/api/queries/history -H "$C" -d '{"query":"…"}'
```

Every ceiling is a refusal that names its number — 64 saved queries per graph,
64 KB per query, 256 KB per file, 512 graphs — and nothing is ever deleted on
your behalf. `kglite-visual queries {list,rm,prune}` is the store's owner, and
`prune` only offers the stores whose graph is gone from disk. See
[storage](../concepts/storage.md) and the {ref}`CLI reference <queries>`.

## A generated table

Select a type node and the type panel offers **table of the N on screen**. The
app writes the query, puts it in the Cypher box **where you can read and edit
it**, and runs it down the ordinary bounded path:

```cypher
MATCH (n:Wellbore) WHERE id(n) IN $ids
RETURN id(n) AS id, n.title AS title, n.wlbWell AS wlbWell, …
```

The columns are the twelve properties most of that type's nodes carry. Click a
header to sort — stably, and *by type*, so a numeric column compares as numbers
and a column of ids does not put 100 before 58.

The panel says when the twelve-column cap dropped something, and when a node on
screen carries no `id` field for a query to name it by. That last case is not
hypothetical: **kglite's `id(n)` reads the node's `id` field** — whatever the
source data called its key — not any engine-internal index. A slice therefore
carries each node's `id` field beside its slot, and a node without one cannot
be named in Cypher at all, so it is reported rather than silently missing.

## The path builder

A multi-hop question from dropdowns, shown as Cypher the whole time.

Pick a start type, add up to three hops. The relationship pickers offer **only
the ones the graph actually has** — read out of the meta-graph, with direction
in the label and the edge count beside it — and any node can be narrowed with a
`property is/contains/>/< value` filter.

The generated Cypher sits in a read-only strip under the builder, and one
button copies it into the editor. Values are always bound as parameters; labels,
relationship types and property names are validated as identifiers and refused
rather than quoted.

Each hop carries a `count(*)` preview, so the size of the answer is known
before anything is drawn. When the last hop's preview is past what the server
will return, the card says so **before** the click rather than after the wait.

**Read the hop counts before pressing Run.** Measured on a real graph: a
three-hop path previewing at 1,941,015 rows took the engine past its own
deadline and 7.3 GB of RSS before the OS killed the server. The preview is the
cheap way to learn that; the run is not.

Run sends exactly the query on screen down the ordinary bounded path — no
per-hop bound, one row ceiling, the same banner.

## PROFILE and EXPLAIN

Both are ordinary Cypher prefixes, and that is the entire interface on purpose.
A checkbox that prepended a keyword behind your back would mean the query in
the editor was not the query that ran.

### PROFILE — what each clause cost

```console
$ curl -s -XPOST $B/api/cypher -H "$C" \
    -d '{"query":"PROFILE MATCH (p:Person) WHERE p.age > 30 RETURN p.title LIMIT 5","params":{},"as_graph":false}'
… "profile":[
     {"clause_name":"Match :Person + Where (fused)","rows_in":0,"rows_out":5,"elapsed_us":208},
     {"clause_name":"Return","rows_in":5,"rows_out":5,"elapsed_us":20}]
```

The panel draws one row per clause above the results: the engine's own clause
name, rows in → rows out, a bar scaled against the slowest clause, and
microseconds. The query actually runs, so the results table is there beside the
profile.

### EXPLAIN — the plan, not the answer

`EXPLAIN` has always returned rows; the panel used to draw them as three
columns of data, and a `step` column counting 1..n beside an `operation` column
is a numbered list wearing a grid.

```console
$ curl -s -XPOST $B/api/cypher -H "$C" \
    -d '{"query":"EXPLAIN MATCH (p:Person)-[:KNOWS]->(q) RETURN q.title","params":{},"as_graph":false}'
"columns":["step","operation","estimated_rows"],
"data":[[1,2,3],
        ["Match :Person","Return","OptimizerPass mark_disjoint_fixed_trails"],
        [60,null,null]],
"explain":true
```

It now gets its own monospace treatment: the step in the gutter, the operation
indented, the estimate on the right where the planner produced one and blank
where it did not. The status line says **not executed**, because the query was
planned rather than run.
