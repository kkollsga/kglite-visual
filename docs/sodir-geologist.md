# Explore SODIR as an NCS geologist

The [downloadable SODIR notebook](_static/notebooks/sodir-geologist.ipynb)
builds the public SODIR knowledge graph, opens its saved `.kgl` file in KGLite
Visual, and drives five bounded geological investigations. It is a test project you can
rerun and adapt, with a live workspace as the main result.

The notebook's one-time setup installs `kglite-visual==0.1.8`,
`kglite==0.17.1`, and `kglite-datasets` from
`95794a2879143f305511e60db315cc61a1c319bd`. Run that setup before choosing
the notebook kernel.

The loader owns a project directory containing its downloaded source cache,
the generated graph, small derived artifacts, and exports. A validated
2026-09-08 preview graph was about 115 MB. SODIR is a changing source, so a
later refresh can change its size and counts. A normal rerun reuses the graph
only when its build record matches the pinned datasets revision and live
capability queries confirm the fields, discoveries, production profiles,
formation tops, cores, and drill-stem-test records used below. Set the documented
`SODIR_FORCE_REBUILD=1` only when you intend to refresh the source under the
loader's cache cooldowns.

To inspect the normalized public source tables without building a graph, the
published `kglite-datasets 0.1.16` supports
`sodir.fetch_all(Path("sodir-review"))`. It writes normalized CSV files under
`sodir-review/csv/` and the source manifest to
`sodir-review/sodir_index.json`; the upstream API payloads are normalized to
CSV rather than retained as raw JSON. The derived play and discovery-volume
tables are not required by this demo; the pinned source revision keeps the
documented graph build reproducible.

## The geological journey

Each notebook section asks a question, puts an explicitly bounded result in
the shared Explore view, and names useful follow-up fields in Data and
Appearance.

1. **Shelf context.** Map twelve recently discovered, coordinate-bearing
   producing fields using their source geometry. Color by hydrocarbon type.
   Each point is a representative location derived from source geometry; it does not display a field outline or define reservoir extent.

<a href="_static/sodir-notebook-map-app.png"><img
src="_static/sodir-notebook-map-app.png"
alt="KGLite Visual showing twelve producing NCS fields in the live geographic layout, coloured by hydrocarbon type"></a>

<a href="_static/sodir-producing-fields-map.png"><img
src="_static/sodir-producing-fields-map.png"
alt="Twelve producing NCS fields on a deterministic coastline map"></a>

The deterministic server image supplies the coastline and graticule that the
live canvas does not draw. It contains all twelve field nodes; several labels
are omitted at this shelf-wide scale. Filter to a region and render again when
clustered names matter.
2. **Early appraisal.** Follow `Field ← Discovery ← Wellbore` for JOHAN
   SVERDRUP. The graph contains 37 linked wildcat/appraisal wellbores; the bounded earliest-twelve chronology starts with discovery well 16/2-6,
   completed 20 September 2010, and continues through eleven appraisal
   completions ending in August 2012. Inspect purpose, content, measured depth,
   and final vertical depth.

<a href="_static/sodir-notebook-appraisal-app.png"><img
src="_static/sodir-notebook-appraisal-app.png"
alt="KGLite Visual radial graph of the Johan Sverdrup field, discovery, and twelve early wildcat and appraisal wellbores"></a>

3. **Well depth context.** Combine twenty formation-level tops from 16/2-6
   with three core records and one drill-stem-test record. Formation tops use
   MD m RKB; DST uses MD m; core intervals retain the source unit metres
   because SODIR does not explicitly establish the same datum. Numeric overlap
   is not a formation assignment or connectivity claim.

<a href="_static/sodir-notebook-well-evidence-app.png"><img
src="_static/sodir-notebook-well-evidence-app.png"
alt="KGLite Visual radial graph of well 16/2-6 linked to formation tops, core records, and a drill-stem test"></a>

