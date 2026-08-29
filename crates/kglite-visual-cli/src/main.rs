//! `kglite-visual <file>` — the localhost viewer's entry point.
//!
//! **stdout discipline:** stdout carries exactly one line, the `LaunchInfo`
//! JSON, and nothing else — ever. Diagnostics, warnings and errors go to
//! stderr. An agent parses that line; a stray `println!` here breaks every
//! harness at once, which is why the rule is stated at the top of the file
//! rather than at the one call site that currently obeys it.

mod api;
mod assets;
mod server;
mod ws;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use kglite_visual_core::{load_graph, GraphSource, Session};

/// Stack size for the runtime's threads.
///
/// kglite's Cypher parser overflows tokio's 2 MiB default; upstream ships the
/// number as `QUERY_THREAD_STACK_SIZE` and every embedder is expected to honour
/// it. P2 runs no Cypher, but the runtime is built once and the failure mode is
/// a stack overflow inside a blocking task — a crash with no useful message —
/// so the size is set here rather than left for P3 to discover.
const QUERY_THREAD_STACK_BYTES: usize = 8 * 1024 * 1024;

#[derive(Parser, Debug)]
#[command(
    name = "kglite-visual",
    version,
    about = "Interactive viewer for .kgl knowledge graphs"
)]
struct Cli {
    /// The `.kgl` file to view.
    file: PathBuf,

    /// Port to bind. `0` asks the OS for a free one; the resolved port is
    /// always reported in the stdout JSON, so nothing needs to guess.
    #[arg(long, default_value_t = 0)]
    port: u16,

    /// Do not open a browser. How every agent and CI invocation runs;
    /// opening a browser is the interactive default, not the only mode.
    #[arg(long)]
    no_open: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("kglite-visual: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    // Load before binding: a LaunchInfo for a graph that turned out to be
    // unreadable would be a URL nothing serves.
    let graph = load_graph(GraphSource::Path(&cli.file))?;
    let session = Session::open(graph, cli.file.display().to_string());
    let info = session.info();
    eprintln!(
        "kglite-visual: {} node types, {} nodes, {} edges; detail tier {}",
        info.stats.node_type_count,
        info.stats.node_count,
        info.stats.edge_count,
        serde_json::to_string(&info.tier)?
    );

    let embedded = assets::embedded_file_count();
    if embedded == 0 {
        // Not fatal — the JSON API still works, and saying so beats a browser
        // tab that renders nothing for a reason the server knows and withheld.
        eprintln!(
            "kglite-visual: WARNING no frontend assets are embedded in this binary; \
             only the /api endpoints will answer"
        );
    } else {
        eprintln!("kglite-visual: {embedded} embedded frontend asset(s)");
    }

    let bound = server::bind(session, cli.port, cli.file.display().to_string())?;
    let url = bound.info.url.clone();

    // THE one stdout line. Printed after the port is resolved and the socket
    // is listening, so a harness that reads it can connect immediately.
    println!("{}", bound.info.to_json_line());
    use std::io::Write as _;
    std::io::stdout().flush()?;

    if !cli.no_open {
        // The `webbrowser` crate, not `open`: it knows about WSL, headless
        // servers and BROWSER=, and it reports failure instead of spawning a
        // process that silently does nothing.
        if let Err(err) = webbrowser::open(&url) {
            eprintln!("kglite-visual: could not open a browser ({err}); visit {url}");
        }
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(QUERY_THREAD_STACK_BYTES)
        .build()?
        .block_on(bound.serve())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn clap_definition_is_valid() {
        // clap's own assertions catch conflicting/duplicated arg definitions,
        // which otherwise only panic at runtime on first parse.
        Cli::command().debug_assert();
    }

    #[test]
    fn flags_parse_as_the_documented_launch_contract() {
        let cli = Cli::parse_from(["kglite-visual", "--port", "8731", "--no-open", "g.kgl"]);
        assert_eq!(cli.port, 8731);
        assert!(cli.no_open);
        assert_eq!(cli.file, PathBuf::from("g.kgl"));

        let defaults = Cli::parse_from(["kglite-visual", "g.kgl"]);
        assert_eq!(defaults.port, 0, "0 means OS-assigned");
        assert!(!defaults.no_open);
    }

    #[test]
    fn the_binary_carries_a_frontend_bundle() {
        // The packaged-consumer question a source-tree test usually cannot
        // ask: is the bundle actually *in* here? It can, because rust-embed's
        // `debug-embed` makes the test binary carry the same assets the
        // release binary does.
        assert!(
            assets::embedded_file_count() > 0,
            "no embedded assets — build the frontend before the binary \
             (`make frontend-build`)"
        );
    }
}
