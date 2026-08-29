"""L5 — the notebook rendering (plan D8, iframe branch).

The rule under test is **never render a silently-blank iframe**: a localhost
iframe emitted by a remote kernel loads the reader's own machine and shows
nothing, with no error anywhere. So the assertions below are about *which of
three renderings* comes out, and each case asserts the absence of the wrong
one — an iframe test that only checks "an iframe appeared" cannot catch the
failure this branch exists to prevent.
"""

from __future__ import annotations

import sys
import types

import pytest

from kglite_visual import _notebook

URL = "http://127.0.0.1:8731/"
PORT = 8731


@pytest.fixture(autouse=True)
def clean_env(monkeypatch):
    """Start every case from "local kernel, no proxy" whatever the machine
    running the suite actually is — a developer on SSH would otherwise see
    different results from CI."""
    for var in _notebook._REMOTE_ENV_SIGNALS:
        monkeypatch.delenv(var, raising=False)
    monkeypatch.setattr(_notebook, "remote_reason", lambda: None)
    monkeypatch.setattr(_notebook, "proxy_url", lambda port: None)


def test_a_local_kernel_gets_an_iframe_at_the_localhost_url():
    html = _notebook.repr_html(URL, PORT, "demo.kgl", 480)
    assert f'<iframe src="{URL}"' in html
    assert 'height="480"' in html
    assert "ssh -N -L" not in html


def test_jupyter_server_proxy_wins_and_produces_a_relative_url(monkeypatch):
    monkeypatch.setattr(_notebook, "proxy_url", lambda port: f"/proxy/{port}/")
    monkeypatch.setattr(_notebook, "remote_reason", lambda: "JupyterHub ($X is set)")
    html = _notebook.repr_html(URL, PORT, "demo.kgl", 480)
    assert f'<iframe src="/proxy/{PORT}/"' in html, (
        "a proxy that can reach the server beats the remote warning"
    )
    assert "127.0.0.1" in html, "the direct URL is still printed above the frame"
    assert "ssh -N -L" not in html


def test_a_remote_kernel_without_a_proxy_gets_no_iframe(monkeypatch):
    monkeypatch.setattr(_notebook, "remote_reason", lambda: "an SSH session")
    html = _notebook.repr_html(URL, PORT, "demo.kgl", 480)
    assert "<iframe" not in html, (
        "a localhost iframe from a remote kernel is the blank frame this "
        "branch exists to prevent"
    )
    assert f"ssh -N -L {PORT}:127.0.0.1:{PORT}" in html
    assert "jupyter-server-proxy" in html
    assert "best-effort" in html


@pytest.mark.parametrize("var", sorted(_notebook._REMOTE_ENV_SIGNALS))
def test_each_remote_signal_is_read(monkeypatch, var):
    monkeypatch.undo()  # drop the autouse stub for remote_reason
    for other in _notebook._REMOTE_ENV_SIGNALS:
        monkeypatch.delenv(other, raising=False)
    monkeypatch.setattr(_notebook, "proxy_url", lambda port: None)
    assert _notebook.remote_reason() is None
    monkeypatch.setenv(var, "1")
    reason = _notebook.remote_reason()
    assert reason is not None and var in reason, (
        "the reason names the signal, so a user can check whether we guessed right"
    )


def test_the_proxy_url_respects_a_jupyterhub_prefix(monkeypatch):
    monkeypatch.undo()
    fake = types.ModuleType("jupyter_server_proxy")
    monkeypatch.setitem(sys.modules, "jupyter_server_proxy", fake)

    monkeypatch.delenv("JUPYTERHUB_SERVICE_PREFIX", raising=False)
    assert _notebook.proxy_url(PORT) == f"/proxy/{PORT}/"

    monkeypatch.setenv("JUPYTERHUB_SERVICE_PREFIX", "/user/ada")
    assert _notebook.proxy_url(PORT) == f"/user/ada/proxy/{PORT}/"


def test_the_html_escapes_the_graph_name():
    html = _notebook.repr_html(URL, PORT, '<img src=x onerror="boom">', 480)
    assert "<img" not in html
    assert "&lt;img" in html


def test_a_live_server_renders_itself(fixture_path):
    import kglite_visual as kv

    with kv.show(fixture_path, open_browser=False) as view:
        html = view._repr_html_()
        assert str(view.port) in html
        assert "kglite-visual" in html
