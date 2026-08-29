//! Regenerate the committed test fixture: `meta.kgl` and its positions
//! baseline. Run it with `make fixture`.
//!
//! **Seeded end to end, and verified byte-stable** (`make fixture` regenerates
//! twice and diffs): kglite's `graphgen` is deterministic per seed, the CSV →
//! Cypher translation below is a straight line, and `.kgl`'s save metadata
//! carries a format version and a library version but no timestamp. A fixture
//! that changed on every regeneration could not be an exact baseline, and an
//! exact baseline is what the meta-graph and positions tests are.
//!
//! **Why so small.** The fixture's job is the *meta-graph*: 5 node types and 7
//! relationship types. That shape is identical at 60 persons and at 20 000, so
//! the fixture is the size that still exercises it — the committed `.kgl`
//! stays a few tens of KB instead of megabytes.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kglite::api::graphgen;
use kglite::api::io::{load_file, prepare_kgl_write, write_kgl};
use kglite::api::session::{execute_mut, ExecuteOptions};
use kglite::api::{DirGraph, GraphGenConfig, SpatialConfig};
use kglite_visual_core::{meta_graph, SlotAllocator};

/// Person count. Everything else scales off it inside graphgen.
const PERSONS: u64 = 60;
/// Average KNOWS out-degree.
const KNOWS_PER: u64 = 3;
/// The plan's fixture seed (P2).
const SEED: u64 = 1234;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fixtures = fixture_dir();
    std::fs::create_dir_all(&fixtures)?;

    let staging = tempfile::tempdir()?;
    let cfg = GraphGenConfig {
        persons: PERSONS,
        knows_per: KNOWS_PER,
        seed: SEED,
        zipf: true,
        zipf_exp: 1.6,
    };
    let stats = graphgen(&cfg, staging.path())?;
    eprintln!(
        "graphgen: {} nodes, {} edges (seed {SEED})",
        stats.nodes, stats.edges
    );

    let mut graph = build_graph(staging.path())?;

    // Fill the type-connectivity cache BEFORE saving. `.kgl` persists the
    // cache if it exists and nothing else; a file saved without it loads with
    // triples *derived* from `connection_type_metadata` whose counts are all
    // zero (kglite 0.16.13, `io/file.rs` `apply_to_with`), and that derived
    // zero-count set is then itself cached — so the meta-graph would show
    // seven relationship types each claiming zero edges. Any graph kglite's
    // own `describe()` or Cypher planner has touched already carries the
    // cache; a fixture built straight from CREATE statements does not.
    //
    // The triples are re-sorted before being cached because kglite builds them
    // by draining a `HashMap`, so their order — and therefore the bytes of the
    // section `.kgl` writes them into — varies run to run. Sorting makes the
    // fixture byte-reproducible; `make fixture` regenerates twice and diffs,
    // which is what caught this.
    let mut triples = graph.get_or_compute_type_connectivity();
    triples.sort_by(|a, b| {
        (&a.src, &a.conn, &a.tgt)
            .cmp(&(&b.src, &b.conn, &b.tgt))
            .then_with(|| a.count.cmp(&b.count))
    });
    eprintln!(
        "type connectivity: {} triples cached before save",
        triples.len()
    );
    graph.set_type_connectivity(triples);

    let kgl_path = fixtures.join("meta.kgl");
    prepare_kgl_write(&mut graph);
    write_kgl(&graph, kgl_path.to_str().expect("ASCII fixture path"))?;
    eprintln!(
        "wrote {} ({} bytes)",
        kgl_path.display(),
        std::fs::metadata(&kgl_path)?.len()
    );

    // Read the positions back off the *saved* graph, not the in-memory one:
    // the baseline must describe what a consumer loading the fixture sees.
    let reloaded = load_file(kgl_path.to_str().expect("ASCII fixture path"))?;
    let mut slots = SlotAllocator::new();
    let meta = meta_graph::compute(&reloaded, &mut slots);
    let positions_path = fixtures.join("meta.positions.json");
    std::fs::write(&positions_path, positions_document(&meta))?;
    eprintln!("wrote {}", positions_path.display());

    Ok(())
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// The positions baseline, as JSON.
///
/// Written by hand rather than through serde so the formatting is fixed here
/// and a serde version bump cannot reformat a committed baseline into a diff
/// nobody can explain. Floats print through `{:?}`, which is the shortest
/// round-tripping form — so parsing this file back yields the identical bits.
fn positions_document(meta: &meta_graph::MetaGraphResponse) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"layout\": \"square-spiral-v1\",\n");
    out.push_str(
        "  \"note\": \"Generated from kglite_visual_core::layout. Regenerate with `make fixture`; \
         a diff here is a layout change, and an unexplained one is a defect.\",\n",
    );
    out.push_str("  \"slots\": [\n");
    for (i, node) in meta.meta.nodes.iter().enumerate() {
        let x = meta.points[i * 2];
        let y = meta.points[i * 2 + 1];
        let comma = if i + 1 == meta.meta.nodes.len() {
            ""
        } else {
            ","
        };
        let _ = writeln!(
            out,
            "    {{\"slot\": {}, \"name\": {:?}, \"x\": {:?}, \"y\": {:?}}}{comma}",
            node.slot, node.name, x, y
        );
    }
    out.push_str("  ]\n}\n");
    out
}

