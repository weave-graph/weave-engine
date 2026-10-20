//! Scoped `nearest` cost against space size (A6 follow-up). Usage:
//! `scoped_nearest_benchmark [VECTORS...]`, default `20000 200000`. Run
//! optimized: `cargo run --release --locked -p weave-engine --example
//! scoped_nearest_benchmark`. Prints one JSON report on stdout.
//!
//! The graph has VECTORS nodes, one `located_at` edge from node `i` to node
//! `(7i + 1) mod n`, and one 32-dimensional embedding per node, on an on-disk
//! WAL store. Each sample is one `Engine::execute` of `nearest` k = 10:
//!
//! * over the whole space (the full scan);
//! * within the 2-hop `expand` neighborhood of a random node;
//! * within explicit candidates: a `query` of the edges from a random node.
//!
//! A scoped ranking should cost in proportion to its candidates, not to
//! the space. Every result is checked against the candidates it may contain.
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::time::Instant;
use weave_contract::{CommandResult, Program, QueryResult, VERSION};
use weave_engine::{Engine, HostContext};

const DIMS: usize = 32;
const SAMPLES: usize = 200;
const SCAN_SAMPLES: usize = 10;
const BATCH: usize = 50_000;

struct Rng(u64);
impl Rng {
    fn next(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) % n as u64) as usize
    }
    fn vector(&mut self) -> Vec<f32> {
        (0..DIMS)
            .map(|_| self.next(2001) as f32 / 1000.0 - 1.0)
            .collect()
    }
}

