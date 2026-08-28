//! `kglite-visual <file>` — the localhost viewer's entry point.
//!
//! **stdout discipline:** stdout carries exactly one line, the `LaunchInfo`
//! JSON, and nothing else — ever. Diagnostics, warnings and errors go to
//! stderr. An agent parses that line; a stray `println!` here breaks every
//! harness at once, which is why the rule is stated at the top of the file
//! rather than at the one call site that currently obeys it.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use kglite_visual_core::{load_graph, GraphSource, LaunchInfo};

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
        Ok(info) => {
            println!("{}", info.to_json_line());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("kglite-visual: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<LaunchInfo, kglite_visual_core::CoreError> {
    // Load before reporting anything: a LaunchInfo for a graph that turned out
    // to be unreadable would be a URL nothing serves.
    let graph = load_graph(GraphSource::Path(&cli.file))?;
    let type_count = kglite_visual_core::node_counts_by_type(&graph).len();

    // No server yet — P2 lands axum, the embedded assets and the WebSocket.
    // Until then the port the JSON reports is the port that was *asked for*,
    // and that is stated here rather than left for a reader to discover: the
    // field means "bound port" from P2 onward.
    eprintln!(
        "kglite-visual: loaded {} node type(s); no server yet (P2)",
        type_count
    );
    if !cli.no_open {
        eprintln!("kglite-visual: --no-open not set, but there is nothing to open yet");
    }

    Ok(LaunchInfo {
        url: format!("http://127.0.0.1:{}/", cli.port),
        port: cli.port,
        pid: std::process::id(),
        graph: cli.file.display().to_string(),
    })
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
}
