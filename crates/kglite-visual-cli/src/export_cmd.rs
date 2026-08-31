//! `kglite-visual export` — the graph, or one query's worth of it, as a file
//! somebody else's tool can open (plan E8).
//!
//! **A thin face over `core::export`.** Flags in, bytes on disk, one JSON line
//! out. No format knowledge, no selection logic, no bound of its own.
//!
//! **This is the one caller allowed to export a whole graph, and the reason is
//! not that the CLI is trusted — it is that the question is different.** The
//! server's export answers a button on a bounded, progressively-disclosed view:
//! nobody clicking it asked for 546 850 nodes, and `core::export` has no
//! whole-graph mode for a handler to reach by accident. Here the caller named a
//! `.kgl` file and a path to write it to, at a terminal, with no view in
//! existence and no browser to hang — "dump this file" is exactly what they
//! typed. So this enumerates every node index and passes the list, which is the
//! same mandatory-selection call the server makes; the difference is *who
//! wrote the list*, and here it is the user.
//!
//! `--cypher` is the narrower form, and the one to reach for on a large graph:
//! the query's nodes are the selection, bounded by the same row and byte
//! ceilings every other query obeys.
//!
//! **stdout discipline, same as `render`** (`cli.rs`): exactly one JSON line,
//! after the file is on disk. Diagnostics — the caveats especially — go to
//! stderr.

use std::path::PathBuf;
use std::time::Duration;

use kglite_visual_core::export::{all_nodes, export_nodes, ExportFormat};
use kglite_visual_core::request::CypherRequest;
use kglite_visual_core::{load_graph_with, query, GraphSource, LoadLimits, QueryConfig};
use serde::Serialize;

/// `--format`.
///
/// A local mirror of [`ExportFormat`], for the reason `render_cmd`'s
/// `FormatArg` is one: `kglite-visual-core` is face-agnostic, and a `clap`
/// derive on one of its enums would make the argument parser part of the
/// engine's public API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum FormatArg {
    /// GraphML. Gephi, yEd, Cytoscape.
    Graphml,
    /// Gephi's native XML.
    Gexf,
    /// `id,type,title`, one row per node.
    Csv,
    /// `source,target,type`, one row per edge — the other half of `csv`.
    CsvEdges,
    /// D3's `{"nodes": [...], "links": [...]}`.
    Json,
}

impl From<FormatArg> for ExportFormat {
    fn from(arg: FormatArg) -> Self {
        match arg {
            FormatArg::Graphml => ExportFormat::Graphml,
            FormatArg::Gexf => ExportFormat::Gexf,
            FormatArg::Csv => ExportFormat::Csv,
            FormatArg::CsvEdges => ExportFormat::CsvEdges,
            FormatArg::Json => ExportFormat::Json,
        }
    }
}

#[derive(clap::Args, Debug)]
pub struct ExportArgs {
    /// The `.kgl` file to export.
    pub file: PathBuf,

    #[arg(long, value_enum, default_value = "graphml")]
    pub format: FormatArg,

    /// Export only the nodes a read-only Cypher query returns, rather than the
    /// whole graph.
    ///
    /// The bounded form, and the one to use on a large graph: the query obeys
    /// the same row and byte ceilings every other query does, so what lands on
    /// disk is what the viewer would have been willing to draw.
    #[arg(long, value_name = "QUERY")]
    pub cypher: Option<String>,

    /// Where to write it. Defaults to a name derived from the graph, in the
    /// current directory.
    #[arg(long, short = 'o', value_name = "PATH")]
    pub out: Option<PathBuf>,

    /// Wall-clock ceiling for `--cypher`, in seconds.
    #[arg(long, default_value_t = 30)]
    pub query_timeout_secs: u64,

    /// Refuse a graph estimated to cost more than this many megabytes to load.
    #[arg(long, value_name = "MB")]
    pub max_load_mb: Option<u64>,
}

/// The single stdout line. Field names are the contract, as `render`'s are.
#[derive(Debug, Serialize)]
struct ExportSummary<'a> {
    out: String,
    format: &'a str,
    nodes: u32,
    bytes: usize,
    /// What the file cannot say about itself — today, the edge superset.
    /// Printed to stderr too, because a person watching a shell will not parse
    /// this line.
    notes: Vec<&'a str>,
}

pub fn run(args: &ExportArgs) -> Result<(), Box<dyn std::error::Error>> {
    let graph = load_graph_with(
        GraphSource::Path(&args.file),
        LoadLimits {
            max_load_mb: args.max_load_mb,
        },
    )?;
    let format: ExportFormat = args.format.into();

    let nodes = match &args.cypher {
        Some(text) => {
            let table = query::run_cypher(
                &graph,
                &CypherRequest {
                    query: text.clone(),
                    params: Default::default(),
                    limit: None,
                    as_graph: true,
                },
                QueryConfig {
                    timeout: Duration::from_secs(args.query_timeout_secs),
                },
            )?;
            if table.bound.truncated {
                eprintln!(
                    "kglite-visual: the query was bounded at {} of {} rows; the export is that \
                     subset",
                    table.bound.returned, table.bound.total
                );
            }
            query::node_indices(&table)
        }
        // The whole graph, named out loud. See the module header for why
        // this is a different question from the server's export and not a hole
        // in the same rule; `core::export::all_nodes` is where the enumeration
        // and its justification live.
        None => all_nodes(&graph),
    };

    let exported = export_nodes(&graph, &nodes, format, &args.file.display().to_string())?;
    // The default name is this command's own, not the download name the
    // server sends: a file written beside the `.kgl` is a dump of that file,
    // and `-view` would be describing a screen that does not exist here.
    let out = args
        .out
        .clone()
        .unwrap_or_else(|| default_out_path(args, format));

    for note in exported.notes() {
        eprintln!("kglite-visual: {note}");
    }
    std::fs::write(&out, &exported.bytes)?;

    let summary = ExportSummary {
        out: out.display().to_string(),
        format: exported.format.as_str(),
        nodes: exported.nodes,
        bytes: exported.bytes.len(),
        notes: exported.notes(),
    };
    // THE one stdout line, printed after the file is on disk.
    println!("{}", serde_json::to_string(&summary)?);
    use std::io::Write as _;
    std::io::stdout().flush()?;
    Ok(())
}

/// `<graph stem>.<ext>` in the current directory, when `-o` says nothing.
fn default_out_path(args: &ExportArgs, format: ExportFormat) -> PathBuf {
    let stem = args
        .file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "graph".to_string());
    let suffix = match (&args.cypher, format) {
        // The two CSVs would otherwise collide on one name, and the second run
        // would silently overwrite the first.
        (_, ExportFormat::CsvEdges) => "-edges",
        (Some(_), _) => "-query",
        (None, _) => "",
    };
    PathBuf::from(format!("{stem}{suffix}.{}", format.extension()))
}
