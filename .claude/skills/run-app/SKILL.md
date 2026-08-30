---
name: run-app
description: Launch, drive, inspect, and stop the kglite-visual app — the agent-operability contract in executable form. Use when a change needs verifying in the real running app (not just tests), when asked to run or screenshot the app, or before claiming any user-visible behaviour works.
---

# run-app

How an agent operates the real app. The contract these steps rely on shipped
in P1/P2 (launch contract, JSON twin, `window.__kglv`); if a step below
doesn't match reality, that is a defect in the app or this skill — fix it,
don't improvise around it (`R17`: this file is a claim).

## 1. Build + resolve the binary (never hard-code a path)

```bash
cd frontend && npm run build && cd ..     # embed-input first: dist is compiled in
cargo build -p kglite-visual-cli
BIN=$(python3 scripts/check_bundle.py --resolve-binary kglite-visual)   # newest-of-profile,
                                                          # refuses stale bundle
```

`check_bundle.py --resolve-binary` fails rather than hand you a binary older
than the bundle it should embed — a stale bundle inside a fresh binary looks
exactly like a backend bug (CLAUDE.md → "Two toolchains, one gate").

## 2. Launch (agent mode)

```bash
"$BIN" crates/kglite-visual-core/tests/fixtures/meta.kgl --no-open --port 0 &
```

- Exactly **one line on stdout**, JSON: `{"url","port","pid","graph","mcp"}`.
  Parse it; never scrape stderr, never race a hardcoded port.
- All diagnostics are on stderr.
- `--no-open` is mandatory for agents/CI (no browser spawn); `--port 0`
  means OS-assigned. Server binds 127.0.0.1 only.
- Error path: bad file → exit 1, **empty stdout**, one stderr line.

### From Python (the wheel)

```python
import kglite_visual as kv
view = kv.show("crates/kglite-visual-core/tests/fixtures/meta.kgl", open_browser=False)
view.launch_info   # {"url","port","pid","graph","mcp"} — the same five keys, same struct
view.close()       # "closed" | "already-closed" | "stale-after-fork"; frees the port
```

