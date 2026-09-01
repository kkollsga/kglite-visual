"""L5 — the `to_bytes()` handover (plan D9).

Two extension modules cannot share a `&KnowledgeGraph`, so an in-memory graph
crosses as a `.kgl` image. The property that matters is that all three entry
points produce the *same graph*: a handover that silently dropped edges would
render a plausible picture of the wrong data.
"""

from __future__ import annotations

import pytest

import kglite_visual as kv
from conftest import get_json

kglite = pytest.importorskip(
    "kglite",
    reason="the object handover needs a real KnowledgeGraph; "
    "`pip install kglite==0.16.19` into this venv to run it",
)


def _stats(view) -> dict:
    return get_json(view.port, "/api/session")["stats"]


def test_bytes_and_path_describe_the_same_graph(fixture_path, fixture_bytes):
    with kv.show(fixture_path, open_browser=False) as from_path:
        expected = _stats(from_path)
    with kv.show(fixture_bytes, open_browser=False, name="fixture-bytes") as from_bytes:
        assert _stats(from_bytes) == expected
        assert from_bytes.graph == "fixture-bytes"


def test_a_knowledge_graph_object_is_duck_typed_through_to_bytes(
    fixture_path, fixture_bytes
):
    graph = kglite.load(fixture_path)
    assert callable(getattr(graph, "to_bytes", None))

    with kv.show(fixture_path, open_browser=False) as from_path:
        expected = _stats(from_path)
    with kv.show(graph, open_browser=False) as from_object:
        assert _stats(from_object) == expected
        assert from_object.graph == "<KnowledgeGraph>", (
            "the default name says what was handed over"
        )


def test_a_bytearray_and_a_memoryview_are_accepted(fixture_bytes):
    for payload in (bytearray(fixture_bytes), memoryview(fixture_bytes)):
        with kv.show(payload, open_browser=False) as view:
            assert _stats(view)["node_count"] > 0