4. **Production comparison.** Show three fields and their packed monthly
   `ProductionProfile` values, then compare GULLFAKS, OSEBERG, and DRAUGEN from
   January 2000 through the latest source month using monthly-average oil rate.
   The graph contains no invented month nodes.

<a href="_static/sodir-notebook-production-graph-app.png"><img
src="_static/sodir-notebook-production-graph-app.png"
alt="KGLite Visual showing Gullfaks, Oseberg, and Draugen connected to their production-profile records"></a>

5. **Handoff.** Save and export an exact visible GraphML slice for the
   reviewable 16/2-6 formation-top, core, and DST neighborhood.

The notebook also includes a production chart recipe. Its query returns flat
monthly rows that can be charted directly in Visual. See
[Charts from query results](query-charts.md) for chart controls and
interpretation boundaries.

<a href="_static/sodir-notebook-production-chart-app.png"><img
src="_static/sodir-notebook-production-chart-app.png"
alt="KGLite Visual Data workspace charting monthly-average oil rates for Gullfaks, Oseberg, and Draugen from 2000 through the latest source month"></a>

The combined well-evidence query returns 24 ordered source rows: twenty
formation-top records, three cores, and one DST. The notebook checks both node
and link truncation metadata before interpreting a picture.

<a href="_static/sodir-notebook-data.png"><img
src="_static/sodir-notebook-data.png"
alt="The Visual Data workspace showing the ordered formation-top query results"></a>

The query-results lane keeps `top_md_m_rkb` attached to the relationship query
that produced it; it is not presented as a Stratigraphy node property.

<a href="_static/sodir-depth-context-16-2-6.png"><img
src="_static/sodir-depth-context-16-2-6.png"
alt="Separate formation-top, DST, and core depth lanes for well 16/2-6"></a>

The lanes intentionally preserve their different source datum statements.
They show where numeric ranges overlap and stop there.

## Monthly production, stated precisely

The notebook uses type-aware Cypher `ts_series(...)` expressions on matched
`ProductionProfile` nodes while the builder graph is open, then writes only
three fields and the oil channel from 2000 through the current year to a small provenance sidecar.
It releases that Python graph before starting the file-backed viewer.

For every source month it calculates:

```text
monthly-average calendar-day rate = monthly aggregate / days in that month
```

Leap-year February uses 29 days. Missing input remains missing. These curves
are derived from monthly aggregates and are not day-by-day measurements. The
end year follows the notebook run date, while the curves stop at the latest
month actually present in the source.

<a href="_static/sodir-production-2000-latest.png"><img
src="_static/sodir-production-2000-latest.png"
alt="Monthly-average calendar-day oil rates for Gullfaks, Oseberg, and Draugen from 2000 through the latest source month"></a>

Oil is sourced in million Sm³ and converted to Sm³ per calendar day. In the validated 2026-09-08
snapshot, all three series contain 318 monthly observations from January 2000
through June 2026. Over that interval Oseberg reports 132.330082 million Sm³
oil, Gullfaks 118.673975, and Draugen 93.967819. The long view makes decline,
plateaus, and interruptions visible; the graph and chart do not establish their causes.

The primary sources are SODIR's
[monthly field production table](https://factpages.sodir.no/en/field/TableView/Production/Saleable/TotalNcsMonth)
and [16/2-6 wellbore record and attributes](https://factpages.sodir.no/en/wellbore/PageView/Exploration/All/6374).
FactPages content is published under the Norwegian Licence for Open
Government Data.

## Run and continue

Download [the notebook](_static/notebooks/sodir-geologist.ipynb), complete its
one-time environment setup, and choose the **KGLite SODIR** kernel. **Run All**
leaves the owned viewer running. Open its link in a new tab to watch later
cells replace the shared view, or use the inline frame in a local notebook.
Remote notebooks use `jupyter-server-proxy` when available and otherwise show
an explicit forwarding instruction.

The last two cells are opt-in cleanup. One closes only the viewer handle owned
by the notebook. The other removes generated figures and exports after you
change its guard; the source cache and graph remain for the next run.
