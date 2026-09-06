use std::collections::HashMap;
use std::time::Instant;

use crate::bookmark::{
    Bookmark, BookmarkCapture, BookmarkCaptureOptions, BookmarkDerivedField, BookmarkFocus,
    BookmarkFocusRequest, BookmarkLayout, BookmarkMember, BookmarkPosition, BookmarkReference,
    BookmarkSource, BOOKMARK_VERSION, MAX_BOOKMARK_BYTES,
};
use crate::bookmark_members;
use crate::records;
use crate::shared::{RevisionStamp, SharedViewState, ViewReference};
use crate::view::{SlotEntry, View};
use crate::{CoreError, Session};

impl Session {
    /// Cheap eligibility only. Global key uniqueness and source bytes are checked on save.
    pub fn bookmark_eligibility(&self) -> crate::bookmark::BookmarkEligibility {
        let check = || -> Result<(), CoreError> {
            let source = self.source_identity().ok_or_else(|| {
                refusal("in-memory graph bookmarks are session-only and lost on close")
            })?;
            source.verify_loaded()?;
            if !source.has_durable_identity() {
                return Err(refusal(
                    "legacy disk directory has no reliable published-generation identity",
                ));
            }
            let state = self.state_read().clone();
            let _guard = self.graph().begin_read_pass();
            let mut keys = std::collections::HashSet::new();
            for (_, entry) in state.view.live_entries() {
                if let SlotEntry::Node {
                    node_id, node_type, ..
                } = entry
                {
                    let key = bookmark_members::scalar_key(self.graph(), *node_id)?;
                    let token = serde_json::to_string(&(node_type, key))
                        .map_err(|error| CoreError::Request(error.to_string()))?;
                    if !keys.insert(token) {
                        return Err(refusal("loaded members have duplicate source keys"));
                    }
                }
            }
            Ok(())
        };
        match check() {
            Ok(()) => crate::bookmark::BookmarkEligibility {
                storage: crate::bookmark::BookmarkStorage::Durable,
                reason: None,
                verification_required: true,
            },
            Err(error) => crate::bookmark::BookmarkEligibility {
                storage: crate::bookmark::BookmarkStorage::Session,
                reason: Some(error.to_string()),
                verification_required: false,
            },
        }
    }
    pub fn capture_bookmark(
        &self,
        expected: Option<&RevisionStamp>,
    ) -> Result<BookmarkCapture, CoreError> {
        self.capture_bookmark_with(&BookmarkCaptureOptions::default(), expected)
    }

    pub fn capture_bookmark_with(
        &self,
        options: &BookmarkCaptureOptions,
        expected: Option<&RevisionStamp>,
    ) -> Result<BookmarkCapture, CoreError> {
        let mut captured = self.state_read().clone();
        let stamp = captured.stamp(self.generation());
        if let Some(expected) = expected {
            crate::shared::check_stamp(expected, &stamp)?;
        }
        if let Some(selected) = &options.selected {
            self.validate_bookmark_references(selected, &captured.view)?;
            captured.selected = selected.clone();
        }
        let deadline = self.config().deadline();
        let _guard = self.graph().begin_read_pass();
        let durable = self
            .source_identity()
            .ok_or_else(|| refusal("in-memory graph has no verified file provenance"))
            .and_then(|source| source.durable_fingerprint(deadline, false))
            .and_then(|fingerprint| {
                fingerprint.ok_or_else(|| {
                    refusal("legacy disk directory has no immutable published-generation identity")
                })
            })
            .and_then(|fingerprint| {
                bookmark_members::durable_members(self.graph(), &captured.view, deadline)
                    .map(|members| (fingerprint, members))
            });
        let (source, members) = match durable {
            Ok((source, members)) => (BookmarkSource::Durable { source }, members),
            Err(error) => (
                BookmarkSource::SessionOnly {
                    generation: self.generation().into(),
                    reason: error.to_string(),
                },
                bookmark_members::session_members(&captured.view, self.generation()),
            ),
        };
        let mut bookmark = self.capture_content(&captured, source, members, deadline)?;
        bookmark.focus = match &options.focus {
            Some(BookmarkFocusRequest::Fit) => Some(BookmarkFocus::Fit),
            Some(BookmarkFocusRequest::References { references }) => {
                self.validate_bookmark_references(references, &captured.view)?;
                Some(BookmarkFocus::References {
                    references: capture_references(references, &member_map(&captured.view))?,
                })
            }
            None => None,
        };
        records::serialized_bytes(&bookmark, MAX_BOOKMARK_BYTES)?;
        Ok(BookmarkCapture {
            captured_selected: captured.selected.clone(),
            durability: bookmark.source.clone(),
            bookmark,
            captured_stamp: stamp,
            content_revision: captured.content_revision.to_string(),
        })
    }

    fn validate_bookmark_references(
        &self,
        references: &[ViewReference],
        view: &View,
    ) -> Result<(), CoreError> {
        if references.len() > view.slot_count() as usize {
            return Err(refusal("bookmark selection exceeds the loaded view bound"));
        }
        for reference in references {
            let exists = match reference {
                ViewReference::Node { handle } => {
                    handle.generation == self.generation()
                        && view.slot_of_node(handle.node_id).is_some()
                }
                ViewReference::Type { name } => view.slot_of_type(name).is_some(),
            };
            if !exists {
                return Err(refusal("selected reference is not in the captured loaded view; load it or clear the selection before saving"));
            }
        }
        Ok(())
    }

