//! Binding, routing and serving.

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Arc;

use axum::routing::{any, get, post};
use axum::Router;
use kglite_visual_core::{LaunchInfo, Session};

use crate::broadcast::AppState;
use crate::mcp::{self, MCP_PATH};
use crate::{api, assets, ws};

/// A bound-but-not-yet-serving server.
///
/// The split matters for the launch contract (D6): the port must be *resolved*
/// before anything is printed, because `--port 0` is how every agent and CI
/// invocation runs, and a harness that has to guess the port races the server
/// it just started.
pub struct Bound {
    listener: TcpListener,
    state: AppState,
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
        state: AppState::new(Arc::new(session), &graph),
        info: LaunchInfo {
            url: format!("http://127.0.0.1:{port}/"),
            port,
            pid: std::process::id(),
            graph: graph.clone(),
            mcp: format!("http://127.0.0.1:{port}{MCP_PATH}"),
        },
    })
}

impl Bound {
    /// Serve until the process is asked to stop.
    ///
    /// **`SIGTERM` is an ask, and it used to cost a directory.** kglite spills
    /// a portable graph into `$TMPDIR/kglite_portable_<pid>_<id>/` and removes
    /// it in `Drop`; with no handler installed, the default disposition for
    /// `SIGTERM` terminates the process, no destructor runs, and the spill
    /// stays. Anything that stops a server this way — a supervisor, a
    /// harness's teardown, `kill` at a shell — leaked one directory per run,
    /// and this project's own working folder had accumulated fifty of them.
    ///
    /// Catching it turns the kill into a return: the future resolves, the
    /// listener closes, the runtime drops the connection tasks, the last
    /// `Arc<Session>` goes, and kglite cleans up after itself. `SIGKILL`
    /// cannot be caught and still leaks — that is the OS's contract, not a gap
    /// here.
    pub async fn serve(self) -> std::io::Result<()> {
        self.serve_until(shutdown_signal()).await
    }

    /// Arm the stop handler now; return the future that waits on it.
    ///
    /// Registration happens at **creation**, inside this call — not at the
    /// returned future's first poll. The distinction is the launch contract's
    /// safety: the stdout line is the signal a supervisor may `kill` on, so
    /// the handler must exist before the line does. Printing first and
    /// arming inside `serve()` left a window where `SIGTERM` met the default
    /// disposition — microseconds on a warm machine, real on a cold CI
    /// runner, and either way a death with no destructor and a leaked spill.
    pub fn arm_shutdown(
        runtime: &tokio::runtime::Runtime,
    ) -> impl std::future::Future<Output = ()> {
        #[cfg(unix)]
        let terminate = {
            use tokio::signal::unix::{signal, SignalKind};
            let _guard = runtime.enter();
            match signal(SignalKind::terminate()) {
                Ok(stream) => Some(stream),
                Err(err) => {
                    eprintln!(
                        "kglite-visual: WARNING could not listen for SIGTERM ({err}); \
                         a `kill` will leave this graph's temporary spill behind"
                    );
                    None
                }
            }
        };
        #[cfg(not(unix))]
        let _ = runtime;
        async move {
            #[cfg(unix)]
            match terminate {
                Some(mut stream) => {
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => {}
                        _ = stream.recv() => {}
                    }
                }
                None => {
                    let _ = tokio::signal::ctrl_c().await;
                }
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
            eprintln!("kglite-visual: shutting down");
        }
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
        let server = axum::serve(listener, router(self.state));
        tokio::select! {
            result = server => result,
            _ = shutdown => Ok(()),
        }
    }
}

/// Resolves the first time the process is asked to stop.
///
/// `SIGINT` as well as `SIGTERM`: a Ctrl-C at the terminal previously ran no
/// destructor either, so the interactive case leaked exactly like the
/// supervised one. On a platform with no Unix signals, Ctrl-C is the whole of
/// what there is to catch.
///
/// A handler that cannot be installed is reported and not fatal — the server
/// still serves, and the failure a reader needs to know about is that stopping
/// it will leave a directory behind.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(err) => {
                eprintln!(
                    "kglite-visual: WARNING could not listen for SIGTERM ({err}); \
                     a `kill` will leave this graph's temporary spill behind"
                );
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
    eprintln!("kglite-visual: shutting down");
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/meta-graph", get(api::meta_graph))
        .route("/api/session", get(api::session_info))
        .route("/api/describe", get(api::describe))
        .route("/api/view-state", get(api::view_state))
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
        // Parse-only; it runs nothing and moves nothing (plan E3).
        .route("/api/validate", post(api::validate))
        // Saved queries (E4). A GET to read, POSTs to mutate — the store is a
        // file on this machine, so the read is idempotent and the writes carry
        // bodies. There is deliberately NO `/api/queries/run`: a saved query is
        // run by putting its text into the ordinary Cypher path, so there stays
        // exactly one place a query executes.
        .route("/api/queries", get(api::saved_queries))
        .route("/api/queries/save", post(api::save_query))
        .route("/api/queries/delete", post(api::delete_query))
        .route("/api/queries/history", post(api::record_query))
        // The three steering commands (D14). They mutate nothing and answer
        // with the size of the audience that heard them, so a caller learns
        // whether anybody is actually watching.
        .route("/api/reset", post(api::reset))
        .route("/api/focus", post(api::focus))
        .route("/api/highlight", post(api::highlight))
        .route("/api/appearance", post(api::appearance))
        // The one route that answers with image bytes rather than JSON (D13).
        // POST like the rest of the vocabulary: it carries a body, and a GET
        // whose query string held a Cypher statement would be logged, cached
        // and re-run by anything in the path.
        .route("/api/render", post(api::render))
        // `any` rather than `get`: a WebSocket upgrade is a GET, but routing it
        // through `any` keeps the 405 for a mistaken POST out of the upgrade
        // path, where it would surface as an opaque handshake failure.
        .route("/ws", any(ws::upgrade))
        // MCP, served by this server rather than beside it (D14). A
        // `route_service` rather than a nest: the path is exact, and
        // `StreamableHttpService` dispatches on HTTP method alone, so a nested
        // prefix would accept `/mcp/anything` and answer identically.
        //
        // Registered before `with_state` because the service carries its own
        // clone of the state and is state-independent as far as the router is
        // concerned.
        .route_service(MCP_PATH, mcp::service(state.clone()))
        .with_state(state)
        // Everything else is the embedded frontend. Registered as a fallback,
        // not a nested route, so the API paths above cannot be shadowed by an
        // asset that happens to share a prefix.
        .fallback(assets::serve)
}
