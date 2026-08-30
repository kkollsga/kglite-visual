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
instead. `caption by` overrides it per type.

No slice is re-sent when a caption changes: the data was already there.

## The legend

The **legend** card sits at the foot of the canvas and covers the colour, size
and link encodings in force. It is built from the same state the renderer's
arrays are filled from, so it cannot describe an encoding the canvas is not
drawing. `window.__kglv.legendEntries` is its size.

(filter)=
## The filter

The filter **hides what is already loaded**. It never fetches, and the panel
says so:

> hides what is already loaded — nothing is fetched. Try "type:Wellbore", or a
> property you are colouring or sizing by. Use Search above to bring nodes in.

It accepts fuzzy text over node titles, `type:Name`, and any property the view
has actually fetched. A term it cannot answer without a fetch is **refused by
name** and points at [Search](index.md#search) — the tool that does go to the
server.

Every filtered view carries an **n of m drawn** line. In the debug hook the
same pair is `pointCount` (live points, *excluding* what the filter is hiding)
and `filteredOut`. Neither number is honest on its own.

## Export

The **export** card beside the legend writes the current view out as GraphML,
GEXF, node CSV, edge CSV or D3 JSON. The scope is the view — exactly the
instance nodes on screen, never the whole graph — and an empty view is refused
by name rather than answered with an empty file.

See [export](../export.md) for the formats and the two caveats that ride with
every file.
