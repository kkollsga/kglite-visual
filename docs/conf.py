# Configuration file for the Sphinx documentation builder.

project = "kglite-visual"
copyright = "2026, Kristian dF Kollsgård"
author = "Kristian dF Kollsgård"

# NO sphinx-autoapi, deliberately — and this is the one place that decision is
# written down.
#
# The sibling KGLite project runs autoapi over its `.pyi` stubs, which is the
# right call there: dozens of public classes and hundreds of methods, all
# documented in typed stubs autoapi can read without importing a compiled
# extension. This package's entire Python surface is `show()` and the `Server`
# handle it returns — two names in `__all__`, seven properties and three
# methods. There are no `.pyi` stubs to parse, so autoapi would have to read
# `python/kglite_visual/*.py`, where `_server.py` and `_notebook.py` are
# private modules whose docstrings are addressed to this repo's maintainers,
# not to users. The generated page would be longer than the surface it
# describes and would leak internals while doing it. `python.md` is written by
# hand and says what a caller needs — including the memory numbers a docstring
# has no room for.
extensions = [
    "myst_parser",
    "sphinx.ext.napoleon",
    "sphinx_copybutton",
]

# -- MyST (Markdown) settings ------------------------------------------------

myst_enable_extensions = [
    "colon_fence",
    "deflist",
    "fieldlist",
]
myst_heading_anchors = 6

# -- General settings ---------------------------------------------------------

exclude_patterns = ["_build", "Thumbs.db", ".DS_Store"]
source_suffix = {
    ".rst": "restructuredtext",
    ".md": "markdown",
}

# The Cypher and JSON-RPC samples throughout these pages are written for people
# rather than for Pygments, and a lexer that cannot colour one of them must not
# be able to fail a `-W` build over presentation. Broken cross-references stay
# fatal, which is the class of error `-W` is here to catch.
suppress_warnings = ["misc.highlighting_failure"]

# -- HTML output --------------------------------------------------------------

html_theme = "furo"
html_title = "kglite-visual"
html_static_path = ["_static"]
html_theme_options = {
    "source_repository": "https://github.com/kkollsga/kglite-visual",
    "source_branch": "main",
    "source_directory": "docs/",
}
