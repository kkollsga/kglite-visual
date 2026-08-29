//! What is currently on screen, in the slot space (plan D4).
//!
//! The meta-graph's type nodes and the instance nodes an expansion appends are
//! **one** index space, from one allocator — the identity contract D12 fixed
//! before either existed. A link between a type node and an instance node is
//! therefore just a pair of indices, and no renderer call site has to know
//! which kind of thing an index names.
//!
//! Three operations move the space, and the third exists because the first two
//! cannot be undone:
//!
//! - **expand** appends. Slots are never reused, so a client that still holds a
//!   reference to slot 42 is never handed a different node under that number.
//! - **collapse** writes a [`SlotEntry::Tombstone`]. On the wire that is a NaN
//!   position, which is cosmos.gl's own absence semantics: the point stops
//!   drawing and the links touching it are faded by the renderer, with no
//!   re-indexing of anything else.
//! - **compaction** is an explicit protocol op with an old→new remap. It is the
//!   only thing that reuses an index, it never happens implicitly, and the
//!   client applies the remap to its own id↔slot map before the next frame. A
//!   compaction the client did not hear about would silently re-label every
//!   selection it holds.

use std::collections::HashMap;

use serde::Serialize;
use ts_rs::TS;

use crate::bound::BoundInfo;
use crate::slots::SlotAllocator;

/// Tombstone ratio past which a compaction is worth its round trip.
///
/// A tombstone costs one slot of every array the renderer holds, so a view that
/// has collapsed most of itself pays for a graph it is no longer showing. 30%
/// is deliberately not tight: compaction invalidates every index the client is
/// holding, so doing it often is worse than carrying some waste.
///
/// **Measured, 2026-08-29, and kept.** Collapsing a slice at the node bound —
/// 5 005 slots down to 5 — costs 58–64 ms end to end in the real app (mean of
/// first events over three cold pages): the server's answer, the remap, and a
/// full re-upload of every array. That is three to four frame budgets, so a
/// threshold that fired often would trade a steady 60 fps for a visible stall,
/// while the waste it would reclaim is a fraction of one upload. The number
/// stays where the round trip is rare.
pub const COMPACTION_TOMBSTONE_RATIO: f32 = 0.30;

/// Below this many slots, compaction is never worth a round trip whatever the
/// ratio says — a five-slot meta-graph with two tombstones is 40% waste and
/// eight bytes.
pub const COMPACTION_MIN_SLOTS: usize = 64;

/// What one slot holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotEntry {
    /// A meta-graph type node.
    Type { name: String },
    /// An instance node, identified by kglite's own node index.
    ///
    /// The title is carried here, not only in the slice that added it. A view
    /// that cannot name what is in it can be counted but not described, and
    /// P10 gave it two consumers that need the names: `view_state`, which is
    /// what an agent reads instead of the screen, and the live-view render,
    /// which draws the labels.
    Node {
        node_id: u32,
        node_type: String,
        title: String,
    },
    /// Collapsed. Rendered as a NaN position; never reused without a
    /// [`Compaction`].
    Tombstone,
}

/// One link in the view, in slot space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ViewEdge {
    pub source_slot: u32,
    pub target_slot: u32,
    /// Relationship type, or the meta-graph's relationship-type name.
    pub name: String,
    /// True for a meta-graph type↔type link, false for a real instance edge.
    /// The client draws the two differently and the count means different
    /// things: a meta link stands for `count` real edges, an instance link for
    /// exactly one.
    pub meta: bool,
}

/// Why a slice was produced. The client shows a different banner for each, and
/// a collapse that arrived looking like an expansion would read as data loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "kebab-case")]
pub enum SliceKind {
    Expand,
    Collapse,
    Query,
    Search,
}

/// One instance node a slice added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SliceNode {
    pub slot: u32,
    /// kglite's own node index — the id every later request names this node by.
    pub node_id: u32,
    pub node_type: String,
    /// The node's title field, for the label overlay. Empty when the type has
    /// no title: an empty label is honest, a fabricated one is not.
    pub title: String,
}

