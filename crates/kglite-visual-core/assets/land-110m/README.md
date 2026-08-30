# `land-110m.json` — a frozen data drop, not a dependency

`land-110m.json` is the world's land outline at 1:110,000,000, in
[TopoJSON](https://github.com/topojson/topojson-specification) form: 130 arcs,
55,207 bytes (≈21 KB gzipped), one `MultiPolygon` named `land`.

**Provenance, recorded 2026-08-30.** Extracted from the npm package
`world-atlas@2.0.2` (`registry.npmjs.org/world-atlas/-/world-atlas-2.0.2.tgz`),
whose `land-110m.json` is byte-identical to this file:

    sha256  ead5f68119c49a9250902e7da303bcb209341bbb8fefe7369a439b48b704658a

- The **package** is ISC, © 2013–2019 Michael Bostock. `LICENSE` beside this
  file is that package's own licence text, copied verbatim.
- The **data** is derived from [Natural Earth](https://www.naturalearthdata.com),
  which is in the public domain: "All versions of Natural Earth raster and
  vector map data found on this website are in the public domain."

**It is vendored as a file rather than declared as a dependency, on purpose.**
This crate embeds it with `include_bytes!`, so the coastline ships inside the
binary and inside the wheel with no network, no npm resolution and no version
that can move under a rendered image. The gate's licence check
(`scripts/check_licenses.py`) walks the *frontend's* npm tree; this asset is not
in that tree, which is why its licence lives here beside it instead.

**It is frozen.** Nothing updates it on a schedule and nothing should: a
coastline that moved would move every geo golden, and the world's coastline at
110m resolution has not changed since 2019. If a finer outline is ever wanted,
that is a new file with a new name and a new decision, not a refresh of this
one.

**What reads it:** `crates/kglite-visual-core/src/render/coastline.rs` — a
~100-line arc decoder, deliberately hand-rolled rather than pulling a TopoJSON
crate in for one file of one known shape.
