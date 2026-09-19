use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use weave_contract::*;
use weave_engine::*;

fn host() -> HostContext {
    HostContext::new("reader", ["g".into(), "proof".into()])
}
fn program(commands: serde_json::Value) -> Program {
    serde_json::from_value(json!({"version": VERSION, "commands":commands})).unwrap()
}
fn query() -> QueryPlan {
    serde_json::from_value(json!({"graph_id":"g"})).unwrap()
}
fn commit(e: &mut Engine, value: i64) -> String {
    let prior = e.head("g", "main").unwrap();
    e.execute(&program(json!([{"op":"commit","graph_id":"g","expected_head":prior,
        "data":{"nodes":[{"id":"n","entity_id":"N","space_id":"s","properties":{"value":value}}]}}])), &host()).unwrap();
    e.head("g", "main").unwrap().unwrap()
}
#[test]
fn one_sample_covers_nested_program_queries_and_receipt_timestamp() {
    let clock = Arc::new(ManualClock::new(25));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    let revision = commit(&mut e, 1);
    assert_eq!(e.recorded_at(&revision).unwrap(), 25);
    assert_eq!(clock.samples(), 1);
    clock.set(30);
    e.execute(
        &program(json!([
            {"op":"query","query":{"graph_id":"g","valid_at":-100}},
            {"op":"query","query":{"graph_id":"g","valid_at":i64::MAX}},
            {"op":"bind","name":"copy","value":{"kind":"query","query":{"graph_id":"g"}}},
            {"op":"evaluate","value":{"kind":"reference","name":"copy"}}
        ])),
        &host(),
    )
    .unwrap();
    assert_eq!(clock.samples(), 2);
    e.query(&query(), &host()).unwrap();
    assert_eq!(clock.samples(), 3);
}

#[test]
fn negative_backward_and_failing_clock_leave_no_program_writes() {
    let clock = Arc::new(ManualClock::new(30));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    commit(&mut e, 1);
    let count = e.event_count().unwrap();
    for now in [29, -1] {
        clock.set(now);
        let error = e
            .execute(
                &program(json!([{"op":"commit","graph_id":"proof","data":{}}])),
                &host(),
            )
            .unwrap_err();
        assert_eq!(error.code, "E_CLOCK_UNAVAILABLE");
        assert_eq!(e.event_count().unwrap(), count);
        assert!(e.head("proof", "main").unwrap().is_none());
        assert_eq!(
            e.query(&query(), &host()).unwrap_err().code,
            "E_CLOCK_UNAVAILABLE"
        );
    }
    clock.set(31);
    assert_eq!(e.query(&query(), &host()).unwrap().graph.nodes.len(), 1);
    struct Broken;
    impl TrustedClock for Broken {
        fn unix_millis(&self) -> Result<i64> {
            Err(Error {
                code: "private clock detail".into(),
                message: "private detail".into(),
            })
        }
    }
    let mut e = Engine::memory_with_clock(Arc::new(Broken)).unwrap();
    let error = e
        .execute(
            &program(json!([{"op":"commit","graph_id":"g","data":{}}])),
            &host(),
        )
        .unwrap_err();
    assert_eq!(error.code, "E_CLOCK_UNAVAILABLE");
    assert!(!error.message.contains("private"));
    assert_eq!(e.event_count().unwrap(), 0);
}

#[test]
fn read_snapshot_is_established_before_the_clock_callback() {
    struct ConcurrentClock {
        path: std::path::PathBuf,
        revision: String,
        calls: AtomicUsize,
    }
    impl TrustedClock for ConcurrentClock {
        fn unix_millis(&self) -> Result<i64> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let connection = rusqlite::Connection::open(&self.path)?;
            connection.execute(
                "UPDATE heads SET revision=?1 WHERE graph_id='g' AND branch_id='main'",
                [&self.revision],
            )?;
            Ok(100)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("snapshot.db");
    let mut seed = Engine::open_with_clock(&path, Arc::new(ManualClock::new(0))).unwrap();
    let old = commit(&mut seed, 1);
    let new = commit(&mut seed, 2);
    drop(seed);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute("UPDATE heads SET revision=?1 WHERE graph_id='g'", [&old])
        .unwrap();
    let clock = Arc::new(ConcurrentClock {
        path: path.clone(),
        revision: new.clone(),
        calls: AtomicUsize::new(0),
    });
    let e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let result = e.query(&query(), &host()).unwrap();
    assert_eq!(result.snapshots["g"], old);
    assert_eq!(result.graph.nodes[0].properties["value"], 1);
    assert_eq!(clock.calls.load(Ordering::SeqCst), 1);
    assert_eq!(e.query(&query(), &host()).unwrap().snapshots["g"], new);
}

#[test]
fn a_panicking_host_clock_fails_closed_and_does_not_leave_a_scope_or_transaction() {
    struct OncePanicking(AtomicUsize);
    impl TrustedClock for OncePanicking {
        fn unix_millis(&self) -> Result<i64> {
            assert_ne!(
                self.0.fetch_add(1, Ordering::SeqCst),
                0,
                "test clock failed"
            );
            Ok(40)
        }
    }
    let mut e = Engine::memory_with_clock(Arc::new(OncePanicking(AtomicUsize::new(0)))).unwrap();
    assert_eq!(
        e.execute(
            &program(json!([{"op":"commit","graph_id":"g","data":{}}])),
            &host()
        )
        .unwrap_err()
        .code,
        "E_CLOCK_UNAVAILABLE"
    );
    assert_eq!(e.event_count().unwrap(), 0);
    let revision = commit(&mut e, 1);
    assert_eq!(e.recorded_at(&revision).unwrap(), 40);
}
