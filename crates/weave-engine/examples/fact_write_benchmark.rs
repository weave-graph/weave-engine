//! Single-fact commit latency and storage growth at several graph sizes
//! (A2 acceptance). Usage: `fact_write_benchmark [FACTS...]`, default
//! `10000 100000 1000000`. Run optimized: `cargo run --release --example
//! fact_write_benchmark`. Prints one JSON report on stdout.
use serde_json::{json, Value};
use std::time::Instant;
use weave_contract::{Program, VERSION};
use weave_engine::{Engine, HostContext};

const SAMPLES: usize = 200;
const BATCH: usize = 100_000;

fn write(e: &mut Engine, changes: Vec<Value>) {
    let program: Program = serde_json::from_value(
        json!({"version": VERSION, "commands": [{"op": "write", "graph_id": "fleet", "changes": changes}]}),
    )
    .expect("fixed program");
    e.execute(&program, &HostContext::new("bench", ["fleet".to_string()]))
        .expect("write");
}
fn node(i: usize) -> Value {
    json!({"change": "assert", "fact": {"kind": "node", "id": format!("unit-{i}"), "entity_id": format!("U-{i}"), "space_id": "fleet"}})
}
fn edge(i: usize, units: usize) -> Value {
    json!({"change": "assert", "fact": {"kind": "edge", "id": format!("at-{i}"), "predicate": "located_at",
        "from": format!("unit-{i}"), "to": format!("unit-{}", (i * 7 + 1) % units), "valid_time": {"start": 0}}})
}
fn used_bytes(path: &std::path::Path) -> u64 {
    let db = rusqlite::Connection::open(path).expect("open");
    let pragma = |name: &str| -> u64 {
        db.pragma_query_value(None, name, |r| r.get::<_, i64>(0))
            .expect("pragma") as u64
    };
    (pragma("page_count") - pragma("freelist_count")) * pragma("page_size")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sizes: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse())
        .collect::<Result<_, _>>()?;
    let sizes = if sizes.is_empty() {
        vec![10_000, 100_000, 1_000_000]
    } else {
        sizes
    };
    let mut rows = vec![];
    for total in sizes {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("bench.db");
        let units = total / 2;
        let mut e = Engine::open(&path)?;
        let started = Instant::now();
        for start in (0..units).step_by(BATCH) {
            write(
                &mut e,
                (start..(start + BATCH).min(units)).map(node).collect(),
            );
        }
        for start in (0..units).step_by(BATCH) {
            write(
                &mut e,
                (start..(start + BATCH).min(units))
                    .map(|i| edge(i, units))
                    .collect(),
            );
        }
        let load = started.elapsed();
        drop(e);
        let before = used_bytes(&path);
        let mut e = Engine::open(&path)?;
        let mut latencies = Vec::with_capacity(SAMPLES);
        for k in 0..SAMPLES {
            let change = vec![node(units + k)];
            let t = Instant::now();
            write(&mut e, change);
            latencies.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        drop(e);
        let growth = used_bytes(&path).saturating_sub(before) as f64 / SAMPLES as f64;
        latencies.sort_by(|a, b| a.total_cmp(b));
        let pick = |q: f64| latencies[((latencies.len() - 1) as f64 * q).round() as usize];
        rows.push(json!({
            "facts": total,
            "load_seconds": load.as_secs_f64(),
            "store_bytes": before,
            "single_fact_commit_ms": {"median": pick(0.5), "p95": pick(0.95), "max": pick(1.0), "samples": SAMPLES},
            "mean_growth_bytes_per_single_fact_commit": growth,
        }));
        eprintln!("{}", rows.last().expect("row"));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "profile": "weave-fact-write-benchmark/1",
            "protocol": VERSION,
            "store": weave_engine::STORAGE_VERSION,
            "workload": "half nodes, half edges, loaded in 100k-change writes; each sample is one write with one new node on an on-disk store (WAL)",
            "results": rows,
        }))?
    );
    Ok(())
}
