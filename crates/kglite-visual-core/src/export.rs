//! Getting the graph out of the viewer and into somebody else's tool (plan E8).
//!
//! kglite already writes GraphML, GEXF, D3 JSON and CSV. What this module adds
//! is the *scope*, and the scope is the whole point: every one of those
//! functions takes an `Option<&CurrentSelection>` where `None` means **the
//! entire graph**. On a 546 850-node file served by a viewer whose whole design
//! is a response bound, an accidental `None` is a defect wearing an export
//! button — the browser asks for "what I am looking at" and receives half a
//! gigabyte.
//!
//! **So there is no `Option` in this module's signatures.** [`export_nodes`]
//! takes the node list, builds the selection itself and always passes `Some`.
//! Nothing a caller can write reaches the whole-graph path without naming every
//! node first — which is what [`all_nodes`] does, out loud, for the one caller
//! whose user typed `kglite-visual export <file>` at a terminal. The server has
//! no route to it at all.
//!
//! **One honesty note travels with every export**, because it is a thing a user
//! would otherwise discover in Gephi: **the edge set can be a superset of the
//! canvas.** kglite writes every edge it holds between two exported nodes; the
//! view drops links its byte budget refused (`GraphSliceMeta::link_bound`) and
//! never learns about edges whose endpoints both arrived by different routes.
//! So an export of a view is node-exact and edge-inclusive. That is the right
//! direction to be wrong in — a file missing edges the screen showed would be
//! worse — but it is not what "export what I see" sounds like.
//!
//! There used to be a second note: GraphML carried no `attr.name="label"` key,
//! so a Gephi import showed `n0`, `n1`, … kglite 0.16.16 added it (nodes carry
//! the title, edges the connection type) and the note is gone. The test that
//! asserted its absence now asserts the key's presence, so the two formats
//! stay in step.

use kglite::api::io::{to_csv, to_d3_json, to_gexf, to_graphml};
use kglite::api::{CurrentSelection, DirGraph, NodeIndex};
use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// What an export can be written as.
///
/// `Csv` and `CsvEdges` are two formats rather than one archive because there
/// is no zip writer in this tree and adding one — a crate, a licence, a
/// lockfile entry, a gate check's opinion — to bundle two text files is a poor
/// trade. Two downloads is a worse UI than one and a much smaller dependency
/// surface, and the nodes file alone is what most spreadsheet users are after.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExportFormat {
    /// XML. Gephi, yEd, Cytoscape.
    Graphml,
    /// Gephi's own XML.
    Gexf,
    /// `id,type,title` — one row per node.
    Csv,
    /// `source,target,type` — one row per edge. The other half of `csv`.
    CsvEdges,
    /// D3's `{"nodes": [...], "links": [...]}`.
    Json,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 5] = [
        ExportFormat::Graphml,
        ExportFormat::Gexf,
        ExportFormat::Csv,
        ExportFormat::CsvEdges,
        ExportFormat::Json,
    ];

    /// The token a caller names it by, on the query string and at the CLI.
    pub const fn as_str(self) -> &'static str {
        match self {
            ExportFormat::Graphml => "graphml",
            ExportFormat::Gexf => "gexf",
            ExportFormat::Csv => "csv",
            ExportFormat::CsvEdges => "csv-edges",
            ExportFormat::Json => "json",
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            ExportFormat::Graphml => "graphml",
            ExportFormat::Gexf => "gexf",
            ExportFormat::Csv | ExportFormat::CsvEdges => "csv",
            ExportFormat::Json => "json",
        }
    }

    /// What a browser should be told it is receiving.
    ///
    /// `charset=utf-8` on every one of them, and that is not decoration: three
    /// of these formats are plain text with no in-band encoding declaration, and
    /// a Norwegian node title read as latin-1 turns `Ærfugl` into mojibake in a
    /// file the user then keeps.
    pub const fn content_type(self) -> &'static str {
        match self {
            // The registered types. `application/xml` for both rather than a
            // vendor tree: nothing dispatches on a `.graphml` media type, and
            // an unregistered one only stops a browser from previewing it.
            ExportFormat::Graphml | ExportFormat::Gexf => "application/xml; charset=utf-8",
            ExportFormat::Csv | ExportFormat::CsvEdges => "text/csv; charset=utf-8",
            ExportFormat::Json => "application/json; charset=utf-8",
        }
    }

    pub fn parse(token: &str) -> Result<Self, CoreError> {
        ExportFormat::ALL
            .into_iter()
            .find(|format| format.as_str() == token)
            .ok_or_else(|| {
                CoreError::Request(format!(
                    "'{token}' is not an export format. Try one of: {}",
                    ExportFormat::ALL.map(|f| f.as_str()).join(", ")
                ))
            })
    }
}

