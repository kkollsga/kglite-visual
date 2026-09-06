"""The linked server exposes the same identity contract for paths and bytes."""

import json

import pytest

import kglite_visual as kv
from conftest import REPO_ROOT, get_json
from test_saved_queries import Mcp, post_json


@pytest.mark.parametrize("as_bytes", [False, True])
def test_query_entities_and_private_fields_survive_python_handover(as_bytes, tmp_path, monkeypatch):
    monkeypatch.setenv("KGLITE_VISUAL_CONFIG_DIR", str(tmp_path))
    path = REPO_ROOT / "crates/kglite-visual-core/tests/fixtures/viewer-identity.kgl"
    source = path.read_bytes() if as_bytes else str(path)
    with kv.show(source, open_browser=False) as view:
        before = get_json(view.port, "/api/view-state")
        status, table = post_json(view.port, "/api/cypher", {
            "query": "MATCH (n:Person) RETURN n, id(n) AS key",
            "as_graph": False,
        })
        assert status == 200, table
        handles = [row["nodes"][0] for row in table["row_references"]]
        assert len(handles) == len({h["node_id"] for h in handles}) == 3
        assert {h["generation"] for h in handles} == {before["stamp"]["generation"]}

        status, records = post_json(view.port, "/api/records", {
            "handles": handles, "fields": ["id", "title"], "offset": 0, "limit": 100,
        })
        assert status == 200, records
        keys = [row["cells"][0] for row in records["rows"]]
        assert sum(cell == {"state": "null"} for cell in keys) == 1
        assert sum(cell.get("value", {}).get("value") == "9007199254740993" for cell in keys) == 2

        mcp = Mcp(view.port)
        mcp.initialize()
        detail = mcp.call("field_detail", {"handle": handles[0], "field": "title"})
        assert not detail["isError"], detail["text"]
        assert json.loads(detail["text"])["handle"] == handles[0]
        assert get_json(view.port, "/api/view-state")["stamp"] == before["stamp"]

        status, loaded = post_json(view.port, "/api/load-entities", {
            "nodes": handles, "relationships": [], "expected": before["stamp"],
        })
        assert status == 200, loaded
        assert get_json(view.port, "/api/view-state")["subset"]["counts"]["loaded_nodes"] == 3
