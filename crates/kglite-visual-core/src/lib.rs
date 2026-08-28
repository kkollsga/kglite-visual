//! Engine-facing core for kglite-visual.
//!
//! **Transport-agnostic is a rule, not a description.** Nothing in this crate
//! may know it is talking to a WebSocket: three consumers (the CLI server, the
//! Python wheel, a possible desktop shell) share this code, and the encoder is
//! the seam between them. A `use axum::…` reaching this crate is the boundary
//! being crossed, not a convenience.

pub mod error;
pub mod launch;
pub mod loader;
pub mod messages;

pub use error::CoreError;
pub use launch::LaunchInfo;
pub use loader::{load_graph, node_counts_by_type, GraphSource};

/// This crate's version, so consumers report one number rather than each
/// baking in its own literal.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
