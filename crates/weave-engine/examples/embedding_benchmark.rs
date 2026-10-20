//! Embedding upsert and exact `nearest` at scale (A4 acceptance: 100,000
//! vectors of 1,536 dimensions). Usage: `embedding_benchmark [VECTORS]
//! [DIMENSIONS] [BATCH]`, default `100000 1536 1000`. Run optimized:
//! `cargo run --release --example embedding_benchmark`. Prints one JSON
//! report on stdout.
//!
//! Each fact write asserts BATCH nodes and one embedding on each, in one
//! cosine space, with deterministic pseudo-random unit vectors; every tenth
//! embedding is restricted to another principal. Then it measures a single
//! incremental embedding upsert on the full store, and exact top-10
//! `nearest` over the whole space as a reader who can see 90% of it, with and
//! without `valid_at`. On Linux it also reports the process's peak resident
//! memory before and after the rankings.
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use weave_contract::{CommandResult, Program, VERSION};
use weave_engine::{Engine, HostContext, ManualClock};

const RANKINGS: usize = 5;

struct Rng(u64);
impl Rng {
    fn unit(&mut self, dims: usize) -> Vec<f32> {
        let mut v: Vec<f32> = (0..dims)
            .map(|_| {
                self.0 = self
                    .0
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                ((self.0 >> 40) as f32 / (1u64 << 24) as f32) - 0.5
            })
            .collect();
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter_mut().for_each(|x| *x /= norm);
        v
    }
}

fn execute(e: &mut Engine, host: &HostContext, commands: Value) -> Vec<CommandResult> {
    let program: Program =
        serde_json::from_value(json!({"version": VERSION, "commands": commands})).expect("program");
    e.execute(&program, host).expect("execute")
}

fn summary(mut ms: Vec<f64>) -> Value {
    ms.sort_by(|a, b| a.total_cmp(b));
    let pick = |q: f64| ms[((ms.len() - 1) as f64 * q).round() as usize];
    json!({"median": pick(0.5), "p95": pick(0.95), "min": ms[0], "max": pick(1.0), "samples": ms.len()})
}

/// Peak resident set size in KiB (Linux only).
fn peak_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find(|l| l.starts_with("VmHWM:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
}

fn embedding(i: usize, dims: usize, rng: &mut Rng) -> Value {
    let readers = if i.is_multiple_of(10) {
        json!(["other"])
    } else {
        json!([])
    };
    json!({"change": "assert", "fact": {"kind": "embedding", "id": format!("e{i}"),
        "space": {"id": "bench", "revision": "1"}, "subject": {"kind": "node", "node_id": format!("n{i}")},
        "vector": rng.unit(dims), "valid_time": {"start": (i % 100) as i64, "end": null}, "readers": readers,
        "provenance": {"method": {"kind": "model", "model": "bench", "version": "1"}}}})
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse())
        .collect::<Result<_, _>>()?;
    let vectors = args.first().copied().unwrap_or(100_000);
    let dims = args.get(1).copied().unwrap_or(1536);
    let batch = args.get(2).copied().unwrap_or(1000);
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("bench.db");
    let mut now = 1_000i64;
    let clock = Arc::new(ManualClock::new(now));
    let mut e = Engine::open_with_clock(&path, clock.clone())?;
    let writer = HostContext::new("bench", ["g".to_string()]);
    let reader = HostContext::new("reader", Vec::<String>::new());
    let mut rng = Rng(42);
    execute(
        &mut e,
        &writer,
        json!([{"op": "write", "graph_id": "g", "changes": [{"change": "assert", "fact": {
            "kind": "embedding_space", "id": "bench", "revision": "1", "encoder": "bench",
            "encoder_version": "1", "preprocessing": "none", "dimensions": dims, "metric": "cosine_distance"}}]}]),
    );
    let started = Instant::now();
    let mut batches = vec![];
    let mut i = 0;
    while i < vectors {
        let mut changes = vec![];
        for j in i..(i + batch).min(vectors) {
            changes.push(
                json!({"change": "assert", "fact": {"kind": "node", "id": format!("n{j}"),
                "entity_id": format!("N{j}"), "space_id": "s"}}),
            );
            changes.push(embedding(j, dims, &mut rng));
        }
        now += 1;
        clock.set(now);
        let t = Instant::now();
        execute(
            &mut e,
            &writer,
            json!([{"op": "write", "graph_id": "g", "changes": changes}]),
        );
        batches.push(t.elapsed().as_secs_f64() * 1000.0);
        i += batch;
    }
    let load_s = started.elapsed().as_secs_f64();
    let first = summary(batches[..batches.len().min(5)].to_vec());
    let last = summary(batches[batches.len().saturating_sub(5)..].to_vec());
    // One more embedding, written incrementally on top of the full space.
    let mut single = vec![];
    for k in 0..5 {
        let n = vectors + k;
        now += 1;
        clock.set(now);
        let t = Instant::now();
        execute(
            &mut e,
            &writer,
            json!([{"op": "write", "graph_id": "g", "changes": [
                {"change": "assert", "fact": {"kind": "node", "id": format!("n{n}"), "entity_id": format!("N{n}"), "space_id": "s"}},
                embedding(n, dims, &mut rng)]}]),
        );
        single.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let store_bytes = std::fs::metadata(&path)?.len()
        + std::fs::metadata(dir.path().join("bench.db-wal")).map_or(0, |m| m.len());
    let rss_before = peak_rss_kib();
    let mut rank = |valid_at: Option<i64>| -> Result<Value, Box<dyn std::error::Error>> {
        let mut ms = vec![];
        let mut returned = 0;
        for _ in 0..RANKINGS {
            let mut plan = json!({"kind": "nearest", "graph_id": "g", "space": {"id": "bench", "revision": "1"},
                "query": {"kind": "vector", "vector": rng.unit(dims)}, "k": 10});
            if let Some(t) = valid_at {
                plan["valid_at"] = json!(t);
            }
            let t = Instant::now();
            let out = execute(&mut e, &reader, json!([{"op": "evaluate", "value": plan}]));
            ms.push(t.elapsed().as_secs_f64() * 1000.0);
            if let CommandResult::Queried { result } = &out[0] {
                returned = result.neighbors.len();
                // Restricted embeddings never reach this reader.
                assert!(result.neighbors.iter().all(|n| n.embedding_id[1..]
                    .parse::<usize>()
                    .unwrap()
                    % 10
                    != 0));
            }
        }
        Ok(json!({"latency_ms": summary(ms), "neighbors": returned}))
    };
    let whole = rank(None)?;
    let at = rank(Some(50))?;
    let report = json!({
        "benchmark": "embedding-upsert-and-exact-nearest",
        "protocol": VERSION,
        "vectors": vectors + 5, "dimensions": dims, "batch": batch, "metric": "cosine_distance",
        "visible_to_reader": "90% (every tenth embedding restricted)",
        "load_seconds": load_s,
        "batch_write_ms_first": first, "batch_write_ms_last": last,
        "single_embedding_upsert_ms_at_full_size": summary(single),
        "store_bytes": store_bytes,
        "exact_top10_whole_space": whole,
        "exact_top10_valid_at": at,
        "peak_rss_kib_before_rankings": rss_before,
        "peak_rss_kib_after_rankings": peak_rss_kib(),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
