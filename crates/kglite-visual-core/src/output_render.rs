use std::collections::{HashMap, HashSet};
use std::time::Instant;

use kglite::api::{DirGraph, EdgeIndex, GraphRead};

use crate::output::{
    check_deadline, refusal, CapturedOutput, CapturedRendered, OutputScope, RenderOutputSettings,
    MAX_OUTPUT_BYTES, OUTPUT_TIMEOUT,
};
use crate::records::{self, RecordCell, TypedValue};
use crate::render::{encoding, Scene, SceneLink, SceneNode};
use crate::shared::{RevisionStamp, SharedViewState, ViewReference};
use crate::view::SlotEntry;
use crate::CoreError;

impl CapturedOutput {
    pub fn render(&self, settings: &RenderOutputSettings) -> Result<CapturedRendered, CoreError> {
        let preview = self.preview_render(settings)?;
        let deadline = Instant::now() + OUTPUT_TIMEOUT;
        let request = crate::render::RenderRequest {
            source: crate::render::RenderSource::LiveView,
            format: settings.format,
            width: settings.width,
            height: settings.height,
            seed: u64::from(settings.seed),
            theme: settings.theme,
            kernel: (settings.kernel != crate::request::LayoutKernel::Auto)
                .then_some(settings.kernel),
        };
        let rendered = crate::render::draw_scene(self.scene.clone(), &request, &self.presentation)?;
        check_deadline(deadline)?;
        if rendered.bytes.len() > MAX_OUTPUT_BYTES {
            return Err(refusal("rendered image exceeds 16 MiB"));
        }
        Ok(CapturedRendered {
            filename: self.filename(settings.format.extension()),
            preview,
            rendered,
        })
    }
}

pub(crate) fn capture_scene(
    graph: &DirGraph,
    state: &SharedViewState,
    node_ids: &[u32],
    edge_ids: &[u32],
    scope: OutputScope,
    stamp: &RevisionStamp,
    deadline: Instant,
) -> Result<Scene, CoreError> {
    let mut nodes = scene_nodes(graph, state, node_ids, deadline)?;
    apply_mapping(state, node_ids, &mut nodes);
    let links = scene_links(graph, node_ids, edge_ids, deadline)?;
    let status = scene_status(state, scope, stamp, nodes.len(), links.len());
    Ok(Scene {
        legend: capture_legend(state),
        nodes,
        links,
        status,
        banners: Vec::new(),
        place_all_labels: false,
        canvas_tier: None,
        seeds: Vec::new(),
    })
}

fn scene_nodes(
    graph: &DirGraph,
    state: &SharedViewState,
    node_ids: &[u32],
    deadline: Instant,
) -> Result<Vec<SceneNode>, CoreError> {
    let selected = node_refs(&state.selected);
    let entries: HashMap<_, _> = state
        .view
        .live_entries()
        .filter_map(|(_, entry)| match entry {
            SlotEntry::Node {
                node_id,
                node_type,
                title,
            } => Some((*node_id, (node_type, title))),
            _ => None,
        })
        .collect();
    let mut nodes = Vec::with_capacity(node_ids.len());
    for (index, &id) in node_ids.iter().enumerate() {
        check_deadline(deadline)?;
        let (node_type, title) = entries
            .get(&id)
            .ok_or_else(|| refusal("captured node is not loaded"))?;
        let caption = state
            .caption_by
            .as_ref()
            .and_then(|field| display_cell(&records::read_cell(graph, id, field)));
        let text = caption.unwrap_or_else(|| {
            if title.is_empty() {
                format!("{node_type} {id}")
            } else {
                (*title).clone()
            }
        });
        nodes.push(SceneNode {
            slot: index as u32,
            text,
            weight: 1,
            radius: encoding::INSTANCE_RADIUS_PX,
            color: encoding::type_hue(node_type),
            badges: Vec::new(),
            dimmed: false,
            node_type: Some((*node_type).clone()),
            show_count: false,
            pinned: state.presentation.prioritize_selected_labels && selected.contains(&id),
            aggregate: None,
            emphasis: false,
            geo: crate::render::geo::position_of(graph, node_type, id),
        });
    }
    Ok(nodes)
}

fn apply_mapping(state: &SharedViewState, node_ids: &[u32], nodes: &mut [SceneNode]) {
    let highlighted = node_refs(&state.highlighted);
    let mapping: HashMap<_, _> = state
        .appearance_mapping
        .nodes
        .iter()
        .map(|node| (node.handle.node_id, node))
        .collect();
    for (node, id) in nodes.iter_mut().zip(node_ids) {
        if let Some(style) = mapping.get(id) {
            if let Some(color) = style.color {
                node.color = color.map(f64::from);
            }
            if let Some(radius) = style.radius {
                node.radius = f64::from(radius);
            }
        }
    }
    for (node, id) in nodes.iter_mut().zip(node_ids) {
        if highlighted.contains(id) {
            node.color = [1.0, 0.86, 0.2, 1.0];
        }
    }
}

fn scene_links(
    graph: &DirGraph,
    node_ids: &[u32],
    edge_ids: &[u32],
    deadline: Instant,
) -> Result<Vec<SceneLink>, CoreError> {
    let indices: HashMap<_, _> = node_ids
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect();
    let mut links = Vec::with_capacity(edge_ids.len());
    for id in edge_ids {
        check_deadline(deadline)?;
        let (source, target) = graph
            .graph
            .edge_endpoints(EdgeIndex::new(*id as usize))
            .ok_or_else(|| refusal("captured relation is absent"))?;
        links.push(SceneLink {
            source: indices[&(source.index() as u32)],
            target: indices[&(target.index() as u32)],
            width: encoding::LINK_MIN_PX,
        });
    }
    Ok(links)
}

