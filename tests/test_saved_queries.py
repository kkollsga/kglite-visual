"""L5 — the saved-query store is one store, and `show()` gets it for free.

The claim being tested is a *sharing* claim, so it cannot be tested by asking
one face twice. A query is saved over the JSON twin, with `curl`'s own verb and
body, and then read back through the MCP tool an agent would call — against a
server that was started by `kglite_visual.show()`, in this interpreter, with
nothing arranged between the two faces.

That is the whole point of the store living in `AppState` beside the broadcast
bus rather than in either handler: the wheel does not opt into it, wire it up,
or pass it along. It links the CLI's server, and the CLI's server has one.

The store's own ceilings and refusals are unit-tested in Rust
(`crates/kglite-visual-cli/src/queries.rs`); this file is about the seam.
"""

from __future__ import annotations

import http.client
import json
import tempfile

import pytest

import kglite_visual as kv


@pytest.fixture()
def store_dir(monkeypatch):
    """Point the store at a throwaway directory for this test only.

    Without it the suite would read and write the *developer's* real saved
    queries: assertions that depend on what somebody saved last week, and a
    test with a side effect on the person running it.
    """
    with tempfile.TemporaryDirectory() as tmp:
        monkeypatch.setenv("KGLITE_VISUAL_CONFIG_DIR", tmp)
        yield tmp


def post_json(port: int, path: str, body: dict, timeout: float = 10.0) -> tuple[int, dict]:
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    try:
        conn.request(
            "POST", path, json.dumps(body), {"content-type": "application/json"}
        )
        response = conn.getresponse()
        payload = response.read()
        return response.status, json.loads(payload)
    finally:
        conn.close()


class Mcp:
    """The three calls this test needs, over streamable HTTP.

    Hand-rolled for the same reason the e2e client is: pulling an MCP SDK into
    the wheel's test suite would make a *dependency* answer for the transport
    under test, and the wheel declares no runtime dependencies at all.

    rmcp answers with `text/event-stream` by default, so the JSON-RPC payload
    is the last `data:` line rather than the body.
    """

    def __init__(self, port: int) -> None:
        self.port = port
        self.session: str | None = None

    def _rpc(self, body: dict) -> dict | None:
        headers = {
            "content-type": "application/json",
            "accept": "application/json, text/event-stream",
        }
        if self.session is not None:
            headers["mcp-session-id"] = self.session
        conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=30)
        try:
            conn.request("POST", "/mcp", json.dumps(body), headers)
            response = conn.getresponse()
            if self.session is None:
                self.session = response.getheader("mcp-session-id")
            raw = response.read().decode("utf-8")
            kind = response.getheader("content-type") or ""
        finally:
            conn.close()
        if "application/json" in kind:
            return json.loads(raw)
        payloads = [
            line[len("data: ") :]
            for line in raw.splitlines()
            if line.startswith("data: ") and line[len("data: ") :].startswith("{")
        ]
        return json.loads(payloads[-1]) if payloads else None

    def initialize(self) -> None:
        self._rpc(
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "pytest", "version": "0"},
                },
            }
        )
        self._rpc({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def call(self, name: str, arguments: dict | None = None) -> dict:
        reply = self._rpc(
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments or {}},
            }
        )
        assert reply is not None, f"{name} answered with no JSON-RPC payload"
        result = reply["result"]
        text = "".join(
            block["text"] for block in result["content"] if block.get("type") == "text"
        )
        return {"isError": bool(result.get("isError")), "text": text}


def test_a_query_saved_over_http_is_the_one_mcp_lists(fixture_path, store_dir):
    view = kv.show(fixture_path, open_browser=False)
    try:
        port = view.port

        status, saved = post_json(
            port,
            "/api/queries/save",
            {"name": "people", "query": "MATCH (p:Person) RETURN p LIMIT 2"},
        )
        assert status == 200, saved
        assert saved["name"] == "people"

        mcp = Mcp(port)
        mcp.initialize()
        listed = mcp.call("list_saved_queries")
        assert not listed["isError"], listed["text"]
        payload = json.loads(listed["text"])
        assert [entry["name"] for entry in payload["saved"]] == ["people"], (
            "the MCP face is reading a different store than the HTTP face"
        )
        assert payload["saved"][0]["query"] == "MATCH (p:Person) RETURN p LIMIT 2"

        # Running it goes through the ordinary Cypher path — the store supplies
        # the text, not a second executor — so the answer is a slice report,
        # and the run lands in the recent list of that same store.
        ran = mcp.call("run_saved_query", {"name": "people"})
        assert not ran["isError"], ran["text"]
        assert len(json.loads(ran["text"])["added"]) == 2

        recent = json.loads(mcp.call("list_saved_queries")["text"])["recent"]
        assert recent[0]["query"] == "MATCH (p:Person) RETURN p LIMIT 2"

        # A name the store does not have is a refusal an agent can act on,
        # never a protocol error clients render as "tool result missing".
        missing = mcp.call("run_saved_query", {"name": "nope"})
        assert missing["isError"]
        assert "no saved query named" in missing["text"]
    finally:
        view.close()


def test_a_graph_with_no_path_still_gets_a_store(fixture_bytes, store_dir):
    """`show(bytes)` has nowhere to sit beside, which is one of the three
    reasons the store is not a sidecar file. It shares `_unbound.json`."""
    view = kv.show(fixture_bytes, name="in-memory.kgl", open_browser=False)
    try:
        status, saved = post_json(
            view.port, "/api/queries/save", {"name": "q", "query": "RETURN 1"}
        )
        assert status == 200, saved
        conn = http.client.HTTPConnection("127.0.0.1", view.port, timeout=10)
        try:
            conn.request("GET", "/api/queries")
            listing = json.loads(conn.getresponse().read())
        finally:
            conn.close()
        assert listing["graph_path"] is None, "a buffer has no path to be keyed by"
        assert listing["store"].endswith("_unbound.json")
        assert [entry["name"] for entry in listing["saved"]] == ["q"]
    finally:
        view.close()
