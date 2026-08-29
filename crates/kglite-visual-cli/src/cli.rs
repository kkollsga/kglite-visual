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
//!
//! **Two modes, one stdout rule.** `kglite-visual <file>` serves; `kglite-visual
//! render <file> …` draws one image and exits (plan D13). Each prints exactly
//! one JSON line — the launch contract, or the render summary — so "read one
//! line of stdout" stays the whole agent-facing protocol.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use kglite_visual_core::{load_graph, GraphSource, QueryConfig, Session, QUERY_THREAD_STACK_BYTES};

use crate::render_cmd::{self, RenderArgs};
use crate::{assets, server};

#[derive(Parser, Debug)]
#[command(
    name = "kglite-visual",
    version,
    about = "Interactive viewer for .kgl knowledge graphs",
    // `kglite-visual <file>` predates the subcommand and stays the bare form:
    // it is what the wheel's console script, the e2e harness and every existing
    // instruction run. `subcommand_negates_reqs` is what lets the positional
    // `file` stay required for that form while `render` supplies its own, and
    // `args_conflicts_with_subcommands` refuses the ambiguous middle
    // (`kglite-visual a.kgl render`) instead of silently picking one reading.
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// The `.kgl` file to view.
    file: Option<PathBuf>,

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

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Draw one image of this graph and exit — no server, no browser.
    Render(RenderArgs),
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
    if cli.command.is_none() && cli.file.is_none() {
        // `subcommand_negates_reqs` makes the positional optional at the parser
        // level *unconditionally*, so a bare `kglite-visual` now parses instead
        // of being refused. Before the render subcommand existed it produced
        // clap's usage text and clap's exit code; a one-line message and a
        // different code would be a silent change to the contract a harness
        // sees. This hands the case back to clap, which formats and exits.
        use clap::CommandFactory as _;
        Cli::command()
            .error(
                clap::error::ErrorKind::MissingRequiredArgument,
                "the following required arguments were not provided:\n  <FILE>",
            )
            .exit();
    }
    let outcome = match &cli.command {
        Some(Command::Render(args)) => render_cmd::run(args),
        None => run(&cli),
    };
    match outcome {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("kglite-visual: {err}");
            1
        }
    }
}

fn run(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    // `run_from` has already handed the empty invocation back to clap, so this
    // is the type system catching up with a case that cannot reach here.
    let file = cli
        .file
        .as_ref()
        .ok_or("no .kgl file was given; run `kglite-visual --help`")?;
    // Load before binding: a LaunchInfo for a graph that turned out to be
    // unreadable would be a URL nothing serves.
    let graph = load_graph(GraphSource::Path(file))?;
    let session = Session::open_with(
        graph,
        file.display().to_string(),
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

    let bound = server::bind(session, cli.port, file.display().to_string())?;
    let url = bound.info.url.clone();

    // `thread_stack_size` reaches the **blocking** pool as well as the worker
    // threads, and that is the half that matters: every Cypher execution runs
    // in `spawn_blocking`, and kglite's parser overflows tokio's 2 MiB default.
    // Verified against tokio 1.53.1 rather than assumed — `runtime/blocking/
    // pool.rs` copies `builder.thread_stack_size` into the pool's `stack_size`
    // and applies it to every thread it spawns. The failure mode if it did not
    // is a stack overflow inside a blocking task: a process abort with no
    // message naming the query.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(QUERY_THREAD_STACK_BYTES)
        .build()?;

    // Armed BEFORE the stdout line: the line is what a supervisor kills on,
    // so the stop handler must predate it (the first CI run caught the
    // reversed order as a SIGTERM death with no destructor).
    let shutdown = server::Bound::arm_shutdown(&runtime);

    // THE one stdout line. Printed after the port is resolved, the socket is
    // listening, and the stop handler is armed — so a harness that reads it
    // can connect, or kill, immediately.
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

    runtime.block_on(bound.serve_until(shutdown))?;
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
        assert_eq!(cli.file, Some(PathBuf::from("g.kgl")));

        let defaults = Cli::parse_from(["kglite-visual", "g.kgl"]);
        assert_eq!(defaults.port, 0, "0 means OS-assigned");
        assert!(!defaults.no_open);
        assert_eq!(defaults.query_timeout_secs, 30);

        let slow = Cli::parse_from(["kglite-visual", "--query-timeout-secs", "300", "g.kgl"]);
        assert_eq!(slow.query_timeout_secs, 300);
    }

    #[test]
    fn the_render_subcommand_does_not_disturb_the_bare_serve_form() {
        // The regression this pairing exists to catch: adding a subcommand to a
        // parser whose first positional was required is exactly how a working
        // `kglite-visual g.kgl` becomes a usage error. Both forms, one test.
        let serve = Cli::parse_from(["kglite-visual", "g.kgl"]);
        assert!(serve.command.is_none());
        assert_eq!(serve.file, Some(PathBuf::from("g.kgl")));

        let render = Cli::parse_from([
            "kglite-visual",
            "render",
            "g.kgl",
            "--cypher",
            "MATCH (n) RETURN n",
            "--format",
            "png",
            "--seed",
            "7",
        ]);
        let Some(Command::Render(args)) = render.command else {
            panic!("the subcommand did not dispatch");
        };
        assert_eq!(args.file, PathBuf::from("g.kgl"));
        assert_eq!(args.cypher.as_deref(), Some("MATCH (n) RETURN n"));
        assert_eq!(args.format, crate::render_cmd::FormatArg::Png);
        assert_eq!(args.seed, 7);
        assert_eq!(
            args.theme,
            crate::render_cmd::ThemeArg::Dark,
            "dark is app parity"
        );
    }

    #[test]
    fn the_three_render_sources_are_mutually_exclusive() {
        // A caller who wrote two of them meant one; picking silently would draw
        // a picture of a question nobody asked.
        assert!(Cli::try_parse_from([
            "kglite-visual",
            "render",
            "g.kgl",
            "--meta",
            "--cypher",
            "MATCH (n) RETURN n",
        ])
        .is_err());
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