`show()` also takes a `bytes` `.kgl` image or any object with `to_bytes()`
(kglite's `KnowledgeGraph`); the object route costs ~2× the graph's size, so
`show(path)` is the large-graph answer. **The wheel writes nothing to
stdout** — the single-JSON-line contract is the CLI's, and a library that
printed there would corrupt its caller's output; read `launch_info` instead.
`open_browser` defaults to auto (quiet in a notebook kernel, a tab
elsewhere). In a notebook the object renders itself: a proxy-prefixed iframe
if `jupyter-server-proxy` is importable, **no iframe at all** but the URL
and an `ssh -N -L` hint if the kernel looks remote, localhost otherwise — a
localhost iframe from a remote kernel is a silently blank frame.
Build/refresh it with `make py-develop`.

## 3. Inspect without a browser — the JSON twin

```bash
curl -s http://127.0.0.1:$PORT/api/session     # protocol_version, tier, counts, bounds
curl -s http://127.0.0.1:$PORT/api/meta-graph  # slots, edges, positions, bounds
curl -s http://127.0.0.1:$PORT/api/describe    # schema tiers, per-type detail
```

The request vocabulary is POST with a JSON body — the same structs the
WebSocket carries, one named route each so a `curl` line says what it asks for.
POST rather than GET because several of them mutate the slot space, and a GET
that appends slots would be re-run by any cache in the path.

```bash
C='content-type: application/json'
curl -s -XPOST $B/api/preview        -H "$C" -d '{"slot":0}'
curl -s -XPOST $B/api/expand         -H "$C" -d '{"slot":0,"relationship":"KNOWS","direction":"out","limit":40}'
curl -s -XPOST $B/api/collapse       -H "$C" -d '{"slot":0}'
curl -s -XPOST $B/api/node           -H "$C" -d '{"slot":5}'
curl -s -XPOST $B/api/cypher         -H "$C" -d '{"query":"MATCH (n) RETURN n LIMIT 3","params":{},"as_graph":false}'
curl -s -XPOST $B/api/search         -H "$C" -d '{"query":"ada","node_type":"Person","property":"title"}'
curl -s -XPOST $B/api/property-stats -H "$C" -d '{"node_type":"Person"}'
curl -s -XPOST $B/api/validate       -H "$C" -d '{"query":"MATCH (n:Persn) RETRN n"}'
curl -s -XPOST $B/api/layout         -H "$C" -d '{"kernel":"islands"}'
```

Same structs as the binary WebSocket protocol — divergence between the twin
and the wire is a bug, not a nuance. Every bounded response carries
`{returned, total, truncated}`; report those numbers, don't hide them. A
**graph slice carries two**: `meta.bound` for its nodes and
`meta.link_bound` for its links, because nodes and links share one byte
budget in core — the node list can be complete while the link list is not,
and a slice that reported only the first would let a partial neighbourhood
read as a whole one. A bad
request is **400** and names what it refused; a query the engine rejected is
**422** and carries kglite's own diagnostic verbatim — quote it, don't
summarise it.

**`/api/validate` is the one endpoint that answers about a query without
running it.** It parses through kglite's own parser — no graph argument,
nothing executed — and answers `{"protocol_version",
"diagnostics":[{"severity","message","line","col"}]}`. `severity` is
`"error"` (it cannot run: a syntax error, or a write this read-only viewer
refuses) or `"warning"` (it runs and may answer nothing: an unknown label or
relationship type, carrying kglite's "did you mean?"). `line`/`col` are
1-indexed and `null` when the finding is about the whole query. It moves
nothing and broadcasts nothing.

**`/api/layout` is the one endpoint that changes what the picture LOOKS like
without changing what is in it.** `{"kernel": "auto"|"radial"|"islands"|
"force"|"simulation", "seed_slot": n}`. It allocates no slot, tombstones
nothing and touches no link; it computes an arrangement server-side and
broadcasts it to every attached browser, which then holds it still — the
simulation stops and dragging is disabled. `"simulation"` hands the layout
back to the viewer's GPU. The answer names `kernel_chosen`, which can differ
from what was asked: `islands` over a graph with no community structure
falls back to `force` and says so. `"geo"` is in the vocabulary and refused,
by name, until the geo kernel lands.

Saved queries live in a file store keyed by the graph's absolute path
(`$KGLITE_VISUAL_CONFIG_DIR` overrides the config dir). Every face reads the
same one, `show()` included.

    curl -s   $B/api/queries
    curl -s -XPOST $B/api/queries/save    -H "$C" -d '{"name":"wells","query":"MATCH (w:Wellbore) RETURN w LIMIT 5"}'
    curl -s -XPOST $B/api/queries/delete  -H "$C" -d '{"name":"wells"}'
    curl -s -XPOST $B/api/queries/history -H "$C" -d '{"query":"…"}'

Ceilings are refusals naming their number (400), never truncations. The store
is a durable tier: `make prune` does not know it exists — `kglite-visual
queries {list,rm,prune}` is the owner, and `prune` only removes stores whose
graph is gone from disk. **Point `KGLITE_VISUAL_CONFIG_DIR` at a tempdir in
any harness**, or it reads and writes the developer's own saved queries.

`--query-timeout-secs N` (default 30) raises the wall-clock ceiling for one
Cypher query. Leave it alone unless a deliberate analytical query needs it.

**The query box is CodeMirror, in its own chunk.** The panel paints a
`<textarea>` and swaps it for a CodeMirror view when `editor-*.js` arrives,
so a driver waits for the swap to *settle* — `[data-testid="query-editor"]
.cm-content` exists, or `[data-testid="editor-note"].kglv-warn` says the
chunk did not load — and then fills whichever is there.
`frontend/tests/e2e/harness.ts` exports `fillQuery` / `queryText` for
exactly this. There is no `[data-testid="query-input"]` on a page where the
editor loaded.

## 3b. Get a picture — no browser, no server

    "$BIN" render crates/kglite-visual-core/tests/fixtures/meta.kgl --meta -o /tmp/m.svg
    # {"out":"/tmp/m.svg","format":"svg","width":2000,"height":1250,"nodes":5,
    #  "links":7,"folded":0,"layout_ms":0.06,"truncated":false,"banners":[],
    #  "bytes":3867}

Same stdout rule as the server: exactly one JSON line, diagnostics on stderr,
**nothing on stdout when it fails** — so a harness that read a line got a
render. Sources are mutually exclusive: `--meta` (default), `--cypher "…"`,
`--expand type=T rel=R dir=out`. `--format png`, `--theme light`,
`--width/--height`, `--seed`, `--limit`.

Four fields appear only when they have something to say, so a key that is
present always carries a number: `folded` (nodes drawn as one wedge rather
than individually), and `types_shown` / `types_total` / `names_shown`. The
last three are the render's own honesty: a meta-graph on a canvas too small
for the schema draws the largest types it can hold and says so —

    "$BIN" render sodir.kgl --meta --width 800 --height 500 -o /tmp/s.svg
    # {…,"nodes":24,"links":20,"types_shown":24,"types_total":98,
    #  "names_shown":17,…}

— and the same two lines are drawn into the picture. 1600x1000 holds sodir's
98 types and reports none of this.

The layout is seeded and deterministic — same request, same bytes, forever —
so `--seed` is how you get a *different* arrangement of the same data, and
`make check-render-baseline` is an exact baseline over it.

Against a running server, the same thing over HTTP, answering with bytes:

    curl -s -XPOST $B/api/render -H "$C" \
      -d '{"source":{"type":"meta"},"format":"png","width":900,"height":600}' -o /tmp/m.png
    # content-type: image/png, plus x-kglv-{nodes,links,truncated,banner}

**It does not move the live view.** `core::render` opens a private session
over the same read-only graph, so an image request is a question and never
changes what the user is looking at. Rendering the *user's actual* geometry
is P10 + the deferred client-position capture, not this.

**What the image is not:** content-identical to the app,
geometry-different. The browser's layout runs on the user's GPU; this is
one of four structure-chosen kernels in core — hop rings, packed islands,
a geographic map when the nodes carry coordinates, or a seeded
Fruchterman-Reingold when the input has no shape to find. Same
nodes, same links, same truncation, a different arrangement — never claim
"your screen shows X at the top left". That holds even when the live view is
under a static kernel (`/api/layout`): this pass folds fans and separates
circles for the page it draws, and the live layout deliberately does
neither.

## 3c. Drive the live view over MCP

The running server speaks MCP at the `mcp` URL its stdout line printed —
streamable HTTP, no second process, no discovery file. Twelve tools:
`view_state`, `show_cypher`, `expand`, `collapse`, `highlight`, `focus`,
`set_appearance`, `set_layout`, `reset_view`, `render`,
`list_saved_queries`, `run_saved_query`.

```bash
M="$(python3 -c 'import json,sys;print(json.load(sys.stdin)["mcp"])' < server.json)"
H=(-H 'content-type: application/json' -H 'accept: application/json, text/event-stream')
# initialize first; keep the mcp-session-id header it returns on every later call.
curl -sD- -XPOST "$M" "${H[@]}" -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"probe","version":"0"}}}'
curl -s   -XPOST "$M" "${H[@]}" -H "mcp-session-id: $S" -d '{"jsonrpc":"2.0","method":"notifications/initialized"}'
curl -s   -XPOST "$M" "${H[@]}" -H "mcp-session-id: $S" -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
```

Responses default to `text/event-stream`; the JSON-RPC payload is the last
`data:` line. Errors an agent can act on come back as `isError: true` with
kglite's own message — quote it, don't summarise it.

**The view is shared, and it is last-writer-wins.** Anything that changes it
is pushed to every attached browser, whoever asked. `view_state` is the
server-side `window.__kglv`; re-read it rather than assuming your last call
still describes the screen.

**Whether the user's geometry is knowable depends on
`view_state.layout_kernel`** (protocol v4). While it reads `simulation` —
the default every session opens in — the layout runs on the viewer's GPU
and the server never learns the final positions: describe what is in the
view, and use `focus` and `highlight` to say "look at this" rather than
naming a screen position you cannot see. `set_layout` with a static kernel
(`auto`, `radial`, `islands`, `force`) computes the arrangement here and
broadcasts it; the viewer's simulation stops, dragging is disabled, and
relative position — "the ring around X", "the island on the left" — becomes
safe to describe. Their camera is still theirs, so a screen coordinate
never is. One carve-out: under the `geo` kernel the arrangement IS
geographic, so "this node is in the Barents Sea" is a claim the picture
supports — "top left of your screen" still is not. Read the `geometry_caveat` that comes back rather than
remembering which mode you are in; core owns both wordings.

**`render` is a separate pass either way.** It has its own fold and its own
separation, so `render{target:"live-view"}` can differ from the canvas even
under a static kernel.

The same verbs exist on the JSON twin — `POST /api/{focus,highlight,appearance,reset}`,
`GET /api/view-state` — which is the `curl`-shaped way to drive the same
broadcast without an MCP client. The steering endpoints answer with
`{"clients":n}`: a command that reached nobody is otherwise indistinguishable
from one that reached the user.

## 4. Drive the real frontend

- Scripted: `make e2e` (Playwright, headless Chromium with
  `--use-gl=angle --use-angle=swiftshader --enable-unsafe-swiftshader`).
  Readiness is `window.__kglv.ready === true` — **never a fixed sleep**
  (cosmos.gl v3 is async-init and draws zero frames when static).
- Interactive (Claude in Chrome / a headed browser): open the reported URL,
  then read `window.__kglv` — `{protocolVersion, tier, layoutMode,
  layoutKernel, pointCount, linkCount, slotCount, tombstoneCount, ready,
  simRunning, lastMessageSeq, positionsHash, deviceFeatures, lastSliceKind,
  compactions, truncation, zoomLevel, focusedSlots, colorBy, sizeBy,
  hoveredSlot, emphasizedCount, highlightedCount, selectedCount,
  previewRows, queryRows, searchHits, legendEntries, filteredOut,
  appearanceCandidates, approximateStats, error}`. `pointCount` is *live*
  points **and excludes whatever the client-side filter is hiding** —
  `filteredOut` is that count, and the two together are the honest pair.
  `slotCount` includes tombstones. `layoutMode` is `force` /
  `deterministic` / `static` and `layoutKernel` names the arrangement in
  force; `positionsHash` only means something where nothing is moving the
  points. `truncation` carries the banner text the user is
  actually reading, so an assertion checks the words rather than a boolean
  beside them. Assert on state; screenshots are artifacts. `error` non-null
  explains any `ready:false`.
- A second hook, `window.__kglvBench`, carries exactly two fields the bench
  harness cannot get from outside: `graph` (the live cosmos.gl instance) and
  `firstDataFrameMs` (navigation start to the first composited frame with
  data). It ships in the production bundle on purpose — a hook compiled out
  of the build being measured measures a different build. Do not grow it;
  everything else a bench needs is expressible from `page.evaluate` on top
  of `graph`.

## 5. Stop

Kill the `pid` from the stdout JSON line. Verify the port is released before
relaunching on a fixed port.

`kill -TERM` (the default) is caught: the server shuts down, exits **0**,
releases the port, and removes kglite's temporary spill from `$TMPDIR` —
370 MB for a half-million-node graph. `kill -9` skips all of it and leaves
the spill behind. Ctrl-C is the same handler.

## Report shape

Paste observed output — the stdout line, curl payloads (trimmed), `__kglv`
dump — not summaries of it. "The tests pass" without "I ran it and here is
what it printed" is an incomplete report (`R2`).
