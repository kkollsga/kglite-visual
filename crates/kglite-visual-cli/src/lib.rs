//! The localhost viewer, as a library.
//!
//! **Why a library at all when the product is one binary.** There are two
//! front doors to the same server — `kglite-visual <file>` and the wheel's
//! `kglite_visual.show(...)` — and the plan (D9) says they must be the *same*
//! server, lib-linked, not two implementations that drift. So the axum
//! routing, the embedded bundle and the bind/serve split live here, `main.rs`
//! is four lines over [`run_from`], and `kglite-visual-py` depends on this
//! crate. The binary stays the only binary.
//!
//! **This is where axum is allowed to exist.** `kglite-visual-core` is
//! transport-agnostic by rule — nothing in it may know it is talking to a
//! WebSocket — so the HTTP/WS layer stops at this crate's boundary and the
//! Python wheel links *through* it rather than growing a second one.

pub mod api;
pub mod assets;
pub mod broadcast;
pub mod cli;
pub mod mcp;
pub mod queries;
pub mod render_cmd;
pub mod server;
pub mod ws;

pub use broadcast::{AppState, Bus};
pub use cli::run_from;
pub use server::{bind, Bound};