/// The metadata half of a graph slice.
///
/// Split from the float arrays for the same reason the meta-graph is (test-plan
/// §2): one encoder feeds the binary path and the JSON twin, and the arrays are
/// the same bytes either way.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct GraphSliceMeta {
    pub protocol_version: u32,
    pub kind: SliceKind,
    /// Slot the `points` array begins at. An expansion appends, so this is the
    /// first new slot and the client splices rather than reuploading a view it
    /// already has.
    pub first_slot: u32,
    /// The nodes this slice added, in slot order from `first_slot`.
    pub nodes: Vec<SliceNode>,
    /// Slots to write NaN at (cosmos.gl absence semantics). Only a collapse
    /// fills this.
    pub tombstones: Vec<u32>,
    /// **Every** link in the view, not just the new ones. D4: links are always
    /// re-sent whole, because `setLinks` replaces the buffer and a partial
    /// upload would silently drop everything it omitted.
    pub edges: Vec<ViewEdge>,
    /// Slots allocated so far, tombstones included.
    pub slot_count: u32,
    pub tombstone_count: u32,
    /// What the bound did to this response (D5). Present whether or not it
    /// fired: "all 12 of 12" is information the UI needs the rest of the time.
    pub bound: BoundInfo,
    /// What the bound did to *this slice's* links — not to `edges`, which
    /// re-sends the whole view.
    ///
    /// The byte budget is shared between nodes and links
    /// (`expand::MAX_EXPANSION_BYTES`), so a dense relationship spends it on
    /// links and the slice arrives with fewer nodes *and* fewer links than the
    /// walk found. A node whose edges were cut is indistinguishable from an
    /// isolated one unless the response says so; this is where it says so.
    pub link_bound: BoundInfo,
}

/// An old→new slot remap.
///
/// `old_to_new[i]` is where slot `i` moved, or `None` for a slot that was
/// dropped. Sent whole rather than as a sparse diff: the client has to rewrite
/// every entry of its id↔slot map anyway, and a sparse form would let it
/// believe an unmentioned slot was unchanged when in fact it moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct Compaction {
    pub protocol_version: u32,
    /// One entry per pre-compaction slot: the new index, or `null` if the slot
    /// was a tombstone and is gone.
    pub old_to_new: Vec<Option<u32>>,
    /// Slots after compaction.
    pub slot_count: u32,
    /// Tombstones reclaimed.
    pub reclaimed: u32,
}

/// The slot space and everything currently in it.
#[derive(Debug, Default)]
pub struct View {
    slots: SlotAllocator,
    entries: Vec<SlotEntry>,
    node_slot: HashMap<u32, u32>,
    type_slot: HashMap<String, u32>,
    edges: Vec<ViewEdge>,
    tombstones: usize,
}

impl View {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn slot_count(&self) -> u32 {
        self.slots.len()
    }

    pub fn tombstone_count(&self) -> u32 {
        self.tombstones as u32
    }

    pub fn edges(&self) -> &[ViewEdge] {
        &self.edges
    }

    pub fn entry(&self, slot: u32) -> Option<&SlotEntry> {
        self.entries.get(slot as usize)
    }

    /// Slot of a type node, if the meta-graph put one on screen.
    pub fn slot_of_type(&self, name: &str) -> Option<u32> {
        self.type_slot.get(name).copied()
    }

    /// Slot of an instance node, if it has been expanded into the view.
    pub fn slot_of_node(&self, node_id: u32) -> Option<u32> {
        self.node_slot.get(&node_id).copied()
    }

    /// Allocate a slot for a type node. Idempotent: a type already on screen
    /// keeps the slot it has.
    pub fn intern_type(&mut self, name: &str) -> u32 {
        if let Some(slot) = self.type_slot.get(name) {
            return *slot;
        }
        let slot = self.slots.alloc();
        self.entries.push(SlotEntry::Type {
            name: name.to_string(),
        });
        self.type_slot.insert(name.to_string(), slot);
        slot
    }

    /// Allocate a slot for an instance node, or return the one it already has.
    ///
    /// Returns `(slot, is_new)` — the caller needs to know whether to send a
    /// position for it, and "already present" is the common case once two
    /// expansions overlap.
    pub fn intern_node(&mut self, node_id: u32, node_type: &str, title: &str) -> (u32, bool) {
        if let Some(slot) = self.node_slot.get(&node_id) {
            return (*slot, false);
        }
        let slot = self.slots.alloc();
        self.entries.push(SlotEntry::Node {
            node_id,
            node_type: node_type.to_string(),
            title: title.to_string(),
        });
        self.node_slot.insert(node_id, slot);
        (slot, true)
    }

    /// Add a link, skipping one already present.
    ///
    /// Deduplicated because two expansions of overlapping neighbourhoods find
    /// the same edge, and cosmos.gl draws a duplicated link twice — which reads
    /// as a heavier relationship rather than as a bug.
    pub fn add_edge(&mut self, edge: ViewEdge) {
        if self.edges.contains(&edge) {
            return;
        }
        self.edges.push(edge);
    }

