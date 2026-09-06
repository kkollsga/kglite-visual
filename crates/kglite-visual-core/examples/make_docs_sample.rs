//! Generate the small, named graph used by the documentation walkthrough.
//!
//! The checked-in `.kgl` is downloadable from the built docs. Its source of
//! truth is this file: run `make docs-sample`, then `make check-docs-sample`.
//! Keep the exact 17-node/34-relationship contract in step with the walkthrough
//! and its browser test; an unexplained byte diff is not a regeneration.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kglite::api::io::{prepare_kgl_write, write_kgl};
use kglite::api::session::{execute_mut, ExecuteOptions};
use kglite::api::DirGraph;
use kglite_visual_core::{meta_graph, View};

const EXPECTED_NODES: u64 = 17;
const EXPECTED_RELATIONSHIPS: u64 = 34;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = parse_out()?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut graph = DirGraph::new();
    execute_mut(&mut graph, SCRIPT, &ExecuteOptions::eager(&HashMap::new()))?;

    let mut graph = Arc::new(graph);
    let mut view = View::new();
    let meta = meta_graph::compute(&graph, &mut view);
    assert_eq!(u64::from(meta.meta.stats.node_count), EXPECTED_NODES);
    assert_eq!(
        u64::from(meta.meta.stats.edge_count),
        EXPECTED_RELATIONSHIPS
    );

    let triples = graph.get_or_compute_type_connectivity();
    graph.set_type_connectivity(triples);
    prepare_kgl_write(&mut graph);
    write_kgl(&graph, out.to_str().expect("UTF-8 docs sample path"))?;
    eprintln!(
        "wrote {} ({} nodes, {} relationships, {} bytes)",
        out.display(),
        EXPECTED_NODES,
        EXPECTED_RELATIONSHIPS,
        std::fs::metadata(&out)?.len()
    );
    Ok(())
}

fn parse_out() -> Result<PathBuf, String> {
    let mut args = std::env::args().skip(1);
    let mut out = None;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--out" => out = Some(PathBuf::from(args.next().ok_or("--out needs a path")?)),
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(out.unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("docs/_static/team.kgl")
    }))
}

const SCRIPT: &str = r#"
CREATE (ada:Person {id: 1, title: 'Ada', role: 'engineer', location: 'Oslo', years: 6})
CREATE (ben:Person {id: 2, title: 'Ben', role: 'designer', location: 'Bergen', years: 4})
CREATE (chloe:Person {id: 3, title: 'Chloe', role: 'engineer', location: 'Oslo', years: 3})
CREATE (diego:Person {id: 4, title: 'Diego', role: 'researcher', location: 'Madrid', years: 8})
CREATE (emi:Person {id: 5, title: 'Emi', role: 'engineer', location: 'Tokyo', years: 5})
CREATE (farah:Person {id: 6, title: 'Farah', role: 'product', location: 'Cairo', years: 7})
CREATE (grace:Person {id: 7, title: 'Grace', role: 'engineer', location: 'London', years: 2})
CREATE (hugo:Person {id: 8, title: 'Hugo', role: 'data', location: 'Lisbon', years: 5})
CREATE (imani:Person {id: 9, title: 'Imani', role: 'engineer', location: 'Nairobi', years: 4})
CREATE (atlas:Project {id: 101, title: 'Atlas', status: 'active'})
CREATE (beacon:Project {id: 102, title: 'Beacon', status: 'active'})
CREATE (cedar:Project {id: 103, title: 'Cedar', status: 'planning'})
CREATE (rust:Skill {id: 201, title: 'Rust', category: 'engineering'})
CREATE (webgl:Skill {id: 202, title: 'WebGL', category: 'engineering'})
CREATE (graphs:Skill {id: 203, title: 'Graphs', category: 'data'})
CREATE (design:Skill {id: 204, title: 'Design', category: 'creative'})
CREATE (research:Skill {id: 205, title: 'Research', category: 'analysis'})

CREATE (ada)-[:WORKS_ON]->(atlas)
CREATE (ada)-[:WORKS_ON]->(beacon)
CREATE (ben)-[:WORKS_ON]->(beacon)
CREATE (chloe)-[:WORKS_ON]->(atlas)
CREATE (diego)-[:WORKS_ON]->(cedar)
CREATE (emi)-[:WORKS_ON]->(atlas)
CREATE (farah)-[:WORKS_ON]->(beacon)
CREATE (grace)-[:WORKS_ON]->(cedar)
CREATE (hugo)-[:WORKS_ON]->(atlas)
CREATE (imani)-[:WORKS_ON]->(beacon)

CREATE (ada)-[:HAS_SKILL]->(rust)
CREATE (ada)-[:HAS_SKILL]->(webgl)
CREATE (ada)-[:HAS_SKILL]->(graphs)
CREATE (ben)-[:HAS_SKILL]->(design)
CREATE (ben)-[:HAS_SKILL]->(webgl)
CREATE (chloe)-[:HAS_SKILL]->(rust)
CREATE (chloe)-[:HAS_SKILL]->(graphs)
CREATE (diego)-[:HAS_SKILL]->(research)
CREATE (diego)-[:HAS_SKILL]->(graphs)
CREATE (emi)-[:HAS_SKILL]->(rust)
CREATE (emi)-[:HAS_SKILL]->(webgl)
CREATE (farah)-[:HAS_SKILL]->(design)
CREATE (farah)-[:HAS_SKILL]->(research)
CREATE (grace)-[:HAS_SKILL]->(rust)
CREATE (hugo)-[:HAS_SKILL]->(graphs)
CREATE (hugo)-[:HAS_SKILL]->(research)
CREATE (imani)-[:HAS_SKILL]->(rust)
CREATE (imani)-[:HAS_SKILL]->(webgl)

CREATE (ada)-[:MENTORS]->(chloe)
CREATE (diego)-[:MENTORS]->(grace)
CREATE (emi)-[:MENTORS]->(imani)
CREATE (farah)-[:MENTORS]->(ben)
CREATE (beacon)-[:DEPENDS_ON]->(atlas)
CREATE (cedar)-[:DEPENDS_ON]->(beacon)
"#;
