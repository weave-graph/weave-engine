//! Phase A baseline (A6): single-fact commit, `expand`, `as_of`, `changes`
//! and `nearest` on one fact graph of each size. Usage: `phase_a_benchmark
//! [FACTS...]`, default `100000 1000000`. Run optimized: `cargo run --release
//! --locked -p weave-engine --example phase_a_benchmark`. Prints one JSON
//! report on stdout, with the host it ran on.
//!
//! Each graph is two fifths nodes, two fifths `located_at` edges (node `i`
//! to node `(7i + 1) mod n`) and one fifth 32-dimensional embeddings on the
//! first nodes, loaded in 100k-change writes into an on-disk WAL store. A
//! trusted test clock gives every write its own recorded time. Then:
//!
//! * `commit`: 200 writes that each assert one new node (one fact);
//! * `expand`: 2-hop `expand` (both directions) from a random node;
//! * `as_of`: the same `expand` as of the recorded time of a random sample
//!   commit;
//! * `changes`: the change set between two consecutive sample commits (one
//!   fact), and between the first and the last sample commit (200 facts);
//! * `nearest`: `nearest` k = 10 over the whole space, and over the 2-hop
//!   `expand` neighborhood of a random node (hybrid).
//!
//! Each sample is one `Engine::execute` of one program, including decoding,
//! authorization and result construction, not JSON output. Each result is
//! checked for the expected shape, so a fast wrong answer cannot pass.
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use weave_contract::{CommandResult, Program, QueryResult, VERSION};
use weave_engine::{Engine, HostContext, ManualClock};

const SAMPLES: usize = 200;
const SCAN_SAMPLES: usize = 30;
const BATCH: usize = 100_000;
const DIMS: usize = 32;
const START: i64 = 1_000;

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

