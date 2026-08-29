"""L5 — the error paths.

The bar these hold: the exception *type* tells a caller what class of problem
it has, and the *message* carries the engine's own words. kglite's version-skew
diagnostic names the version to install; a paraphrase of it would be the one
sentence a stuck user needs, deleted.
"""

from __future__ import annotations

import pytest

import kglite_visual as kv


def test_a_missing_file_is_a_filenotfounderror(tmp_path):
    missing = tmp_path / "nope.kgl"
    with pytest.raises(FileNotFoundError) as excinfo:
        kv.show(str(missing), open_browser=False)
    assert "nope.kgl" in str(excinfo.value) or "No such file" in str(excinfo.value)


def test_a_directory_is_not_a_graph(tmp_path):
    with pytest.raises(ValueError):
        kv.show(str(tmp_path), open_browser=False)


def test_bytes_that_are_not_a_kgl_image_are_rejected():
    with pytest.raises(ValueError) as excinfo:
        kv.show(b"definitely not a kgl file", open_browser=False)
    # kglite's own diagnostic, forwarded rather than summarised.
    assert "could not load graph" in str(excinfo.value)


def test_a_truncated_image_is_rejected(fixture_bytes):
    with pytest.raises(ValueError):
        kv.show(fixture_bytes[: len(fixture_bytes) // 2], open_browser=False)


def test_empty_bytes_are_rejected():
    with pytest.raises(ValueError):
        kv.show(b"", open_browser=False)


@pytest.mark.parametrize("bad", [5, 4.2, None, object(), {"a": 1}])
def test_a_source_that_is_neither_a_path_nor_a_graph_is_a_typeerror(bad):
    """`int` is in here on purpose: `int.to_bytes()` exists and needs no
    arguments from 3.11 on, so a naive duck-type check accepts `show(5)`."""
    with pytest.raises(TypeError) as excinfo:
        kv.show(bad, open_browser=False)
    assert "to_bytes" in str(excinfo.value)
