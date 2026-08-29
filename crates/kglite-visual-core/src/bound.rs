//! The response bound (plan D5) and the metadata a bounded answer carries.
//!
//! **The bound is enforced here, in core, never in the UI.** A guarantee the
//! client implements is not a guarantee: the whole point of progressive
//! disclosure is that kglite reaches 100M nodes and no browser renders that,
//! so the server decides what crosses the wire. A change that lets an
//! unbounded result reach the renderer is a defect, not a feature.
//!
//! **Bytes AND count**, because either alone is defeated by the other's shape:
//! ten thousand one-character labels are cheap, and a hundred labels carrying
//! long type names are not.

use serde::Serialize;
use ts_rs::TS;

/// A ceiling on one response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bound {
    /// Maximum items (type nodes, or graph nodes in P3's expansion).
    pub max_items: usize,
    /// Maximum serialized bytes for the item list.
    pub max_bytes: usize,
}

/// What a bounded response tells the client about what it did not send.
///
/// Present on every bounded list, truncated or not: a UI that only learns
/// about a bound when it fires has no way to say "all 12 of 12" the rest of
/// the time, and "12" alone reads as complete when it is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct BoundInfo {
    /// Items in this response.
    pub returned: u32,
    /// Items that existed before the bound was applied.
    pub total: u32,
    /// True when `returned < total`. The UI must display it — a silently
    /// truncated answer reads as a complete one.
    pub truncated: bool,
}

impl BoundInfo {
    pub fn new(returned: usize, total: usize) -> Self {
        Self {
            returned: returned as u32,
            total: total as u32,
            truncated: returned < total,
        }
    }
}

/// Apply a [`Bound`] to an already-prioritised list.
///
/// The caller sorts first — the bound keeps a *prefix*, so "which items
/// matter" stays a decision of whoever knows the domain, and this function
/// only decides how many fit. `measure` reports one item's serialized size.
pub fn apply<T>(items: Vec<T>, bound: Bound, measure: impl Fn(&T) -> usize) -> (Vec<T>, BoundInfo) {
    let total = items.len();
    let mut kept = items;
    kept.truncate(bound.max_items);

    let mut bytes: usize = kept.iter().map(&measure).sum();
    while bytes > bound.max_bytes && kept.len() > 1 {
        let doomed = kept.pop().expect("len > 1");
        bytes -= measure(&doomed);
    }

    let info = BoundInfo::new(kept.len(), total);
    (kept, info)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEN_EACH: fn(&u32) -> usize = |_| 10;

    #[test]
    fn an_unbounded_list_reports_untruncated() {
        let bound = Bound {
            max_items: 10,
            max_bytes: 1000,
        };
        let (kept, info) = apply(vec![1u32, 2, 3], bound, TEN_EACH);
        assert_eq!(kept, vec![1, 2, 3]);
        assert_eq!(
            info,
            BoundInfo {
                returned: 3,
                total: 3,
                truncated: false
            }
        );
    }

    #[test]
    fn the_count_bound_can_fail_and_says_so() {
        let bound = Bound {
            max_items: 2,
            max_bytes: 1000,
        };
        let (kept, info) = apply(vec![1u32, 2, 3, 4], bound, TEN_EACH);
        assert_eq!(kept, vec![1, 2], "the bound keeps the prioritised prefix");
        assert_eq!(
            info,
            BoundInfo {
                returned: 2,
                total: 4,
                truncated: true
            }
        );
    }

    #[test]
    fn the_byte_bound_can_fail_independently_of_the_count_bound() {
        // 4 items well inside max_items, but 40 bytes against a 25-byte
        // ceiling: the count bound alone would have let this through.
        let bound = Bound {
            max_items: 100,
            max_bytes: 25,
        };
        let (kept, info) = apply(vec![1u32, 2, 3, 4], bound, TEN_EACH);
        assert_eq!(kept, vec![1, 2]);
        assert!(info.truncated);
    }

    #[test]
    fn one_oversized_item_still_crosses_the_wire() {
        // Trimming to zero would answer a legitimate question with an empty
        // view and no way to make progress. One item plus `truncated` is an
        // answer the UI can explain.
        let bound = Bound {
            max_items: 100,
            max_bytes: 1,
        };
        let (kept, info) = apply(vec![1u32, 2, 3], bound, TEN_EACH);
        assert_eq!(kept, vec![1]);
        assert_eq!(info.total, 3);
        assert!(info.truncated);
    }
}
