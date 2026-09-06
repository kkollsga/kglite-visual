# Appearance, legend and filtering

## Colour and size

The **Appearance** panel maps properties to colour and size. Clearing a channel
returns it to the structural encoding. The core publishes the same bounded
mapping and legend to every browser and to captured server images; opening a
property inspector in one browser does not determine another browser's colours.

Property domains describe loaded instances, so hiding nodes with a filter does
not silently change the scale. Typed category labels distinguish a number from
a string containing the same characters. Missing and unavailable values remain
explicit rather than becoming invented measurements.

[Calculated fields](calculations.md) can also supply colour and size. Their
names identify the frozen calculation, separately from source properties.

```bash
curl -s -XPOST $B/api/appearance -H "$C" -d '{"color_by":"city","size_by":"age"}'
```

## Readability

Shared controls adjust instance-label density, selected/hovered label priority,
edge opacity, the numeric size-by range and legend visibility. Schema labels
remain present. Numeric size limits affect property-based sizes; structural
schema and instance sizes keep their established meaning. These choices survive
reconnect and saved-view restoration, and captured images receive the same
presentation settings.

## Captions

Node labels are drawn from each type's title. Where a type's title names
nothing — few distinct values, or poor coverage — the server **suggests** the
property its nodes read best under, and the client draws that on the labels
instead. An explicit `caption by` choice is shared across the loaded view and
survives reconnects. The server acknowledges the choice in a snapshot; the
client reads bounded fields by source handles and keeps the existing topology.

## The legend

The **legend** card sits at the foot of the canvas and covers the colour, size
and link encodings in force. It is built from the same state the renderer's
arrays are filled from, so it cannot describe an encoding the canvas is not
drawing. `window.__kglv.legendEntries` is its size.

(filter)=
## The filter

Filters narrow **loaded instances and their retained relationships**. They never
load source search hits. Choose type, category, numeric range, missing values,
relationship type or isolated-node predicates. Enable, disable or remove each
predicate, or clear the set to recover the loaded exploration.

Core evaluates the predicates and sends the same visible subset to every
attached browser. The scope line separates source totals, loaded instances and
visible instances. Field distributions describe the loaded subset after the
other enabled predicates, with missing, null and unavailable counts kept apart.

Selection is retained by identity when a filter hides a node. Clearing filters
restores that node without another source search or an unbounded load.

## Export

Open **Export** to preview visible instances, loaded instances with
source-induced relationships, or a deterministic server image. The preview
names its scope and revision; download refuses after the shared view changes.

See [export](../export.md) for formats, bounds and relationship-scope caveats.
