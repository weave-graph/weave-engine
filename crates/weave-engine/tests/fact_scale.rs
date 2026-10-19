//! Large fact graphs (A2 acceptance): no per-graph object cap, indexed
//! queries, reopen, and O(change) bytes per single-fact write.
//!
//! The default suite runs at `WEAVE_SCALE_FACTS` = 20,000 facts. The separate
//! release CI job (`ci.yml`, `fact-scale-1m`) runs it at 1,000,000.
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use weave_contract::{CommandResult, Program, QueryResult, VERSION};
use weave_engine::{Engine, HostContext, ManualClock};

fn facts() -> usize {
    std::env::var("WEAVE_SCALE_FACTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000)
}
fn host() -> HostContext {
    HostContext::new("loader", ["fleet".to_string()])
}
fn write(e: &mut Engine, changes: Vec<Value>) -> CommandResult {
    let program: Program = serde_json::from_value(
        json!({"version": VERSION, "commands": [{"op": "write", "graph_id": "fleet", "changes": changes}]}),
    )
    .unwrap();
    e.execute(&program, &host()).unwrap().remove(0)
}
fn query(e: &mut Engine, plan: Value) -> weave_engine::Result<QueryResult> {
    let program: Program = serde_json::from_value(
        json!({"version": VERSION, "commands": [{"op": "query", "query": plan}]}),
    )
    .unwrap();
    e.execute(&program, &HostContext::new("reader", Vec::<String>::new()))
        .map(|mut r| match r.remove(0) {
            CommandResult::Queried { result } => *result,
            other => panic!("{other:?}"),
        })
}
fn assert_(fact: Value) -> Value {
    json!({"change": "assert", "fact": fact})
}
fn node(i: usize) -> Value {
    json!({"kind": "node", "id": format!("unit-{i}"), "entity_id": format!("U-{i}"), "space_id": "fleet"})
}
fn edge(i: usize, units: usize) -> Value {
    let predicate = if i.is_multiple_of(10_000) {
        "audited_by"
    } else {
        "located_at"
    };
    json!({"kind": "edge", "id": format!("at-{i}"), "predicate": predicate,
        "from": format!("unit-{i}"), "to": format!("unit-{}", (i * 7 + 1) % units), "valid_time": {"start": i as i64 % 100}})
}
/// Bytes of pages in use (page count minus free pages), measured on a closed store.
fn stored_bytes(path: &Path) -> u64 {
    let db = rusqlite::Connection::open(path).unwrap();
    let pragma = |name: &str| -> u64 {
        db.pragma_query_value(None, name, |r| r.get::<_, i64>(0))
            .unwrap() as u64
    };
    (pragma("page_count") - pragma("freelist_count")) * pragma("page_size")
}

#[test]
fn large_fact_graph_is_created_queried_reopened_and_grows_by_the_change() {
    let total = facts();
    let units = total / 2;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fleet.db");
    let clock = Arc::new(ManualClock::new(1));
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let started = Instant::now();
    let batch = 100_000;
    for start in (0..units).step_by(batch) {
        let end = (start + batch).min(units);
        let committed = write(&mut e, (start..end).map(|i| assert_(node(i))).collect());
        assert!(matches!(committed, CommandResult::Committed { .. }));
    }
    for start in (0..units).step_by(batch) {
        let end = (start + batch).min(units);
        write(
            &mut e,
            (start..end).map(|i| assert_(edge(i, units))).collect(),
        );
    }
    let loaded = started.elapsed();
    // Indexed reads: a rare predicate and a single subject.
    let audited = query(
        &mut e,
        json!({"graph_id": "fleet", "predicate": "audited_by"}),
    )
    .unwrap();
    assert_eq!(audited.graph.edges.len(), units.div_ceil(10_000));
    let one = query(
        &mut e,
        json!({"graph_id": "fleet", "from": "unit-5", "valid_at": 50}),
    )
    .unwrap();
    assert_eq!(one.graph.edges.len(), 1);
    assert_eq!(one.graph.nodes.len(), 2);
    assert_eq!(one.fact_provenance.len(), 3);
    if total >= 1_000_000 {
        // Per-request limits remain: a whole 1M-fact graph exceeds the read budget.
        let whole = query(&mut e, json!({"graph_id": "fleet"})).unwrap_err();
        assert_eq!(whole.code, "E_BUDGET");
    }
    let head = e.head("fleet", "main").unwrap().unwrap();
    drop(e);
    // Reopen and read the same answers.
    let before = stored_bytes(&path);
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    assert_eq!(
        e.head("fleet", "main").unwrap().as_deref(),
        Some(head.as_str())
    );
    let again = query(
        &mut e,
        json!({"graph_id": "fleet", "predicate": "audited_by"}),
    )
    .unwrap();
    assert_eq!(again.graph, audited.graph);
    // Single-fact writes cost bytes in proportion to the change, not the graph.
    // Pages fill gradually, so measure the mean over many separate revisions.
    let writes = 200;
    let single = Instant::now();
    for k in 0..writes {
        clock.set(2 + k as i64);
        write(&mut e, vec![assert_(node(units + k))]);
    }
    let latency = single.elapsed() / writes as u32;
    drop(e);
    let growth = stored_bytes(&path).saturating_sub(before) / writes as u64;
    eprintln!(
        "fact_scale: {total} facts loaded in {loaded:?}; mean single-fact write {latency:?}; mean growth {growth} bytes per write; store {before} bytes"
    );
    // A snapshot write would add the whole graph (hundreds of bytes per fact).
    assert!(
        growth <= 8 * 1024,
        "a single-fact write grew the store by {growth} bytes"
    );
}

fn writes() -> usize {
    std::env::var("WEAVE_SCALE_WRITES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2_000)
}

/// Many small head-advancing writes: the former 100,000-observation store cap
/// is gone, and recorded-time selection uses the index instead of a walk that
/// the 4,096-read budget bounded.
#[test]
fn many_single_fact_writes_and_deep_recorded_selection() {
    let count = writes();
    let clock = Arc::new(ManualClock::new(1));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    for k in 0..count {
        clock.set(10 + k as i64);
        write(&mut e, vec![assert_(node(k))]);
    }
    clock.set(10 + count as i64);
    // As known after the first five writes: exactly five units.
    let program: Program = serde_json::from_value(json!({"version": VERSION, "commands": [{"op": "evaluate", "value": {
        "kind": "recorded_query", "query": {"graph_id": "fleet"}, "selection": {"kind": "local_time", "unix_millis": 14}}}]})).unwrap();
    let result = e
        .execute(&program, &HostContext::new("reader", Vec::<String>::new()))
        .unwrap();
    let CommandResult::Queried { result } = &result[0] else {
        panic!("query expected")
    };
    assert_eq!(result.graph.nodes.len(), 5);
    assert_eq!(result.recorded_observations.len(), 1);
    eprintln!("fact_scale: {count} single-fact writes, deep recorded selection ok");
}
