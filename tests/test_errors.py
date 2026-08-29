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


def test_a_load_over_the_ceiling_is_a_memoryerror_naming_the_estimate(fixture_path):
    """`MemoryError`, not `ValueError` — the file is fine and this process
    declined to pay for it.

    Getting the class wrong here sends the reader to check a graph that is not
    broken. `max_load_mb=0` is the deterministic form of "too big": every
    graph exceeds it, so the assertion is about the mechanism rather than about
    any file's size. Nothing is decompressed before the refusal, which is what
    makes the ceiling usable as a guard rather than as a post-mortem.
    """
    with pytest.raises(MemoryError) as excinfo:
        kv.show(fixture_path, open_browser=False, max_load_mb=0)
    message = str(excinfo.value)
    assert "ceiling" in message, message
    assert "Nothing was decompressed" in message, message


def test_the_ceiling_is_a_ceiling_and_not_an_off_switch(fixture_path):
    with kv.show(fixture_path, open_browser=False, max_load_mb=4096) as view:
        assert view.port > 0


def test_the_ceiling_reaches_the_bytes_handover_too(fixture_bytes):
    # show(graph) crosses as a .kgl image, and an image large enough to matter
    # is exactly where a caller wants the ceiling to apply.
    with pytest.raises(MemoryError):
        kv.show(fixture_bytes, open_browser=False, max_load_mb=0)
