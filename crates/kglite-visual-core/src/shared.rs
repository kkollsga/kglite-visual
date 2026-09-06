//! Prepared shared transactions, coherent snapshots and transport-neutral events.
use std::ops::{Deref, DerefMut};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::control::{Appearance, Focus, HighlightConcept};
use crate::records::{self, NodeHandle};
use crate::render::{LayoutMeta, LayoutResult};
use crate::request::{LayoutKernel, Request};
use crate::session::{LastSlice, Response, Session};
use crate::subset::{self, FrozenFields, SubsetFilter, SubsetSnapshot};
use crate::view::{GraphSliceMeta, SliceKind, SlotEntry, View};
use crate::CoreError;

pub const MAX_SHARED_EVENT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct RevisionStamp {
    pub generation: String,
    pub revision: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SharedRequest {
    #[serde(flatten)]
    pub request: Request,
    #[serde(default)]
    #[ts(optional)]
    pub expected: Option<RevisionStamp>,
    #[serde(default)]
    #[ts(optional)]
    pub request_id: Option<String>,
}
impl SharedRequest {
    pub fn new(request: Request) -> Self {
        Self {
            request,
            expected: None,
            request_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ViewReference {
    Node { handle: NodeHandle },
    Type { name: String },
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct CaptionRequest {
    pub caption_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SharedSnapshotMeta {
    pub stamp: RevisionStamp,
    pub topology_revision: String,
    pub subset_revision: String,
    pub content_revision: String,
    pub saved_view: Option<crate::bookmark::SavedViewMarker>,
    pub history: crate::history::HistorySnapshot,
    pub presentation: crate::presentation::PresentationSettings,
    pub slice: GraphSliceMeta,
    pub subset: SubsetSnapshot,
    pub appearance: Appearance,
    pub caption_by: Option<String>,
    pub highlighted: Vec<ViewReference>,
    pub selected: Vec<ViewReference>,
    pub layout_kernel: LayoutKernel,
    pub layout: Option<LayoutMeta>,
    pub last_slice: Option<LastSlice>,
}
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SharedSnapshot {
    pub meta: SharedSnapshotMeta,
    pub points: Vec<f32>,
    pub links: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SharedWireMeta {
    pub restored: bool,
    pub compacted: bool,
    pub snapshot: SharedSnapshotMeta,
    pub request_id: Option<String>,
    pub focus: Option<Focus>,
    pub mutation_kind: Option<SliceKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, thiserror::Error)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[error("{message}")]
pub struct RevisionConflict {
    pub code: String,
    pub expected: RevisionStamp,
    pub actual: RevisionStamp,
    pub message: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SharedViewState {
    pub view: View,
    pub revision: u64,
    pub topology_revision: u64,
    pub subset_revision: u64,
    pub content_revision: u64,
    pub saved_view: Option<crate::bookmark::SavedViewMarker>,
    pub history: crate::history::RecoveryHistory,
    pub presentation: crate::presentation::PresentationSettings,
    pub predicates: Vec<SubsetFilter>,
    pub subset: SubsetSnapshot,
    pub appearance: Appearance,
    pub caption_by: Option<String>,
    pub highlighted: Vec<ViewReference>,
    pub selected: Vec<ViewReference>,
    pub layout_kernel: LayoutKernel,
    pub last_layout: Option<LayoutResult>,
    pub last_slice: Option<LastSlice>,
    pub derived: FrozenFields,
}
impl SharedViewState {
    pub fn new(view: View) -> Self {
        Self {
            view,
            revision: 0,
            topology_revision: 0,
            subset_revision: 0,
            content_revision: 0,
            saved_view: None,
            history: Default::default(),
            presentation: Default::default(),
            predicates: Vec::new(),
            subset: SubsetSnapshot::default(),
            appearance: Appearance::new(None, None),
            caption_by: None,
            highlighted: Vec::new(),
            selected: Vec::new(),
            layout_kernel: LayoutKernel::Simulation,
            last_layout: None,
            last_slice: None,
            derived: FrozenFields::new(),
        }
    }
    pub(crate) fn stamp(&self, generation: &str) -> RevisionStamp {
        RevisionStamp {
            generation: generation.into(),
            revision: self.revision.to_string(),
        }
    }
}
impl Deref for SharedViewState {
    type Target = View;
    fn deref(&self) -> &View {
        &self.view
    }
}
impl DerefMut for SharedViewState {
    fn deref_mut(&mut self) -> &mut View {
        &mut self.view
    }
}

pub struct PreparedShared {
    restored: bool,
    base: RevisionStamp,
    state: SharedViewState,
    snapshot: SharedSnapshot,
    response: Response,
    focus: Option<Focus>,
    request_id: Option<String>,
}

#[derive(Default)]
pub(crate) struct ReplacementContext {
    pub restored: bool,
    pub action: Option<crate::history::HistoryAction>,
    pub focus: Option<Focus>,
    pub request_id: Option<String>,
    pub mark_saved: Option<crate::bookmark::BookmarkName>,
}
#[derive(Debug, Clone)]
pub struct CommittedEvent {
    pub restored: bool,
    pub snapshot: SharedSnapshot,
    pub response: Response,
    pub focus: Option<Focus>,
    pub request_id: Option<String>,
}
impl CommittedEvent {
    pub fn wire_meta(&self) -> SharedWireMeta {
        SharedWireMeta {
            restored: self.restored,
            compacted: matches!(&self.response,Response::Slice(slice) if slice.compaction.is_some()),
            snapshot: self.snapshot.meta.clone(),
            request_id: self.request_id.clone(),
            focus: self.focus.clone(),
            mutation_kind: match &self.response {
                Response::Slice(slice) => Some(slice.meta.kind),
                _ => None,
            },
        }
    }
}

impl Session {
    pub fn shared_stamp(&self) -> RevisionStamp {
        self.state_read().stamp(self.generation())
    }
    pub fn prepare_shared(&self, request: &SharedRequest) -> Result<PreparedShared, CoreError> {
        if !request.request.is_shared() {
            return Err(CoreError::Request(
                "private reads do not enter the shared mutation path".into(),
            ));
        }
        if request.request_id.as_ref().is_some_and(|id| id.len() > 128) {
            return Err(CoreError::Request("request_id exceeds 128 bytes".into()));
        }
        let before = self.state_read().clone();
        let base = before.stamp(self.generation());
        if let Some(expected) = &request.expected {
            check_stamp(expected, &base)?;
        }
        let candidate = self.fork_state(before.clone());
        let response = candidate.handle_uncommitted(&request.request)?;
        let focus = match &request.request {
            Request::Focus(request) => Some(Focus::new(request.slots.clone())),
            _ => None,
        };
        let action = crate::history::HistoryAction::for_request(
            &request.request,
            &before,
            self.generation(),
            &response,
        );
        self.prepared_candidate(
            before,
            candidate,
            response,
            ReplacementContext {
                restored: false,
                action,
                focus,
                request_id: request.request_id.clone(),
                mark_saved: None,
            },
            false,
        )
    }

    pub(crate) fn prepare_replacement(
        &self,
        before: SharedViewState,
        replacement: SharedViewState,
        context: ReplacementContext,
    ) -> Result<PreparedShared, CoreError> {
        let candidate = self.fork_state(replacement);
        let response = Response::Shared(Box::new(candidate.snapshot_direct()));
        self.prepared_candidate(before, candidate, response, context, true)
    }

    fn prepared_candidate(
        &self,
        before: SharedViewState,
        candidate: Session,
        response: Response,
        context: ReplacementContext,
        preserve_layout: bool,
    ) -> Result<PreparedShared, CoreError> {
        if context.request_id.as_ref().is_some_and(|id| id.len() > 128) {
            return Err(CoreError::Request("request_id exceeds 128 bytes".into()));
        }
        let base = before.stamp(self.generation());
        candidate.finish_preparation(&before, preserve_layout)?;
        candidate.finish_recovery(&before, &context)?;
        let snapshot = candidate.snapshot_direct();
        let response = if matches!(response, Response::Shared(_)) {
            Response::Shared(Box::new(snapshot.clone()))
        } else {
            response
        };
        let wire = SharedWireMeta {
            restored: context.restored,
            compacted: matches!(&response,Response::Slice(slice) if slice.compaction.is_some()),
            snapshot: snapshot.meta.clone(),
            request_id: context.request_id.clone(),
            focus: context.focus.clone(),
            mutation_kind: match &response {
                Response::Slice(slice) => Some(slice.meta.kind),
                _ => None,
            },
        };
        let arrays = (snapshot.points.len() + snapshot.links.len()) * 4;
        records::serialized_bytes(&wire, MAX_SHARED_EVENT_BYTES.saturating_sub(arrays + 1024))?;
        let state = candidate.state_read().clone();
        Ok(PreparedShared {
            restored: context.restored,
            base,
            state,
            snapshot,
            response,
            focus: context.focus,
            request_id: context.request_id,
        })
    }
    pub fn commit_shared(&self, prepared: PreparedShared) -> Result<CommittedEvent, CoreError> {
        let mut state = self.state_write();
        check_stamp(&prepared.base, &state.stamp(self.generation()))?;
        *state = prepared.state;
        Ok(CommittedEvent {
            restored: prepared.restored,
            snapshot: prepared.snapshot,
            response: prepared.response,
            focus: prepared.focus,
            request_id: prepared.request_id,
        })
    }
    pub fn apply_shared(&self, request: &SharedRequest) -> Result<CommittedEvent, CoreError> {
        self.commit_shared(self.prepare_shared(request)?)
    }
    pub fn snapshot_shared(&self) -> SharedSnapshot {
        let captured = self.state_read().clone();
        self.fork_state(captured).snapshot_direct()
    }
    fn snapshot_direct(&self) -> SharedSnapshot {
        let slice = self.sync_slice();
        let state = self.state_read();
        let points = state
            .last_layout
            .as_ref()
            .map(|layout| layout.points.clone())
            .unwrap_or(slice.points);
        SharedSnapshot {
            meta: SharedSnapshotMeta {
                stamp: state.stamp(self.generation()),
                topology_revision: state.topology_revision.to_string(),
                subset_revision: state.subset_revision.to_string(),
                content_revision: state.content_revision.to_string(),
                saved_view: state.saved_view.clone(),
                history: state.history.snapshot(),
                presentation: state.presentation.clone(),
                slice: slice.meta,
                subset: state.subset.clone(),
                appearance: state.appearance.clone(),
                caption_by: state.caption_by.clone(),
                highlighted: state.highlighted.clone(),
                selected: state.selected.clone(),
                layout_kernel: state.layout_kernel,
                layout: state.last_layout.as_ref().map(|layout| layout.meta.clone()),
                last_slice: state.last_slice.clone(),
            },
            points,
            links: slice.links,
        }
    }
    fn finish_preparation(
        &self,
        before: &SharedViewState,
        preserve_layout: bool,
    ) -> Result<(), CoreError> {
        let mut state = self.state_write();
        let topology_changed = !state.view.same_topology(&before.view);
        if topology_changed
            || state.predicates != before.predicates
            || state.derived != before.derived
        {
            state.subset = subset::evaluate(
                self.graph(),
                self.generation(),
                &state.view,
                &state.predicates,
                &state.derived,
                self.config().deadline(),
            )?;
        }
        let subset_changed = topology_changed
            || state.subset.visible_nodes != before.subset.visible_nodes
            || state.subset.visible_edge_ids != before.subset.visible_edge_ids;
        state.revision = next(before.revision)?;
        state.topology_revision = if topology_changed {
            next(before.topology_revision)?
        } else {
            before.topology_revision
        };
        state.subset_revision = if subset_changed {
            next(before.subset_revision)?
        } else {
            before.subset_revision
        };
        if topology_changed && !preserve_layout {
            state.layout_kernel = LayoutKernel::Simulation;
            state.last_layout = None;
        }
        state.highlighted = retained_references(&state.highlighted, &state.view);
        state.selected = retained_references(&state.selected, &state.view);
        state.content_revision = if crate::recovery::same_content(&state, before) {
            before.content_revision
        } else {
            next(before.content_revision)?
        };
        let content_revision = state.content_revision.to_string();
        if let Some(marker) = &mut state.saved_view {
            marker.dirty = marker.content_revision != content_revision;
        }
        Ok(())
    }
    pub(crate) fn settings_uncommitted(&self, request: &Request) -> Result<Response, CoreError> {
        match request {
            Request::Subset(request) => {
                subset::validate(&request.predicates)?;
                self.state_write().predicates = request.predicates.clone();
            }
            Request::Appearance(request) => {
                check_name(&request.color_by)?;
                check_name(&request.size_by)?;
                self.state_write().appearance =
                    Appearance::new(request.color_by.clone(), request.size_by.clone());
            }
            Request::Caption(request) => {
                check_name(&request.caption_by)?;
                self.state_write().caption_by = request.caption_by.clone();
            }
            Request::Focus(request) => self.check_live_slots(&request.slots)?,
            Request::Highlight(request) => {
                self.check_live_slots(&request.slots)?;
                let mut state = self.state_write();
                let refs = request
                    .slots
                    .iter()
                    .filter_map(|slot| reference(&state.view, *slot, self.generation()))
                    .collect();
                match request.concept {
                    HighlightConcept::Highlighted => state.highlighted = refs,
                    HighlightConcept::Selected => state.selected = refs,
                }
            }
            _ => return Err(CoreError::Request("not a settings request".into())),
        }
        Ok(Response::Shared(Box::new(self.snapshot_direct())))
    }
}
fn next(value: u64) -> Result<u64, CoreError> {
    value
        .checked_add(1)
        .ok_or_else(|| CoreError::Request("shared revision exhausted".into()))
}
pub fn check_stamp(expected: &RevisionStamp, actual: &RevisionStamp) -> Result<(), CoreError> {
    if expected != actual {
        return Err(CoreError::Conflict(Box::new(RevisionConflict {
            code: "revision-conflict".into(),
            expected: expected.clone(),
            actual: actual.clone(),
            message:
                "shared view changed; refresh and apply the action against the current revision"
                    .into(),
        })));
    }
    Ok(())
}
pub(crate) fn check_name(name: &Option<String>) -> Result<(), CoreError> {
    if name
        .as_ref()
        .is_some_and(|name| name.is_empty() || name.len() > 256)
    {
        return Err(CoreError::Request(
            "field name must contain 1–256 bytes".into(),
        ));
    }
    Ok(())
}
fn reference(view: &View, slot: u32, generation: &str) -> Option<ViewReference> {
    match view.entry(slot)? {
        SlotEntry::Node { node_id, .. } => Some(ViewReference::Node {
            handle: NodeHandle {
                generation: generation.into(),
                node_id: *node_id,
            },
        }),
        SlotEntry::Type { name } => Some(ViewReference::Type { name: name.clone() }),
        SlotEntry::Tombstone => None,
    }
}
fn retained_references(refs: &[ViewReference], view: &View) -> Vec<ViewReference> {
    refs.iter()
        .filter(|reference| match reference {
            ViewReference::Node { handle } => view.slot_of_node(handle.node_id).is_some(),
            ViewReference::Type { name } => view.slot_of_type(name).is_some(),
        })
        .cloned()
        .collect()
}

pub fn shared_frames(meta: &SharedWireMeta, points: &[f32], links: &[f32]) -> Vec<Vec<u8>> {
    let mut encoder = crate::ResponseEncoder::new();
    encoder.push_json(
        crate::MessageType::SharedUpdate,
        &serde_json::to_string(meta).expect("shared metadata serializes"),
    );
    encoder.push_f32(crate::MessageType::Points, points);
    encoder.push_f32(crate::MessageType::Links, links);
    encoder.finish()
}
