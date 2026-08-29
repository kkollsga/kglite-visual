"""`show()`, the handle it returns, and the shutdown paths that must not leak.

Three lifetimes have to agree here: the Python object, the Rust server thread,
and the process. `Server.close()` is the explicit one; `atexit` catches the
interactive session that never called it; the native handle's own `Drop`
catches a garbage collection. All three funnel into the same native `close`,
which refuses to act on a handle inherited by a `fork()` — threads do not
survive `fork`, so a forked worker's exit would otherwise join a thread that
does not exist in its process.
"""

from __future__ import annotations

import atexit
import os
import sys
import warnings
import weakref
from typing import Any

from . import _notebook
from ._native import _serve_bytes, _serve_path

#: Every handle that has not been closed, weakly held so an abandoned one can
#: still be collected. `atexit` walks this; a strong set would keep every
#: server object alive for the life of the process just to be able to stop it,
#: which would also defeat the native handle's own `Drop`.
_LIVE: weakref.WeakSet[Server] = weakref.WeakSet()


class Server:
    """A running kglite-visual server, and the handle that stops it.

    Returned by :func:`show`. In a notebook the object renders itself as the
    view; everywhere else it is a small record with the launch contract on it.
    """

    def __init__(self, native: Any, height: int = 640) -> None:
        self._native = native
        self._height = height
        self._owner_pid = os.getpid()
        _LIVE.add(self)

    # -- the launch contract (plan D6) ---------------------------------
    @property
    def launch_info(self) -> dict[str, Any]:
        """`{"url", "port", "pid", "graph", "mcp"}` — the same five keys the
        CLI writes to stdout, from the same Rust struct.

        `mcp` is the streamable-HTTP MCP endpoint this same server exposes
        (plan D14): hand it to an agent and it can drive the view you are
        looking at.

        The wheel returns them instead of printing them: a library that writes
        to stdout corrupts whatever its caller was writing there, and a
        notebook cell's value is a better channel than its output stream.
        """
        return self._native.launch_info

    @property
    def url(self) -> str:
        return self._native.url

    @property
    def port(self) -> int:
        return self._native.port

    @property
    def pid(self) -> int:
        return self._native.pid

    @property
    def graph(self) -> str:
        return self._native.graph

    @property
    def closed(self) -> bool:
        return self._native.closed

    # -- shutdown ------------------------------------------------------
    def close(self) -> str:
        """Stop the server and release the port. Idempotent.

        Returns `"closed"`, `"already-closed"`, or `"stale-after-fork"`. The
        last one is a real outcome rather than an exception: a forked worker
        inherits this object, its server thread stayed in the parent, and the
        worker exiting normally is not an error.
        """
        status = self._close()
        if status == "stale-after-fork":
            warnings.warn(
                f"kglite_visual: this handle for {self.url} was inherited by a "
                f"forked process (pid {os.getpid()}, launched in "
                f"{self._owner_pid}); its server thread lives in the parent and "
                "cannot be stopped from here.",
                RuntimeWarning,
                stacklevel=2,
            )
        return status

    def _close(self) -> str:
        _LIVE.discard(self)
        return self._native.close()

    def __enter__(self) -> Server:
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()

    # -- rendering -----------------------------------------------------
    def _repr_html_(self) -> str:
        return _notebook.repr_html(self.url, self.port, self.graph, self._height)

    def __repr__(self) -> str:
        state = "closed" if self.closed else "serving"
        return f"<kglite_visual.Server {state} {self.url} graph={self.graph!r}>"


@atexit.register
def _close_all() -> None:
    """Close whatever is still serving when the interpreter goes down.

    Without this, `python -c "import kglite_visual; kglite_visual.show(g)"`
    would hang: the server thread is not a daemon in any sense Python controls,
    and the process would wait on a runtime nobody asked to stop. Quiet by
    design — an interpreter already on its way out is not a place to raise, and
    a forked worker exiting is the expected case, not a warning-worthy one.
    """
    for server in list(_LIVE):
        try:
            server._close()
        except Exception:  # pragma: no cover - shutdown must not raise
            pass


