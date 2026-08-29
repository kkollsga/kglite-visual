//! Engine-facing core for kglite-visual.
//!
//! **Transport-agnostic is a rule, not a description.** Nothing in this crate
//! may know it is talking to a WebSocket: three consumers (the CLI server, the
//! Python wheel, a possible desktop shell) share this code, and the encoder is
//! the seam between them. A `use axum::…` reaching this crate is the boundary
//! being crossed, not a convenience.

pub mod bound;
pub mod error;
pub mod expand;
pub mod launch;
pub mod layout;
pub mod loader;
pub mod meta_graph;
pub mod protocol;
pub mod query;
pub mod request;
pub mod session;
pub mod slots;
pub mod stats;
pub mod values;
pub mod view;

pub use bound::{Bound, BoundInfo};
pub use error::CoreError;
pub use expand::{ExpansionPreview, MAX_EXPANSION_NODES};
pub use launch::LaunchInfo;
pub use loader::{load_graph, node_counts_by_type, GraphSource};
pub use meta_graph::{DetailTier, MetaGraphResponse};
pub use protocol::{
    decode_frame, DecodedFrame, MessageType, ProtocolError, ResponseEncoder, PROTOCOL_VERSION,
};
pub use query::{QueryConfig, QueryTable, SearchResponse, QUERY_THREAD_STACK_BYTES};
pub use request::Request;
pub use session::{
    response_frames, DescribeResponse, ErrorMessage, GraphSlice, Response, Session, SessionInfo,
};
pub use slots::SlotAllocator;
pub use stats::{NodeDetail, PropertyStatsResponse};
pub use view::View;

/// This crate's version, so consumers report one number rather than each
/// baking in its own literal.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