/// The honesty line every export answer repeats, in the truncation banner's
/// voice.
///
/// One constant because it is written into an HTTP header, an MCP tool result
/// and the CLI's stdout line, and three wordings of one caveat is two of them
/// going stale.
///
/// Worded for the *selection* rather than for the canvas: the CLI's
/// whole-graph and `--cypher` dumps go through the same function, and "more
/// than the canvas drew" would describe a screen that does not exist for
/// either. What holds for all three is that the nodes are the ones asked for
/// and the edges are every edge between them.
pub const EDGE_SUPERSET_NOTE: &str =
    "the nodes are exactly the ones selected; the edges are every edge this graph holds between \
     them, which can be MORE than you saw — a link the view's byte budget refused, or one a \
     query's rows never mentioned, is still an edge in this file";

/// One export, ready to write or to send.
#[derive(Debug, Clone)]
pub struct ExportedView {
    pub format: ExportFormat,
    pub bytes: Vec<u8>,
    /// Nodes the selection named — the count a caller checks against what it
    /// thought it was exporting.
    pub nodes: u32,
    /// A filename with no directory part, built from the graph's own name.
    /// Whatever it is called, it is UTF-8 and may well not be ASCII.
    pub filename: String,
}

impl ExportedView {
    /// The caveats that apply to *this* export.
    ///
    /// One today, and it is format-independent: [`EDGE_SUPERSET_NOTE`] is true
    /// of every format. The signature stays a `Vec` because the per-format
    /// shape is the honest one — GraphML carried a second note until kglite
    /// 0.16.16 closed the gap, and the next format-specific caveat lands here.
    pub fn notes(&self) -> Vec<&'static str> {
        vec![EDGE_SUPERSET_NOTE]
    }
}

/// Write exactly these nodes, and whatever edges the graph holds between them.
///
/// **The one export entry point, and it has no whole-graph mode.** The node
/// list is the scope; an empty one is refused rather than silently answered
/// with an empty file, because "export" on a view with nothing loaded is a
/// question with a better answer than three bytes of CSV header.
pub fn export_nodes(
    graph: &DirGraph,
    nodes: &[NodeIndex],
    format: ExportFormat,
    label: &str,
) -> Result<ExportedView, CoreError> {
    if nodes.is_empty() {
        return Err(CoreError::Request(
            "there is nothing to export: no instance nodes are loaded. Expand a type or run a \
             query with 'show in graph' first."
                .to_string(),
        ));
    }

    // The selection is built here and passed as `Some` unconditionally. This is
    // the choke point the module header is about: `None` is kglite's
    // whole-graph mode, and no caller of this function can reach it.
    let mut selection = CurrentSelection::new();
    selection
        .get_level_mut(0)
        .expect("CurrentSelection::new always opens a level")
        .add_selection(None, nodes.to_vec());

    let text = match format {
        ExportFormat::Graphml => to_graphml(graph, Some(&selection)),
        ExportFormat::Gexf => to_gexf(graph, Some(&selection)),
        ExportFormat::Json => to_d3_json(graph, Some(&selection)),
        ExportFormat::Csv => to_csv(graph, Some(&selection)).map(|(nodes, _)| nodes),
        ExportFormat::CsvEdges => to_csv(graph, Some(&selection)).map(|(_, edges)| edges),
    }
    // kglite's exporters answer `Result<_, String>`; the message is the
    // engine's own and is forwarded rather than replaced.
    .map_err(CoreError::Request)?;

    Ok(ExportedView {
        format,
        bytes: text.into_bytes(),
        nodes: nodes.len() as u32,
        filename: filename_for(label, format),
    })
}

