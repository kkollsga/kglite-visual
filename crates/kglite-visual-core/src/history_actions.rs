use crate::history::HistoryAction;
use crate::request::{EdgeDirection, Request};
use crate::session::Response;
use crate::shared::{SharedViewState, ViewReference};
use crate::view::SlotEntry;

impl HistoryAction {
    pub(crate) fn named(kind: &str, label: Option<&str>) -> Self {
        Self {
            kind: kind.into(),
            label: label.map(str::to_string),
            target: None,
            node_type: None,
            relationship: None,
            direction: None,
            query: None,
            query_truncated: false,
            nodes: None,
            relations: None,
        }
    }

    pub(crate) fn for_request(
        request: &Request,
        before: &SharedViewState,
        generation: &str,
        response: &Response,
    ) -> Option<Self> {
        let mut action = match request {
            Request::Focus(_) | Request::Highlight(_) => return None,
            Request::BrowseType(request) => {
                let mut action = Self::named("browse-type", None);
                action.node_type = Some(request.node_type.clone());
                action
            }
            Request::LoadNodes(_) => Self::named("load-nodes", None),
            Request::LoadEntities(_) => Self::named("load-entities", None),
            Request::Reset => Self::named("reset", None),
            Request::Expand(request) => {
                let mut action = Self::named("expand", None);
                action.target = reference(before, request.slot, generation);
                action.relationship = request.relationship.clone();
                action.direction = Some(
                    match request.direction {
                        EdgeDirection::Both => "both",
                        EdgeDirection::In => "in",
                        EdgeDirection::Out => "out",
                    }
                    .into(),
                );
                action
            }
            Request::Collapse(request) => {
                let mut action = Self::named("collapse", None);
                action.target = reference(before, request.slot, generation);
                action
            }
            Request::Cypher(request) => {
                let mut action = Self::named("cypher", None);
                let mut end = request.query.len().min(4096);
                while !request.query.is_char_boundary(end) {
                    end -= 1;
                }
                action.query = Some(request.query[..end].into());
                action.query_truncated = end < request.query.len();
                action
            }
            Request::Layout(request) => {
                let mut action = Self::named("layout", Some(request.kernel.as_str()));
                action.target = request
                    .seed_slot
                    .and_then(|slot| reference(before, slot, generation));
                action
            }
            Request::Calculate(request) => Self::named(
                if request.calculation_id.is_some() {
                    "recompute"
                } else {
                    "calculate"
                },
                Some(match request.kind {
                    crate::calculations::CalculationKind::Degree => "degree",
                    crate::calculations::CalculationKind::WeakComponents => "weak-components",
                }),
            ),
            Request::Subset(_) => Self::named("subset", None),
            Request::Appearance(_) => Self::named("appearance", None),
            Request::Presentation(_) => Self::named("presentation", None),
            Request::Style(_) => Self::named("style", None),
            Request::Caption(_) => Self::named("caption", None),
            _ => return None,
        };
        if let Response::Slice(slice) = response {
            action.nodes = Some(slice.meta.bound);
            action.relations = Some(slice.meta.link_bound);
        }
        Some(action)
    }
}

fn reference(state: &SharedViewState, slot: u32, generation: &str) -> Option<ViewReference> {
    match state.view.entry(slot)? {
        SlotEntry::Type { name } => Some(ViewReference::Type { name: name.clone() }),
        SlotEntry::Node { node_id, .. } => Some(ViewReference::Node {
            handle: crate::records::NodeHandle {
                generation: generation.into(),
                node_id: *node_id,
            },
        }),
        SlotEntry::Tombstone => None,
    }
}
