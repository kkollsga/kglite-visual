# CLI reference

```text
Usage: kglite-visual [OPTIONS] [FILE]
       kglite-visual <COMMAND>

Commands:
  render   Draw one image of this graph and exit — no server, no browser
  export   Write this graph out as GraphML, GEXF, CSV or D3 JSON — no server, no browser
  queries  Inspect and collect the saved-query store — no server, no graph
```

The same binary is installed by `pip install kglite-visual` and by
`cargo install`: the wheel's console script re-enters the CLI crate's own
parser, so the flags, the stdout contract and the exit codes are one
implementation rather than two.

## stdout is JSON, always

Four modes, one rule. The serving form prints exactly one line — the launch
contract. `render` prints one line — the render summary. `export` prints one
line — the export summary. `queries list` prints **one JSON object per store
file**, because a listing is the one command here with more than one thing to
say, and JSON Lines says that without inventing a second format.

Everything else — counts, warnings, the export caveats, errors — is on stderr.
A failed command prints **nothing** on stdout, so a harness that read a line
got an answer.

(serve)=
## `kglite-visual <FILE>` — serve

Loads the graph, binds an HTTP server on `127.0.0.1`, prints the launch line,
opens a browser.

```json
{"url":"http://127.0.0.1:54137/","port":54137,"pid":69850,"graph":"/path/graph.kgl","mcp":"http://127.0.0.1:54137/mcp"}
```

`--port <PORT>` *(default `0`)*
: Port to bind. `0` asks the OS for a free one; the resolved port is always
  reported in the stdout JSON, so nothing needs to guess.

`--no-open`
: Do not open a browser. How every agent and CI invocation runs; opening a
  browser is the interactive default, not the only mode.

`--query-timeout-secs <N>` *(default `30`)*
: Wall-clock ceiling for one Cypher query. A viewer is interactive, so an
  unbounded query is a hung tab; the default is what an accidental cartesian
  product costs. Raise it for a deliberate analytical query on a large graph.

