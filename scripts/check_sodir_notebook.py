#!/usr/bin/env python3
"""Validate the downloadable SODIR notebook without executing its setup."""

from __future__ import annotations

import ast
import calendar
import json
import math
from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parents[1]
NOTEBOOK = ROOT / "docs" / "_static" / "notebooks" / "sodir-geologist.ipynb"
RUNTIME_REVISION = "61e0534a89057535ba5a638dc9a83e2e0281cd78"
PIN_PATTERNS = {
    "kglite==0.17.1": r"(?<![\w-])kglite==0[.]17[.]1(?![\w.])",
    "kglite-datasets==0.1.16": r"(?<![\w-])kglite-datasets==0[.]1[.]16(?![\w.])",
    RUNTIME_REVISION: re.escape(RUNTIME_REVISION),
}
RATE_HELPER = "monthly_average_daily_rate"
MAX_QUERY_LIMIT = 1_000


def source_text(cell: dict) -> str:
    source = cell.get("source")
    assert isinstance(source, (str, list)), "cell source must be text or text lines"
    if isinstance(source, list):
        assert all(isinstance(line, str) for line in source), "cell source lines must be text"
        return "".join(source)
    return source


def load_notebook(path: Path) -> dict:
    assert path.is_file(), f"missing downloadable notebook: {path.relative_to(ROOT)}"
    try:
        notebook = json.loads(path.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise AssertionError(f"notebook is not valid UTF-8 JSON: {error}") from error
    assert isinstance(notebook, dict), "notebook root must be an object"
    assert notebook.get("nbformat") == 4, "notebook must use nbformat 4"
    assert isinstance(notebook.get("nbformat_minor"), int), "nbformat_minor must be an integer"
    assert isinstance(notebook.get("metadata"), dict), "notebook metadata must be an object"
    cells = notebook.get("cells")
    assert isinstance(cells, list) and cells, "notebook must contain cells"
    return notebook


def check_clean_and_compilable(cells: list[dict]) -> list[str]:
    sources = []
    ids = []
    for index, cell in enumerate(cells):
        assert isinstance(cell, dict), f"cell {index} must be an object"
        kind = cell.get("cell_type")
        assert kind in {"code", "markdown", "raw"}, f"cell {index} has invalid cell_type"
        assert isinstance(cell.get("metadata"), dict), f"cell {index} metadata must be an object"
        cell_id = cell.get("id")
        if cell_id is not None:
            assert isinstance(cell_id, str) and cell_id, f"cell {index} has an invalid id"
            ids.append(cell_id)
        source = source_text(cell)
        sources.append(source)
        if kind == "code":
            assert cell.get("execution_count") is None, f"cell {index} retains an execution count"
            assert cell.get("outputs") == [], f"cell {index} retains output"
            try:
                compile(source, f"notebook cell {index}", "exec")
            except SyntaxError as error:
                raise AssertionError(f"cell {index} does not compile: {error}") from error
    assert len(ids) == len(set(ids)), "notebook cell ids must be unique"
    return sources


def check_pins_and_paths(sources: list[str]) -> None:
    text = "\n".join(sources)
    for pin, pattern in PIN_PATTERNS.items():
        assert re.search(pattern, text), f"notebook does not contain exact setup pin {pin!r}"
    assert not re.search(r"/(?:Users|Volumes)/[^\s'\"`)]+", text), (
        "notebook contains a machine-local absolute path"
    )
    for phrase in (
        "SODIR_ENRICHED_GRAPH",
        "HAS_DEPOSIT_PROSPECT",
        "reported-resource subtotal",
        "Missing estimates remain missing",
    ):
        assert phrase in text, f"notebook is missing creaming-curve contract {phrase!r}"
    assert "NJU1_EXAMPLE_DISCOVERY_IDS" not in text, (
        "notebook must not embed an unrefreshable NJU-1 membership list"
    )


def check_query_bounds(cells: list[dict]) -> int:
    queries = []
    for index, cell in enumerate(cells):
        if cell.get("cell_type") != "code":
            continue
        tree = ast.parse(source_text(cell), filename=f"notebook cell {index}")
        for node in ast.walk(tree):
            if isinstance(node, ast.Constant) and isinstance(node.value, str):
                if re.search(r"\bMATCH\b", node.value, re.IGNORECASE):
                    queries.append((index, node.value))
    assert queries, "notebook contains no statically inspectable Cypher queries"
    for index, query in queries:
        limits = [int(value) for value in re.findall(r"\bLIMIT\s+(\d+)\b", query, re.IGNORECASE)]
        assert limits, f"Cypher query in cell {index} has no literal LIMIT"
        assert max(limits) <= MAX_QUERY_LIMIT, (
            f"Cypher query in cell {index} exceeds LIMIT {MAX_QUERY_LIMIT}"
        )
    return len(queries)


def load_rate_helper(cells: list[dict]):
    definitions = []
    for index, cell in enumerate(cells):
        if cell.get("cell_type") != "code":
            continue
        tree = ast.parse(source_text(cell), filename=f"notebook cell {index}")
        definitions.extend(
            node for node in tree.body if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
            and node.name == RATE_HELPER
        )
    assert len(definitions) == 1, f"notebook must define {RATE_HELPER} exactly once"
    assert isinstance(definitions[0], ast.FunctionDef), f"{RATE_HELPER} must be synchronous"
    module = ast.fix_missing_locations(ast.Module(body=[definitions[0]], type_ignores=[]))
    namespace = {"calendar": calendar, "math": math}
    exec(compile(module, "notebook rate helper", "exec"), namespace)
    return namespace[RATE_HELPER]


def check_rate_helper(cells: list[dict]) -> None:
    rate = load_rate_helper(cells)
    oil = rate(1.0, 2024, 1, 1_000_000)
    gas = rate(1.0, 2024, 1, 1_000_000_000)
    leap = rate(1.0, 2024, 2, 1_000_000)
    ordinary = rate(1.0, 2023, 2, 1_000_000)
    assert math.isclose(oil, 1_000_000 / 31), "oil million-Sm3 conversion is wrong"
    assert math.isclose(gas, 1_000_000_000 / 31), "gas billion-Sm3 conversion is wrong"
    assert math.isclose(leap, 1_000_000 / 29), "leap-February conversion is wrong"
    assert math.isclose(ordinary, 1_000_000 / 28), "ordinary-February conversion is wrong"
    assert rate(None, 2024, 2, 1_000_000) is None, "missing monthly values must remain missing"
    assert rate(math.nan, 2024, 2, 1_000_000) is None, "non-finite monthly values must remain missing"


def main() -> None:
    path = Path(sys.argv[1]).resolve() if len(sys.argv) == 2 else NOTEBOOK
    notebook = load_notebook(path)
    cells = notebook["cells"]
    sources = check_clean_and_compilable(cells)
    check_pins_and_paths(sources)
    query_count = check_query_bounds(cells)
    check_rate_helper(cells)
    print(
        "SODIR notebook: valid nbformat, clean outputs, compilable source, "
        f"exact pins, {query_count} bounded queries and executable rate semantics verified"
    )


if __name__ == "__main__":
    main()
