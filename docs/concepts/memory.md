# Memory

A viewer for graph files loads graph files. This page is the honest accounting,
because "it uses some memory" is not something anyone can plan against.

## What a graph costs

Measured on a 133 MB `.kgl` holding 546,850 nodes, 765,373 edges and 98 node
types:

| | |
|---|---|
| Load time | ~0.8 s |
| Resident after load | ~627 MB |
| Resident after a session that also served several renders | 737 MB *(measured)* |
| kglite's temporary spill in `$TMPDIR` | 370 MB |

The spill is not optional and not this project's: kglite writes any column of
256 KB or more to `$TMPDIR` while decoding. Both `show(path)` and
`show(graph)` need a writable temporary directory.

## Handing a graph over costs twice

`kglite_visual.show(graph)` on an in-memory kglite `KnowledgeGraph` costs
roughly **2× the graph's size** at the moment of the call: `to_bytes()`
materialises a complete `.kgl` image in the Python process, and this wheel
decodes a second, independent copy inside its own extension module.

That is not an optimisation waiting to happen. Two extension modules cannot
share a graph handle, and a serialised image is the only sound handover.
`show(path)` reads the file directly and pays once — so on a large graph, pass
the path.

## The load ceiling

`--max-load-mb N` (and `show(..., max_load_mb=N)`) refuses a graph estimated to
cost more than N megabytes, in hundredths of a second, before anything is
decompressed. Measured: **0.026 s** to refuse a file that takes 0.8 s to load.

The case it exists for is a 16 GB machine that would otherwise spend twenty
minutes in swap. The estimate is deliberately conservative and can refuse a
graph that would have fitted, which makes it a **guard rather than a budget** —
set it where a failure is what you want.

Full detail: {ref}`the load ceiling <the-load-ceiling>`.

## Render against a running server, not in a loop

Each `kglite-visual render` invocation loads the whole graph fresh. A script
that renders twenty images of one graph pays that twenty times.

If a server is already up, `POST /api/render` reuses the graph already in
memory and answers with the image bytes. It renders against a **private
session** over the same read-only graph, so asking for an image never moves
what the user is looking at.

## The path builder is where a query gets expensive

The bound protects the *response*, not the query's own execution. A query the
engine has to run through before it can return five rows still costs what it
costs.

Measured on a real graph: a three-hop path previewing at **1,941,015 rows**
took the engine past its own deadline and **7.3 GB of RSS** before the OS
killed the server.

That is exactly why every hop in the [path builder](../viewer/queries.md#the-path-builder)
carries a `count(*)` preview, and why the card warns before the click rather
than after the wait. Read the counts.

## Shutting down cleanly matters

`SIGTERM` — which is what `kill` sends by default — is caught. The server shuts
down, exits **0**, releases the port, and removes kglite's temporary spill.
Ctrl-C is the same handler, and so is `Server.close()` from Python.

`kill -9` skips all of it and leaves 370 MB behind. Nothing inside a process
can prevent that.
