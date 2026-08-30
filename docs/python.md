# The Python API

```python
import kglite_visual as kv

view = kv.show("graph.kgl")
view.url          # 'http://127.0.0.1:54137/'
view.close()      # 'closed'
```

Two public names: `show()` and the `Server` handle it returns. This is the
whole surface, which is why this page is written rather than generated.

The wheel and the `kglite-visual` command are the same program: the console
script re-enters the CLI crate's own parser through PyO3, so the flags, the
stdout contract and the exit codes are one implementation rather than two that
drift.

## `show()`

```python
kv.show(
    source,
    *,
    port=0,
    open_browser=None,
    query_timeout_secs=30,
    height=640,
    name=None,
    max_load_mb=None,
) -> Server
```

`source`
: A path to a `.kgl` file, a `bytes` image of one, or an in-memory kglite
  `KnowledgeGraph` — anything with a `to_bytes()` method. Duck-typed rather
  than imported, so this wheel declares no dependency on kglite's.

`port`
: `0` (the default) asks the OS for a free port. The resolved one is in
  `launch_info`; nothing has to guess it.

`open_browser`
: `None` means *auto*: open a tab in a script or a terminal session, stay quiet
  inside a notebook kernel, where the returned object renders the view in the
  cell instead.

`query_timeout_secs`
: Wall-clock ceiling for one Cypher query.

`height`
: Height in pixels of the notebook frame.

`name`
: What to call this graph in the launch contract and the notebook caption.
  Defaults to the path, or to the source's type.

`max_load_mb`
: See [the load ceiling](#py-load-ceiling).

## The `Server` handle

```python
view.launch_info    # {'url', 'port', 'pid', 'graph', 'mcp'} — the same five keys the CLI prints
view.url            # str
view.port           # int
view.pid            # int
view.graph          # str
view.closed         # bool
view.close()        # 'closed' | 'already-closed' | 'stale-after-fork'
```

`launch_info` is the same struct the CLI's stdout line carries, including
`mcp` — hand it to an agent and it can drive the view. The wheel **returns**
those keys instead of printing them: a library that wrote to stdout would
corrupt its caller's output.

`close()` stops the server and frees the port, and returns a string rather than
raising, because all three outcomes are real states. `stale-after-fork` is a
handle inherited by a forked worker that never owned the server thread —
closing it there would be closing somebody else's.

The handle is also a context manager, and everything still open is closed at
interpreter exit:

```python
with kv.show("graph.kgl") as view:
    print(view.url)
# server stopped, port released
```

## Handing over an in-memory graph

```python
import kglite, kglite_visual as kv

graph = kglite.load("graph.kgl")
graph.cypher("MATCH (n:Person) SET n.flagged = true")   # your working graph
view = kv.show(graph)                                    # a picture of it
```

The graph crosses through `to_bytes()`, so it never touches your disk as a file
you have to clean up. See [memory](#memory) for what it costs.

(memory)=
## Memory

**Use `show(path)` for a large graph.** Handing over an in-memory graph costs
roughly **2× the graph's size** at the moment of the call: `to_bytes()`
materialises a complete `.kgl` image in the Python process, and this wheel
decodes a second, independent copy inside its own extension module.

That is not an optimisation waiting to happen. Two extension modules cannot
share a graph handle, and the image is the only sound handover. `show(path)`
reads the file directly and pays once.

Neither path is purely in-memory in any case: kglite spills any column of
256 KB or more to `$TMPDIR` while decoding, so both need a writable temporary
directory.

Real numbers, so the trade-off is arguable rather than vague. On a 546,850-node
/ 765,373-edge graph from a 133 MB `.kgl`:

| | |
|---|---|
| Load time | ~0.8 s |
| Resident after load | ~627 MB |
| Resident after a session that also rendered several images | 737 MB (measured) |
| kglite's temporary spill in `$TMPDIR` | 370 MB, removed on a clean shutdown |

A `kill -9` skips the cleanup and leaves the spill behind; nothing inside a
process can prevent that. `SIGTERM`, Ctrl-C and `close()` all remove it.

(py-load-ceiling)=
## The load ceiling

`max_load_mb` refuses a graph estimated to cost more than N megabytes to load,
so a machine that would otherwise spend twenty minutes in swap gets an
exception in hundredths of a second instead.

```python
kv.show("big.kgl", max_load_mb=100)
```

```pytb
MemoryError: loading this .kgl is estimated to peak at 327 MB of memory, over
the 100 MB ceiling this load was given … Nothing was decompressed. Estimated
terms: 208 MB for the graph's 546850 node rows and their columns, 174 KB to
rebuild the 5 declared index(es), and 118 MB held transiently while the largest
section decompresses. Two ways forward: raise the ceiling, or load with
defer_index_rebuild … This is an ESTIMATE read from the file's metadata head,
not a measurement …
```

Three things about that:

- It is **`MemoryError`, not `ValueError`**. Nothing is wrong with the file.
- It is **immediate** — measured at 0.02 s against a 0.8 s load, because the
  estimate is read from the `.kgl`'s metadata head with nothing decompressed.
- It is **conservative** and can refuse a graph that would have fitted. Set it
  where a failure is what you want, not as a tight budget.

Unset, kglite's process-wide `KGLITE_MAX_LOAD_MB` still applies; the argument
outranks it.

A `.kgl` written by a newer engine than this wheel embeds fails to load, and
kglite's own version-skew message is raised verbatim — it names the version to
install, which no paraphrase of it would.

(jupyter)=
## Jupyter

In a notebook, `show()` opens no tab and the returned object renders itself in
the cell. Which rendering you get depends on where the kernel is, and the rule
the module obeys is: **never render a silently-blank iframe.**

A localhost iframe emitted by a remote kernel points at the *reader's* machine,
not the kernel's. It loads nothing and reports nothing — the page is blank and
no error appears anywhere. That is worse than printing a URL, because the user
has no reason to suspect the URL is the problem.

So `_repr_html_` picks one of three, in this order:

1. **`jupyter-server-proxy` is importable** — the kernel's own server can proxy
   the port, so the iframe uses the proxy-prefixed URL. The frontend builds
   every URL relative to the document it was served from, which is what lets it
   survive a `/proxy/8731/` prefix with no rewriting.
2. **The kernel looks remote and nothing can proxy** — no iframe at all: the
   URL, the reason it was skipped, and an `ssh -N -L` tunnel command.
3. **Anything else** — a local kernel, so a plain localhost iframe.

Detection reads environment variables that mean "the reader's browser cannot
reach this on 127.0.0.1": `JUPYTERHUB_SERVICE_PREFIX`, `JUPYTERHUB_API_URL`,
`BINDER_SERVICE_HOST`, `BINDER_LAUNCH_HOST`, `CODESPACES`, `REMOTE_CONTAINERS`,
`SSH_CONNECTION`, `SSH_CLIENT`. It is best-effort and the rendered text says
so, naming which signal it saw.

The asymmetry is deliberate: guessing "local" when the kernel is remote
produces exactly the blank frame this exists to prevent, while guessing
"remote" when it is local costs a working user one click on a printed link.

## The console script

`pip install kglite-visual` also installs the `kglite-visual` command. It is
not a second binary shipped inside the wheel — it re-enters the CLI crate's
`run_from`, the same function the standalone `main.rs` calls. Same flags, same
single JSON line on stdout, same exit codes. See the
[CLI reference](cli.md).
