"""L5 — the launch contract, from Python.

`show()` returns the same four keys the CLI writes to stdout (plan D6). These
tests hold that contract by name, because an agent parses it by name: a rename
here is a breaking change to every harness, not a refactor.
"""

from __future__ import annotations

import os

import pytest

import kglite_visual as kv
from conftest import get_json, port_is_listening


def test_show_returns_the_launch_contract(fixture_path):
    view = kv.show(fixture_path, open_browser=False)
    try:
        info = view.launch_info
        assert sorted(info) == ["graph", "pid", "port", "url"], (
            "an extra or missing key is a contract change, not an addition"
        )
        assert info["port"] > 0, "--port 0 must be resolved, never reported as 0"
        assert info["url"] == f"http://127.0.0.1:{info['port']}/"
        assert info["pid"] == os.getpid(), "the server runs in this process"
        assert info["graph"] == fixture_path
        # The accessors and the dict are two views of one struct.
        assert (view.url, view.port, view.pid, view.graph) == (
            info["url"],
            info["port"],
            info["pid"],
            info["graph"],
        )
    finally:
        view.close()


def test_the_server_actually_answers(fixture_path):
    view = kv.show(fixture_path, open_browser=False)
    try:
        session = get_json(view.port, "/api/session")
        assert session["protocol_version"] >= 1
        assert session["graph"] == fixture_path
        assert session["stats"]["node_count"] > 0
        assert session["core_version"] == kv.__version__
    finally:
        view.close()


def test_the_frontend_bundle_is_served_from_the_extension(fixture_path):
    """The packaged-consumer question, asked from inside the source tree: is
    the bundle in the artifact at all? A `.so` built without `frontend/dist`
    answers `/` with the no-bundle-embedded message instead of the app."""
    import http.client

    view = kv.show(fixture_path, open_browser=False)
    try:
        conn = http.client.HTTPConnection("127.0.0.1", view.port, timeout=5)
        conn.request("GET", "/")
        response = conn.getresponse()
        body = response.read().decode()
        conn.close()
        assert response.status == 200
        assert "<div id=\"app\"></div>" in body, body[:300]
        assert "no frontend bundle is embedded" not in body
    finally:
        view.close()


def test_close_frees_the_port(fixture_path):
    view = kv.show(fixture_path, open_browser=False)
    port = view.port
    assert port_is_listening(port)
    assert view.close() == "closed"
    assert view.closed
    assert not port_is_listening(port), (
        "close() must release the port, not merely stop answering"
    )
    # Idempotent: a second close, and the atexit hook after it, are no-ops.
    assert view.close() == "already-closed"


def test_close_is_reachable_as_a_context_manager(fixture_path):
    with kv.show(fixture_path, open_browser=False) as view:
        port = view.port
        assert port_is_listening(port)
    assert not port_is_listening(port)


def test_an_explicit_port_is_honoured(fixture_path):
    """`port=0` is the default, but a caller who names a port gets it — and
    gets a real error rather than a silent reassignment if it is taken."""
    first = kv.show(fixture_path, open_browser=False)
    try:
        chosen = first.port
        with pytest.raises(OSError) as excinfo:
            kv.show(fixture_path, port=chosen, open_browser=False)
        assert str(chosen) in str(excinfo.value)
    finally:
        first.close()

    second = kv.show(fixture_path, port=chosen, open_browser=False)
    try:
        assert second.port == chosen, "the port is free again and was honoured"
    finally:
        second.close()
