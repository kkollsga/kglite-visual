//! Protocol message types.
//!
//! Every type here is the single source of the TypeScript the frontend
//! compiles against: `#[derive(TS)]` writes `frontend/src/generated/`, and
//! `make check-generated-ts` regenerates and diffs, so a Rust-side change that
//! is not mirrored fails the gate instead of surfacing as a runtime shape
//! mismatch in the browser (plan D4).
//!
//! `export_to` is resolved against `TS_RS_EXPORT_DIR`, whose default is
//! `<crate manifest dir>/bindings/` — not the manifest dir itself. That is why
//! the path below climbs three levels to reach the repo root and not two.

use serde::Serialize;
use ts_rs::TS;

/// One row of the type-level meta-graph: a label (or relationship type) and
/// how many members it has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct TypeCount {
    pub name: String,
    pub count: u32,
}

/// The entry screen's payload: what types exist and how big they are.
///
/// Always small — it is O(#types), not O(V) — which is what makes it a safe
/// default view for a graph no browser could render whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct MetaGraphSummary {
    pub node_types: Vec<TypeCount>,
    pub relationship_types: Vec<TypeCount>,
    /// True when a response bound clipped the lists above. The UI must show
    /// it: a silently truncated answer reads as a complete one.
    pub truncated: bool,
}
