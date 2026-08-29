"""Shared fixtures for the wheel's test suite (test-plan L5).

These tests exercise the *installed* `kglite_visual`, whichever one the
interpreter running pytest resolves. `make pytest` builds it with
`maturin develop` first, so the suite never silently tests a stale extension.
"""

from __future__ import annotations

import http.client
import json
import pathlib
import socket

import pytest

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
FIXTURE = REPO_ROOT / "crates/kglite-visual-core/tests/fixtures/meta.kgl"


@pytest.fixture(scope="session")
def fixture_path() -> str:
    assert FIXTURE.is_file(), f"missing committed fixture: {FIXTURE}"
    return str(FIXTURE)


@pytest.fixture(scope="session")
def fixture_bytes() -> bytes:
    return FIXTURE.read_bytes()


def get_json(port: int, path: str, timeout: float = 5.0) -> dict:
    """One HTTP GET against the running server, with no dependency on
    `requests` — the wheel declares no runtime dependencies and its test suite
    should not need one either."""
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    try:
        conn.request("GET", path)
        response = conn.getresponse()
        body = response.read()
        assert response.status == 200, f"{path} -> {response.status}: {body[:200]!r}"
        return json.loads(body)
    finally:
        conn.close()


def port_is_listening(port: int, timeout: float = 1.0) -> bool:
    with socket.socket() as sock:
        sock.settimeout(timeout)
        try:
            sock.connect(("127.0.0.1", port))
        except OSError:
            return False
        return True
