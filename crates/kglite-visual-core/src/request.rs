//! The request vocabulary — what a client may ask a session for.
//!
//! **Every request that names something in the view names it by slot.** That is
//! the D4 identity contract paying for itself: a meta-graph type node and an
//! expanded instance node are the same kind of handle, so "preview what
//! expanding this would add", "expand it" and "collapse it" are each one
//! message rather than one per kind of thing. A request that carried a type
//! *name* for one case and a node id for the other would need the client to
//! know which it was holding — which is exactly what the shared slot space
//! exists to stop.
//!
//! One vocabulary, two transports. The WebSocket sends these as JSON text
//! frames; the JSON twin's `POST /api/*` bodies are the same structs. Neither
//! is a translation of the other (test-plan §2).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Which way an expansion walks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "kebab-case")]
pub enum EdgeDirection {
    Out,
    In,
    /// Both directions, deduplicated by node — a node reachable both ways is
    /// one node on screen, not two. The default, because a user who has not
    /// said which way an edge runs wants the neighbourhood, not half of it.
    #[default]
    Both,
}

/// How a search matches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "kebab-case")]
pub enum SearchMode {
    /// `CONTAINS`. Never index-served, so it is the slower of the two — and the
    /// default anyway, because a search box that only matched prefixes would
    /// silently miss what the user typed.
    #[default]
    Contains,
    /// `STARTS WITH` — pushed into the MATCH pattern on a disk-backed graph
    /// with a string index, a full scan otherwise.
    StartsWith,
}

/// Which arrangement the server should compute for the live view (plan E5).
///
/// **A vocabulary, not a string**, so a kernel this build does not implement is
/// refused by name — `serde` reports the value it could not read — rather than
/// silently falling through to the default. [`LayoutKernel::Geo`] is the case
/// that makes the distinction load-bearing: it is a *named* kernel with no
/// implementation until G4, and a caller asking for it deserves the sentence
/// saying so instead of a force layout it did not ask for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "kebab-case")]
pub enum LayoutKernel {
    /// Let `render::structure::plan` read the scene and choose. The default,
    /// because the structure-aware choice is what the headless render already
    /// makes and a caller with no opinion wants the same answer.
    #[default]
    Auto,
    /// Hop rings around one centre.
    Radial,
    /// Communities laid out separately and packed.
    Islands,
    /// The seeded Fruchterman–Reingold fallback.
    Force,
    /// Geographic projection. Named here and refused until G4 lands it, so the
    /// refusal is a sentence rather than a parse error.
    Geo,
    /// **Not a kernel: the absence of one.** Hand the geometry back to the
    /// viewer's GPU force simulation, which is where it starts.
    ///
    /// It is a request rather than a purely client-side switch because the
    /// server has to know: the whole value of a static layout, for a peer that
    /// cannot see the screen, is that the arrangement is then *knowable* — and
    /// a client that dropped back to its own simulation without saying so would
    /// leave `view_state` claiming knowledge of a picture the GPU is now
    /// moving. It is also what keeps two clients and an agent agreeing about
    /// which mode the shared view is in.
    Simulation,
}

impl LayoutKernel {
    /// The wire spelling, for a message that has to name the kernel it chose.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Radial => "radial",
            Self::Islands => "islands",
            Self::Force => "force",
            Self::Geo => "geo",
            Self::Simulation => "simulation",
        }
    }

    /// True while the server knows where the points are.
    ///
    /// The one question [`crate::session::geometry_caveat`] asks, kept beside
    /// the vocabulary rather than in the caveat, so a kernel added later
    /// answers it here and every caveat follows.
    pub const fn is_static(self) -> bool {
        matches!(self, Self::Radial | Self::Islands | Self::Force | Self::Geo)
    }
}

