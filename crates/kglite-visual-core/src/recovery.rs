use crate::bookmark::{BookmarkCapture, BookmarkName, SavedViewMarker};
use crate::history::{HistoryAction, HistoryEntry, HistoryState, RecoveryState, MAX_HISTORY_BYTES};
use crate::shared::{PreparedShared, ReplacementContext, RevisionStamp, SharedViewState};
use crate::{CoreError, Session};

impl Session {
    pub fn history_state(&self) -> HistoryState {
        let state = self.state_read();
        HistoryState {
            stamp: state.stamp(self.generation()),
            history: state.history.snapshot(),
        }
    }

    pub fn prepare_history_restore(
        &self,
        id: &str,
        expected: Option<&RevisionStamp>,
        request_id: Option<String>,
    ) -> Result<PreparedShared, CoreError> {
        let before = self.state_read().clone();
        if let Some(expected) = expected {
            crate::shared::check_stamp(expected, &before.stamp(self.generation()))?;
        }
        let saved = before.history.get(id).ok_or_else(|| {
            refusal("recovery checkpoint is no longer available; inspect oldest_available")
        })?;
        let mut replacement = before.clone();
        saved.apply(&mut replacement);
        // Catalog entries can have been deleted after the checkpoint. Recovery
        // restores the exploration without reviving an unverified saved marker.
        replacement.saved_view = None;
        self.prepare_replacement(
            before,
            replacement,
            ReplacementContext {
                action: Some(HistoryAction::named("restore-history", Some(id))),
                restored: true,
                request_id,
                ..Default::default()
            },
        )
    }

    pub fn prepare_bookmark_saved(
        &self,
        name: &BookmarkName,
        capture: &BookmarkCapture,
        expected: Option<&RevisionStamp>,
        request_id: Option<String>,
    ) -> Result<PreparedShared, CoreError> {
        validate_name(name)?;
        let before = self.state_read().clone();
        let actual = before.stamp(self.generation());
        if let Some(expected) = expected {
            crate::shared::check_stamp(expected, &actual)?;
        }
        if capture.captured_stamp.generation != self.generation()
            || capture.content_revision != before.content_revision.to_string()
        {
            return Err(CoreError::Conflict(Box::new(crate::shared::RevisionConflict {
                code: "saved-content-conflict".into(), expected: capture.captured_stamp.clone(), actual,
                message: "bookmark was saved, but the live exploration changed; its saved marker was not applied".into(),
            })));
        }
        let mut replacement = before.clone();
        replacement.selected = capture.captured_selected.clone();
        self.prepare_replacement(
            before,
            replacement,
            ReplacementContext {
                request_id,
                mark_saved: Some(name.clone()),
                ..Default::default()
            },
        )
    }

    pub fn prepare_bookmark_deleted(
        &self,
        name: &BookmarkName,
        expected: Option<&RevisionStamp>,
        request_id: Option<String>,
    ) -> Result<PreparedShared, CoreError> {
        validate_name(name)?;
        let before = self.state_read().clone();
        if let Some(expected) = expected {
            crate::shared::check_stamp(expected, &before.stamp(self.generation()))?;
        }
        let mut replacement = before.clone();
        if replacement
            .saved_view
            .as_ref()
            .is_some_and(|marker| &marker.bookmark == name)
        {
            replacement.saved_view = None;
        }
        self.prepare_replacement(
            before,
            replacement,
            ReplacementContext {
                request_id,
                ..Default::default()
            },
        )
    }

    pub(crate) fn finish_recovery(
        &self,
        before: &SharedViewState,
        context: &ReplacementContext,
    ) -> Result<(), CoreError> {
        let mut state = self.state_write();
        if let Some(action) = &context.action {
            if state.content_revision != before.content_revision {
                let recovery = RecoveryState::capture(before);
                let bytes = recovery.bytes()?;
                let summary = HistoryEntry {
                    id: before.revision.to_string(),
                    before: before.stamp(self.generation()),
                    after: state.stamp(self.generation()),
                    action: action.clone(),
                };
                state.history.push(summary, recovery, bytes);
            }
        }
        if let Some(name) = &context.mark_saved {
            validate_name(name)?;
            state.saved_view = Some(SavedViewMarker {
                bookmark: name.clone(),
                content_revision: state.content_revision.to_string(),
                dirty: false,
                selected: state.selected.clone(),
            });
        }
        Ok(())
    }
}

