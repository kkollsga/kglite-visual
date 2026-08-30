# Storage and the config directory

This program writes exactly one durable thing: your saved queries.

## Where

A small JSON file per graph, under the platform config directory:

| Platform | Path |
|---|---|
| macOS | `~/Library/Application Support/kglite-visual/queries/` |
| Linux | `$XDG_CONFIG_HOME/kglite-visual/queries/`, else `~/.config/kglite-visual/queries/` |
| Windows | `%APPDATA%\kglite-visual\queries\` |

`KGLITE_VISUAL_CONFIG_DIR` overrides the config directory outright. **Point it
at a temporary directory in any test harness**, or the harness reads and writes
the developer's own saved queries.

If no config directory can be resolved at all — a headless container with no
`HOME` — the store is simply disabled. That is not a startup error: serving
graphs is the job, and every *operation* on a disabled store refuses and says
why.

## Keyed by the graph's absolute path

Each store file is named from a hash of the graph's absolute path. Graphs
handed over as bytes — `show(graph)`, `show(b"…")` — have no path to key on and
share one file, `_unbound.json`.

The discriminator is **existence**, checked once when the store opens: a launch
string that resolves to a file on disk is a graph with an identity that
survives the process, and anything else is not.

## Why not `localStorage`

An origin includes the port, and `--port 0` is the documented default. Browser
storage would therefore hand a different store to every launch — you would save
a query, restart the viewer, and find it gone because the OS picked a different
free port.

Keeping the store beside the session rather than in a handler has a second
consequence, and it is the more valuable one: **every face reads the same
store.** The panel, `kglite_visual.show()`, `GET /api/queries`, and an agent
calling `list_saved_queries` are all looking at one file. An agent can read what
you saved, run it by name, and the result appears on the screen you are both
looking at.

## Ceilings are refusals

Every limit names its number and returns a `400`. None of them truncates,
evicts or drops the oldest entry.

| Limit | Value |
|---|---|
| Saved queries per graph | 64 |
| Bytes per query | 64 KB |
| Bytes per store file | 256 KB |
| Store files | 512 |
| Recent-query history | last 20 |

The history list is the one bounded thing that *does* roll, and it is reported
as capped (`recent_cap`) so a short list never reads as "this is everything
that ran".

## Nothing is deleted on your behalf

The store is a **durable tier**. `make prune` — which sweeps this project's
disposable directories by age — does not know it exists, and that is
deliberate: an age sweep over somebody's saved work is a scheduled data loss
with a date on it.

Its owner is a person running {ref}`kglite-visual queries <queries>`, and
even `prune` applies exactly one rule: it offers only the store files whose
graph is **gone from disk**.