/// Everything a client can ask for.
///
/// Externally tagged on `"type"`, kebab-case, so a hand-written `curl` body and
/// the generated TypeScript agree without either side reading the other.
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Request {
    /// Run a read-only Cypher query.
    Cypher(CypherRequest),
    /// Per-relationship counts for what expanding `slot` would add — computed
    /// without fetching a single node (plan D12).
    Preview(SlotRequest),
    /// Fetch neighbours, bounded in core (D5).
    Expand(ExpandRequest),
    /// Tombstone. A type slot collapses every instance of its type; an
    /// instance slot collapses itself.
    Collapse(SlotRequest),
    /// One node's stored properties, plus its expansion preview.
    NodeDetail(SlotRequest),
    /// Type-scoped title/property search, server-side (never a client index).
    Search(SearchRequest),
    /// Per-property statistics for a type, for the color-by / size-by menus.
    PropertyStats(TypeRequest),
    /// Compute a static arrangement for the live view and push it to every
    /// client (plan E5). Changes no slot, tombstones nothing, adds no link —
    /// it moves the picture, not the graph.
    Layout(LayoutRequest),
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct CypherRequest {
    pub query: String,
    /// `$name` bindings. Passed through kglite's own JSON→`Value` converter, so
    /// a parameter is never string-interpolated into the query text.
    #[serde(default)]
    #[ts(type = "Record<string, unknown>")]
    pub params: BTreeMap<String, serde_json::Value>,
    /// Rows wanted. Clamped in core to the response bound — a client asking for
    /// more gets the ceiling and `truncated: true`, never the rows.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Also map any nodes and relationships in the result into the slot space,
    /// so "show in graph" needs no second round trip.
    #[serde(default)]
    pub as_graph: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SlotRequest {
    pub slot: u32,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ExpandRequest {
    pub slot: u32,
    /// Relationship type to walk. `None` walks every type, which is the
    /// expensive case and exactly why the preview exists.
    #[serde(default)]
    pub relationship: Option<String>,
    #[serde(default)]
    pub direction: EdgeDirection,
    /// Nodes wanted. Clamped to the core ceiling; see [`crate::expand`].
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SearchRequest {
    /// The needle. Matched case-insensitively against the type's title field
    /// and, when `property` is set, against that property instead.
    pub query: String,
    /// Restrict to one node type. `None` searches every type, which is the
    /// slow path on a large graph and is reported as such.
    #[serde(default)]
    pub node_type: Option<String>,
    /// Property to match. Defaults to the type's title field.
    #[serde(default)]
    pub property: Option<String>,
    #[serde(default)]
    pub mode: SearchMode,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct TypeRequest {
    pub node_type: String,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct LayoutRequest {
    #[serde(default)]
    pub kernel: LayoutKernel,
    /// The slot a radial layout should be centred on. Ignored by every other
    /// kernel, and optional even for `radial`: with no hint the structure pass
    /// picks the centre it would have picked on its own.
    #[serde(default)]
    pub seed_slot: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_shape_is_the_one_a_curl_body_would_write() {
        let request: Request = serde_json::from_str(
            r#"{"type":"expand","slot":0,"relationship":"KNOWS","direction":"out","limit":50}"#,
        )
        .expect("the documented body must parse");
        let Request::Expand(expand) = request else {
            panic!("tag dispatch went to the wrong variant");
        };
        assert_eq!(expand.slot, 0);
        assert_eq!(expand.relationship.as_deref(), Some("KNOWS"));
        assert_eq!(expand.direction, EdgeDirection::Out);
        assert_eq!(expand.limit, Some(50));
    }

    #[test]
    fn omitted_fields_take_the_documented_defaults() {
        let request: Request =
            serde_json::from_str(r#"{"type":"expand","slot":3}"#).expect("minimal body");
        let Request::Expand(expand) = request else {
            panic!("wrong variant");
        };
        assert_eq!(expand.direction, EdgeDirection::Both);
        assert_eq!(expand.limit, None);
        assert_eq!(expand.relationship, None);

        let request: Request =
            serde_json::from_str(r#"{"type":"cypher","query":"RETURN 1"}"#).expect("minimal body");
        let Request::Cypher(cypher) = request else {
            panic!("wrong variant");
        };
        assert!(cypher.params.is_empty());
        assert!(!cypher.as_graph);
    }

    #[test]
    fn a_layout_request_defaults_to_auto_and_names_an_unknown_kernel() {
        let request: Request = serde_json::from_str(r#"{"type":"layout"}"#).expect("minimal body");
        let Request::Layout(layout) = request else {
            panic!("wrong variant");
        };
        assert_eq!(layout.kernel, LayoutKernel::Auto);
        assert_eq!(layout.seed_slot, None);

        let request: Request =
            serde_json::from_str(r#"{"type":"layout","kernel":"islands","seed_slot":4}"#)
                .expect("the documented body must parse");
        let Request::Layout(layout) = request else {
            panic!("wrong variant");
        };
        assert_eq!(layout.kernel, LayoutKernel::Islands);
        assert_eq!(layout.seed_slot, Some(4));

        // A kernel this build has never heard of is refused BY NAME rather
        // than defaulting to `auto`: an arrangement the caller did not ask for
        // is indistinguishable on screen from the one it did.
        let err = serde_json::from_str::<Request>(r#"{"type":"layout","kernel":"spiral"}"#)
            .expect_err("an unknown kernel must not parse");
        assert!(
            err.to_string().contains("spiral"),
            "the message must name the kernel it refused: {err}"
        );
    }

    #[test]
    fn an_unknown_request_type_is_refused_rather_than_ignored() {
        // A client that sends a request this server does not implement must
        // get an error, not silence: waiting forever for a response that was
        // dropped is the harder bug to diagnose of the two.
        let err = serde_json::from_str::<Request>(r#"{"type":"delete-everything"}"#)
            .expect_err("an unknown tag must not parse");
        assert!(
            err.to_string().contains("delete-everything"),
            "the message must name the request it refused: {err}"
        );
    }
}
