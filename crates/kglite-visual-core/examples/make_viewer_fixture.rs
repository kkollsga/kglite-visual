//! Identity and subset fixture for browser tests. Regenerate with this example.
use kglite::api::io::{prepare_kgl_write, write_kgl};
use kglite::api::session::{execute_mut, ExecuteOptions};
use kglite::api::DirGraph;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut graph = DirGraph::new();
    let summary = std::env::args().any(|arg| arg == "--summary");
    if summary {
        for index in 0..5001 {
            execute_mut(
                &mut graph,
                &format!("CREATE (:Type{index} {{id:{index},title:'Instance {index}'}})"),
                &ExecuteOptions::eager(&Default::default()),
            )?;
        }
    } else {
        execute_mut(&mut graph,"CREATE (a:Person {id:9007199254740993,title:'Ada',score:0,active:false,category:'alpha'}) CREATE (b:Person {id:9007199254740993,title:'Duplicate Ada key',score:10,active:true,category:'beta'}) CREATE (c:Person {id:null,title:'Null key',score:5,category:''}) CREATE (:Disconnected {id:'outside',title:'Outside the relationships'}) CREATE (a)-[:KNOWS]->(b) CREATE (a)-[:KNOWS]->(b) CREATE (b)-[:KNOWS]->(b) CREATE (b)-[:LIKES]->(c)",&ExecuteOptions::eager(&Default::default()))?;
    }
    let triples = graph.get_or_compute_type_connectivity();
    graph.set_type_connectivity(triples);
    let mut graph = std::sync::Arc::new(graph);
    prepare_kgl_write(&mut graph);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(if summary {
        "tests/fixtures/viewer-summary.kgl"
    } else {
        "tests/fixtures/viewer-identity.kgl"
    });
    write_kgl(&graph, path.to_str().expect("fixture path"))?;
    println!("{}", path.display());
    Ok(())
}