fn scene_status(
    state: &SharedViewState,
    scope: OutputScope,
    stamp: &RevisionStamp,
    node_count: usize,
    edge_count: usize,
) -> Vec<String> {
    let mut status = vec![
        format!(
            "{} · {} nodes · {} relation records",
            match scope {
                OutputScope::Visible => "Visible subset",
                OutputScope::LoadedInduced => "Loaded induced graph",
            },
            node_count,
            edge_count
        ),
        format!(
            "Revision {} · subset {}",
            stamp.revision, state.subset_revision
        ),
        "Deterministic server image".into(),
    ];
    if state.presentation.legend_visible {
        if let Some(field) = &state.appearance.color_field {
            let field = channel_label(state, field);
            status.push(format!(
                "Color: {field} · {} loaded categories",
                state.appearance_mapping.categories.len()
                    + state.appearance_mapping.other_categories as usize
            ));
        }
        if let Some(field) = &state.appearance.size_field {
            let field = channel_label(state, field);
            status.push(format!("Size: {field} · loaded values"));
        }
    }
    status.extend(frozen_input_status(state));
    status
}

fn channel_label(state: &SharedViewState, field: &crate::subset::FieldRef) -> String {
    match field {
        crate::subset::FieldRef::Property { name } => name.clone(),
        crate::subset::FieldRef::Derived {
            calculation_id,
            column,
        } => state
            .calculations
            .iter()
            .find(|meta| &meta.id == calculation_id)
            .and_then(|meta| {
                meta.fields
                    .iter()
                    .find(|definition| &definition.field == field)
            })
            .map(|definition| definition.label.clone())
            .unwrap_or_else(|| format!("Unavailable calculation ({column})")),
    }
}

fn frozen_input_status(state: &SharedViewState) -> Vec<String> {
    let mut seen = HashSet::new();
    [&state.appearance.color_field, &state.appearance.size_field].into_iter().flatten()
        .filter_map(|field| {
            let crate::subset::FieldRef::Derived { calculation_id, .. } = field else { return None; };
            if !seen.insert(calculation_id) { return None; }
            let Some(meta) = state.calculations.iter().find(|meta| &meta.id == calculation_id) else {
                return Some(format!("{} · frozen input unavailable", channel_label(state, field)));
            };
            let label = match meta.kind {
                crate::calculations::CalculationKind::Degree => "Degree",
                crate::calculations::CalculationKind::WeakComponents => "Weak components",
            };
            Some(format!("{label} · frozen visible input · revision {} · subset {} · {} nodes · {} relation records",
                meta.input_stamp.revision, meta.input_subset_revision, meta.node_count, meta.edge_count))
        }).collect()
}

fn node_refs(references: &[ViewReference]) -> HashSet<u32> {
    references
        .iter()
        .filter_map(|reference| match reference {
            ViewReference::Node { handle } => Some(handle.node_id),
            _ => None,
        })
        .collect()
}

fn display_cell(cell: &RecordCell) -> Option<String> {
    let text = match cell {
        RecordCell::Value {
            value:
                TypedValue::String(value)
                | TypedValue::Int64(value)
                | TypedValue::UniqueId(value)
                | TypedValue::Date(value)
                | TypedValue::Timestamp(value),
        } => value.clone(),
        RecordCell::Value {
            value: TypedValue::Float64(value),
        } => value.to_string(),
        RecordCell::Value {
            value: TypedValue::Boolean(value),
        } => value.to_string(),
        _ => return None,
    };
    Some(text.chars().take(256).collect())
}

fn capture_legend(state: &SharedViewState) -> Vec<crate::render::SceneLegend> {
    if !state.presentation.legend_visible {
        return Vec::new();
    }
    let mapping = &state.appearance_mapping;
    let mut legend: Vec<_> = mapping
        .categories
        .iter()
        .map(|category| crate::render::SceneLegend {
            text: typed_label(&category.value),
            color: Some(category.color.map(f64::from)),
        })
        .collect();
    if mapping.other_categories > 0 {
        legend.push(crate::render::SceneLegend {
            text: format!(
                "{} other categories share palette hues",
                mapping.other_categories
            ),
            color: None,
        });
    }
    if let (Some(low), Some(high)) = (&mapping.size_min, &mapping.size_max) {
        legend.push(crate::render::SceneLegend {
            text: format!(
                "{} → {} px",
                typed_label(low),
                state.presentation.node_size_min
            ),
            color: None,
        });
        legend.push(crate::render::SceneLegend {
            text: format!(
                "{} → {} px",
                typed_label(high),
                state.presentation.node_size_max
            ),
            color: None,
        });
    }
    legend
}
fn typed_label(value: &TypedValue) -> String {
    match value {
        TypedValue::String(value) => {
            format!("string {:?}", value.chars().take(80).collect::<String>())
        }
        TypedValue::Int64(value) | TypedValue::UniqueId(value) => format!("integer {value}"),
        TypedValue::Float64(value) => format!("number {value}"),
        other => serde_json::to_string(other)
            .unwrap_or_default()
            .chars()
            .take(100)
            .collect(),
    }
}