/// Build a `DirGraph` from graphgen's CSVs.
///
/// One Cypher statement for the whole graph: nodes bind variables, edges use
/// them. kglite has no DataFrame-free node-ingest path a downstream Rust crate
/// can call — `api::mutation::add_nodes` takes `kglite::datatypes::values::
/// DataFrame`, which the curated `kglite::api` facade does not export, so the
/// function is public but not callable from here (reported upstream). Cypher
/// is the surface that *is* reachable, and at fixture scale it is instant.
fn build_graph(dir: &Path) -> Result<Arc<DirGraph>, Box<dyn std::error::Error>> {
    let mut script = String::new();

    for node_type in graphgen_node_types() {
        for row in read_csv(&dir.join(format!("{node_type}.csv")))? {
            let gid = row.get("gid").expect("graphgen always emits gid").clone();
            let props: Vec<String> = row
                .iter()
                .map(|(col, val)| format!("{col}: {}", cypher_literal(val)))
                .collect();
            let _ = writeln!(
                script,
                "CREATE (n{gid}:{node_type} {{{}}})",
                props.join(", ")
            );
        }
    }

    for (edge_type, csv) in graphgen_edge_types() {
        for row in read_csv(&dir.join(csv))? {
            let src = row.get("src").expect("graphgen edge CSVs are src,dst");
            let dst = row.get("dst").expect("graphgen edge CSVs are src,dst");
            let _ = writeln!(script, "CREATE (n{src})-[:{edge_type}]->(n{dst})");
        }
    }

    let mut graph = DirGraph::new();
    let params = std::collections::HashMap::new();
    let opts = ExecuteOptions::eager(&params);
    execute_mut(&mut graph, &script, &opts)?;

    // City really does carry latitude/longitude (graphgen's own comment says
    // so), so declaring the spatial config is a fact about the data, not a
    // prop: it is what makes the `loc` capability badge true for City. Written
    // through the public field because kglite exposes no setter for it.
    graph.spatial_configs.insert(
        "City".to_string(),
        SpatialConfig {
            location: Some(("latitude".to_string(), "longitude".to_string())),
            ..Default::default()
        },
    );

    Ok(Arc::new(graph))
}

fn graphgen_node_types() -> [&'static str; 5] {
    ["City", "Skill", "Company", "Project", "Person"]
}

fn graphgen_edge_types() -> [(&'static str, &'static str); 7] {
    [
        ("KNOWS", "KNOWS.csv"),
        ("WORKS_AT", "WORKS_AT.csv"),
        ("CONTRIBUTES_TO", "CONTRIBUTES_TO.csv"),
        ("HAS_SKILL", "HAS_SKILL.csv"),
        ("OWNS", "OWNS.csv"),
        ("DEPENDS_ON", "DEPENDS_ON.csv"),
        ("LOCATED_IN", "LOCATED_IN.csv"),
    ]
}

/// Minimal CSV reader for graphgen's output.
///
/// graphgen quotes exactly one field — Person's `embedding` JSON array — and
/// never emits an embedded quote or newline, so the quote handling below is
/// complete for this input and deliberately not a general CSV parser.
fn read_csv(path: &Path) -> std::io::Result<Vec<BTreeMap<String, String>>> {
    let text = std::fs::read_to_string(path)?;
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap_or_default().split(',').collect();
    let mut rows = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let mut row = BTreeMap::new();
        for (col, value) in header.iter().zip(split_csv_line(line)) {
            row.insert((*col).to_string(), value);
        }
        rows.push(row);
    }
    Ok(rows)
}

fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in line.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => fields.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    fields.push(current);
    fields
}

/// Render a CSV field as a Cypher literal.
///
/// Numeric-looking fields stay numeric so the graph's property types match
/// what a DataFrame ingest would produce; everything else is a quoted string.
/// The JSON-array `embedding` column is left as a string: turning it into a
/// list would make the fixture claim vector support it does not have (the
/// `vec` capability badge reads the embedding *store*, not a property).
fn cypher_literal(raw: &str) -> String {
    if raw.parse::<i64>().is_ok() || raw.parse::<f64>().is_ok() {
        // Reject the shapes that parse as a number but are not one here:
        // a leading '[' cannot, and an empty field must become a string.
        if !raw.is_empty() && !raw.starts_with('[') {
            return raw.to_string();
        }
    }
    format!("'{}'", raw.replace('\\', "\\\\").replace('\'', "\\'"))
}
