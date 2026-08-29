//! Two jobs, both about the bundle this binary embeds.
//!
//! **Fail legibly when the bundle is missing.** `rust-embed` reports an absent
//! folder as a macro expansion error inside generated code, which reads as a
//! Rust problem and sends the reader to the wrong toolchain entirely.
//!
//! **Rebuild when the bundle changes.** Without the `rerun-if-changed` below,
//! cargo has no idea `frontend/dist` is an input: rebuilding the frontend
//! leaves a binary carrying last week's chunks, and a stale bundle inside a
//! fresh binary reads exactly like a backend bug (CLAUDE.md → "Two toolchains,
//! one gate"). This covers the cargo half; `make check-bundle` covers the half
//! cargo cannot see, which is a bundle older than its own sources.

use std::path::Path;

fn main() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../frontend/dist");
    println!("cargo:rerun-if-changed={}", dist.display());

    if !dist.join("index.html").is_file() {
        println!(
            "cargo:warning=frontend/dist/index.html is missing — the embedded bundle will be \
             empty or this build will fail. Run `make frontend-build` first."
        );
    }
}
