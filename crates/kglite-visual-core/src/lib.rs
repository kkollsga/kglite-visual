//! Engine-facing core for kglite-visual.
//!
//! **Transport-agnostic is a rule, not a description.** Nothing in this crate
//! may know it is talking to a WebSocket: three consumers (the CLI server, the
//! Python wheel, a possible desktop shell) share this code, and the encoder is
//! the seam between them. A `use axum::…` reaching this crate is the boundary
//! being crossed, not a convenience.

pub mod appearance_mapping;
pub mod bookmark;
mod bookmark_capture;
mod bookmark_members;
mod bookmark_restore;
pub mod bound;
pub mod control;
pub mod error;
pub mod expand;
pub mod export;
pub mod history;
mod history_actions;
pub mod launch;
pub mod layout;
pub mod loader;
pub mod meta_graph;
pub mod output;
mod output_capture;
mod output_render;
mod output_values;
pub mod presentation;
pub mod protocol;
pub mod query;
pub mod records;
mod recovery;
pub mod render;
pub mod request;
pub mod session;
pub mod slots;
pub mod source_identity;
pub mod stats;
pub mod validate;
pub mod values;
pub mod view;

pub use bound::{Bound, BoundInfo};
pub use control::{control_frames, Appearance, Command, Focus, Highlight, HighlightConcept};
pub use error::CoreError;
pub use expand::{ExpansionPreview, MAX_EXPANSION_NODES};
pub use export::{ExportFormat, ExportedView};
pub use launch::LaunchInfo;
pub use loader::{
    load_graph, load_graph_with, load_session_with, node_counts_by_type, GraphSource, LoadLimits,
};
pub use meta_graph::{DetailTier, MetaGraphResponse};
pub use protocol::{
    decode_frame, DecodedFrame, MessageType, ProtocolError, ResponseEncoder, PROTOCOL_VERSION,
};
pub use query::{QueryConfig, QueryTable, SearchResponse, QUERY_THREAD_STACK_BYTES};
pub use render::{
    layout_live_view, render, render_for, ExpandSource, LayoutMeta, LayoutResult, RenderFormat,
    RenderRequest, RenderSource, Rendered, Theme,
};
pub use request::{LayoutKernel, LayoutRequest, Request};
pub use session::{
    geometry_caveat, response_frames, DescribeResponse, ErrorMessage, GraphSlice, LastSlice,
    Response, Session, SessionInfo, ViewBounds, ViewState, ViewTypeNode, GEOMETRY_CAVEAT,
    GEOMETRY_STATIC_CAVEAT,
};
pub use slots::SlotAllocator;
pub use stats::{NodeDetail, PropertyStatsResponse};
pub use validate::{validate_query, Diagnostic, DiagnosticSeverity, ValidateResponse};
pub use view::View;

/// This crate's version, so consumers report one number rather than each
/// baking in its own literal.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod shared;
pub mod subset;

pub mod field_detail;
mod query_cells;
pub mod query_provenance;
