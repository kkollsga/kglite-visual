use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use kglite::api::{EdgeIndex, GraphRead};

use crate::bookmark::{
    Bookmark, BookmarkFocus, BookmarkLayout, BookmarkMember, BookmarkName, BookmarkReference,
    BookmarkSource, BOOKMARK_VERSION, MAX_BOOKMARK_BYTES,
};
use crate::bookmark_members;
use crate::control::{Appearance, Focus};
use crate::history::HistoryAction;
use crate::query_provenance::{LoadEntitiesRequest, RelationHandle};
use crate::records;
use crate::render::{LayoutMeta, LayoutResult};
use crate::request::Request;
use crate::shared::{PreparedShared, ReplacementContext, RevisionStamp, ViewReference};
use crate::source_identity::check_deadline;
use crate::subset::{self, FrozenField};
use crate::view::View;
use crate::{CoreError, Session};

impl Session {
    pub fn prepare_bookmark_restore(
        &self,
        bookmark: &Bookmark,
        name: Option<&BookmarkName>,
        expected: Option<&RevisionStamp>,
        request_id: Option<String>,
    ) -> Result<PreparedShared, CoreError> {
        let before = self.state_read().clone();
        if let Some(expected) = expected {
            crate::shared::check_stamp(expected, &before.stamp(self.generation()))?;
        }
        validate_bookmark(bookmark)?;
        let deadline = self.config().deadline();
        self.verify_bookmark_source(&bookmark.source, deadline)?;
        let _guard = self.graph().begin_read_pass();
        let nodes = bookmark_members::resolve_members(
            self.graph(),
            &bookmark.members,
            self.generation(),
            deadline,
        )?;
        let relationships = self.restore_relations(bookmark, &nodes, deadline)?;
        let mut replacement = before.clone();
        replacement.view = View::new();
        crate::meta_graph::compute(self.graph(), &mut replacement.view);
        let candidate = self.fork_state(replacement);
        candidate.handle_uncommitted(&Request::LoadEntities(LoadEntitiesRequest {
            nodes: nodes.iter().map(|id| self.node_handle(*id)).collect(),
            relationships,
        }))?;
        {
            let mut state = candidate.state_write();
            state.predicates = bookmark.predicates.clone();
            state.appearance = Appearance::new(bookmark.color_by.clone(), bookmark.size_by.clone());
            state.caption_by = bookmark.caption_by.clone();
            state.presentation = bookmark.presentation.clone();
            state.highlighted =
                restore_references(self, &bookmark.highlighted, &nodes, &state.view)?;
            state.selected = restore_references(self, &bookmark.selected, &nodes, &state.view)?;
            state.last_layout = restore_layout(self, &bookmark.layout, &nodes, &state.view)?;
            state.layout_kernel = bookmark.layout.kernel_chosen;
            state.derived = self.restore_derived(bookmark, deadline)?;
            state.saved_view = None;
        }
        let replacement = candidate.state_read().clone();
        let focus = restore_focus(self, bookmark.focus.as_ref(), &nodes, &replacement.view)?;
        self.prepare_replacement(
            before,
            replacement,
            ReplacementContext {
                restored: true,
                action: Some(HistoryAction::named(
                    "restore-bookmark",
                    name.map(|name| name.name.as_str()),
                )),
                request_id,
                focus,
                mark_saved: name.cloned(),
            },
        )
    }

    fn verify_bookmark_source(
        &self,
        source: &BookmarkSource,
        deadline: Option<Instant>,
    ) -> Result<(), CoreError> {
        match source {
            BookmarkSource::SessionOnly { generation, .. } if generation == self.generation() => {
                Ok(())
            }
            BookmarkSource::SessionOnly { .. } => Err(refusal(
                "session-only bookmark belongs to another session and is lost on close",
            )),
            BookmarkSource::Durable { source: expected } => {
                let source = self
                    .source_identity()
                    .ok_or_else(|| refusal("this session has no verified path provenance"))?;
                let actual = source.durable_fingerprint(deadline, true)?;
                if actual.as_ref() != Some(expected) {
                    return Err(refusal(
                        "bookmark source fingerprint does not match the loaded graph",
                    ));
                }
                Ok(())
            }
        }
    }

