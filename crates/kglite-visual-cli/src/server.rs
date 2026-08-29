//! Binding, routing and serving.

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Arc;

use axum::routing::{any, get};
use axum::Router;
use kglite_visual_core::{LaunchInfo, Session};

use crate::{api, assets, ws};

/// A bound-but-not-yet-serving server.
///
/// The split matters for the launch contract (D6): the port must be *resolved*
/// before anything is printed, because `--port 0` is how every agent and CI
/// invocation runs, and a harness that has to guess the port races the server
/// it just started.
pub struct Bound {
    listener: TcpListener,
    session: Arc<Session>,
    pub info: LaunchInfo,
}

/// Bind to loopback and resolve the port.
///
/// **127.0.0.1 only.** `.kgl` files are other people's data; a viewer that
/// binds `0.0.0.0` publishes them to the local network the moment someone runs
/// it on a laptop in a café. A `--host` flag, if it is ever added, is explicit
/// and loud, never a default.
pub fn bind(session: Session, requested_port: u16, graph: String) -> std::io::Result<Bound> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, requested_port)))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();

    Ok(Bound {
        listener,
        session: Arc::new(session),
        info: LaunchInfo {
            url: format!("http://127.0.0.1:{port}/"),
            port,
            pid: std::process::id(),
            graph,
        },
    })
}

impl Bound {
    /// Serve until the process is killed.
    pub async fn serve(self) -> std::io::Result<()> {
        let listener = tokio::net::TcpListener::from_std(self.listener)?;
        axum::serve(listener, router(self.session)).await
    }
}

fn router(session: Arc<Session>) -> Router {
    Router::new()
        .route("/api/meta-graph", get(api::meta_graph))
        .route("/api/session", get(api::session_info))
        .route("/api/describe", get(api::describe))
        // `any` rather than `get`: a WebSocket upgrade is a GET, but routing it
        // through `any` keeps the 405 for a mistaken POST out of the upgrade
        // path, where it would surface as an opaque handshake failure.
        .route("/ws", any(ws::upgrade))
        .with_state(session)
        // Everything else is the embedded frontend. Registered as a fallback,
        // not a nested route, so the API paths above cannot be shadowed by an
        // asset that happens to share a prefix.
        .fallback(assets::serve)
}