impl RecoveryState {
    fn capture(state: &SharedViewState) -> Self {
        Self {
            view: state.view.clone(),
            predicates: state.predicates.clone(),
            appearance: state.appearance.clone(),
            caption_by: state.caption_by.clone(),
            presentation: state.presentation.clone(),
            highlighted: state.highlighted.clone(),
            selected: state.selected.clone(),
            layout_kernel: state.layout_kernel,
            last_layout: state.last_layout.clone(),
            last_slice: state.last_slice.clone(),
            derived: state.derived.clone(),
        }
    }
    fn apply(&self, state: &mut SharedViewState) {
        state.view = self.view.clone();
        state.predicates = self.predicates.clone();
        state.appearance = self.appearance.clone();
        state.caption_by = self.caption_by.clone();
        state.presentation = self.presentation.clone();
        state.highlighted = self.highlighted.clone();
        state.selected = self.selected.clone();
        state.layout_kernel = self.layout_kernel;
        state.last_layout = self.last_layout.clone();
        state.last_slice = self.last_slice.clone();
        state.derived = self.derived.clone();
    }
    fn bytes(&self) -> Result<usize, CoreError> {
        let entries: Vec<_> = self.view.entries_with_tombstones().collect();
        let fields: Vec<_> = self
            .derived
            .iter()
            .map(|(key, field)| (key, &field.input_subset_revision, &field.values))
            .collect();
        crate::records::serialized_bytes(
            &(
                &entries,
                self.view.edges(),
                &self.predicates,
                &self.appearance,
                &self.caption_by,
                &self.presentation,
                &self.highlighted,
                &self.selected,
                &self.last_layout,
                &self.last_slice,
                fields,
            ),
            MAX_HISTORY_BYTES,
        )
    }
}

pub(crate) fn same_content(a: &SharedViewState, b: &SharedViewState) -> bool {
    a.view.same_topology(&b.view)
        && a.predicates == b.predicates
        && a.appearance == b.appearance
        && a.caption_by == b.caption_by
        && a.presentation == b.presentation
        && a.highlighted == b.highlighted
        && a.selected == b.selected
        && a.layout_kernel == b.layout_kernel
        && a.derived == b.derived
        && same_layout(a.last_layout.as_ref(), b.last_layout.as_ref())
}
fn same_layout(a: Option<&crate::LayoutResult>, b: Option<&crate::LayoutResult>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.meta == b.meta
                && a.points.len() == b.points.len()
                && a.points
                    .iter()
                    .zip(&b.points)
                    .all(|(a, b)| a.to_bits() == b.to_bits())
        }
        _ => false,
    }
}
fn validate_name(name: &BookmarkName) -> Result<(), CoreError> {
    if name.name.trim().is_empty() || name.name.len() > 128 {
        return Err(refusal("bookmark name must contain 1–128 bytes"));
    }
    Ok(())
}
fn refusal(message: &str) -> CoreError {
    CoreError::Request(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_capacity_evicts_oldest_even_below_twenty_entries() {
        let session = Session::open(std::sync::Arc::new(kglite::api::DirGraph::new()), "history");
        let state = session.state_read();
        let mut history = crate::history::RecoveryHistory::default();
        for (i, bytes) in [8 * 1024 * 1024, 8 * 1024 * 1024, 1]
            .into_iter()
            .enumerate()
        {
            history.push(
                HistoryEntry {
                    id: i.to_string(),
                    before: state.stamp(session.generation()),
                    after: state.stamp(session.generation()),
                    action: HistoryAction::named("test", None),
                },
                RecoveryState::capture(&state),
                bytes,
            );
        }
        let snapshot = history.snapshot();
        assert_eq!(snapshot.entries.len(), 2);
        assert_eq!(snapshot.oldest_available.as_deref(), Some("1"));
        assert_eq!(snapshot.evicted_count, "1");
        assert!(history.get("0").is_none());
    }
}