def _as_source(source: Any, name: str | None) -> tuple[bool, Any, str]:
    """Classify what the caller handed us: `(is_path, payload, display_name)`.

    Duck-typed on `to_bytes()` rather than importing kglite: the wheel declares
    no dependency on the engine's Python package, and a viewer that fails to
    import when kglite is absent would be broken for every `show(path)` user.
    `int` is excluded explicitly because `int.to_bytes()` exists and takes no
    required arguments from 3.11 on — `show(5)` would otherwise "work".
    """
    if isinstance(source, (str, os.PathLike)):
        return True, os.fspath(source), name or str(source)
    if isinstance(source, (bytes, bytearray, memoryview)):
        return False, bytes(source), name or "<bytes>"
    if not isinstance(source, (int, float, complex)) and callable(
        getattr(source, "to_bytes", None)
    ):
        return False, bytes(source.to_bytes()), name or f"<{type(source).__name__}>"
    raise TypeError(
        "show() takes a path to a .kgl file, a bytes image of one, or an object "
        "with a to_bytes() method (kglite's KnowledgeGraph); got "
        f"{type(source).__name__}"
    )


def show(
    source: Any,
    *,
    port: int = 0,
    open_browser: bool | None = None,
    query_timeout_secs: int = 30,
    height: int = 640,
    name: str | None = None,
) -> Server:
    """Serve a knowledge graph and return the handle that stops it.

    Parameters
    ----------
    source
        A path to a `.kgl` file, a `bytes` image of one, or an in-memory
        kglite `KnowledgeGraph` — anything with a `to_bytes()` method.
    port
        `0` (the default) asks the OS for a free port. The resolved one is in
        :attr:`Server.launch_info`; nothing has to guess it.
    open_browser
        `None` (the default) means *auto*: open a tab when this is a script or
        a terminal session, stay quiet inside a notebook kernel, where the
        returned object renders the view in the cell instead.
    query_timeout_secs
        Wall-clock ceiling for one Cypher query.
    height
        Height in pixels of the notebook frame.
    name
        What to call this graph in the launch contract and the notebook
        caption. Defaults to the path, or to the source's type.

    Memory
    ------
    **`show(path)` for a large graph.** Handing over an in-memory graph costs
    roughly **2× the graph's size** at the moment of the call: `to_bytes()`
    materialises a complete `.kgl` image in the Python process and this wheel
    decodes a second, independent copy inside its own extension module. That
    is not an implementation detail waiting to be optimised — two extension
    modules cannot share a graph handle (plan D9), and the image is the only
    sound handover. `show(path)` reads the file directly and pays once.

    Neither path is purely in-memory in any case: kglite spills any column of
    256 KB or more to `$TMPDIR` while decoding, so both need a writable
    temporary directory.

    A `.kgl` written by a newer engine than this wheel embeds fails to load,
    and kglite's own version-skew message is raised verbatim — it names the
    version to install, which no paraphrase of it would.
    """
    is_path, payload, display = _as_source(source, name)
    if is_path:
        native = _serve_path(payload, port, query_timeout_secs)
    else:
        native = _serve_bytes(payload, display, port, query_timeout_secs)

    server = Server(native, height=height)

    if open_browser is None:
        open_browser = not _notebook.in_notebook()
    if open_browser:
        _open_browser(server.url)
    return server


def _open_browser(url: str) -> None:
    """Best-effort, and never fatal: a server that is running is worth more
    than a browser that would not start."""
    try:
        import webbrowser

        if not webbrowser.open(url):
            raise RuntimeError("no browser found")
    except Exception as err:  # pragma: no cover - environment-dependent
        print(
            f"kglite_visual: could not open a browser ({err}); visit {url}",
            file=sys.stderr,
        )
