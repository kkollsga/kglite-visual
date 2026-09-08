#!/usr/bin/env python3
"""Check that preview documentation points at and labels its actual revision."""

from pathlib import Path
import os
import runpy
from unittest.mock import patch


CONF = Path(__file__).resolve().parents[1] / "docs" / "conf.py"
STATIC = CONF.parent / "_static"
REQUIRED_PAGES = ("sodir-geologist.md",)
REQUIRED_ASSETS = (
    "team.kgl",
    "team-overview.png",
    "team-records.png",
    "team-export.png",
    "notebooks/sodir-geologist.ipynb",
    "sodir-producing-fields-map.png",
    "sodir-depth-context-16-2-6.png",
    "sodir-notebook-data.png",
    "sodir-production-2024.png",
    "query-production.png",
    "query-chart-workspace.png",
)


def load(env):
    with patch.dict(os.environ, env, clear=True):
        namespace = runpy.run_path(CONF)
    options = namespace["html_theme_options"]
    return options["source_branch"], options.get("announcement")


def main():
    missing_pages = [name for name in REQUIRED_PAGES if not (CONF.parent / name).is_file()]
    assert not missing_pages, f"missing documentation page(s): {', '.join(missing_pages)}"
    missing = [name for name in REQUIRED_ASSETS if not (STATIC / name).is_file()]
    assert not missing, f"missing documentation asset(s): {', '.join(missing)}"
    revision = "0123456789abcdef0123456789abcdef01234567"
    branch, banner = load(
        {
            "READTHEDOCS_GIT_COMMIT_HASH": revision,
            "READTHEDOCS_VERSION_TYPE": "external",
        }
    )
    assert branch == revision
    assert banner is not None and "unreleased" in banner and "/en/stable/" in banner

    branch, banner = load({})
    assert branch == "main" and banner is None

    branch, _ = load({"READTHEDOCS_GIT_COMMIT_HASH": "../../not-a-revision"})
    assert branch == "main"
    print("docs config: preview SHA, banner, local fallback and malformed fallback verified")


if __name__ == "__main__":
    main()