fn write(e: &mut Engine, host: &HostContext, changes: Vec<Value>) {
    let program: Program = serde_json::from_value(
        json!({"version": VERSION, "commands": [{"op": "write", "graph_id": "g", "changes": changes}]}),
    )
    .expect("program");
    e.execute(&program, host).expect("write");
}
fn evaluate(e: &mut Engine, host: &HostContext, plan: Value, ms: &mut Vec<f64>) -> QueryResult {
    let program: Program = serde_json::from_value(
        json!({"version": VERSION, "commands": [{"op": "evaluate", "value": plan}]}),
    )
    .expect("program");
    let t = Instant::now();
    let mut out = e.execute(&program, host).expect("evaluate");
    ms.push(t.elapsed().as_secs_f64() * 1000.0);
    match out.remove(0) {
        CommandResult::Queried { result } => *result,
        other => panic!("{other:?}"),
    }
}
fn summary(mut ms: Vec<f64>) -> Value {
    ms.sort_by(|a, b| a.total_cmp(b));
    let pick = |q: f64| ms[((ms.len() - 1) as f64 * q).round() as usize];
    json!({"median": pick(0.5), "p95": pick(0.95), "max": pick(1.0), "samples": ms.len()})
}
fn node_ids(result: &QueryResult) -> BTreeSet<String> {
    result.graph.nodes.iter().map(|n| n.id.clone()).collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sizes: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse())
        .collect::<Result<_, _>>()?;
    let sizes = if sizes.is_empty() {
        vec![20_000, 200_000]
    } else {
        sizes
    };
    let writer = HostContext::new("bench", ["g".to_string()]);
    let reader = HostContext::new("reader", Vec::<String>::new());
    let mut rows = vec![];
    for n in sizes {
        let dir = tempfile::tempdir()?;
        let mut e = Engine::open(dir.path().join("bench.db"))?;
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let started = Instant::now();
        write(
            &mut e,
            &writer,
            vec![
                json!({"change": "assert", "fact": {"kind": "embedding_space", "id": "notes", "revision": "1",
                "encoder": "bench", "encoder_version": "1", "preprocessing": "none", "dimensions": DIMS,
                "metric": "cosine_distance"}}),
            ],
        );
        for start in (0..n).step_by(BATCH) {
            let end = (start + BATCH).min(n);
            write(
                &mut e,
                &writer,
                (start..end)
                    .map(|i| {
                        json!({"change": "assert", "fact": {"kind": "node",
                "id": format!("u{i}"), "entity_id": format!("U{i}"), "space_id": "s"}})
                    })
                    .collect(),
            );
        }
        for start in (0..n).step_by(BATCH) {
            let end = (start + BATCH).min(n);
            let mut changes = vec![];
            for i in start..end {
                changes.push(json!({"change": "assert", "fact": {"kind": "edge", "id": format!("e{i}"),
                    "predicate": "located_at", "from": format!("u{i}"), "to": format!("u{}", (i * 7 + 1) % n),
                    "valid_time": {"start": 0}}}));
                changes.push(json!({"change": "assert", "fact": {"kind": "embedding", "id": format!("v{i}"),
                    "space": {"id": "notes", "revision": "1"}, "subject": {"kind": "node", "node_id": format!("u{i}")},
                    "vector": rng.vector(), "valid_time": {"start": 0}}}));
            }
            write(&mut e, &writer, changes);
        }
        // A last single-fact write, so the head revision is small. Every
        // read re-verifies its revision's digest from that revision's own
        // operations, which would otherwise add the cost of re-hashing a
        // 100k-change batch to every sample (measured separately below).
        let mut verify_big = vec![];
        for _ in 0..SCAN_SAMPLES {
            let plan = json!({"kind": "query", "query": {"graph_id": "g", "from": "u0"}});
            evaluate(&mut e, &reader, plan, &mut verify_big);
        }
        write(
            &mut e,
            &writer,
            vec![
                json!({"change": "assert", "fact": {"kind": "node", "id": "last", "entity_id": "LAST", "space_id": "s"}}),
            ],
        );
        let mut verify_small = vec![];
        for _ in 0..SCAN_SAMPLES {
            let plan = json!({"kind": "query", "query": {"graph_id": "g", "from": "u0"}});
            evaluate(&mut e, &reader, plan, &mut verify_small);
        }
        let load = started.elapsed();
        let space = json!({"id": "notes", "revision": "1"});
        let mut whole = vec![];
        for _ in 0..SCAN_SAMPLES {
            let plan = json!({"kind": "nearest", "graph_id": "g", "space": space, "k": 10,
                "query": {"kind": "vector", "vector": rng.vector()}});
            let got = evaluate(&mut e, &reader, plan, &mut whole);
            assert_eq!(got.neighbors.len(), 10);
        }
        let mut expand = vec![];
        let mut expand_sizes = vec![];
        let mut explicit = vec![];
        for _ in 0..SAMPLES {
            let seed = format!("u{}", rng.next(n));
            let region = json!({"kind": "expand", "graph_id": "g", "seeds": [seed], "hops": 2, "direction": "both"});
            let plan = json!({"kind": "nearest", "graph_id": "g", "space": space, "k": 10,
                "query": {"kind": "vector", "vector": rng.vector()}, "candidates": region});
            let got = evaluate(&mut e, &reader, plan, &mut expand);
            let within = node_ids(&evaluate(&mut e, &reader, region, &mut vec![]));
            expand_sizes.push(within.len() as f64);
            assert!(!got.neighbors.is_empty());
            for hit in &got.neighbors {
                let weave_contract::embedding::EmbeddingSubject::Node { node_id } = &hit.subject
                else {
                    panic!("node subject")
                };
                assert!(within.contains(node_id), "hit outside the neighborhood");
            }
            let edges = json!({"kind": "query", "query": {"graph_id": "g", "from": seed}});
            let plan = json!({"kind": "nearest", "graph_id": "g", "space": space, "k": 10,
                "query": {"kind": "vector", "vector": rng.vector()}, "candidates": edges});
            let got = evaluate(&mut e, &reader, plan, &mut explicit);
            assert!(!got.neighbors.is_empty() && got.neighbors.len() <= 2);
        }
        rows.push(json!({
            "vectors": n, "dimensions": DIMS, "load_seconds": load.as_secs_f64(),
            "nearest_whole_space_ms": summary(whole),
            "nearest_in_2_hop_expand_ms": summary(expand),
            "expand_candidate_nodes_mean": expand_sizes.iter().sum::<f64>() / expand_sizes.len() as f64,
            "nearest_in_query_candidates_ms": summary(explicit),
            "query_at_batch_head_ms": summary(verify_big),
            "query_at_single_fact_head_ms": summary(verify_small),
        }));
        eprintln!("{}", rows.last().expect("row"));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "profile": "weave-scoped-nearest-benchmark/1",
            "protocol": VERSION,
            "store": weave_engine::STORAGE_VERSION,
            "workload": "VECTORS nodes with one located_at edge each (i to 7i+1 mod n) and one 32-dimensional embedding each, on an on-disk WAL store; nearest k=10 over the whole space, within a 2-hop expand of a random node, and within a query of the edges from a random node; each sample one Engine::execute",
            "results": rows,
        }))?
    );
    Ok(())
}