    fn restore_relations(
        &self,
        bookmark: &Bookmark,
        nodes: &[u32],
        deadline: Option<Instant>,
    ) -> Result<Vec<RelationHandle>, CoreError> {
        let mut ids = HashSet::new();
        let mut bytes = 0;
        bookmark
            .relations
            .iter()
            .map(|saved| {
                check_deadline(deadline)?;
                if !ids.insert(saved.edge_id) {
                    return Err(refusal("bookmark repeats a source relation"));
                }
                let source = *nodes
                    .get(saved.source_member as usize)
                    .ok_or_else(|| refusal("bookmark relation endpoint is out of range"))?;
                let target = *nodes
                    .get(saved.target_member as usize)
                    .ok_or_else(|| refusal("bookmark relation endpoint is out of range"))?;
                let index = EdgeIndex::new(saved.edge_id as usize);
                let endpoints = self
                    .graph()
                    .graph
                    .edge_endpoints(index)
                    .ok_or_else(|| refusal("bookmark relation no longer exists"))?;
                let edge = self
                    .graph()
                    .graph
                    .edge_weight(index)
                    .ok_or_else(|| refusal("bookmark relation no longer exists"))?;
                if (endpoints.0.index() as u32, endpoints.1.index() as u32) != (source, target)
                    || edge.connection_type_str(&self.graph().interner) != saved.name
                    || bookmark_members::relation_fingerprint(
                        self.graph(),
                        saved.edge_id,
                        &mut bytes,
                        deadline,
                    )? != saved.attributes_sha256
                {
                    return Err(refusal(
                        "bookmark relation identity, endpoints or attributes changed",
                    ));
                }
                Ok(RelationHandle {
                    generation: self.generation().into(),
                    edge_id: saved.edge_id,
                    source: self.node_handle(source),
                    target: self.node_handle(target),
                })
            })
            .collect()
    }

    fn restore_derived(
        &self,
        bookmark: &Bookmark,
        deadline: Option<Instant>,
    ) -> Result<subset::FrozenFields, CoreError> {
        let mut fields = BTreeMap::new();
        for field in &bookmark.derived {
            let members: Vec<_> = field
                .values
                .iter()
                .map(|(member, _)| member.clone())
                .collect();
            let ids = bookmark_members::resolve_members(
                self.graph(),
                &members,
                self.generation(),
                deadline,
            )?;
            let values = ids
                .into_iter()
                .zip(field.values.iter().map(|(_, value)| value.clone()))
                .collect();
            if fields
                .insert(
                    (field.calculation_id.clone(), field.column.clone()),
                    FrozenField {
                        input_subset_revision: field.input_subset_revision.clone(),
                        values,
                    },
                )
                .is_some()
            {
                return Err(refusal("bookmark repeats a derived field"));
            }
        }
        Ok(fields)
    }
}

pub fn validate_bookmark(bookmark: &Bookmark) -> Result<(), CoreError> {
    if bookmark.version != BOOKMARK_VERSION {
        return Err(refusal("unsupported bookmark version"));
    }
    records::serialized_bytes(bookmark, MAX_BOOKMARK_BYTES)?;
    if bookmark.members.len() > records::MAX_LOADED_NODES
        || bookmark.relations.len() > records::MAX_LOADED_EDGES
    {
        return Err(refusal(
            "bookmark exceeds 5000 nodes or 20000 source relations",
        ));
    }
    for field in [&bookmark.color_by, &bookmark.size_by, &bookmark.caption_by] {
        crate::shared::check_name(field)?;
    }
    subset::validate(&bookmark.predicates)?;
    bookmark.presentation.validate()?;
    let durable = matches!(bookmark.source, BookmarkSource::Durable { .. });
    let members = bookmark.members.iter().chain(
        bookmark
            .derived
            .iter()
            .flat_map(|field| field.values.iter().map(|(member, _)| member)),
    );
    if members
        .into_iter()
        .any(|member| durable != matches!(member, BookmarkMember::Key { .. }))
    {
        return Err(refusal("bookmark mixes durable keys and session handles"));
    }
    if bookmark.derived.len() > 32
        || bookmark.derived.iter().any(|field| {
            field.calculation_id.is_empty()
                || field.calculation_id.len() > 128
                || field.column.is_empty()
                || field.column.len() > 128
        })
    {
        return Err(refusal("bookmark derived fields exceed their bound"));
    }
    Ok(())
}