`--max-load-mb <MB>`
: See [the load ceiling](#the-load-ceiling).

Ctrl-C and `SIGTERM` are caught: clean shutdown, exit 0, port released, and
kglite's temporary spill removed from `$TMPDIR`.

(render)=
## `kglite-visual render <FILE>` — one image

Full page: **[render](render.md)**.

**Sources** — mutually exclusive; naming two is a usage error.

`--meta`
: The type-level meta-graph. The default when no other source is named.

`--cypher <QUERY>`
: The graph a read-only Cypher query returns. The query must `RETURN` nodes,
  relationships or paths.

`--expand <KEY=VALUE>…`
: A bounded neighbourhood expansion, as `type=T [rel=R] [dir=out|in|both]`.
  `dir` defaults to `both`; omitting `rel` walks every relationship, which is
  the expensive case.

**Output**

`--format <svg|png>` *(default `svg`)*
`-o, --out <PATH>`
: Defaults to a name derived from the graph and the source, in the current
  directory.

`--width <N>` *(default `2000`)* · `--height <N>` *(default `1250`)*
`--theme <dark|light>` *(default `dark`)*
: Dark matches the app; light is for a white page.

**Layout**

`--layout <auto|radial|islands|force|geo>`
: Force one arrangement instead of letting the structure choose. Unset is
  `auto`. `geo` answers with an error rather than a picture when nothing in the
  slice has a coordinate. `simulation` is deliberately not offered: a headless
  render has no viewer's GPU to hand the geometry back to.

`--seed <N>` *(default `0`)*
: Reaches the initial placement only; the force pass has no randomness at all,
  so the same seed is the same image forever. This is how you get a *different*
  arrangement, not a random one.

**Bounds**

`--limit <N>`
: Rows (for `--cypher`) or nodes (for `--expand`) wanted. Clamped in core to
  the response bound, whatever is asked for.

`--query-timeout-secs <N>` *(default `30`)* · `--max-load-mb <MB>`

(export)=
## `kglite-visual export <FILE>` — one file

Full page: **[export](export.md)**.

`--format <graphml|gexf|csv|csv-edges|json>` *(default `graphml`)*

| | |
|---|---|
| `graphml` | Gephi, yEd, Cytoscape |
| `gexf` | Gephi's native XML |
| `csv` | `id,type,title`, one row per node |
| `csv-edges` | `source,target,type`, one row per edge |
| `json` | D3's `{"nodes": [...], "links": [...]}` |

`--cypher <QUERY>`
: Export only the nodes a read-only query returns, rather than the whole graph.
  The bounded form, and the one to use on a large graph.

`-o, --out <PATH>` · `--query-timeout-secs <N>` *(default `30`)* ·
`--max-load-mb <MB>`

This is the **one place a whole-graph dump is on offer** — because here the
caller named a file and a path at a terminal, with no view in existence. The
server's export is always scoped to the view.

(queries)=
## `kglite-visual queries` — the saved-query store

Takes no graph and touches none. The store is a
[durable tier](concepts/storage.md): nothing sweeps it by age, because an age
sweep over somebody's saved work is a scheduled data loss with a date on it.
This subcommand is its owner.

`queries` with no action is a usage error, not a silent default: guessing
between `list`, `rm` and `prune` is guessing about a delete.

### `list`

One JSON line per store file: which graph it belongs to, how much it holds, and
whether that graph still exists.

```console
$ kglite-visual queries list
kglite-visual: 2 store file(s) in /Users/me/Library/Application Support/kglite-visual/queries
{"file":"1cc2f649dc848c3b.json","graph_path":"/data/sodir.kgl","graph_label":"/data/sodir.kgl","saved":4,"history":12,"bytes":3180,"graph_missing":false}
{"file":"_unbound.json","graph_path":null,"graph_label":"…","saved":1,"history":3,"bytes":410,"graph_missing":false}
```

`graph_missing` is what `prune` acts on. `_unbound.json` is the shared store
for graphs handed over as bytes, which have no path to key on — it names no
path, so it can never be stale and `prune` never offers it. An unreadable file
is reported rather than skipped: a listing that silently omitted it would leave
`prune` looking as though it had nothing to do.

### `rm <FILE>`

Delete one store file, by the `file` name `list` printed. A name that is not
there is reported on stderr and answered `{"file":…,"removed":false}` — "there
is no such file" is the state the caller asked for, not an error exit.

### `prune [--dry-run]`

Delete the store files whose graph is **no longer on disk**. That is the only
rule it applies; a store whose graph still exists is never offered.

```console
$ kglite-visual queries prune --dry-run
{"dry_run":true,"removed":1,"files":["3a91….json"]}
```

`$KGLITE_VISUAL_CONFIG_DIR` overrides where the store lives — point it at a
temporary directory in any harness, or it reads and writes the developer's own
saved queries.

(the-load-ceiling)=
## `--max-load-mb`

Accepted by the serve form, `render` and `export`, and by
`kglite_visual.show(..., max_load_mb=N)`.

It asks the engine what the `.kgl` will cost — read from the file's metadata
head, with **nothing decompressed** — and refuses above the ceiling:

```console
$ time kglite-visual big.kgl --no-open --max-load-mb 100
kglite-visual: could not load graph: loading this .kgl is estimated to peak at 327 MB
of memory, over the 100 MB ceiling this load was given … Nothing was decompressed.
Estimated terms: 208 MB for the graph's 546850 node rows and their columns, 174 KB to
rebuild the 5 declared index(es), and 118 MB held transiently while the largest
section decompresses. Two ways forward: raise the ceiling, or load with
defer_index_rebuild … This is an ESTIMATE read from the file's metadata head, not a
measurement …
0,02s user 0,01s system 89% cpu 0,026 total
```

Exit 1, empty stdout, 0.026 s against a 0.8 s load of the same file. The wheel
raises `MemoryError` for it, not `ValueError`: nothing is wrong with the file.

Unset, kglite's process-wide `KGLITE_MAX_LOAD_MB` still applies; the flag
outranks it. The estimate is deliberately conservative and can refuse a graph
that would have fitted, so it is a **guard rather than a budget**.

## Environment

`KGLITE_VISUAL_CONFIG_DIR`
: Where the saved-query store lives. Overrides the platform config directory.

`KGLITE_MAX_LOAD_MB`
: kglite's own process-wide load ceiling. `--max-load-mb` outranks it.

`TMPDIR`
: kglite spills any column of 256 KB or more here while decoding — 370 MB for a
  half-million-node graph, removed on a clean shutdown.

`BROWSER`
: Honoured by the browser-opening path, which uses the `webbrowser` crate: it
  knows about WSL and headless servers, and reports failure instead of spawning
  a process that silently does nothing.
