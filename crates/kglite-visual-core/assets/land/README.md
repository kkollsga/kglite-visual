# `land-*.json.gz` — three frozen data drops, not a dependency

The world's land outline in [TopoJSON](https://github.com/topojson/topojson-specification)
form, at three scales, one `MultiPolygon` named `land` in each:

| file | scale | arcs | points | raw | stored (gzip -9) |
| --- | --- | --- | --- | --- | --- |
| `land-110m.json.gz` | 1:110 000 000 | 130 | 5 129 | 55 207 B | 20 707 B |
| `land-50m.json.gz` | 1:50 000 000 | 1 425 | 60 635 | 545 534 B | 169 399 B |
| `land-10m.json.gz` | 1:10 000 000 | 4 075 | 408 957 | 3 086 715 B | 774 487 B |

**Three, because the render picks one by how much of the world the frame
covers** — see `resolution_for_span` in `coastline.rs`. A world map drawn from
the 10m outline is 3 MB of path data describing detail no pixel can hold; a
5°-wide North Sea crop drawn from the 110m outline is a coast made of
kilometre-long straight lines. The tiers cost 964 KB in the binary together,
which is what buys both cases being right.

**Provenance, recorded 2026-08-30.** All three are extracted from the npm
package `world-atlas@2.0.2`
(`registry.npmjs.org/world-atlas/-/world-atlas-2.0.2.tgz`, sha256
`032e7765f2ce00edaeafec23ff22bc3b77e42987c257da944cc585b452c05b97`), and each
is byte-identical to the file of the same name inside it:

    sha256  ead5f68119c49a9250902e7da303bcb209341bbb8fefe7369a439b48b704658a  land-110m.json
    sha256  619477ff690c086885e45cb91707d783805561bd75ae8e437b7d4694b0204e0f  land-50m.json
    sha256  9b9f584709c119d63fbadf484ce425260497697b7166be7a2fe360e8c0b171a8  land-10m.json

Those are the sha256s of the **uncompressed** JSON, which is the thing upstream
publishes; verify one with

    gunzip -c land-50m.json.gz | shasum -a 256

- The **package** is ISC, © 2013–2019 Michael Bostock. `LICENSE` beside this
  file is that package's own licence text, copied verbatim and verified
  byte-identical against the tarball on 2026-08-30.
- The **data** is derived from [Natural Earth](https://www.naturalearthdata.com),
  which is in the public domain: "All versions of Natural Earth raster and
  vector map data found on this website are in the public domain."

**Stored gzipped, decompressed at first use.** `flate2` is already in this
workspace's dependency tree (kglite pulls it for `.kgl` compression), so the
decoder costs no new crate and no new licence, and the three tiers ship as
964 KB of binary rather than 3.7 MB. Each is decompressed and parsed at most
once per process, behind a `OnceLock` per tier, so a render that never draws a
map never pays for one.

**Vendored as files rather than declared as a dependency, on purpose.** This
crate embeds them with `include_bytes!`, so the coastline ships inside the
binary and inside the wheel with no network, no npm resolution and no version
that can move under a rendered image. The gate's licence check
(`scripts/check_licenses.py`) walks the *frontend's* npm tree; these assets are
not in that tree, which is why their licence lives here beside them instead.

**They are frozen.** Nothing updates them on a schedule and nothing should: a
coastline that moved would move every geo golden, and the world's coastline has
not changed since 2019. A finer outline would be a new file with a new name and
a new decision, not a refresh of one of these.

**What reads them:** `crates/kglite-visual-core/src/render/coastline.rs` — a
hand-rolled arc decoder, deliberately not a TopoJSON crate for three files of
one known shape.
