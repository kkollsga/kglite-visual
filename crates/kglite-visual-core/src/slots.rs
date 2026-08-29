//! The slot space — the server-owned index space every wire index refers to
//! (plan D4, "index-native").
//!
//! **One allocator, both kinds of node.** A meta-graph type node and an
//! instance node drawn in P3's expansion take their slots from the same
//! counter. That is the identity contract D12 fixes now rather than later: if
//! meta-nodes and instance nodes had separate spaces, a link between them
//! could not be expressed as a pair of indices, and every renderer call site
//! would need to know which space an index came from.
//!
//! Slots are never reused. Collapse writes a NaN tombstone at the slot (P3);
//! reclaiming space is an explicit compaction op with an id remap, never an
//! implicit reuse — a reused slot silently re-labels whatever the client still
//! holds a reference to.

/// Monotonic slot allocator.
#[derive(Debug, Default)]
pub struct SlotAllocator {
    next: u32,
}

impl SlotAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    /// An allocator whose next slot is `next`.
    ///
    /// The one legitimate way to move the counter backwards: compaction
    /// renumbers the live slots densely and the allocator has to resume at the
    /// new end, or it would leave a hole no position array has an entry for.
    /// Every other caller uses [`Self::new`] — a counter that could be set
    /// arbitrarily is how a slot gets handed out twice.
    pub fn starting_at(next: u32) -> Self {
        Self { next }
    }

    /// Allocate the next slot.
    ///
    /// `u32` is the counter, but the wire carries indices as `f32` (exact to
    /// 2^24, straight into cosmos.gl's `setLinks`), so a session that ever
    /// allocated more than 2^24 slots would silently start aliasing. The
    /// assertion makes that a crash at the allocation instead of wrong edges
    /// on screen; the response bound (D5) is what keeps a real session orders
    /// of magnitude below it.
    pub fn alloc(&mut self) -> u32 {
        assert!(
            self.next < (1 << 24),
            "slot space exhausted at 2^24: wire indices are f32 and would start aliasing"
        );
        let slot = self.next;
        self.next += 1;
        slot
    }

    /// How many slots have been handed out.
    pub fn len(&self) -> u32 {
        self.next
    }

    pub fn is_empty(&self) -> bool {
        self.next == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_are_dense_and_monotonic_from_zero() {
        let mut slots = SlotAllocator::new();
        assert!(slots.is_empty());
        let handed: Vec<u32> = (0..5).map(|_| slots.alloc()).collect();
        assert_eq!(handed, vec![0, 1, 2, 3, 4]);
        assert_eq!(slots.len(), 5);
    }
}
