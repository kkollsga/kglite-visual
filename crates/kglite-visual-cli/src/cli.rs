//! `kglite-visual <file>` — the localhost viewer's argument parsing and run
//! sequence.
//!
//! **stdout discipline:** stdout carries exactly one line, the `LaunchInfo`
//! JSON, and nothing else — ever. Diagnostics, warnings and errors go to
//! stderr. An agent parses that line; a stray `println!` here breaks every
//! harness at once, which is why the rule is stated at the top of the file
//! rather than at the one call site that currently obeys it.
//!
//! Two callers: `main.rs`, and the wheel's `kglite-visual` console script,
//! which reaches [`run_from`] through PyO3. Both get the same parser, the same
//! stdout line and the same exit codes, because there is one of each.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use kglite_visual_core::{load_graph, GraphSource, QueryConfig, Session, QUERY_THREAD_STACK_BYTES};

use crate::{assets, server};

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

    /// Wall-clock ceiling for one Cypher query, in seconds.
    ///
    /// A viewer is interactive, so an unbounded query is a hung tab; the
    /// default is what an accidental cartesian product costs. Raise it for a
    /// deliberate analytical query on a large graph.
    #[arg(long, default_value_t = 30)]
    query_timeout_secs: u64,
}

/// Parse `args` and run to completion, returning the process exit code.
///
/// `u8` rather than `ExitCode`: the console-script caller crosses a PyO3
/// boundary and hands the number to `sys.exit`, and `ExitCode` cannot be read
/// back out. `main.rs` converts.
///
/// Argument errors and `--help` are clap's to handle, and clap exits the
/// process for both. That is the same behaviour under `python -m` as under the
/// binary, which is the point of sharing the parser.
pub fn run_from<I, T>(args: I) -> u8
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::parse_from(args);
    match run(&cli) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("kglite-visual: {err}");
            1
        }
    }
}

fn run(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    // Load before binding: a LaunchInfo for a graph that turned out to be
    // unreadable would be a URL nothing serves.
    let graph = load_graph(GraphSource::Path(&cli.file))?;
    let session = Session::open_with(
        graph,
        cli.file.display().to_string(),
        QueryConfig {
            timeout: Duration::from_secs(cli.query_timeout_secs),
        },
    );
    let info = session.info();
    eprintln!(
        "kglite-visual: {} node types, {} nodes, {} edges; detail tier {}",
        info.stats.node_type_count,
        info.stats.node_count,
        info.stats.edge_count,
        serde_json::to_string(&info.tier)?
    );
    eprintln!(
        "kglite-visual: response bound {} nodes/expansion, {} rows/query, {}s query timeout",
        info.max_expansion_nodes, info.max_query_rows, info.query_timeout_secs
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

    // `thread_stack_size` reaches the **blocking** pool as well as the worker
    // threads, and that is the half that matters: every Cypher execution runs
    // in `spawn_blocking`, and kglite's parser overflows tokio's 2 MiB default.
    // Verified against tokio 1.53.1 rather than assumed — `runtime/blocking/
    // pool.rs` copies `builder.thread_stack_size` into the pool's `stack_size`
    // and applies it to every thread it spawns. The failure mode if it did not
    // is a stack overflow inside a blocking task: a process abort with no
    // message naming the query.
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
        assert_eq!(defaults.query_timeout_secs, 30);

        let slow = Cli::parse_from(["kglite-visual", "--query-timeout-secs", "300", "g.kgl"]);
        assert_eq!(slow.query_timeout_secs, 300);
    }

    #[test]
    fn the_runtime_stack_is_the_engines_number_not_a_local_literal() {
        // The floor kglite documents. A second literal here is a copy that
        // stops moving when the engine's does, and the failure it hides is a
        // stack overflow inside a blocking task.
        assert_eq!(
            QUERY_THREAD_STACK_BYTES,
            kglite_visual_core::query::QUERY_THREAD_STACK_BYTES
        );
        const {
            assert!(
                QUERY_THREAD_STACK_BYTES >= 8 * 1024 * 1024,
                "kglite's Cypher parser overflows anything smaller"
            )
        };
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
