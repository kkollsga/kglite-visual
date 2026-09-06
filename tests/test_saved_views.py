"""Saved-view durability follows the Python input, never its display name."""

import shutil

import kglite_visual as kv
from conftest import REPO_ROOT, get_json
from test_saved_queries import post_json


def load_edge(view):
    status, table = post_json(view.port, "/api/cypher", {
        "query": "MATCH (a:Person)-[r:KNOWS]->(b:Person) RETURN r LIMIT 1",
        "as_graph": False,
    })
    assert status == 200, table
    relations = table["row_references"][0]["relationships"]
    assert len(relations) == 1
    status, result = post_json(view.port, "/api/load-entities", {
        "nodes": [], "relationships": relations, "expected": table["stamp"],
    })
    assert status == 200, result
    return get_json(view.port, "/api/view-state")


def save(view, name):
    before = get_json(view.port, "/api/view-state")
    status, result = post_json(view.port, "/api/views/save", {
        "name": name, "replace": False, "expected": before["stamp"],
    })
    assert status == 200, result
    return result


def restore(view, name, storage):
    return post_json(view.port, "/api/views/restore", {
        "name": name, "storage": storage,
        "expected": get_json(view.port, "/api/view-state")["stamp"],
    })


def test_path_view_restores_exact_relation_membership_after_relaunch(fixture_path, tmp_path, monkeypatch):
    monkeypatch.setenv("KGLITE_VISUAL_CONFIG_DIR", str(tmp_path))
    with kv.show(fixture_path, open_browser=False) as view:
        original = load_edge(view)
        save(view, "one relation")
    with kv.show(fixture_path, open_browser=False) as reopened:
        assert get_json(reopened.port, "/api/view-state")["subset"]["counts"]["loaded_nodes"] == 0
        status, result = restore(reopened, "one relation", "durable")
        assert status == 200, result
        restored = get_json(reopened.port, "/api/view-state")
        assert restored["stamp"]["generation"] != original["stamp"]["generation"]
        assert restored["subset"]["counts"] == original["subset"]["counts"]
        assert restored["subset"]["visible_edge_ids"] == original["subset"]["visible_edge_ids"]


def test_bytes_named_after_real_path_remain_session_only(fixture_path, fixture_bytes, tmp_path, monkeypatch):
    monkeypatch.setenv("KGLITE_VISUAL_CONFIG_DIR", str(tmp_path))
    with kv.show(fixture_bytes, name=fixture_path, open_browser=False) as view:
        original = load_edge(view)
        save(view, "temporary")
        status, result = post_json(view.port, "/api/reset", {})
        assert status == 200, result
        status, result = restore(view, "temporary", "session")
        assert status == 200, result
        assert get_json(view.port, "/api/view-state")["subset"]["counts"] == original["subset"]["counts"]
    with kv.show(fixture_bytes, name=fixture_path, open_browser=False) as reopened:
        before = get_json(reopened.port, "/api/view-state")
        status, _ = restore(reopened, "temporary", "session")
        assert status != 200
        assert get_json(reopened.port, "/api/view-state")["stamp"] == before["stamp"]
        status, _ = restore(reopened, "temporary", "durable")
        assert status != 200


def test_changed_source_refuses_restore_without_changing_live_view(fixture_path, tmp_path, monkeypatch):
    monkeypatch.setenv("KGLITE_VISUAL_CONFIG_DIR", str(tmp_path / "config"))
    source = tmp_path / "source.kgl"
    shutil.copyfile(fixture_path, source)
    with kv.show(str(source), open_browser=False) as view:
        load_edge(view)
        save(view, "original source")
    replacement = REPO_ROOT / "crates/kglite-visual-core/tests/fixtures/viewer-identity.kgl"
    shutil.copyfile(replacement, source)
    with kv.show(str(source), open_browser=False) as reopened:
        before = get_json(reopened.port, "/api/view-state")
        status, _ = restore(reopened, "original source", "durable")
        assert status != 200
        assert get_json(reopened.port, "/api/view-state") == before
