# Calculations on the visible graph

Degree and weak components answer questions about the **visible instances and
their visible relationship records**. They do not measure the whole source
graph. Load and narrow an exploration before calculating.

Open **Data → Calculated fields**, choose **Directed degree** or **Weak
components**, then choose **Calculate visible subset**. Each result offers
**Inspect fields**, **Recompute on current visible subset**, and its definition.

| Result | Meaning |
|---|---|
| In degree | Number of visible relationships entering the node |
| Out degree | Number of visible relationships leaving the node |
| Total degree | In degree plus out degree |
| Weak component | Group reachable when relationship direction is ignored |
| Component size | Number of visible nodes in that group |

Parallel relationship records count separately. A self-loop contributes one
incoming and one outgoing relationship, giving two total degree. An isolated
visible node has zero degree and belongs to a component of size one. Component
IDs are ordered by the smallest source node identity in each component; they
are identifiers for this calculation, not persistent source properties.

## Inspect, style and narrow

Calculated fields are available in Data, appearance channels and filters. A
field belongs to a particular calculation, so a source property with the same
display name remains a separate field. Calculation values never overwrite
source properties.

Appearance groups these choices under **Frozen calculated fields**. In Filters,
choose a calculated field through **Field source** before defining a predicate.

To share the values as a table, inspect the calculated columns and download
the fetched Records CSV. Graph-format exports carry source graph attributes;
calculated values remain viewer-owned fields. Captured images use the chosen
calculated appearance channels and name their frozen input.

Each result records its input revision, visible node and relationship counts,
semantics and completion state. Values stay **frozen** when you subsequently
filter the graph. A filter using degree therefore evaluates that saved result
once; it does not repeatedly recalculate degree as nodes disappear.

Use explicit recomputation to replace a result with a calculation over the
current visible input. Loaded nodes outside that input show **unavailable** for
its fields. They do not acquire an invented zero degree. A stale completion is
refused when another shared action changes the input during preparation.

## Save and restore

Saved views preserve the frozen values, their input metadata and the fields
chosen for appearance and filtering. Restoring a view does not silently run
the calculations again. The original input revision remains recorded even
when a durable view is reopened in a new session.

Calculations use the viewer's existing limits of 5,000 loaded instances and
20,000 retained relationship records. A session allows eight calculations and
8 MiB of serialized derived values; saved-view and history limits also apply.
Oversize or expired calculations refuse without changing the shared view.

## API

Create a result with `POST /api/calculate`, supplying `kind: "degree"` or
`kind: "weak-components"` and the current `expected` generation/revision stamp.
To recompute, also supply the existing `calculation_id`; an unknown ID or a
different kind is refused. The `calculate` MCP tool uses the same operation.

Use canonical field references when reading or applying the result:

```json
{"kind":"derived","calculation_id":"the-returned-id","column":"total"}
```

Pass this object through `field_refs` in a Records request, or as `color_field`
or `size_field` in an appearance request. Existing string `fields`, `color_by`
and `size_by` inputs continue to name source properties.
