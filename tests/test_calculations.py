"""The lib-linked viewer carries frozen measures through private reads and recovery."""

import pytest

import kglite_visual as kv
from conftest import get_json
from test_saved_queries import post_json
from test_saved_views import load_edge, restore, save


def calculate(view, kind):
    before = get_json(view.port, "/api/view-state")
    status, result = post_json(view.port, "/api/calculate", {
        "kind": kind, "expected": before["stamp"],
    })
    assert status == 200, result
    return result["meta"]["calculations"][-1]


def derived_rows(view, handles, field):
    status, result = post_json(view.port, "/api/records", {
        "handles": handles, "fields": [], "field_refs": [field],
    })
    assert status == 200, result
    assert result["columns"][0]["field"] == field
    return [row["cells"][0] for row in result["rows"]]


@pytest.mark.parametrize("as_bytes", [False, True])
def test_frozen_degree_survives_python_restore(as_bytes, fixture_path, fixture_bytes, tmp_path, monkeypatch):
    monkeypatch.setenv("KGLITE_VISUAL_CONFIG_DIR", str(tmp_path))
    source = fixture_bytes if as_bytes else fixture_path
    with kv.show(source, open_browser=False) as view:
        original = load_edge(view)
        handles = original["subset"]["visible_nodes"]
        assert len(handles) == 2
        degree = calculate(view, "degree")
        assert (degree["node_count"], degree["edge_count"], degree["status"]) == (2, 1, "ready")
        assert degree["input_stamp"] == original["stamp"]
        total = next(item["field"] for item in degree["fields"] if item["field"]["column"] == "total")
        expected = [{"state": "value", "value": {"type": "int64", "value": "1"}}] * 2
        before_read = get_json(view.port, "/api/view-state")
        assert derived_rows(view, handles, total) == expected
        assert get_json(view.port, "/api/view-state") == before_read
        status, styled = post_json(view.port, "/api/appearance", {
            "size_field": total, "color_field": None, "expected": before_read["stamp"],
        })
        assert status == 200, styled
        save(view, "frozen degree")
        status, reset = post_json(view.port, "/api/reset", {})
        assert status == 200, reset
        status, restored = restore(view, "frozen degree", "session" if as_bytes else "durable")
        assert status == 200, restored
        assert derived_rows(view, handles, total) == expected

    if not as_bytes:
        with kv.show(fixture_path, open_browser=False) as reopened:
            status, restored = restore(reopened, "frozen degree", "durable")
            assert status == 200, restored
            current = get_json(reopened.port, "/api/view-state")
            assert current["stamp"]["generation"] != original["stamp"]["generation"]
            assert derived_rows(reopened, current["subset"]["visible_nodes"], total) == expected


def test_components_and_stale_calculation_use_shared_revision(fixture_path, tmp_path, monkeypatch):
    monkeypatch.setenv("KGLITE_VISUAL_CONFIG_DIR", str(tmp_path))
    with kv.show(fixture_path, open_browser=False) as view:
        original = load_edge(view)
        components = calculate(view, "weak-components")
        size = next(item["field"] for item in components["fields"] if item["field"]["column"] == "component_size")
        assert derived_rows(view, original["subset"]["visible_nodes"], size) == [
            {"state": "value", "value": {"type": "int64", "value": "2"}},
            {"state": "value", "value": {"type": "int64", "value": "2"}},
        ]
        before = get_json(view.port, "/api/view-state")
        status, _ = post_json(view.port, "/api/calculate", {"kind": "degree", "expected": original["stamp"]})
        assert status == 409
        assert get_json(view.port, "/api/view-state") == before
