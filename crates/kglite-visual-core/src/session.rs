//! One open graph, and every answer a consumer can ask it for.
//!
//! A session owns the `Arc<DirGraph>`, the slot space, and the meta-graph
//! computed once at open. It knows nothing about HTTP or WebSockets: it hands
//! back response structs and, for the binary path, framed byte vectors. The
//! CLI, the wheel and a desktop shell each move those bytes their own way.

use std::sync::Arc;

use kglite::api::introspection::{compute_schema, schema_overview_to_json};
use kglite::api::DirGraph;
use serde::Serialize;
use ts_rs::TS;

use crate::meta_graph::{self, DetailTier, MetaGraphResponse, MetaGraphStats};
use crate::protocol::{MessageType, ResponseEncoder, PROTOCOL_VERSION};
use crate::slots::SlotAllocator;

/// What the client needs to know about the session it is attached to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SessionInfo {
    /// The wire format this server speaks. A client that decodes a different
    /// number refuses rather than guessing (`protocol.rs`).
    pub protocol_version: u32,
    /// `kglite-visual-core`'s version.
    pub core_version: String,
    /// The graph, as the caller named it.
    pub graph: String,
    /// The tier the server chose for this graph's meta-graph.
    pub tier: DetailTier,
    /// Slots handed out so far. Meta-nodes today; P3's expansion appends.
    pub slot_count: u32,
    pub stats: MetaGraphStats,
}

/// A server-side failure, delivered in band.
///
/// A client that shows an empty graph on failure is indistinguishable from one
/// showing an empty graph on success, so every error takes a message frame of
/// its own rather than closing the socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ErrorMessage {
    pub message: String,
}

/// The schema document behind `/api/describe` (D12).
///
/// Deliberately **not** a ts-rs type: `schema` is kglite's own JSON shape,
/// rendered by the engine's `schema_overview_to_json` so every binding's
/// schema document is byte-identical. Generating a TypeScript type for it here
/// would be this crate claiming ownership of a shape kglite owns. The frontend
/// does not consume this endpoint; agents and `curl` do.
#[derive(Debug, Clone, Serialize)]
pub struct DescribeResponse {
    pub protocol_version: u32,
    /// The same tier the meta-graph carries, so an agent reading only this
    /// endpoint learns how much of the schema it is being shown.
    pub tier: DetailTier,
    pub core_type_count: u32,
    /// kglite's canonical schema JSON.
    pub schema: serde_json::Value,
}

/// An open graph.
pub struct Session {
    graph: Arc<DirGraph>,
    source: String,
    slots: SlotAllocator,
    meta_graph: MetaGraphResponse,
}

impl Session {
    /// Open a session over an already-loaded graph.
    ///
    /// The meta-graph is computed here, once: it is the entry screen, it is
    /// O(#types), and recomputing it per request would make a page reload
    /// re-walk the type index for no new information.
    pub fn open(graph: Arc<DirGraph>, source: impl Into<String>) -> Self {
        let mut slots = SlotAllocator::new();
        let meta_graph = meta_graph::compute(&graph, &mut slots);
        Self {
            graph,
            source: source.into(),
            slots,
            meta_graph,
        }
    }

    pub fn graph(&self) -> &Arc<DirGraph> {
        &self.graph
    }

    pub fn meta_graph(&self) -> &MetaGraphResponse {
        &self.meta_graph
    }

    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            protocol_version: PROTOCOL_VERSION,
            core_version: crate::VERSION.to_string(),
            graph: self.source.clone(),
            tier: self.meta_graph.meta.tier,
            slot_count: self.slots.len(),
            stats: self.meta_graph.meta.stats,
        }
    }

    /// kglite's schema document plus the tier this session chose.
    ///
    /// `compute_schema` reads per-type metadata the engine already holds; it
    /// is not a node scan. It is still the heaviest call on this type, so a
    /// server runs it off the async runtime.
    pub fn describe(&self) -> DescribeResponse {
        let schema = compute_schema(&self.graph);
        DescribeResponse {
            protocol_version: PROTOCOL_VERSION,
            tier: self.meta_graph.meta.tier,
            core_type_count: self.meta_graph.meta.stats.core_type_count,
            schema: schema_overview_to_json(&schema),
        }
    }

    /// The meta-graph as protocol frames: metadata JSON, then points, then
    /// links, with the terminal flag on the last.
    pub fn meta_graph_frames(&self) -> Vec<Vec<u8>> {
        let mut enc = ResponseEncoder::new();
        enc.push_json(
            MessageType::MetaGraphMeta,
            &serde_json::to_string(&self.meta_graph.meta)
                .expect("MetaGraphMeta is plain data and always serializes"),
        );
        enc.push_f32(MessageType::Points, &self.meta_graph.points);
        enc.push_f32(MessageType::Links, &self.meta_graph.links);
        enc.finish()
    }

    /// The session info as a single terminal frame.
    pub fn session_info_frames(&self) -> Vec<Vec<u8>> {
        let mut enc = ResponseEncoder::new();
        enc.push_json(
            MessageType::SessionInfo,
            &serde_json::to_string(&self.info())
                .expect("SessionInfo is plain data and always serializes"),
        );
        enc.finish()
    }
}

/// Frame an error for the binary transport.
///
/// Free-standing rather than a `Session` method: the failures worth reporting
/// most are the ones that happen before a session exists.
pub fn error_frames(message: impl Into<String>) -> Vec<Vec<u8>> {
    let payload = ErrorMessage {
        message: message.into(),
    };
    let mut enc = ResponseEncoder::new();
    enc.push_json(
        MessageType::Error,
        &serde_json::to_string(&payload).expect("ErrorMessage is plain data"),
    );
    enc.finish()
}