fn restore_reference(
    session: &Session,
    reference: &BookmarkReference,
    nodes: &[u32],
    view: &View,
) -> Result<ViewReference, CoreError> {
    Ok(match reference {
        BookmarkReference::Member { member } => ViewReference::Node {
            handle: session.node_handle(
                *nodes
                    .get(*member as usize)
                    .ok_or_else(|| refusal("bookmark reference is out of range"))?,
            ),
        },
        BookmarkReference::Type { name } => {
            if view.slot_of_type(name).is_none() {
                return Err(refusal("bookmark schema reference is absent"));
            }
            ViewReference::Type { name: name.clone() }
        }
    })
}
fn restore_references(
    session: &Session,
    references: &[BookmarkReference],
    nodes: &[u32],
    view: &View,
) -> Result<Vec<ViewReference>, CoreError> {
    if references.len() > view.slot_count() as usize {
        return Err(refusal("bookmark selection exceeds the loaded view bound"));
    }
    references
        .iter()
        .map(|reference| restore_reference(session, reference, nodes, view))
        .collect()
}
fn reference_slot(
    session: &Session,
    reference: &BookmarkReference,
    nodes: &[u32],
    view: &View,
) -> Result<u32, CoreError> {
    let slot = match restore_reference(session, reference, nodes, view)? {
        ViewReference::Node { handle } => view.slot_of_node(handle.node_id),
        ViewReference::Type { name } => view.slot_of_type(&name),
    };
    slot.ok_or_else(|| refusal("bookmark reference is not loaded"))
}
fn restore_focus(
    session: &Session,
    focus: Option<&BookmarkFocus>,
    nodes: &[u32],
    view: &View,
) -> Result<Option<Focus>, CoreError> {
    match focus {
        None => Ok(None),
        Some(BookmarkFocus::Fit) => Ok(Some(Focus::new(Vec::new()))),
        Some(BookmarkFocus::References { references }) => {
            restore_references(session, references, nodes, view)?;
            let slots = references
                .iter()
                .map(|reference| reference_slot(session, reference, nodes, view))
                .collect::<Result<_, _>>()?;
            Ok(Some(Focus::new(slots)))
        }
    }
}

fn restore_layout(
    session: &Session,
    saved: &BookmarkLayout,
    nodes: &[u32],
    view: &View,
) -> Result<Option<LayoutResult>, CoreError> {
    if saved.kernel_chosen == crate::request::LayoutKernel::Simulation {
        if !saved.positions.is_empty() {
            return Err(refusal(
                "simulation bookmark cannot contain static coordinates",
            ));
        }
        return Ok(None);
    }
    if !saved.kernel_chosen.is_static() {
        return Err(refusal("bookmark has no concrete static layout kernel"));
    }
    let live = view.live_entries().count();
    if saved.positions.len() != live {
        return Err(refusal(
            "bookmark static layout does not cover the complete restored view",
        ));
    }
    let mut points = vec![f32::NAN; view.slot_count() as usize * 2];
    let mut seen = HashSet::new();
    for position in &saved.positions {
        if !position.x.is_finite() || !position.y.is_finite() {
            return Err(refusal("bookmark contains non-finite coordinates"));
        }
        let slot = reference_slot(session, &position.reference, nodes, view)?;
        if !seen.insert(slot) {
            return Err(refusal("bookmark repeats a static coordinate"));
        }
        points[slot as usize * 2] = position.x;
        points[slot as usize * 2 + 1] = position.y;
    }
    let seed_slot = saved
        .seed
        .as_ref()
        .map(|reference| reference_slot(session, reference, nodes, view))
        .transpose()?;
    Ok(Some(LayoutResult {
        meta: LayoutMeta {
            protocol_version: crate::PROTOCOL_VERSION,
            kernel_requested: saved.kernel_requested,
            kernel_chosen: saved.kernel_chosen,
            seed_slot,
            slot_count: view.slot_count(),
            live_count: live as u32,
            layout_ms: 0.0,
        },
        points,
    }))
}

fn refusal(message: &str) -> CoreError {
    CoreError::Request(message.into())
}
