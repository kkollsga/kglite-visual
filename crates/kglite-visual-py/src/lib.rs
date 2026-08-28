//! PyO3 wrapper for kglite-visual.
//!
//! This crate declares no global allocator, and must not (plan D9). KGLite's
//! wheel installs mimalloc; a notebook that imports both wheels loads two
//! extension modules into one interpreter, and two allocators there is the
//! SIGSEGV shape KGLite already runs a canary for. `make
//! check-no-global-allocator` scans this crate's sources so the rule survives
//! a session that has not read this comment.
//!
//! P1 ships the module and one function. `show()`, the background server
//! thread and the `to_bytes()` handover land in P5.

use pyo3::prelude::*;

/// Version of the Rust core this extension was built against.
///
/// Underscore-prefixed: private to the Python package, which re-exports what
/// it wants under its own names.
#[pyfunction]
fn _version() -> &'static str {
    kglite_visual_core::VERSION
}

/// The compiled extension, imported by `python/kglite_visual/__init__.py`.
///
/// The name must match `module-name` in pyproject.toml, or the wheel builds a
/// `.so` the package cannot import.
#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(_version, m)?)?;
    Ok(())
}
