"""Interactive WebGL visualization for ``.kgl`` knowledge graphs.

    import kglite_visual as kv
    view = kv.show("graph.kgl")     # a browser tab, or an inline notebook frame
    view.launch_info                # {"url", "port", "pid", "graph", "mcp"}
    view.close()                    # stops the server, frees the port

The same server the ``kglite-visual`` command runs, linked into this extension
rather than reimplemented — one axum server, one embedded frontend bundle, one
launch contract.
"""

from ._native import _version
from ._server import Server, show

__all__ = ["Server", "show", "__version__"]

#: Version of the Rust core the compiled extension was built against. Read
#: from the extension rather than duplicated here, so the package cannot
#: disagree with the binary it loaded.
__version__: str = _version()


def _run_cli() -> int:
    """The ``kglite-visual`` console script.

    Delegates to the CLI crate's own parser and run sequence, so the command
    installed by this wheel is the standalone binary in every respect that a
    caller can observe: the same flags, the same single JSON line on stdout,
    the same exit codes.
    """
    import sys

    from ._native import _run_cli as _native_run_cli

    return _native_run_cli(sys.argv)