    fn capture_content(
        &self,
        state: &SharedViewState,
        source: BookmarkSource,
        members: Vec<BookmarkMember>,
        deadline: Option<Instant>,
    ) -> Result<Bookmark, CoreError> {
        let nodes = member_map(&state.view);
        let relations =
            bookmark_members::capture_relations(self.graph(), &state.view, &nodes, deadline)?;
        let derived = self.capture_derived(
            state,
            matches!(source, BookmarkSource::Durable { .. }),
            deadline,
        )?;
        Ok(Bookmark {
            version: BOOKMARK_VERSION,
            source,
            members,
            relations,
            predicates: state.predicates.clone(),
            color_by: state.appearance.color_by.clone(),
            size_by: state.appearance.size_by.clone(),
            caption_by: state.caption_by.clone(),
            presentation: state.presentation.clone(),
            highlighted: capture_references(&state.highlighted, &nodes)?,
            selected: capture_references(&state.selected, &nodes)?,
            focus: None,
            layout: capture_layout(state, &nodes)?,
            derived,
        })
    }

    fn capture_derived(
        &self,
        state: &SharedViewState,
        durable: bool,
        deadline: Option<Instant>,
    ) -> Result<Vec<BookmarkDerivedField>, CoreError> {
        state
            .derived
            .iter()
            .map(|((calculation_id, column), field)| {
                let values = field
                    .values
                    .iter()
                    .map(|(id, value)| {
                        let member = if durable {
                            let node = self
                                .graph()
                                .node_view(kglite::api::NodeIndex::new(*id as usize))
                                .ok_or_else(|| refusal("frozen field source node is absent"))?;
                            BookmarkMember::Key {
                                node_type: node.node_type_str(&self.graph().interner).into(),
                                key: bookmark_members::scalar_key(self.graph(), *id)?,
                            }
                        } else {
                            BookmarkMember::Handle {
                                handle: self.node_handle(*id),
                            }
                        };
                        Ok((member, value.clone()))
                    })
                    .collect::<Result<Vec<_>, CoreError>>()?;
                let members: Vec<_> = values.iter().map(|(member, _)| member.clone()).collect();
                bookmark_members::resolve_members(
                    self.graph(),
                    &members,
                    self.generation(),
                    deadline,
                )?;
                Ok(BookmarkDerivedField {
                    calculation_id: calculation_id.clone(),
                    column: column.clone(),
                    input_subset_revision: field.input_subset_revision.clone(),
                    values,
                })
            })
            .collect()
    }
}

fn member_map(view: &View) -> HashMap<u32, u32> {
    view.live_entries()
        .filter_map(|(_, entry)| match entry {
            SlotEntry::Node { node_id, .. } => Some(*node_id),
            _ => None,
        })
        .enumerate()
        .map(|(member, id)| (id, member as u32))
        .collect()
}

fn capture_reference(
    reference: &ViewReference,
    nodes: &HashMap<u32, u32>,
) -> Result<BookmarkReference, CoreError> {
    Ok(match reference {
        ViewReference::Node { handle } => BookmarkReference::Member {
            member: *nodes
                .get(&handle.node_id)
                .ok_or_else(|| refusal("saved reference names an unloaded member"))?,
        },
        ViewReference::Type { name } => BookmarkReference::Type { name: name.clone() },
    })
}
fn capture_references(
    references: &[ViewReference],
    nodes: &HashMap<u32, u32>,
) -> Result<Vec<BookmarkReference>, CoreError> {
    references
        .iter()
        .map(|reference| capture_reference(reference, nodes))
        .collect()
}

fn capture_layout(
    state: &SharedViewState,
    nodes: &HashMap<u32, u32>,
) -> Result<BookmarkLayout, CoreError> {
    let Some(layout) = &state.last_layout else {
        return Ok(BookmarkLayout {
            kernel_requested: state.layout_kernel,
            kernel_chosen: state.layout_kernel,
            seed: None,
            positions: Vec::new(),
        });
    };
    let mut seed = None;
    let mut positions = Vec::new();
    for (slot, entry) in state.view.live_entries() {
        let reference = match entry {
            SlotEntry::Type { name } => BookmarkReference::Type { name: name.clone() },
            SlotEntry::Node { node_id, .. } => BookmarkReference::Member {
                member: nodes[node_id],
            },
            SlotEntry::Tombstone => continue,
        };
        let x = *layout
            .points
            .get(slot as usize * 2)
            .ok_or_else(|| refusal("static layout is incomplete"))?;
        let y = *layout
            .points
            .get(slot as usize * 2 + 1)
            .ok_or_else(|| refusal("static layout is incomplete"))?;
        if !x.is_finite() || !y.is_finite() {
            return Err(refusal(
                "static layout contains a non-finite live coordinate",
            ));
        }
        if layout.meta.seed_slot == Some(slot) {
            seed = Some(reference.clone());
        }
        positions.push(BookmarkPosition { reference, x, y });
    }
    Ok(BookmarkLayout {
        kernel_requested: layout.meta.kernel_requested,
        kernel_chosen: layout.meta.kernel_chosen,
        seed,
        positions,
    })
}

fn refusal(message: &str) -> CoreError {
    CoreError::Request(message.into())
}