    /// Every occupied slot and what is in it, in slot order.
    ///
    /// Tombstones are skipped: a caller iterating this is describing what is on
    /// screen, and a hole is not on screen.
    pub fn live_entries(&self) -> impl Iterator<Item = (u32, &SlotEntry)> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| !matches!(entry, SlotEntry::Tombstone))
            .map(|(slot, entry)| (slot as u32, entry))
    }

    /// Tombstone every instance slot, whatever its type — the entry screen,
    /// restored.
    ///
    /// The type nodes stay. Collapsing those too would leave the user with a
    /// blank canvas and nothing to navigate from, which is not "reset" but
    /// "close".
    pub fn tombstone_all_instances(&mut self) -> Vec<u32> {
        let doomed: Vec<u32> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(slot, entry)| match entry {
                SlotEntry::Node { .. } => Some(slot as u32),
                _ => None,
            })
            .collect();
        self.tombstone(&doomed)
    }

    /// Tombstone every instance slot of `node_type`, and drop the links that
    /// touched them.
    ///
    /// Returns the tombstoned slots, in ascending order — the client writes NaN
    /// at exactly these and nowhere else.
    pub fn collapse_type(&mut self, node_type: &str) -> Vec<u32> {
        let doomed: Vec<u32> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(slot, entry)| match entry {
                SlotEntry::Node { node_type: t, .. } if t == node_type => Some(slot as u32),
                _ => None,
            })
            .collect();
        self.tombstone(&doomed);
        doomed
    }

    /// Tombstone specific slots. A slot that is already a tombstone, or is a
    /// meta type node, is left alone: collapsing the entry screen would leave
    /// the user with nothing to navigate from.
    pub fn tombstone(&mut self, slots: &[u32]) -> Vec<u32> {
        let mut hit = Vec::new();
        for slot in slots {
            let Some(entry) = self.entries.get_mut(*slot as usize) else {
                continue;
            };
            let SlotEntry::Node { node_id, .. } = entry else {
                continue;
            };
            self.node_slot.remove(node_id);
            *entry = SlotEntry::Tombstone;
            self.tombstones += 1;
            hit.push(*slot);
        }
        if !hit.is_empty() {
            self.edges
                .retain(|e| !hit.contains(&e.source_slot) && !hit.contains(&e.target_slot));
        }
        hit
    }

    /// True when enough of the space is dead to be worth an explicit remap.
    pub fn should_compact(&self) -> bool {
        let total = self.entries.len();
        total >= COMPACTION_MIN_SLOTS
            && (self.tombstones as f32) / (total as f32) >= COMPACTION_TOMBSTONE_RATIO
    }

    /// Reclaim every tombstone, renumbering the live slots densely from zero.
    ///
    /// The returned [`Compaction`] is the *whole* contract — the caller sends it
    /// and the client rewrites its map from it. Live slots keep their relative
    /// order, so a view does not visually reshuffle when it compacts.
    pub fn compact(&mut self, protocol_version: u32) -> Compaction {
        let mut old_to_new: Vec<Option<u32>> = Vec::with_capacity(self.entries.len());
        let mut kept: Vec<SlotEntry> = Vec::with_capacity(self.entries.len() - self.tombstones);
        for entry in self.entries.drain(..) {
            if matches!(entry, SlotEntry::Tombstone) {
                old_to_new.push(None);
                continue;
            }
            old_to_new.push(Some(kept.len() as u32));
            kept.push(entry);
        }

        let reclaimed = self.tombstones as u32;
        self.entries = kept;
        self.tombstones = 0;
        self.slots = SlotAllocator::starting_at(self.entries.len() as u32);

        self.node_slot.clear();
        self.type_slot.clear();
        for (slot, entry) in self.entries.iter().enumerate() {
            match entry {
                SlotEntry::Type { name } => {
                    self.type_slot.insert(name.clone(), slot as u32);
                }
                SlotEntry::Node { node_id, .. } => {
                    self.node_slot.insert(*node_id, slot as u32);
                }
                SlotEntry::Tombstone => unreachable!("tombstones were drained above"),
            }
        }

        // An edge whose endpoint was a tombstone cannot exist: `tombstone`
        // drops those when it writes them. The `expect` is therefore an
        // invariant check, not error handling — if it ever fires, the two
        // functions have drifted apart.
        for edge in &mut self.edges {
            edge.source_slot = old_to_new[edge.source_slot as usize]
                .expect("an edge endpoint was tombstoned without dropping the edge");
            edge.target_slot = old_to_new[edge.target_slot as usize]
                .expect("an edge endpoint was tombstoned without dropping the edge");
        }

        Compaction {
            protocol_version,
            old_to_new,
            slot_count: self.entries.len() as u32,
            reclaimed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(source_slot: u32, target_slot: u32) -> ViewEdge {
        ViewEdge {
            source_slot,
            target_slot,
            name: "KNOWS".to_string(),
            meta: false,
        }
    }

    fn seeded() -> View {
        let mut view = View::new();
        view.intern_type("Person");
        for node_id in 100..105 {
            view.intern_node(node_id, "Person", "");
        }
        view
    }

    #[test]
    fn types_and_instances_draw_from_one_allocator() {
        let view = seeded();
        assert_eq!(view.slot_of_type("Person"), Some(0));
        assert_eq!(view.slot_of_node(100), Some(1));
        assert_eq!(view.slot_of_node(104), Some(5));
        assert_eq!(view.slot_count(), 6);
    }

    #[test]
    fn interning_is_idempotent() {
        let mut view = seeded();
        assert_eq!(view.intern_type("Person"), 0);
        assert_eq!(view.intern_node(102, "Person", ""), (3, false));
        assert_eq!(view.slot_count(), 6, "no slot was burned on a repeat");
    }

    #[test]
    fn a_collapse_tombstones_and_round_trips_the_slots_it_killed() {
        let mut view = seeded();
        view.add_edge(edge(1, 2));
        view.add_edge(edge(2, 3));
        let killed = view.collapse_type("Person");

        assert_eq!(
            killed,
            vec![1, 2, 3, 4, 5],
            "the type node is not an instance"
        );
        assert_eq!(view.tombstone_count(), 5);
        assert_eq!(view.slot_count(), 6, "tombstoning never returns slots");
        assert_eq!(view.entry(1), Some(&SlotEntry::Tombstone));
        assert_eq!(
            view.entry(0),
            Some(&SlotEntry::Type {
                name: "Person".into()
            })
        );
        assert_eq!(view.slot_of_node(100), None, "a tombstone answers nothing");
        assert!(
            view.edges().is_empty(),
            "a link to a tombstone is an index into nothing"
        );
    }

    #[test]
    fn a_re_expanded_node_gets_a_fresh_slot_never_the_tombstoned_one() {
        // The reuse this whole module exists to prevent: if slot 1 came back,
        // a client still holding "slot 1 = node 100" would be right by accident
        // here and wrong the moment the re-expansion found a different node.
        let mut view = seeded();
        view.collapse_type("Person");
        let (slot, is_new) = view.intern_node(100, "Person", "");
        assert!(is_new);
        assert_eq!(slot, 6);
    }

    #[test]
    fn compaction_remaps_slots_edges_and_lookups_together() {
        let mut view = View::new();
        view.intern_type("Person");
        for node_id in 0..5 {
            view.intern_node(node_id, "Person", "");
        }
        view.add_edge(edge(0, 5)); // type → last instance
        view.add_edge(edge(4, 5));
        // Kill the middle three instances (slots 1, 2, 3), keeping 0, 4 and 5.
        view.tombstone(&[1, 2, 3]);
        assert_eq!(view.edges().len(), 2, "neither edge touched a tombstone");

        let remap = view.compact(7);
        assert_eq!(remap.protocol_version, 7);
        assert_eq!(remap.reclaimed, 3);
        assert_eq!(remap.slot_count, 3);
        assert_eq!(
            remap.old_to_new,
            vec![Some(0), None, None, None, Some(1), Some(2)]
        );

        assert_eq!(view.slot_of_type("Person"), Some(0));
        assert_eq!(view.slot_of_node(3), Some(1), "old slot 4 → new slot 1");
        assert_eq!(view.slot_of_node(4), Some(2));
        assert_eq!(view.slot_of_node(0), None, "node 0 was tombstoned");
        assert_eq!(view.tombstone_count(), 0);
        assert_eq!(view.slot_count(), 3, "the allocator resumes at the new end");

        let mapped: Vec<(u32, u32)> = view
            .edges()
            .iter()
            .map(|e| (e.source_slot, e.target_slot))
            .collect();
        assert_eq!(mapped, vec![(0, 2), (1, 2)]);

        // The allocator must hand out 3 next, not 6: a stale counter would
        // leave a hole the position array has no entry for.
        assert_eq!(view.intern_node(99, "Person", ""), (3, true));
    }

    #[test]
    fn compaction_fires_on_ratio_and_size_together() {
        let mut small = View::new();
        for node_id in 0..10 {
            small.intern_node(node_id, "Person", "");
        }
        small.tombstone(&[0, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(
            !small.should_compact(),
            "90% dead but 10 slots: the remap costs more than the waste"
        );

        let mut big = View::new();
        for node_id in 0..100 {
            big.intern_node(node_id, "Person", "");
        }
        let doomed: Vec<u32> = (0..29).collect();
        big.tombstone(&doomed);
        assert!(!big.should_compact(), "29% is under the threshold");
        big.tombstone(&[29]);
        assert!(big.should_compact(), "30% is the threshold");
    }

    #[test]
    fn duplicate_links_are_dropped() {
        let mut view = seeded();
        view.add_edge(edge(1, 2));
        view.add_edge(edge(1, 2));
        assert_eq!(view.edges().len(), 1);
    }
}
