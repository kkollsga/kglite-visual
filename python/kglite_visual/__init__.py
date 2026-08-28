"""Interactive visualization for ``.kgl`` knowledge graphs.

P1 ships the package and its version. ``show()`` — the browser-tab launch and
the notebook transport — lands in P5.
"""

from ._native import _version

__all__ = ["__version__"]

#: Version of the Rust core the compiled extension was built against. Read
#: from the extension rather than duplicated here, so the package cannot
#: disagree with the binary it loaded.
__version__: str = _version()