/// Every node in the graph, in index order — the caller-writes-the-list form
/// of a whole-graph dump.
///
/// **Not a shortcut past [`export_nodes`]'s mandatory selection; the long way
/// round it, taken deliberately.** The module header's rule is that nothing
/// reaches kglite's `None` (= everything) by passing the wrong argument, and
/// this does not: it enumerates, and the caller that asks for it has said out
/// loud that a whole graph is what it means. `kglite-visual export <file>` at a
/// terminal is that caller; a browser clicking Export on a bounded view is not,
/// and the server has no route here.
///
/// Through `type_indices` rather than a storage node iterator: the trait that
/// offers one is not part of `kglite::api`, and every node in a `.kgl` carries
/// a type, so the type index *is* the node set. Sorted, because the store's
/// iteration order over type names is a hash map's and two dumps of one file
/// should be byte-identical.
pub fn all_nodes(graph: &DirGraph) -> Vec<NodeIndex> {
    let mut all: Vec<NodeIndex> = graph
        .type_indices
        .iter()
        // `to_vec` rather than `iter`: the store hands out a borrowed view that
        // does not outlive the closure, so an iterator over it cannot be
        // flattened into the outer chain.
        .flat_map(|(_, nodes)| nodes.to_vec())
        .collect();
    all.sort_by_key(|index| index.index());
    all
}

