//! PyO3 wrapper for kglite-visual.
//!
//! This crate declares no global allocator, and must not (plan D9). KGLite's
//! wheel installs mimalloc; a notebook that imports both wheels loads two
//! extension modules into one interpreter, and two allocators there is the
//! SIGSEGV shape KGLite already runs a canary for. `make
//! check-no-global-allocator` scans this crate's sources so the rule survives
//! a session that has not read this comment.
//!
//! **Everything here is underscore-private.** `python/kglite_visual/` is the
//! public surface: it owns the docstrings, the duck-typed `to_bytes()`
//! handover, the notebook rendering and the `atexit` hook. This file owns the
//! parts that must be Rust — loading, binding, the server thread, and the
//! shutdown that has to survive a fork.

use std::sync::mpsc;
use std::time::Duration;

use pyo3::exceptions::{PyFileNotFoundError, PyOSError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

use kglite_visual_core::{
    load_graph, CoreError, GraphSource, LaunchInfo, QueryConfig, Session, QUERY_THREAD_STACK_BYTES,
};

/// Version of the Rust core this extension was built against.
///
/// Underscore-prefixed: private to the Python package, which re-exports what
/// it wants under its own names.
#[pyfunction]
fn _version() -> &'static str {
    kglite_visual_core::VERSION
}

/// A running server, and the only handle that can stop it.
///
/// Held by the Python `Server` wrapper, which adds the notebook rendering and
/// the `atexit` registration. Nothing here knows what a notebook is.
#[pyclass(module = "kglite_visual._native", name = "_Server")]
struct PyServer {
    info: LaunchInfo,
    /// PID at launch. A `fork()` gives the child a copy of this object whose
    /// server thread does not exist — threads do not survive `fork` — so every
    /// operation compares against `getpid()` first. Without the check, a
    /// forked worker's `atexit` would join a thread that was never started and
    /// hang, or claim to have stopped a server still running in the parent.
    owner_pid: u32,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// The three outcomes of asking a handle to close, as strings the Python side
/// branches on. Not an exception: `atexit` runs this on every live handle, and
/// a forked worker exiting normally is not an error condition.
const CLOSED: &str = "closed";
const ALREADY_CLOSED: &str = "already-closed";
const STALE_AFTER_FORK: &str = "stale-after-fork";

#[pymethods]
impl PyServer {
    #[getter]
    fn url(&self) -> &str {
        &self.info.url
    }

    #[getter]
    fn port(&self) -> u16 {
        self.info.port
    }

    #[getter]
    fn pid(&self) -> u32 {
        self.info.pid
    }

    #[getter]
    fn graph(&self) -> &str {
        &self.info.graph
    }

    /// The launch contract (D6) as a dict — the same four keys the CLI writes
    /// to stdout, built from the same struct so the two cannot drift. The
    /// wheel deliberately prints nothing: a library that writes to stdout
    /// corrupts whatever its caller was writing there.
    #[getter]
    fn launch_info<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        dict.set_item("url", &self.info.url)?;
        dict.set_item("port", self.info.port)?;
        dict.set_item("pid", self.info.pid)?;
        dict.set_item("graph", &self.info.graph)?;
        Ok(dict)
    }

    #[getter]
    fn closed(&self) -> bool {
        self.thread.is_none()
    }

    /// Whether this handle belongs to a process that no longer exists as far
    /// as its server thread is concerned.
    #[getter]
    fn stale(&self) -> bool {
        std::process::id() != self.owner_pid
    }

