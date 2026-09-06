use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use kglite::api::{DirGraph, EdgeData, EdgeIndex, GraphRead, GraphWrite, NodeData, NodeIndex};

use crate::output::{
    check_deadline, refusal, CaptureOutputRequest, CapturedOutput, OutputIdentity, OutputScope,
    OUTPUT_TIMEOUT,
};
use crate::output_values::{PropertyBudget, MAX_PROPERTIES_PER_ENTITY};
use crate::records::{MAX_LOADED_EDGES, MAX_LOADED_NODES};
use crate::shared::{RevisionConflict, SharedViewState};
use crate::view::SlotEntry;
use crate::{CoreError, Session};

impl Session {
    pub fn capture_output(
        &self,
        request: &CaptureOutputRequest,
    ) -> Result<CapturedOutput, CoreError> {
        let deadline = Instant::now() + OUTPUT_TIMEOUT;
        let state = self.state_read().clone();
        let stamp = state.stamp(self.generation());
        crate::shared::check_stamp(&request.expected, &stamp)?;
        if request.subset_revision != state.subset_revision.to_string() {
            return Err(CoreError::Conflict(Box::new(RevisionConflict {
                code: "subset-revision-conflict".into(),
                expected: request.expected.clone(),
                actual: stamp,
                message: "visible subset changed; refresh the output preview".into(),
            })));
        }
        let node_ids = captured_nodes(&state, request.scope)?;
        let _guard = self.graph().begin_read_pass();
        let edge_ids = captured_edges(self.graph(), &state, &node_ids, request.scope, deadline)?;
        let mut budget = PropertyBudget::new(deadline);
        let mut graph = DirGraph::new();
        let mut remap = HashMap::new();
        let mut nodes = Vec::with_capacity(node_ids.len());
        for &id in &node_ids {
            let node = copy_node(self.graph(), &mut graph, id, &mut budget)?;
            remap.insert(id, node);
            nodes.push(node);
        }
        let mut edge_remap = HashMap::new();
        let mut json_collision = false;
        for id in edge_ids {
            let (staged, collision) = copy_edge(self.graph(), &mut graph, id, &remap, &mut budget)?;
            edge_remap.insert(staged.index(), id);
            json_collision |= collision;
        }
        let identity = OutputIdentity {
            nodes: node_ids.iter().map(|&id| self.node_handle(id)).collect(),
            edge_ids: nodes.iter().flat_map(|&node| graph.graph.edges(node).map(|edge| edge_remap[&edge.id().index()])).collect(),
            semantics: "nodes array position is the export-local JSON/CSV/GEXF node index and GraphML n<index>; edge_ids array position is the exported relation order, GraphML e<index>, and numeric GEXF edge index. Values are generation-scoped source handles/edge IDs, not durable keys.".into(),
        };
        let scene = crate::output_render::capture_scene(
            self.graph(),
            &state,
            &node_ids,
            &identity.edge_ids,
            request.scope,
            &stamp,
            deadline,
        )?;
        check_deadline(deadline)?;
        Ok(CapturedOutput {
            stamp,
            subset_revision: state.subset_revision.to_string(),
            scope: request.scope,
            graph,
            nodes,
            identity,
            scene,
            presentation: state.presentation,
            encoded_bound: budget.encoded,
            json_collision,
        })
    }
}

fn captured_nodes(state: &SharedViewState, scope: OutputScope) -> Result<Vec<u32>, CoreError> {
    let nodes: BTreeSet<u32> = match scope {
        OutputScope::Visible => state
            .subset
            .visible_nodes
            .iter()
            .map(|handle| handle.node_id)
            .collect(),
        OutputScope::LoadedInduced => state
            .view
            .live_entries()
            .filter_map(|(_, entry)| match entry {
                SlotEntry::Node { node_id, .. } => Some(*node_id),
                _ => None,
            })
            .collect(),
    };
    if nodes.is_empty() {
        return Err(refusal("there are no instance nodes in this output scope; load nodes or change the filters first"));
    }
    if nodes.len() > MAX_LOADED_NODES {
        return Err(refusal("output exceeds 5000 instance nodes"));
    }
    Ok(nodes.into_iter().collect())
}