/// `<graph stem>.<ext>`, with the stem reduced to something a filesystem and a
/// header can both carry.
///
/// The stem keeps its non-ASCII letters — a Norwegian graph is called what it
/// is called, and RFC 5987 exists so a header can say so. What it loses is the
/// directory separators, quotes and control characters that would let a graph's
/// *name* decide where a download lands.
fn filename_for(label: &str, format: ExportFormat) -> String {
    let stem: String = std::path::Path::new(label)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '"' | '\'' | ';' | '*' | '?' | '<' | '>' | '|' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let stem = stem.trim_matches(['.', '-', ' ']).to_string();
    let stem = if stem.is_empty() {
        "graph".to_string()
    } else {
        stem
    };
    format!("{stem}-view.{}", format.extension())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kglite::api::session::{execute_mut, ExecuteOptions};
    use std::collections::HashMap;

    /// The consumer quirk matrix (plan E8), as one graph.
    ///
    /// Every node here is a shape that has broken somebody's importer: a title
    /// with a quote and a comma (CSV escaping), Norwegian letters (encoding), a
    /// self-loop (Gephi refuses some), a node with no properties at all beyond
    /// its title, and an edge to a node that will be left *outside* the
    /// selection (the boundary edge — it must not be written, because its
    /// target id names nothing in the file).
    fn quirky() -> DirGraph {
        let mut graph = DirGraph::new();
        let empty = HashMap::new();
        execute_mut(
            &mut graph,
            r#"
            CREATE (a:Item {title: 'He said "hei, du", loudly', size: 3})
            CREATE (b:Item {title: 'Ærfugl på Åsgård — ø', size: 1})
            CREATE (c:Item {title: 'bare'})
            CREATE (d:Item {title: 'outsider'})
            CREATE (a)-[:LINKS]->(b)
            CREATE (b)-[:LINKS]->(b)
            CREATE (c)-[:LINKS]->(d)
            "#,
            &ExecuteOptions::eager(&empty),
        )
        .expect("the quirk fixture builds");
        graph
    }

    /// The three nodes inside the selection; `outsider` is deliberately left
    /// out so the boundary edge has somewhere to fail.
    fn selected(graph: &DirGraph) -> Vec<NodeIndex> {
        let mut inside: Vec<NodeIndex> = graph
            .type_indices
            .get("Item")
            .expect("Item exists")
            .iter()
            .filter(|index| {
                graph
                    .node_view(*index)
                    .map(|node| {
                        !crate::values::value_to_display(&node.title()).contains("outsider")
                    })
                    .unwrap_or(false)
            })
            .collect();
        inside.sort_by_key(|index| index.index());
        assert_eq!(
            inside.len(),
            3,
            "the fixture holds four Items, three inside"
        );
        inside
    }

    fn text_of(format: ExportFormat) -> String {
        let graph = quirky();
        let nodes = selected(&graph);
        let out = export_nodes(&graph, &nodes, format, "/tmp/Sokkelkart Æ.kgl")
            .expect("the export succeeds");
        assert_eq!(out.nodes, 3);
        String::from_utf8(out.bytes).expect("every exporter writes UTF-8")
    }

    #[test]
    fn every_format_keeps_norwegian_letters_and_excludes_the_boundary_edge() {
        for format in ExportFormat::ALL {
            let text = text_of(format);
            if format != ExportFormat::CsvEdges {
                assert!(
                    text.contains("Ærfugl på Åsgård"),
                    "{} mangled the non-ASCII title:\n{text}",
                    format.as_str()
                );
            }
            assert!(
                !text.contains("outsider"),
                "{} wrote a node outside the selection",
                format.as_str()
            );
            // The boundary edge `c -> d` leaves the selection, so it must not be
            // written: its target id would name nothing in the file, and an
            // importer that resolves ids either drops the edge silently or
            // invents a node for it.
            //
            // Counted structurally, per format, and not by counting the
            // relationship type's name: kglite 0.16.16 made GraphML write the
            // type twice per edge (`edge_type` and the new Gephi-readable
            // `edge_label`), which doubled a name count on a file whose edge
            // count had not changed.
            let edges = match format {
                ExportFormat::Graphml | ExportFormat::Gexf => text.matches("<edge ").count(),
                ExportFormat::Json => text.matches(r#"{"source":"#).count(),
                // Data rows, so the header line does not count as an edge.
                ExportFormat::CsvEdges => text.lines().filter(|l| !l.is_empty()).count() - 1,
                // The nodes-only CSV holds no relationship at all.
                ExportFormat::Csv => 0,
            };
            let expected = match format {
                ExportFormat::Csv => 0,
                _ => 2,
            };
            assert_eq!(
                edges,
                expected,
                "{} wrote {edges} LINKS edges, expected {expected} (a->b and b->b, never c->d)",
                format.as_str()
            );
        }
    }

    #[test]
    fn a_quote_and_a_comma_survive_the_csv_round_trip() {
        let text = text_of(ExportFormat::Csv);
        // kglite quotes the field and doubles the inner quote — the RFC 4180
        // form every spreadsheet reads. Asserted on the exact bytes because
        // "it contains the title" would pass on an unquoted field that breaks
        // the row into three columns.
        assert!(
            text.contains(r#""He said ""hei, du"", loudly""#),
            "the CSV did not escape a quoted, comma-bearing title:\n{text}"
        );
        // One header plus one row per node, and no row split by the comma.
        let rows: Vec<&str> = text.lines().collect();
        assert_eq!(rows.len(), 4, "{rows:?}");
        assert_eq!(rows[0], "id,type,title");
    }

    #[test]
    fn a_self_loop_is_written_once_and_a_bare_node_still_gets_a_row() {
        let edges = text_of(ExportFormat::CsvEdges);
        let loop_rows: Vec<&str> = edges
            .lines()
            .skip(1)
            .filter(|row| {
                let mut cells = row.split(',');
                cells.next() == cells.next()
            })
            .collect();
        assert_eq!(loop_rows.len(), 1, "the self-loop: {edges}");

        // A node carrying nothing but a title is still a node. GraphML omits
        // the properties element for it rather than writing an empty one, and
        // an importer that required the element would drop the node.
        let graphml = text_of(ExportFormat::Graphml);
        assert!(
            graphml.contains(">bare<"),
            "the zero-property node vanished"
        );
    }

    #[test]
    fn each_format_is_internally_consistent() {
        let json: serde_json::Value =
            serde_json::from_str(&text_of(ExportFormat::Json)).expect("the D3 JSON parses");
        assert_eq!(json["nodes"].as_array().map(Vec::len), Some(3));
        assert_eq!(json["links"].as_array().map(Vec::len), Some(2));

        for format in [ExportFormat::Graphml, ExportFormat::Gexf] {
            let xml = text_of(format);
            assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
            let root = if format == ExportFormat::Graphml {
                "graphml"
            } else {
                "gexf"
            };
            assert!(xml.trim_end().ends_with(&format!("</{root}>")), "{xml}");
            assert_eq!(
                xml.matches("<node ").count() + xml.matches("<node>").count(),
                3,
                "{} node count:\n{xml}",
                format.as_str()
            );
            // The quote in a title has to leave the XML as an entity or the
            // document does not parse.
            assert!(
                !xml.contains(r#"He said "hei"#),
                "{} left a raw quote in an attribute-bearing document",
                format.as_str()
            );
        }
    }

    /// Both XML formats name their elements, and the export note says nothing
    /// about labels any more.
    ///
    /// This test is the previous one inverted. It used to assert that GraphML
    /// had **no** `attr.name="label"` key — the upstream gap, pinned as current
    /// behaviour so it would retire itself — and it fired the moment the floor
    /// moved to a kglite that closed it (0.16.16, verified on the exported
    /// bytes: `node_label` carries the title, `edge_label` the connection
    /// type). Kept in the presence direction so a regression on either side is
    /// still a red test rather than a Gephi import full of `n0`, `n1`, ….
    #[test]
    fn both_xml_formats_carry_a_label_and_graphml_keeps_its_title_key() {
        let graphml = text_of(ExportFormat::Graphml);
        assert!(
            graphml.contains(r#"attr.name="title""#),
            "the title key went away; a reader consuming it is now broken"
        );
        assert!(
            graphml.contains(r#"attr.name="label""#),
            "kglite stopped writing the GraphML label key — a Gephi import is back to n0, n1, …"
        );
        assert!(
            text_of(ExportFormat::Gexf).contains("label="),
            "GEXF stopped naming its nodes"
        );
    }

    /// The export notes stopped mentioning labels, and must not start again.
    #[test]
    fn no_export_note_still_warns_about_a_missing_graphml_label() {
        for format in ExportFormat::ALL {
            let view = ExportedView {
                format,
                bytes: Vec::new(),
                nodes: 0,
                filename: String::new(),
            };
            let notes = view.notes();
            assert!(
                !notes.is_empty(),
                "{} lost its edge-superset note",
                format.as_str()
            );
            assert!(
                notes.iter().all(|note| !note.contains("label")),
                "{} still carries a label caveat kglite 0.16.16 made false",
                format.as_str()
            );
        }
    }

    #[test]
    fn an_empty_selection_is_refused_rather_than_exported() {
        // The whole-graph dump this module exists to make unreachable would
        // arrive exactly here: kglite reads an empty selection as an empty
        // answer, but a caller that got a three-byte file back would reasonably
        // conclude the export was broken rather than the view empty.
        let graph = quirky();
        let err = export_nodes(&graph, &[], ExportFormat::Csv, "g.kgl")
            .expect_err("an empty view has nothing to export");
        assert!(err.to_string().contains("nothing to export"), "{err}");
    }

    #[test]
    fn a_graph_name_cannot_decide_where_a_download_lands() {
        assert_eq!(
            filename_for("/tmp/Sokkelkart Æ.kgl", ExportFormat::Graphml),
            "Sokkelkart Æ-view.graphml",
            "a Norwegian name keeps its letters"
        );
        for hostile in ["../../etc/passwd", "a\"b", "a\nb", "..", "", "."] {
            let name = filename_for(hostile, ExportFormat::Csv);
            assert!(!name.contains('/'), "{hostile:?} -> {name}");
            assert!(!name.contains('\\'), "{hostile:?} -> {name}");
            assert!(!name.contains('"'), "{hostile:?} -> {name}");
            assert!(!name.starts_with('.'), "{hostile:?} -> {name}");
            assert!(name.ends_with("-view.csv"), "{hostile:?} -> {name}");
        }
    }

    #[test]
    fn every_format_token_round_trips_and_an_unknown_one_names_the_alternatives() {
        for format in ExportFormat::ALL {
            assert_eq!(ExportFormat::parse(format.as_str()).unwrap(), format);
        }
        let err = ExportFormat::parse("xlsx").expect_err("there is no spreadsheet exporter");
        let message = err.to_string();
        for format in ExportFormat::ALL {
            assert!(message.contains(format.as_str()), "{message}");
        }
    }
}
