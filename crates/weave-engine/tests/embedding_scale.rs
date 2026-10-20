//! Whole-space exact `nearest` beyond the read budget (A4): the ranking
//! streams under its own scan budget, so a space larger than the 128 MiB
//! read budget allows ranks exactly, against an independent oracle.
//!
//! Size: `WEAVE_A4_VECTORS` (default 600) vectors of `WEAVE_A4_DIMENSIONS`
//! (default 1536). The release `fact-scale-1m` CI job runs 20,000, about
//! three times what the read budget alone would have allowed.
use serde_json::json;
use std::sync::Arc;
use weave_contract::{CommandResult, Program, VERSION};
use weave_engine::{Engine, HostContext, ManualClock};

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn vector(seed: &mut u64, dims: usize) -> Vec<f32> {
    (0..dims)
        .map(|_| {
            *seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((*seed >> 40) as f32 / (1u64 << 24) as f32) - 0.5
        })
        .collect()
}

fn distance(a: &[f32], b: &[f32]) -> f64 {
    let (mut dot, mut na, mut nb) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (f64::from(*x), f64::from(*y));
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    (1.0 - dot / (na.sqrt() * nb.sqrt())).clamp(0.0, 2.0)
}

#[test]
fn whole_space_exact_nearest_streams_past_the_read_budget() {
    let n = env("WEAVE_A4_VECTORS", 600);
    let dims = env("WEAVE_A4_DIMENSIONS", 1536);
    let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(5))).unwrap();
    let writer = HostContext::new("writer", ["g".to_string()]);
    let run = |e: &mut Engine, host: &HostContext, commands: serde_json::Value| {
        let program: Program =
            serde_json::from_value(json!({"version": VERSION, "commands": commands})).unwrap();
        e.execute(&program, host).unwrap()
    };
    run(
        &mut e,
        &writer,
        json!([{"op": "write", "graph_id": "g", "changes": [{"change": "assert", "fact": {
            "kind": "embedding_space", "id": "s", "revision": "1", "encoder": "e", "encoder_version": "1",
            "preprocessing": "none", "dimensions": dims, "metric": "cosine_distance"}}]}]),
    );
    let mut seed = 9;
    let mut stored = vec![];
    for start in (0..n).step_by(1000) {
        let mut changes = vec![];
        for i in start..(start + 1000).min(n) {
            let v = vector(&mut seed, dims);
            let readers = if i.is_multiple_of(4) {
                json!(["other"])
            } else {
                json!([])
            };
            changes.push(json!({"change": "assert", "fact": {"kind": "node", "id": format!("n{i:06}"), "entity_id": format!("N{i}"), "space_id": "x"}}));
            changes.push(json!({"change": "assert", "fact": {"kind": "embedding", "id": format!("e{i:06}"),
                "space": {"id": "s", "revision": "1"}, "subject": {"kind": "node", "node_id": format!("n{i:06}")},
                "vector": v, "valid_time": {"start": 0}, "readers": readers}}));
            stored.push((
                format!("e{i:06}"),
                format!("n{i:06}"),
                v,
                i.is_multiple_of(4),
            ));
        }
        run(
            &mut e,
            &writer,
            json!([{"op": "write", "graph_id": "g", "changes": changes}]),
        );
    }
    let query = vector(&mut seed, dims);
    let reader = HostContext::new("reader", Vec::<String>::new());
    let out = run(
        &mut e,
        &reader,
        json!([{"op": "evaluate", "value": {"kind": "nearest", "graph_id": "g",
            "space": {"id": "s", "revision": "1"}, "query": {"kind": "vector", "vector": query}, "k": 10}}]),
    );
    let CommandResult::Queried { result } = &out[0] else {
        panic!()
    };
    let mut want: Vec<(f64, &str, &str)> = stored
        .iter()
        .filter(|(_, _, _, hidden)| !hidden)
        .map(|(id, node, v, _)| (distance(&query, v), node.as_str(), id.as_str()))
        .collect();
    want.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(b.1)).then(a.2.cmp(b.2)));
    let want: Vec<(f64, &str)> = want.iter().take(10).map(|(s, _, id)| (*s, *id)).collect();
    let got: Vec<(f64, &str)> = result
        .neighbors
        .iter()
        .map(|h| (h.score, h.embedding_id.as_str()))
        .collect();
    assert_eq!(got, want);
    println!("exact top-10 over {n} vectors of {dims} dimensions matches the oracle");
}