fn captured_edges(
    graph: &DirGraph,
    state: &SharedViewState,
    nodes: &[u32],
    scope: OutputScope,
    deadline: Instant,
) -> Result<Vec<u32>, CoreError> {
    let members: BTreeSet<_> = nodes.iter().copied().collect();
    let mut edges = BTreeSet::new();
    match scope {
        OutputScope::Visible => {
            let visible: BTreeSet<_> = state.subset.visible_edge_ids.iter().copied().collect();
            for edge in state.view.edges() {
                if let Some(id) = edge.edge_id.filter(|id| visible.contains(id)) {
                    edges.insert(id);
                }
            }
        }
        OutputScope::LoadedInduced => {
            for &node in nodes {
                for edge in graph.graph.edges(NodeIndex::new(node as usize)) {
                    check_deadline(deadline)?;
                    if members.contains(&(edge.target().index() as u32)) {
                        edges.insert(edge.id().index() as u32);
                    }
                    if edges.len() > MAX_LOADED_EDGES {
                        return Err(refusal(
                            "loaded induced output exceeds 20000 relation records",
                        ));
                    }
                }
            }
        }
    }
    if edges.len() > MAX_LOADED_EDGES {
        return Err(refusal("output exceeds 20000 relation records"));
    }
    for &id in &edges {
        check_deadline(deadline)?;
        let (source, target) = graph
            .graph
            .edge_endpoints(EdgeIndex::new(id as usize))
            .ok_or_else(|| refusal("captured relation is absent from the source"))?;
        if !members.contains(&(source.index() as u32))
            || !members.contains(&(target.index() as u32))
        {
            return Err(refusal(
                "captured relation has an endpoint outside the output scope",
            ));
        }
    }
    Ok(edges.into_iter().collect())
}

fn copy_node(
    source: &DirGraph,
    target: &mut DirGraph,
    id: u32,
    budget: &mut PropertyBudget,
) -> Result<NodeIndex, CoreError> {
    budget.entity(true)?;
    let node = source
        .node_view(NodeIndex::new(id as usize))
        .ok_or_else(|| refusal("captured node is absent from the source"))?;
    if node.property_count() > MAX_PROPERTIES_PER_ENTITY {
        return Err(refusal("output node exceeds 4096 properties"));
    }
    preflight_string(&node, "id", budget)?;
    preflight_string(&node, "title", budget)?;
    let id = budget.clone_value(&node.id())?;
    let title = budget.clone_value(&node.title())?;
    let node_type = budget.clone_text(node.node_type_str(&source.interner))?;
    let mut keys = node.property_keys(&source.interner);
    keys.sort_unstable();
    let mut properties = HashMap::new();
    for key in keys {
        preflight_string(&node, key, budget)?;
        let value = node.get_property(key);
        let value = value.as_deref().unwrap_or(&kglite::api::Value::Null);
        properties.insert(budget.clone_text(key)?, budget.clone_value(value)?);
    }
    Ok(target.graph.add_node(NodeData::new(
        id,
        title,
        node_type,
        properties,
        &mut target.interner,
    )))
}

fn preflight_string(
    node: &kglite::api::NodeView<'_>,
    field: &str,
    budget: &mut PropertyBudget,
) -> Result<(), CoreError> {
    let value = match field {
        "id" => node.id_field(),
        "title" => node.title_field(),
        _ => node.str_field(kglite::api::InternedKey::from_str(field)),
    };
    let mut result = Ok(());
    value.is(|text| {
        result = budget.text(text);
        true
    });
    result
}

fn copy_edge(
    source: &DirGraph,
    target: &mut DirGraph,
    id: u32,
    remap: &HashMap<u32, NodeIndex>,
    budget: &mut PropertyBudget,
) -> Result<(EdgeIndex, bool), CoreError> {
    budget.entity(false)?;
    let index = EdgeIndex::new(id as usize);
    let edge = source
        .graph
        .edge_weight(index)
        .ok_or_else(|| refusal("captured relation is absent"))?;
    if edge.property_count() > MAX_PROPERTIES_PER_ENTITY {
        return Err(refusal("output relation exceeds 4096 properties"));
    }
    let name = budget.clone_text(edge.connection_type_str(&source.interner))?;
    let mut properties = Vec::new();
    let mut collision = false;
    for (key, value) in edge.property_iter(&source.interner) {
        collision |= matches!(key, "source" | "target" | "type");
        properties.push((budget.clone_text(key)?, budget.clone_value(value)?));
    }
    properties.sort_by(|left, right| left.0.cmp(&right.0));
    let properties = properties.into_iter().collect();
    let (source_id, target_id) = source
        .graph
        .edge_endpoints(index)
        .ok_or_else(|| refusal("captured relation endpoints are absent"))?;
    let data = EdgeData::new(name, properties, &mut target.interner);
    let staged = target.graph.add_edge(
        remap[&(source_id.index() as u32)],
        remap[&(target_id.index() as u32)],
        data,
    );
    Ok((staged, collision))
}
