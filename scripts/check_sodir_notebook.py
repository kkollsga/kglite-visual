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
DATASETS_REVISION = "8d190236e36df3e324faa445d641a75c8abd14df"
PIN_PATTERNS = {
    "kglite==0.17.1": r"(?<![\w-])kglite==0[.]17[.]1(?![\w.])",
    DATASETS_REVISION: re.escape(DATASETS_REVISION),
    RUNTIME_REVISION: re.escape(RUNTIME_REVISION),
}
RATE_HELPER = "monthly_average_daily_rate"
DISCOVERY_ASSET_HELPER = "prepare_discovery_assets"
CACHE_HELPER = "graph_cache_is_reusable"
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
        "Discovery -[:IN_PLAY]-> Play",
        "match_method",
        "distance_m",
        "age_compatibility",
        "matched_ages",
        "candidate_tie_count",
        "DiscoveryVolume",
        "volume.recoverable_oil AS discovery_recoverable_mill_sm3_oil",
        "latest.fldRecoverableOil AS field_original_recoverable_mill_sm3_oil",
        "cumulative_mill_sm3_oil",
        "coalesce(d.dscName, d.title) AS discovery",
        "single-discovery field estimates",
        "DISCOVERED_BY",
        "wlbCompletionDate",
        "CANDIDATE_PLAY",
        "HC1, then HC2, then HC3",
        "known discovery-oil subtotal",
        "Missing, conflicted, and unresolved estimates remain gaps",
    ):
        assert phrase in text, f"notebook is missing creaming-curve contract {phrase!r}"
    assert "recoverable_oe AS discovery_recoverable" not in text, (
        "oil-only curve must not read the oil-equivalent volume component"
    )
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


def check_discovery_asset_helper(cells: list[dict]) -> None:
    definitions = []
    for index, cell in enumerate(cells):
        if cell.get("cell_type") != "code":
            continue
        tree = ast.parse(source_text(cell), filename=f"notebook cell {index}")
        definitions.extend(
            node for node in tree.body
            if isinstance(node, ast.FunctionDef) and node.name == DISCOVERY_ASSET_HELPER
        )
    assert len(definitions) == 1, f"notebook must define {DISCOVERY_ASSET_HELPER} exactly once"
    namespace = {"math": math}
    module = ast.fix_missing_locations(ast.Module(body=definitions, type_ignores=[]))
    exec(compile(module, "notebook discovery asset helper", "exec"), namespace)
    prepare = namespace[DISCOVERY_ASSET_HELPER]

    def row(
        discovery_id, resource_class, value, snapshot="2025-12-31", *,
        basis="discovery_reserves", method="reported_observation", generated=False,
        usable=True, conflict=False, identity=None,
    ):
        return {
            "discovery_id": discovery_id, "reported_discovery_year": 2000 + discovery_id,
            "discovery_well_completion_date": f"{2000 + discovery_id}-06-01",
            "discovery": f"D{discovery_id}", "discovery_resource_class": resource_class,
            "discovery_recoverable_mill_sm3_oil": value, "resource_snapshot": snapshot,
            "volume_generated": generated, "volume_usable": usable, "volume_method": method,
            "volume_coverage": "reported", "volume_basis": basis,
            "volume_source_discovery_id": discovery_id,
            "volume_source_identity": identity or f"{discovery_id}-{resource_class}-{snapshot}",
            "volume_conflict": conflict, "volume_unresolved_reason": None,
        }

    assets = prepare([
        row(1, "4F", 2.0), row(1, "4F", 2.0), row(1, "5F", 3.0),
        row(1, "7F", 99.0, snapshot="2024-12-31"),
        row(2, None, 7.0, basis="latest_original_recoverable_field_snapshot",
            method="singleton_field_copy", generated=True),
        row(3, None, 11.0, basis="field_inclusion_window_delta",
            method="inclusion_delta", generated=True),
        row(4, None, None, snapshot=None, usable=False),
        row(5, "4F", 13.0, conflict=True),
        row(6, None, 0.0, basis="latest_original_recoverable",
            method="troll_published_component_allocation", generated=True),
    ])
    assert [item["value"] for item in assets] == [5.0, 7.0, None, None, None, 0.0], (
        "discovery assets must deduplicate reported classes, admit single-discovery field estimates, "
        "preserve a generated Troll East zero, and gap different-basis, unusable, "
        "or conflicted observations"
    )



def check_cache_helper(cells: list[dict]) -> None:
    definitions = []
    capability_assignment = None
    for index, cell in enumerate(cells):
        if cell.get("cell_type") != "code":
            continue
        tree = ast.parse(source_text(cell), filename=f"notebook cell {index}")
        for node in tree.body:
            if isinstance(node, ast.FunctionDef) and node.name == CACHE_HELPER:
                definitions.append(node)
            if (isinstance(node, ast.Assign)
                    and any(isinstance(target, ast.Name)
                            and target.id == "REQUIRED_GRAPH_CAPABILITIES"
                            for target in node.targets)):
                capability_assignment = node
    assert len(definitions) == 1, f"notebook must define {CACHE_HELPER} exactly once"
    assert capability_assignment is not None, "notebook must name required graph capabilities"
    module = ast.fix_missing_locations(ast.Module(
        body=[capability_assignment, definitions[0]], type_ignores=[]))
    namespace: dict = {}
    exec(compile(module, "notebook cache helper", "exec"), namespace)
    reusable = namespace[CACHE_HELPER]
    expected = {"kglite_datasets_revision": DATASETS_REVISION,
                "default_discovery_play_enhancement": True}
    capabilities = {name: 1 for name in namespace["REQUIRED_GRAPH_CAPABILITIES"]}
    record = {**expected, "capabilities": capabilities}
    assert reusable(record, expected, capabilities), "matching capable graph must be reusable"
    stale = {**record, "kglite_datasets_revision": "0" * 40}
    assert not reusable(stale, expected, capabilities), "stale datasets revision must rebuild"
    missing = {**capabilities, "discovery_volumes": 0}
    assert not reusable({**record, "capabilities": missing}, expected, missing), (
        "graph without discovery volumes must rebuild"
    )
    changed = {**capabilities, "assigned_discoveries": capabilities["assigned_discoveries"] + 1}
    assert not reusable(record, expected, changed), "graph/build capability drift must rebuild"


def main() -> None:
    path = Path(sys.argv[1]).resolve() if len(sys.argv) == 2 else NOTEBOOK
    notebook = load_notebook(path)
    cells = notebook["cells"]
    sources = check_clean_and_compilable(cells)
    check_pins_and_paths(sources)
    query_count = check_query_bounds(cells)
    check_rate_helper(cells)
    check_discovery_asset_helper(cells)
    check_cache_helper(cells)
    print(
        "SODIR notebook: valid nbformat, clean outputs, compilable source, "
        f"exact pins, {query_count} bounded queries and executable rate, resource, and cache semantics verified"
    )


if __name__ == "__main__":
    main()