fn execute(e: &mut Engine, host: &HostContext, commands: Value) -> Vec<CommandResult> {
    let program: Program =
        serde_json::from_value(json!({"version": VERSION, "commands": commands})).expect("program");
    e.execute(&program, host).expect("execute")
}
fn timed_eval(e: &mut Engine, host: &HostContext, plan: Value, ms: &mut Vec<f64>) -> QueryResult {
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
fn used_bytes(path: &std::path::Path) -> u64 {
    let db = rusqlite::Connection::open(path).expect("open");
    let pragma = |name: &str| -> u64 {
        db.pragma_query_value(None, name, |r| r.get::<_, i64>(0))
            .expect("pragma") as u64
    };
    (pragma("page_count") - pragma("freelist_count")) * pragma("page_size")
}
fn host() -> Value {
    let cpu = std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|s| {
        s.lines()
            .find(|l| l.starts_with("model name"))
            .and_then(|l| l.split(':').nth(1))
            .map(|m| m.trim().to_string())
    });
    let memory = std::fs::read_to_string("/proc/meminfo").ok().and_then(|s| {
        s.lines()
            .find(|l| l.starts_with("MemTotal"))
            .map(|l| l.split_whitespace().nth(1).unwrap_or("").to_string() + " kB")
    });
    json!({
        "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
        "logical_cpus": std::thread::available_parallelism().map(|n| n.get()).ok(),
        "cpu": cpu, "memory": memory,
    })
}
fn peak_rss() -> Option<String> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|l| l.starts_with("VmHWM"))
        .map(|l| l.split_whitespace().skip(1).collect::<Vec<_>>().join(" "))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sizes: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse())
        .collect::<Result<_, _>>()?;
    let sizes = if sizes.is_empty() {
        vec![100_000, 1_000_000]
    } else {
        sizes
    };
    let writer = HostContext::new("bench", ["fleet".to_string()]);
    let reader = HostContext::new("reader", Vec::<String>::new());
    let mut rows = vec![];
    for total in sizes {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("bench.db");
        let clock = Arc::new(ManualClock::new(START));
        let mut e = Engine::open_with_clock(&path, clock.clone())?;
        let nodes = total * 2 / 5;
        let edges = total * 2 / 5;
        let vectors = total - nodes - edges;
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut changes: Vec<Value> = vec![json!({"change": "assert", "fact": {
            "kind": "embedding_space", "id": "notes", "revision": "1", "encoder": "bench",
            "encoder_version": "1", "preprocessing": "none", "dimensions": DIMS, "metric": "cosine_distance"}})];
        let mut tick = START;
        let started = Instant::now();
        let mut flush = |e: &mut Engine, changes: &mut Vec<Value>, force: bool| {
            if changes.len() >= BATCH || (force && !changes.is_empty()) {
                tick += 1;
                clock.set(tick);
                let batch = std::mem::take(changes);
                execute(
                    e,
                    &writer,
                    json!([{"op": "write", "graph_id": "fleet", "changes": batch}]),
                );
            }
        };
        for i in 0..nodes {
            changes.push(
                json!({"change": "assert", "fact": {"kind": "node", "id": format!("unit-{i}"),
                "entity_id": format!("U-{i}"), "space_id": "fleet"}}),
            );
            flush(&mut e, &mut changes, false);
        }
        for i in 0..edges {
            changes.push(
                json!({"change": "assert", "fact": {"kind": "edge", "id": format!("at-{i}"),
                "predicate": "located_at", "from": format!("unit-{}", i % nodes),
                "to": format!("unit-{}", (i * 7 + 1) % nodes), "valid_time": {"start": 0}}}),
            );
            flush(&mut e, &mut changes, false);
        }
        for i in 0..vectors {
            changes.push(json!({"change": "assert", "fact": {"kind": "embedding", "id": format!("v-{i}"),
                "space": {"id": "notes", "revision": "1"}, "subject": {"kind": "node", "node_id": format!("unit-{}", i % nodes)},
                "vector": rng.vector(), "valid_time": {"start": 0}}}));
            flush(&mut e, &mut changes, false);
        }
        flush(&mut e, &mut changes, true);
        let load = started.elapsed();
        let loaded_bytes = used_bytes(&path);

        // Single-fact commits, each at its own recorded time.
        let mut commit = vec![];
        let mut commits: Vec<(i64, String)> = vec![];
        for i in 0..SAMPLES {
            tick += 1;
            clock.set(tick);
            let program: Program = serde_json::from_value(
                json!({"version": VERSION, "commands": [
                {"op": "write", "graph_id": "fleet", "changes": [{"change": "assert", "fact": {
                    "kind": "node", "id": format!("new-{i}"), "entity_id": format!("N-{i}"), "space_id": "fleet"}}]}]}),
            )?;
            let t = Instant::now();
            let out = e.execute(&program, &writer)?;
            commit.push(t.elapsed().as_secs_f64() * 1000.0);
            let CommandResult::Committed { revision, .. } = &out[0] else {
                panic!("commit result")
            };
            commits.push((tick, revision.clone()));
        }
        let growth = (used_bytes(&path) - loaded_bytes) as f64 / SAMPLES as f64;
        clock.set(tick + 1_000);

        let mut expand = vec![];
        let mut as_of = vec![];
        for _ in 0..SAMPLES {
            let seed = format!("unit-{}", rng.next(nodes));
            let plan = json!({"kind": "expand", "graph_id": "fleet", "seeds": [seed], "hops": 2, "direction": "both"});
            let got = timed_eval(&mut e, &reader, plan, &mut expand);
            assert!(got.graph.nodes.len() >= 2, "a seed has neighbors");
            let (at, _) = &commits[rng.next(SAMPLES)];
            let plan = json!({"kind": "expand", "graph_id": "fleet", "seeds": [seed], "hops": 2, "direction": "both", "as_of": at});
            let past = timed_eval(&mut e, &reader, plan, &mut as_of);
            assert_eq!(past.graph.nodes.len(), got.graph.nodes.len());
        }
        let mut changes_one = vec![];
        for _ in 0..SAMPLES {
            let i = rng.next(SAMPLES - 1);
            let plan = json!({"kind": "changes", "graph_id": "fleet",
                "from": {"kind": "cursor", "revision": commits[i].1}, "to": {"kind": "cursor", "revision": commits[i + 1].1}});
            let got = timed_eval(&mut e, &reader, plan, &mut changes_one);
            let set = got.changes.expect("change set");
            assert_eq!(set.entries.len(), 1);
        }
        let mut changes_window = vec![];
        for _ in 0..SCAN_SAMPLES {
            let plan = json!({"kind": "changes", "graph_id": "fleet",
                "from": {"kind": "cursor", "revision": commits[0].1}, "to": {"kind": "cursor", "revision": commits[SAMPLES - 1].1}});
            let got = timed_eval(&mut e, &reader, plan, &mut changes_window);
            assert_eq!(got.changes.expect("change set").entries.len(), SAMPLES - 1);
        }
        let mut nearest_all = vec![];
        for _ in 0..SCAN_SAMPLES {
            let plan = json!({"kind": "nearest", "graph_id": "fleet", "space": {"id": "notes", "revision": "1"},
                "query": {"kind": "vector", "vector": rng.vector()}, "k": 10});
            let got = timed_eval(&mut e, &reader, plan, &mut nearest_all);
            assert_eq!(got.neighbors.len(), 10.min(vectors));
        }
        let mut nearest_hybrid = vec![];
        for _ in 0..SAMPLES {
            let seed = format!("unit-{}", rng.next(vectors.min(nodes)));
            let plan = json!({"kind": "nearest", "graph_id": "fleet", "space": {"id": "notes", "revision": "1"},
                "query": {"kind": "vector", "vector": rng.vector()}, "k": 10,
                "candidates": {"kind": "expand", "graph_id": "fleet", "seeds": [seed], "hops": 2, "direction": "both"}});
            let got = timed_eval(&mut e, &reader, plan, &mut nearest_hybrid);
            assert!(
                !got.neighbors.is_empty(),
                "the seed's own vector is a candidate"
            );
        }
        rows.push(json!({
            "facts": total, "nodes": nodes, "edges": edges, "embeddings": vectors, "dimensions": DIMS,
            "load_seconds": load.as_secs_f64(), "store_bytes": used_bytes(&path),
            "single_fact_commit_ms": summary(commit),
            "mean_growth_bytes_per_single_fact_commit": growth,
            "expand_2_hop_ms": summary(expand),
            "as_of_expand_2_hop_ms": summary(as_of),
            "changes_one_fact_ms": summary(changes_one),
            "changes_199_facts_ms": summary(changes_window),
            "nearest_k10_whole_space_ms": summary(nearest_all),
            "nearest_k10_in_2_hop_expand_ms": summary(nearest_hybrid),
            "peak_rss": peak_rss(),
        }));
        eprintln!("{}", rows.last().expect("row"));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "profile": "weave-phase-a-benchmark/1",
            "protocol": VERSION,
            "store": weave_engine::STORAGE_VERSION,
            "host": host(),
            "workload": "two fifths nodes, two fifths located_at edges, one fifth 32-dimensional embeddings, loaded in 100k-change fact writes into an on-disk WAL store; then 200 single-fact commits; expand 2 hops both ways from a random node; the same expand as of a random commit's recorded time; changes between consecutive commits and across all 200; nearest k=10 over the whole space and within a 2-hop expand; each sample one Engine::execute including decoding, authorization and result construction, excluding JSON output",
            "results": rows,
        }))?
    );
    Ok(())
}
