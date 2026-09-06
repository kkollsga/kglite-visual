# Appearance, legend and filtering

## Colour and size

The **Appearance** panel drives two channels from node properties: `colour by`
and `size by`. Clearing one returns it to the app's structural encoding — type
colour, log-scaled member count.

The candidates in each picker come from the type's
[property statistics](index.md#inspecting-a-node), not from a list of every
property name: a property with ten distinct values over full coverage makes a
readable colour channel, and one with sixty unique strings does not.

The same two channels are drivable from outside the browser, which is how an
agent recolours the view you are looking at:

```bash
curl -s -XPOST $B/api/appearance -H "$C" -d '{"color_by":"city","size_by":"age"}'
```

The property name is not validated: the viewer's own statistics decide what is
meaningful, and a name nothing carries renders uniformly rather than failing.

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

The **export** card beside the legend writes the current view out as GraphML,
GEXF, node CSV, edge CSV or D3 JSON. The default scope is loaded instances, including those hidden by filters, with
every source relationship between the selected nodes. An empty view is refused
by name rather than answered with an empty file.

See [export](../export.md) for the format and relationship-scope caveats.
