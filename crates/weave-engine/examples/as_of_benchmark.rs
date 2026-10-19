//! Recorded-time (`as_of`) selection latency against history length (A3
//! acceptance: O(log n) in history length). Usage: `as_of_benchmark
//! [REVISIONS...]`, default `10000 1000000`. Run optimized: `cargo run
//! --release --example as_of_benchmark`. Prints one JSON report on stdout.
//!
//! Each history asserts one new node per revision, so the store holds
//! REVISIONS fact revisions, head observations and facts on one branch. Each
//! sample is one `expand` `as_of` a uniformly random recorded time, of the
//! node recorded at that time, and of the node recorded just after it, which
//! must not exist yet. A pinned `expand` of the head revision is the
//! baseline: the difference is the selection.
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use weave_contract::{CommandResult, Program, VERSION};
use weave_engine::{Engine, HostContext, ManualClock};

const SAMPLES: usize = 500;
const START: i64 = 1_000;

fn execute(e: &mut Engine, host: &HostContext, commands: Value) -> Vec<CommandResult> {
    let program: Program =
        serde_json::from_value(json!({"version": VERSION, "commands": commands})).expect("program");
    e.execute(&program, host).expect("execute")
}

fn summary(mut ms: Vec<f64>) -> Value {
    ms.sort_by(|a, b| a.total_cmp(b));
    let pick = |q: f64| ms[((ms.len() - 1) as f64 * q).round() as usize];
    json!({"median": pick(0.5), "p95": pick(0.95), "max": pick(1.0), "samples": ms.len()})
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sizes: Vec<i64> = std::env::args()
        .skip(1)
        .map(|a| a.parse())
        .collect::<Result<_, _>>()?;
    let sizes = if sizes.is_empty() {
        vec![10_000, 1_000_000]
    } else {
        sizes
    };
    let writer = HostContext::new("bench", ["fleet".to_string()]);
    let reader = HostContext::new("reader", Vec::<String>::new());
    let mut rows = vec![];
    for revisions in sizes {
        let dir = tempfile::tempdir()?;
        let clock = Arc::new(ManualClock::new(START));
        let mut e = Engine::open_with_clock(dir.path().join("bench.db"), clock.clone())?;
        let started = Instant::now();
        for i in 0..revisions {
            clock.set(START + i);
            execute(
                &mut e,
                &writer,
                json!([{"op": "write", "graph_id": "fleet", "changes": [{"change": "assert", "fact":
                    {"kind": "node", "id": format!("unit-{i}"), "entity_id": format!("U-{i}"), "space_id": "fleet"}}]}]),
            );
        }
        let load = started.elapsed();
        clock.set(START + revisions + 1);
        let head = e.head("fleet", "main")?.expect("head");
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut as_of = Vec::with_capacity(SAMPLES);
        let mut pinned = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let i = ((state >> 33) % (revisions as u64 - 1)) as i64;
            let seeds = [format!("unit-{i}"), format!("unit-{}", i + 1)];
            let plan = json!({"kind": "expand", "graph_id": "fleet", "seeds": seeds, "hops": 1, "as_of": START + i});
            let t = Instant::now();
            let result = execute(&mut e, &reader, json!([{"op": "evaluate", "value": plan}]));
            as_of.push(t.elapsed().as_secs_f64() * 1000.0);
            let CommandResult::Queried { result } = &result[0] else {
                panic!("expand result")
            };
            let found: Vec<&str> = result.graph.nodes.iter().map(|n| n.id.as_str()).collect();
            assert_eq!(found, [seeds[0].as_str()], "as_of {i}");
            let plan = json!({"kind": "expand", "graph_id": "fleet", "seeds": seeds, "hops": 1, "revision": head});
            let t = Instant::now();
            execute(&mut e, &reader, json!([{"op": "evaluate", "value": plan}]));
            pinned.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        rows.push(json!({
            "revisions": revisions,
            "load_seconds": load.as_secs_f64(),
            "as_of_expand_ms": summary(as_of),
            "pinned_expand_ms": summary(pinned),
        }));
        eprintln!("{}", rows.last().expect("row"));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "profile": "weave-as-of-benchmark/1",
            "protocol": VERSION,
            "store": weave_engine::STORAGE_VERSION,
            "workload": "one new node asserted per fact revision on one branch of an on-disk store (WAL); each sample is one expand as_of a uniformly random recorded time of the node recorded then and the next one, checked to find only the first, and the same expand pinned to the head revision",
            "results": rows,
        }))?
    );
    Ok(())
}
