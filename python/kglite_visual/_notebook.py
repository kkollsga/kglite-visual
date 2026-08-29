"""Where a notebook cell's output actually renders, and where it cannot.

The rule this module exists to obey: **never render a silently-blank iframe.**
A localhost iframe emitted by a remote kernel points at the *reader's* machine,
not the kernel's, so it loads nothing and reports nothing — the page is blank
and no error appears anywhere (ipython#14232; not fixable from the client
side). A viewer that fails that way is worse than one that prints a URL,
because the user has no reason to suspect the URL is the problem.

So `_repr_html_` picks one of three renderings, in this order:

1. **jupyter-server-proxy is importable** — the kernel's own server can proxy
   the port, so embed the proxy-prefixed URL. The frontend builds every URL
   relative to the document it was served from (plan D7), which is what lets it
   survive a `/proxy/8731/` prefix with no rewriting.
2. **the kernel looks remote and nothing can proxy** — no iframe at all: the
   URL, the reason, and a tunnel command.
3. **anything else** — a local kernel, so a plain localhost iframe.

Detection is best-effort and the rendered text says so. The asymmetry is
deliberate: guessing "local" when the kernel is remote produces exactly the
blank frame this module exists to prevent, while guessing "remote" when it is
local costs a working user one click on a printed link.
"""

from __future__ import annotations

import html
import importlib.util
import os
import sys

#: Environment variables that mean the kernel is somewhere the reader's browser
#: cannot reach on 127.0.0.1. Value is the human-readable reason, which is
#: rendered — a user told "this looks like a remote kernel" and not told why
#: cannot check whether we guessed right.
_REMOTE_ENV_SIGNALS = {
    "JUPYTERHUB_SERVICE_PREFIX": "JupyterHub",
    "JUPYTERHUB_API_URL": "JupyterHub",
    "BINDER_SERVICE_HOST": "Binder",
    "BINDER_LAUNCH_HOST": "Binder",
    "CODESPACES": "GitHub Codespaces",
    "REMOTE_CONTAINERS": "a VS Code dev container",
    "SSH_CONNECTION": "an SSH session",
    "SSH_CLIENT": "an SSH session",
}


def _installed(name: str) -> bool:
    """Is `name` importable — without importing it.

    `sys.modules` is checked first because `find_spec` returns the *module's*
    `__spec__` for an already-imported module, and that is `None` for anything
    created outside the normal import machinery. Asking find_spec alone
    therefore answers "no" for a package that is not merely installed but
    already loaded.
    """
    if name.split(".")[0] in sys.modules and name in sys.modules:
        return True
    try:
        return importlib.util.find_spec(name) is not None
    except (ImportError, ValueError, AttributeError):
        return False


def in_notebook() -> bool:
    """True inside an IPython **kernel** — not merely inside IPython.

    A terminal IPython shell has a `get_ipython()` too, and there `show()`
    should open a browser like any other script. The distinguishing attribute
    is `kernel`, which only `ipykernel`'s shell carries.
    """
    try:
        from IPython import get_ipython  # type: ignore[import-not-found]
    except Exception:
        return False
    shell = get_ipython()
    return shell is not None and hasattr(shell, "kernel")


def remote_reason() -> str | None:
    """A human-readable reason to believe the kernel is not on the reader's
    machine, or None."""
    if _installed("google.colab"):
        return "Google Colab"
    for var, reason in _REMOTE_ENV_SIGNALS.items():
        if os.environ.get(var):
            return f"{reason} (${var} is set)"
    return None


def proxy_url(port: int) -> str | None:
    """The jupyter-server-proxy URL for `port`, if that package is installed.

    Root-relative rather than absolute: the Jupyter server's own origin is the
    one the reader's browser is already on, and its base path is
    `JUPYTERHUB_SERVICE_PREFIX` under a Hub and `/` otherwise. The kernel is not
    told its server's `base_url` by any documented API, so a non-Hub deployment
    that moved its base path is the known gap — it renders a 404 in the frame
    rather than a blank one, which at least names itself.
    """
    if not _installed("jupyter_server_proxy"):
        return None
    prefix = os.environ.get("JUPYTERHUB_SERVICE_PREFIX", "/")
    if not prefix.endswith("/"):
        prefix += "/"
    return f"{prefix}proxy/{port}/"


def repr_html(url: str, port: int, graph: str, height: int) -> str:
    """The notebook rendering for a running server."""
    reason = remote_reason()
    proxied = proxy_url(port)

    if proxied is not None:
        return _frame(proxied, url, graph, height, via="jupyter-server-proxy")
    if reason is not None:
        return _tunnel_hint(url, port, graph, reason)
    return _frame(url, url, graph, height, via=None)


def _frame(src: str, url: str, graph: str, height: int, via: str | None) -> str:
    note = f" via {via}" if via else ""
    return (
        f'<div style="font:12px ui-monospace,monospace;color:#8b949e;'
        f'margin:0 0 4px">kglite-visual — {html.escape(graph)} — '
        f'<a href="{html.escape(url, quote=True)}" target="_blank" '
        f'style="color:#58a6ff">{html.escape(url)}</a>{html.escape(note)}</div>'
        f'<iframe src="{html.escape(src, quote=True)}" width="100%" '
        f'height="{int(height)}" frameborder="0" '
        f'style="border:1px solid #30363d;border-radius:6px;background:#0d1117">'
        f"</iframe>"
    )


def _tunnel_hint(url: str, port: int, graph: str, reason: str) -> str:
    """No iframe, deliberately. See this module's docstring."""
    tunnel = f"ssh -N -L {port}:127.0.0.1:{port} &lt;this-kernel's-host&gt;"
    return (
        '<div style="font:13px/1.6 ui-sans-serif,system-ui,sans-serif;'
        "color:#c9d1d9;background:#0d1117;border:1px solid #30363d;"
        'border-radius:6px;padding:12px">'
        f"<b>kglite-visual is serving {html.escape(graph)} on "
        f"{html.escape(url)}</b><br>"
        f"This kernel looks remote ({html.escape(reason)}), so that address is "
        "on the kernel's machine, not on yours — an embedded frame would load "
        "your own computer and show nothing at all. It is not embedded here "
        "for that reason.<br>"
        "Either install <code>jupyter-server-proxy</code> in the kernel's "
        "environment (then re-run this cell, and the view embeds itself), or "
        "forward the port:"
        f'<pre style="background:#161b22;padding:8px;border-radius:4px;'
        f'overflow-x:auto"><code>{tunnel}</code></pre>'
        f'and open <a href="{html.escape(url, quote=True)}" '
        f'style="color:#58a6ff">{html.escape(url)}</a> locally.<br>'
        '<span style="color:#8b949e">Remote-kernel detection is best-effort. '
        "If this kernel is in fact local, the link above works as printed.</span>"
        "</div>"
    )
