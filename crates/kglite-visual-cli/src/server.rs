//! Binding, routing and serving.

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Arc;

use axum::routing::{any, get, post};
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
    /// Serve until the process is killed. The CLI's shape: nothing ever asks
    /// it to stop.
    pub async fn serve(self) -> std::io::Result<()> {
        self.serve_until(std::future::pending()).await
    }

    /// Serve until `shutdown` resolves, then stop **without draining**.
    ///
    /// The wheel's `close()` shape. Deliberately not
    /// `axum::serve(..).with_graceful_shutdown(..)`: graceful shutdown waits
    /// for every in-flight connection to end, and a viewer's connections are
    /// WebSockets held open by a browser tab for as long as the tab exists. A
    /// `close()` that blocks until the user closes their tab is a hang, and
    /// the caller asked for the port back. Dropping the server future here
    /// closes the listener immediately; the runtime's own shutdown then drops
    /// the connection tasks.
    pub async fn serve_until(
        self,
        shutdown: impl std::future::Future<Output = ()>,
    ) -> std::io::Result<()> {
        let listener = tokio::net::TcpListener::from_std(self.listener)?;
        let server = axum::serve(listener, router(self.session));
        tokio::select! {
            result = server => result,
            _ = shutdown => Ok(()),
        }
    }
}

fn router(session: Arc<Session>) -> Router {
    Router::new()
        .route("/api/meta-graph", get(api::meta_graph))
        .route("/api/session", get(api::session_info))
        .route("/api/describe", get(api::describe))
        // The request vocabulary, one named route each so a `curl` line says
        // what it is asking for. POST rather than GET because every one of
        // them carries a body and several of them mutate the slot space —
        // an expansion is not idempotent, and a GET that appends slots would
        // be re-run by any cache or prefetcher in the path.
        .route("/api/cypher", post(api::cypher))
        .route("/api/search", post(api::search))
        .route("/api/preview", post(api::preview))
        .route("/api/expand", post(api::expand))
        .route("/api/collapse", post(api::collapse))
        .route("/api/node", post(api::node_detail))
        .route("/api/property-stats", post(api::property_stats))
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
