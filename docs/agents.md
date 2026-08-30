# Agents and MCP

The running server speaks the Model Context Protocol at `/mcp`. There is no
second process to start, no discovery file to write and nothing to install:
attaching an agent is pointing it at a URL that the launch line already printed.

The point of the feature is not "an agent can query a graph" — the graph's own
[kglite MCP server](https://kglite.readthedocs.io/en/latest/operators/mcp-server.html)
does that better. The point is that **the agent and the human are looking at
the same screen.** Whoever changes the view — you, an agent, a `curl` — every
connected window sees the change immediately. Watching an agent expand a type
and zoom to what it found is the feature, not a side effect of it.

This page is written for someone wiring that up.

## The launch contract

```bash
kglite-visual graph.kgl --no-open --port 0 &
```

`--no-open` is mandatory for anything unattended; `--port 0` means OS-assigned.
The server binds `127.0.0.1` only.

**Exactly one line on stdout, and it is JSON:**

```json
{"url":"http://127.0.0.1:54137/","port":54137,"pid":69850,"graph":"/path/graph.kgl","mcp":"http://127.0.0.1:54137/mcp"}
```

Parse it. Never scrape stderr, never race a hardcoded port. All diagnostics are
on stderr, and the error path is **exit 1 with empty stdout** and one line on
stderr — so a harness that read a line has a server.

```bash
BIN=kglite-visual
"$BIN" graph.kgl --no-open --port 0 > server.json 2> server.err &
B=$(python3 -c 'import json;print(json.load(open("server.json"))["url"].rstrip("/"))')
M=$(python3 -c 'import json;print(json.load(open("server.json"))["mcp"])')
```

From Python, the same five keys come back as a dict instead:

```python
view = kglite_visual.show("graph.kgl", open_browser=False)
view.launch_info    # {'url', 'port', 'pid', 'graph', 'mcp'}
view.close()
```

The wheel writes nothing to stdout — the single-line contract is the CLI's, and
a library that printed there would corrupt its caller's output.

## The JSON twin

Every request the WebSocket carries has a named HTTP route with the same
structs. A `curl` line therefore says what it asks for, and divergence between
the twin and the wire is a bug rather than a nuance.

```bash
C='content-type: application/json'

curl -s $B/api/session      # protocol_version, tier, counts, bounds
curl -s $B/api/meta-graph   # slots, edges, positions, bounds
curl -s $B/api/describe     # schema tiers, per-type detail
curl -s $B/api/view-state   # what is on screen right now
```

The mutating half is `POST` with a JSON body. `POST` rather than `GET` because
several of them mutate the slot space, and a `GET` that appended slots would be
re-run by any cache in the path.

```bash
curl -s -XPOST $B/api/preview        -H "$C" -d '{"slot":0}'
curl -s -XPOST $B/api/expand         -H "$C" -d '{"slot":0,"relationship":"KNOWS","direction":"out","limit":40}'
curl -s -XPOST $B/api/collapse       -H "$C" -d '{"slot":0}'
curl -s -XPOST $B/api/node           -H "$C" -d '{"slot":5}'
curl -s -XPOST $B/api/cypher         -H "$C" -d '{"query":"MATCH (n) RETURN n LIMIT 3","params":{},"as_graph":false}'
curl -s -XPOST $B/api/search         -H "$C" -d '{"query":"ada","node_type":"Person","property":"title"}'
curl -s -XPOST $B/api/property-stats -H "$C" -d '{"node_type":"Person"}'
curl -s -XPOST $B/api/validate       -H "$C" -d '{"query":"MATCH (n:Persn) RETRN n"}'
curl -s -XPOST $B/api/layout         -H "$C" -d '{"kernel":"islands"}'
curl -s -XPOST $B/api/render         -H "$C" -d '{"source":{"type":"meta"},"format":"png"}' -o m.png

# steering — these three answer with {"clients":n}
curl -s -XPOST $B/api/focus      -H "$C" -d '{"slots":[3,4,5]}'
curl -s -XPOST $B/api/highlight  -H "$C" -d '{"slots":[3],"concept":"selected"}'
curl -s -XPOST $B/api/appearance -H "$C" -d '{"color_by":"city","size_by":"age"}'
# reset takes no body and answers with the slice it collapsed back to
curl -s -XPOST $B/api/reset

# the saved-query store
curl -s       $B/api/queries
curl -s -XPOST $B/api/queries/save    -H "$C" -d '{"name":"wells","query":"MATCH (w:Wellbore) RETURN w LIMIT 5"}'
curl -s -XPOST $B/api/queries/delete  -H "$C" -d '{"name":"wells"}'
curl -s -XPOST $B/api/queries/history -H "$C" -d '{"query":"…"}'

# the one GET in the vocabulary: a download is an <a href download>, an anchor
# issues a GET, and this route reads the view and mutates nothing.
curl -sD- "$B/api/export?format=graphml&source=live-view" -o view.graphml
```

Three rules hold across all of it:

- **Every bounded response carries `{returned, total, truncated}`.** Report
  those numbers; do not hide them. A graph slice carries *two* — `meta.bound`
  for nodes and `meta.link_bound` for links — because nodes and links share one
  byte budget, so the node list can be complete while the link list is not.
- **A bad request is a `400` that names what it refused.**
- **A query the engine rejected is a `422` carrying kglite's own diagnostic
  verbatim.** Quote it; do not summarise it.

The steering endpoints — `focus`, `highlight`, `appearance` — answer
`{"clients":n}`, because a command that reached nobody is otherwise
indistinguishable from one that reached the user. Every MCP steering tool
carries the same number as `connected_viewers`.

## MCP at `/mcp`

Streamable HTTP, mounted as one more route on the same axum router that serves
the frontend, the JSON twin and the WebSocket.

```bash
M="$(python3 -c 'import json;print(json.load(open("server.json"))["mcp"])')"
H=(-H 'content-type: application/json' -H 'accept: application/json, text/event-stream')

# initialize first; keep the mcp-session-id header it returns, on every later call
S=$(curl -sD - -XPOST "$M" "${H[@]}" -o /dev/null \
     -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"probe","version":"0"}}}' \
   | grep -i '^mcp-session-id' | tr -d '\r' | cut -d' ' -f2)

curl -s -XPOST "$M" "${H[@]}" -H "mcp-session-id: $S" \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized"}'

curl -s -XPOST "$M" "${H[@]}" -H "mcp-session-id: $S" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
```

Responses default to `text/event-stream`. The JSON-RPC payload is a `data:`
line — but not necessarily the last one, since the stream opens with an empty
`data:` and a `retry:`. Parse it rather than tailing it:

```bash
mcp() {  # mcp <tool> <json-args>
  curl -s -XPOST "$M" "${H[@]}" -H "mcp-session-id: $S" \
    -d "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}" \
  | python3 -c 'import json,sys
for line in sys.stdin:
    line = line.strip()
    if line.startswith("data: ") and len(line) > 6:
        d = json.loads(line[6:])
        for c in (d.get("result", {}).get("content") or []):
            print(c.get("text", ""))'
}

mcp view_state '{}'
mcp show_cypher '{"query":"MATCH (f:Field) RETURN f"}'
mcp set_layout '{"kernel":"geo"}'
```

Errors an agent can act on come back as `isError: true` with kglite's own
message. Quote it; do not summarise it.

## The thirteen tools

Ten are verbs about the screen. The two saved-query tools are the exception
that proves the rule — they read a store belonging to *this* window and to the
human who filled it, which is not a fact any other server has — and
`export_view` is the thirteenth, which takes what is on the screen out of the
screen.

| Tool | What it does |
|---|---|
| `view_state` | What is on the shared screen right now: the slot space, type nodes and their drill-in state, instance counts by type, tombstones, what the response bound did to the last change, and `connected_viewers`. Read it before acting, and after anything surprising |
| `show_cypher` | Run read-only Cypher and put the resulting nodes and relationships **into** the shared view. Bounded in core. A display verb — to read a table, ask the graph's own MCP server |
| `expand` | Load a slot's neighbours. A type slot loads instances; an instance slot loads what it is connected to. `limit` is a request, not a guarantee |
| `collapse` | Remove a slot's expansion. Slot numbers are not reissued unless the answer carries a compaction, which renumbers everything and says so |
| `highlight` | Make things stand out. Name `slots`, or give a `search` string and let the server find them — hits already loaded are marked, hits that are not are counted back. `concept` is `highlighted` (a result set) or `selected` (the one thing you are talking about) |
| `focus` | Zoom the human's camera to frame these slots — the honest way to say "look at this". An empty list frames the whole view. Changes nothing about what is loaded |
| `set_appearance` | Drive `color_by` / `size_by` from node properties. Omitting a field clears that channel back to the structural encoding |
| `set_layout` | Re-arrange the view with a layout computed **here**, and hold it still. See [what an agent may claim](#what-an-agent-may-claim) |
| `reset_view` | Collapse everything back to the entry screen. Destructive to the human's place in the graph — prefer `collapse` on what *you* added |
| `render` | Draw an image. `target: live-view` (default), `meta` or `cypher`. Geometry differs from their screen — always |
| `list_saved_queries` | The Cypher this user saved for this graph, plus recently run queries. **Read it before writing a query of your own** |
| `run_saved_query` | Run one by name, into the shared view. Same path, same bound; added to the user's recent list, because they are watching it happen |
| `export_view` | Write the nodes currently in the view out as GraphML / GEXF / CSV / D3 JSON and hand back the text |

### The shared-view model

**One view, two callers, last writer wins.** The human and the agent are
collaborators on one slot space, not two tenants of two. Anything either of
them changes is broadcast to both, and neither is *notified* that the other did
something — they see it.

The consequences for an agent are concrete:

- `view_state` is the server-side twin of the browser's `window.__kglv`.
  **Re-read it** rather than assuming your last call still describes the
  screen.
- `reset_view` throws away the human's place in the graph. Collapse what you
  added instead.
- Every steering answer carries `connected_viewers`. Zero means you are talking
  to yourself.

### `export_view` takes the view, not the graph

Its scope is the instance nodes on the human's screen. On an empty view it
refuses by name rather than dumping the graph:

```json
{"error":"there is nothing to export: no instance nodes are loaded. Expand a type or run a query with 'show in graph' first."}
```

So `expand` or `show_cypher` what you want first, check `view_state`, then
export — and **read the `notes` in the reply** before telling the user what
they have:

```json
{"bytes":27033,"filename":"graph-view.gexf","format":"gexf","nodes":144,
 "notes":["the nodes are exactly the ones selected; the edges are every edge this graph holds between them, which can be MORE than you saw …"]}
```

The two caveats are in [export](export.md#the-two-caveats).

### Saved queries first

`list_saved_queries` returns what this user decided was worth keeping, under
the names *they* chose, plus the last 20 queries run from the panel and the cap
on that list. Reading it before writing your own Cypher over a schema you have
just met is the difference between joining someone's session and starting your
own next to it.

(what-an-agent-may-claim)=
## What an agent may claim

This is the part that is easy to get wrong and expensive when you do.

**Content is knowable. Geometry depends on the layout.**

While `view_state.layout_kernel` reads `simulation` — the default every session
opens in — the layout runs on the viewer's GPU and the server never learns
where the points ended up:

> The live layout runs on the viewer's GPU and the server does not know where
> the points ended up (`layout_kernel` is `simulation`). A render of this view
> is content-identical and geometry-different: same nodes, same links, same
> truncation, a different arrangement. Describe what is in the view, never
> where it is on the user's screen — or ask for a static layout, after which
> the arrangement is this server's own and can be described.

Use `focus` and `highlight` to say "look at this" instead of naming a position
you cannot see.

`set_layout` with a static kernel (`auto`, `radial`, `islands`, `force`, `geo`)
computes the arrangement here and broadcasts it. The viewer's simulation stops,
dragging is disabled, and the caveat changes:

```console
$ mcp set_layout '{"kernel":"geo"}'
{"connected_viewers":0,"geometry_caveat":"This view is under a static layout THIS
 SERVER computed (`layout_kernel` names the kernel): the viewer's simulation is off,
 dragging is disabled, and nothing moves a point until the next layout request. So
 the arrangement on their screen is the one that was sent, and relative position is
 safe to describe — 'the ring around X', 'the island on the left'. Their camera is
 still their own, so never name a screen coordinate; and `render` lays out
 independently (it folds fans and separates circles for the page it draws), so its
 picture may still differ from what they see.",
 "kernel_chosen":"geo","kernel_requested":"geo","layout_ms":0.13,"seed_slot":null,
 "slots_placed":242}
```

**Read `geometry_caveat` from the answer rather than remembering which mode you
are in.** Core owns both wordings and picks by `layout_kernel`; nothing else
writes its own.

Three rules survive every mode:

1. **The camera is always theirs.** They zoom and pan freely, so a *screen*
   coordinate is never a claim you can make — only a relative one, and only
   under a static kernel.
2. **`kernel_chosen` can differ from `kernel_requested`.** `islands` over a
   graph with no community structure falls back to `force` and says so.
3. **`render` is a separate pass either way.** It has its own fold and its own
   separation, so `render{target:"live-view"}` can differ from the canvas *even
   under a static kernel*. Describe what is in the picture, never where it sits.

Under `geo` the arrangement genuinely *is* geographic, so "this node is in the
Barents Sea" is a claim the picture supports. "Top left of your screen" still
is not.

## `window.__kglv`

The browser's own state hook, for a driver in a real browser (Playwright,
Claude in Chrome, a console). Readiness is `window.__kglv.ready === true` —
**never a fixed sleep**: cosmos.gl v3 is async-init and draws zero frames when
static.

```json
{"protocolVersion":4,"tier":"compact","layoutMode":"force","layoutKernel":"simulation",
 "pointCount":98,"linkCount":124,"slotCount":98,"tombstoneCount":0,"namedSlots":98,
 "ready":true,"simRunning":true,"lastMessageSeq":2,"positionsHash":"80499c25",
 "deviceFeatures":{"webgl2":true,"float32Renderable":true,"textureBlendFloat":true},
 "lastSliceKind":"sync","compactions":0,
 "truncation":{"returned":0,"total":0,"truncated":false,"banner":null},
 "zoomLevel":0.42,"focusedSlots":[],"colorBy":null,"sizeBy":null,"hoveredSlot":null,
 "emphasizedCount":0,"highlightedCount":0,"selectedCount":0,"previewRows":0,
 "queryRows":0,"searchHits":0,"legendEntries":4,"exportNodes":0,"filteredOut":0,
 "appearanceCandidates":0,"approximateStats":0,"error":null}
```

Two of these are **honest pairs**, and neither half is honest alone:

`pointCount` + `filteredOut`
: `pointCount` is live points **excluding whatever the client-side filter is
  hiding**; `filteredOut` is that count. `slotCount` includes tombstones.

`namedSlots` + `slotCount`
: A client holds a *position* for every slot it was told about and an
  *identity* only for the ones whose `SliceNode` it received. These are unequal
  on any browser that joined mid-session, until the connect-time resync.

The rest, briefly: `layoutMode` is `force` / `deterministic` / `static` and
`layoutKernel` names the arrangement in force; `positionsHash` only means
anything where nothing is moving the points; `truncation` carries the banner
text the user is actually reading, so an assertion can check the *words* rather
than a boolean beside them; `exportNodes` is what the Export card would write;
`error` non-null explains any `ready:false`.

**Assert on state; screenshots are artifacts.**

A second hook, `window.__kglvBench`, carries exactly two fields a benchmark
harness cannot get from outside: `graph` (the live cosmos.gl instance) and
`firstDataFrameMs`. It ships in the production bundle on purpose — a hook
compiled out of the build being measured measures a different build.

## Joining a session in progress

A browser that attaches to a view an agent has already drilled into is greeted
with **the whole current view** — every live node named, holes marked,
positions from slot zero, and the static arrangement in force if there is one.

That was not always true, and the failure was ugly: every client used to be
greeted with the entry screen, so the next change arrived indexing slots that
browser had never been told about, and the points appeared with no label, no id
and nothing to click.

## Stopping

Kill the `pid` from the launch line. `kill -TERM` (the default) is caught: the
server shuts down, exits **0**, releases the port and removes kglite's
temporary spill from `$TMPDIR` — 370 MB for a half-million-node graph.
`kill -9` skips all of it and leaves the spill behind. Ctrl-C is the same
handler.

Verify the port is released before relaunching on a fixed port.

## A note on scope

These tools navigate a graph. Querying one — schema, Cypher reference, result
formatting, CSV export of a large answer — is
[kglite's own MCP server](https://kglite.readthedocs.io/en/latest/operators/mcp-server.html)'s
job, and keeping the surface here small is what stops this becoming a worse
copy of it, one tool at a time. Run both; they are complementary.
