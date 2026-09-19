//! Reproducible local workload baseline; numbers describe this host, not a service SLO.
use serde_json::{json, Value};
use std::time::Instant;
use weave_contract::{ContextSelection, GraphRef, Program, QueryPlan, VERSION};
use weave_engine::{ClusterRequest, Engine, HostContext};
fn fixture(nodes: usize, edges: usize, generation: usize) -> Value {
    json!({"nodes":(0..nodes).map(|i| json!({"id":format!("n{i}"),"entity_id":format!("entity-{i}"),"space_id":"sample","properties":{"generation":generation}})).collect::<Vec<_>>(),
    "edges":(0..edges).map(|i|json!({"id":format!("e{i}"),"predicate":"related","from":format!("n{}",i%nodes),"to":format!("n{}",(i+1)%nodes),"valid_time":{"start":0,"end":100}})).collect::<Vec<_>>()})
}
fn commit(
    e: &mut Engine,
    graph: &str,
    data: Value,
    host: &HostContext,
) -> Result<String, Box<dyn std::error::Error>> {
    let p: Program = serde_json::from_value(
        json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"expected_head":e.head(graph,"main")?,"data":data}]}),
    )?;
    e.execute(&p, host)?;
    Ok(e.head(graph, "main")?.unwrap())
}
fn stats(mut values: Vec<f64>) -> Value {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    json!({"samples":n,"min_ms":values[0],"median_ms":values[(n-1)/2],"p95_ms":values[(n*95).div_ceil(100)-1],"max_ms":values[n-1]})
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if cfg!(debug_assertions) {
        return Err("run cargo run --release --example benchmark_baseline".into());
    }
    let temp = tempfile::tempdir()?;
    let mut e = Engine::open(temp.path().join("bench.sqlite"))?;
    let host = HostContext::new("benchmark", ["large".into(), "cluster".into()]);
    let mut commits = Vec::new();
    for generation in 0..33 {
        let data = fixture(256, 1024, generation);
        let start = Instant::now();
        commit(&mut e, "large", data, &host)?;
        if generation >= 3 {
            commits.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let query: QueryPlan =
        serde_json::from_value(json!({"graph_id":"large","valid_at":5,"predicate":"related"}))?;
    let mut reads = Vec::new();
    let mut output_bytes = 0;
    for iteration in 0..33 {
        let start = Instant::now();
        let output = e.query(&query, &host)?;
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(output.graph.nodes.len(), 256);
        assert_eq!(output.graph.edges.len(), 1024);
        if iteration >= 3 {
            reads.push(elapsed);
        }
        output_bytes = serde_json::to_vec(&output)?.len();
    }
    let source = commit(&mut e, "cluster", fixture(64, 63, 0), &host)?;
    let request = ClusterRequest {
        source: GraphRef {
            graph_id: "cluster".into(),
            revision: source,
        },
        context: ContextSelection::Default,
        valid_at: 5,
        predicate: "related".into(),
        levels: 10000,
    };
    let mut navigation = Vec::new();
    let mut nav_shape = json!(null);
    for iteration in 0..13 {
        let start = Instant::now();
        let output = e.cluster_navigation(&request, &host)?;
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        if iteration >= 3 {
            navigation.push(elapsed);
        }
        nav_shape = json!({"nodes":output.graph.nodes.len(),"edges":output.graph.edges.len(),"serialized_bytes":serde_json::to_vec(&output)?.len()});
    }
    let before = e.event_count()?;
    let start = Instant::now();
    drop(e);
    let e = Engine::open(temp.path().join("bench.sqlite"))?;
    let reopen_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(e.event_count()?, before);
    assert_eq!(e.query(&query, &host)?.graph.edges.len(), 1024);
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "profile":"release","os":std::env::consts::OS,"arch":std::env::consts::ARCH,"contract":VERSION,
            "storage":"temporary local filesystem SQLite WAL; warm OS caches; one process and principal",
            "workload":{"large_nodes":256,"large_positive_edges":1024,"commit_revisions":33,"cluster_nodes":64,"cluster_chain_edges":63,"warmups_per_operation":3},
            "commit_including_program_construction":stats(commits),"authorized_query":stats(reads),
            "navigation_full_available_hierarchy":stats(navigation),"reopen_ms_single_sample":reopen_ms,
            "query_serialized_bytes":output_bytes,"navigation_output":nav_shape,
            "events":before,"limitations":["no concurrency or network","no browser or mobile measurement","latency excludes final JSON serialization for reads and navigation","no SLO or percentile population claim from this small sample","RSS measured separately for whole executable, not individual operations"]
        }))?
    );
    Ok(())
}