    /// Stop the server and wait for its thread. Returns which of the three
    /// outcomes happened.
    fn close(&mut self, py: Python<'_>) -> &'static str {
        if self.stale() {
            // Drop the handles without touching them. The thread they name
            // exists only in the parent, and joining it here would block
            // forever on a thread this process never started.
            self.shutdown = None;
            self.thread = None;
            return STALE_AFTER_FORK;
        }
        let Some(thread) = self.thread.take() else {
            return ALREADY_CLOSED;
        };
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        // `Python::detach` around the join: the server thread has to run to
        // finish shutting down, and any of its tasks may need the GIL on the
        // way out. Holding it here is the deadlock.
        py.detach(move || {
            let _ = thread.join();
        });
        CLOSED
    }

    fn __repr__(&self) -> String {
        format!(
            "<kglite_visual._native._Server url={} closed={}>",
            self.info.url,
            self.closed()
        )
    }
}

impl Drop for PyServer {
    /// A handle that is garbage-collected without `close()` still frees its
    /// port. Signal only — never join: this runs under the GC, where blocking
    /// on another thread is how an interpreter shutdown turns into a hang.
    fn drop(&mut self) {
        if self.stale() {
            return;
        }
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// Whether the graph came off the filesystem or out of memory. It decides how
/// a load failure is classified, and nothing else.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    File,
    Buffer,
}

/// Map core failures onto the exception a Python caller can act on, carrying
/// kglite's own diagnostic verbatim. A `.kgl` written by a newer engine
/// arrives here with the engine's version-skew message inside it —
/// summarising that would throw away the only sentence that says which version
/// to install.
///
/// **`Origin` is load-bearing, not decoration.** kglite reports every load
/// failure through the `std::io::Error` family, including "these bytes do not
/// start with the `.kgl` magic", which arrives with a kind that would
/// otherwise map to `OSError`. For a buffer there is no file and no I/O: every
/// way it can fail is a statement about its contents, so it is a `ValueError`.
/// Classifying by kind alone made `show(bad_bytes)` raise `OSError` while
/// `show(a_directory)` raised `ValueError` for the same underlying mistake —
/// two exception types for one class of error, neither a subclass of the
/// other, so a caller had to catch both to catch either.
fn to_py_err(err: CoreError, origin: Origin) -> PyErr {
    let message = err.to_string();
    match err {
        CoreError::Load(io) => match io.kind() {
            std::io::ErrorKind::NotFound => PyFileNotFoundError::new_err(message),
            std::io::ErrorKind::InvalidData | std::io::ErrorKind::InvalidInput => {
                PyValueError::new_err(message)
            }
            _ if origin == Origin::Buffer => PyValueError::new_err(message),
            _ => PyOSError::new_err(message),
        },
        CoreError::Request(_) => PyValueError::new_err(message),
        CoreError::Query(_) => PyRuntimeError::new_err(message),
    }
}

/// Bind, then serve on a thread of our own.
///
/// **The port is resolved before this function returns**, because the listener
/// is bound here, on the calling thread, and only the already-bound `Bound` is
/// moved across. That is stronger than handing the port back over a channel
/// after the thread starts: a port conflict raises out of `show()` as an
/// exception instead of arriving asynchronously, and the socket is listening —
/// so a client that connects the instant `show()` returns is queued by the
/// kernel, never refused. The channel below still exists, and carries the one
/// thing the caller cannot learn on its own thread: whether the tokio runtime
/// was actually built.
fn start(py: Python<'_>, session: Session, graph: String, port: u16) -> PyResult<PyServer> {
    let bound = kglite_visual_cli::bind(session, port, graph)
        .map_err(|err| PyOSError::new_err(format!("could not bind 127.0.0.1:{port}: {err}")))?;
    let info = bound.info.clone();

    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();

    let thread = std::thread::Builder::new()
        .name("kglite-visual-server".to_string())
        .spawn(move || {
            // `thread_stack_size` reaches the blocking pool too, and that is
            // the half that matters: every Cypher execution runs in
            // `spawn_blocking` and kglite's parser overflows tokio's 2 MiB
            // default. Same reasoning, same constant, as the CLI.
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(QUERY_THREAD_STACK_BYTES)
                .build()
            {
                Ok(runtime) => {
                    let _ = ready_tx.send(Ok(()));
                    runtime
                }
                Err(err) => {
                    let _ = ready_tx.send(Err(err.to_string()));
                    return;
                }
            };
            let _ = runtime.block_on(bound.serve_until(async {
                // A dropped sender means the handle was dropped without a
                // close; shut down either way rather than serving a graph
                // nothing can reach.
                let _ = stop_rx.await;
            }));
            // Bounded, not indefinite: a Cypher query already inside
            // `spawn_blocking` cannot be cancelled, and `close()` must return.
            runtime.shutdown_timeout(Duration::from_millis(250));
        })
        .map_err(|err| PyOSError::new_err(format!("could not start the server thread: {err}")))?;

    let ready = py.detach(move || ready_rx.recv());
    match ready {
        Ok(Ok(())) => Ok(PyServer {
            info,
            owner_pid: std::process::id(),
            shutdown: Some(stop_tx),
            thread: Some(thread),
        }),
        Ok(Err(err)) => Err(PyRuntimeError::new_err(format!(
            "could not build the server runtime: {err}"
        ))),
        // The thread died before signalling. Nothing is listening.
        Err(_) => Err(PyRuntimeError::new_err(
            "the server thread exited before it started serving",
        )),
    }
}

/// Load and open in one step so the whole blocking half runs under one
/// `Python::detach`. Loading a `.kgl` is seconds of I/O and decode on a large
/// graph, and computing the meta-graph is the rest of the startup cost;
/// holding the GIL through either freezes every other thread in the notebook.
fn open_session(source: GraphSource<'_>, name: &str, timeout: u64) -> Result<Session, CoreError> {
    let graph = load_graph(source)?;
    Ok(Session::open_with(
        graph,
        name.to_string(),
        QueryConfig {
            timeout: Duration::from_secs(timeout),
        },
    ))
}

#[pyfunction]
#[pyo3(signature = (path, port = 0, query_timeout_secs = 30))]
fn _serve_path(
    py: Python<'_>,
    path: &str,
    port: u16,
    query_timeout_secs: u64,
) -> PyResult<PyServer> {
    let session = py
        .detach(|| {
            open_session(
                GraphSource::Path(std::path::Path::new(path)),
                path,
                query_timeout_secs,
            )
        })
        .map_err(|err| to_py_err(err, Origin::File))?;
    start(py, session, path.to_string(), port)
}

#[pyfunction]
#[pyo3(signature = (data, name, port = 0, query_timeout_secs = 30))]
fn _serve_bytes(
    py: Python<'_>,
    data: &Bound<'_, PyBytes>,
    name: &str,
    port: u16,
    query_timeout_secs: u64,
) -> PyResult<PyServer> {
    // Borrowed, not copied: the caller already paid for one copy of the image
    // when `to_bytes()` produced it, and the decoded graph is a second. A copy
    // here would make it three. `bytes` is immutable, so the borrow is stable
    // across the GIL release.
    let bytes = data.as_bytes();
    let session = py
        .detach(|| open_session(GraphSource::Bytes(bytes), name, query_timeout_secs))
        .map_err(|err| to_py_err(err, Origin::Buffer))?;
    start(py, session, name.to_string(), port)
}

/// The `kglite-visual` console script.
///
/// Runs the CLI crate's own parser and run sequence — the same code path
/// `main.rs` calls — so the wheel's command and the standalone binary cannot
/// diverge in flags, stdout contract or exit codes.
#[pyfunction]
fn _run_cli(py: Python<'_>, argv: Vec<String>) -> u8 {
    py.detach(|| kglite_visual_cli::run_from(argv))
}

/// The compiled extension, imported by `python/kglite_visual/__init__.py`.
///
/// The name must match `module-name` in pyproject.toml, or the wheel builds a
/// `.so` the package cannot import.
#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(_version, m)?)?;
    m.add_function(wrap_pyfunction!(_serve_path, m)?)?;
    m.add_function(wrap_pyfunction!(_serve_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(_run_cli, m)?)?;
    m.add_class::<PyServer>()?;
    Ok(())
}
