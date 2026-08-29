//! `kglite-visual <file>` — the one binary.
//!
//! Everything it does lives in the library beside it, so the wheel's console
//! script runs the same parser and the same server rather than a copy
//! (plan D9, lib-link). Keep this file thin: a second front door that grows
//! logic here is a front door the Python side does not have.

use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(kglite_visual_cli::run_from(std::env::args_os()))
}
