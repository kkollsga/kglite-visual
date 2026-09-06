//! Rolling shared recovery excludes transient focus and highlight actions.
use std::collections::VecDeque;
use std::sync::Arc;

use serde::Serialize;
use ts_rs::TS;

use crate::control::Appearance;
use crate::render::LayoutResult;
use crate::request::LayoutKernel;
use crate::session::LastSlice;
use crate::shared::{RevisionStamp, ViewReference};
use crate::subset::{FrozenFields, SubsetFilter};
use crate::view::View;
use crate::BoundInfo;

pub const MAX_HISTORY_ENTRIES: usize = 20;
pub const MAX_HISTORY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct HistoryAction {
    pub kind: String,
    pub label: Option<String>,
    pub target: Option<ViewReference>,
    pub node_type: Option<String>,
    pub relationship: Option<String>,
    pub direction: Option<String>,
    pub query: Option<String>,
    pub query_truncated: bool,
    pub nodes: Option<BoundInfo>,
    pub relations: Option<BoundInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct HistoryEntry {
    pub id: String,
    pub before: RevisionStamp,
    pub after: RevisionStamp,
    pub action: HistoryAction,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct HistorySnapshot {
    pub entries: Vec<HistoryEntry>,
    pub oldest_available: Option<String>,
    pub evicted_count: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct HistoryState {
    pub stamp: RevisionStamp,
    pub history: HistorySnapshot,
}

#[derive(Debug, Clone)]
pub(crate) struct RecoveryState {
    pub view: View,
    pub predicates: Vec<SubsetFilter>,
    pub appearance: Appearance,
    pub caption_by: Option<String>,
    pub presentation: crate::presentation::PresentationSettings,
    pub highlighted: Vec<ViewReference>,
    pub selected: Vec<ViewReference>,
    pub layout_kernel: LayoutKernel,
    pub last_layout: Option<LayoutResult>,
    pub last_slice: Option<LastSlice>,
    pub derived: FrozenFields,
    pub calculations: Vec<crate::calculations::CalculationMeta>,
}

#[derive(Debug, Clone)]
struct Checkpoint {
    summary: HistoryEntry,
    state: Arc<RecoveryState>,
    bytes: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RecoveryHistory {
    entries: VecDeque<Checkpoint>,
    bytes: usize,
    evicted: u64,
}

impl RecoveryHistory {
    pub fn snapshot(&self) -> HistorySnapshot {
        HistorySnapshot {
            entries: self
                .entries
                .iter()
                .map(|entry| entry.summary.clone())
                .collect(),
            oldest_available: self.entries.front().map(|entry| entry.summary.id.clone()),
            evicted_count: self.evicted.to_string(),
        }
    }

    pub fn push(&mut self, summary: HistoryEntry, state: RecoveryState, bytes: usize) {
        while !self.entries.is_empty()
            && (self.entries.len() >= MAX_HISTORY_ENTRIES
                || self.bytes.saturating_add(bytes) > MAX_HISTORY_BYTES)
        {
            self.bytes -= self.entries.pop_front().unwrap().bytes;
            self.evicted = self.evicted.saturating_add(1);
        }
        // A single over-limit checkpoint is explicitly unavailable. This cannot
        // remove named bookmarks, which live in a separate catalog.
        if bytes > MAX_HISTORY_BYTES {
            self.evicted = self.evicted.saturating_add(1);
            return;
        }
        self.bytes += bytes;
        self.entries.push_back(Checkpoint {
            summary,
            state: Arc::new(state),
            bytes,
        });
    }

    pub fn get(&self, id: &str) -> Option<Arc<RecoveryState>> {
        self.entries
            .iter()
            .find(|entry| entry.summary.id == id)
            .map(|entry| Arc::clone(&entry.state))
    }
}
