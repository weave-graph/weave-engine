//! Independent owner/fan-out and rollback acceptance for the native scheduler.
use serde_json::json;
use weave_contract::*;
use weave_engine::*;

fn owner(name: &str) -> HostContext {
    HostContext::new(name, ["source".into()])
}
fn write(engine: &mut Engine, turn: i64) {
    let expected_head = engine.head("source", "main").unwrap();
    engine
        .execute(
            &Program {
                version: VERSION.into(),
                source_revisions: vec![],
                commands: vec![Command::Commit {
            graph_id: "source".into(), branch_id: "main".into(), expected_head,
            data: serde_json::from_value(json!({
                "nodes":[{"id":"n","entity_id":"n","space_id":"s","properties":{"turn":turn}}],
                "edges":[]
            })).unwrap(),
        }],
            },
            &owner("alice"),
        )
        .unwrap();
}
fn enable(engine: &mut Engine, host: &HostContext, id: &str) {
    engine
        .register_view(
            &ViewDefinition {
                id: id.into(),
                clock: ViewClock::Fixed,
                expression: GraphExpression::Query {
                    query: serde_json::from_value(json!({"graph_id":"source"})).unwrap(),
                },
            },
            None,
            host,
        )
        .unwrap();
    engine.enroll_incremental_view(id, host).unwrap();
    engine.enable_view_schedule(id, host).unwrap();
}
fn turn(engine: &Engine, host: &HostContext, id: &str) -> i64 {
    engine
        .read_view(id, None, ViewFreshness::RequireCurrent, host)
        .unwrap()
        .result
        .graph
        .nodes[0]
        .properties["turn"]
        .as_i64()
        .unwrap()
}

#[test]
fn root_scheduler_partial_fanout_restart_and_principal_cursor_isolation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("schedule.db");
    let mut engine = Engine::open(&path).unwrap();
    write(&mut engine, 0);
    let alice = owner("alice");
    let bob = owner("bob");
    for host in [&alice, &bob] {
        for id in ["a", "b", "c"] {
            enable(&mut engine, host, id);
        }
        for _ in 0..3 {
            assert!(engine.drain_view_work(host).unwrap().is_some());
        }
        assert!(engine.drain_view_work(host).unwrap().is_none());
    }
    write(&mut engine, 1);
    write(&mut engine, 2);
    // Each call processes at most one dependent view. Closing after every page
    // exercises the durable event+view cursor, not just an in-memory iterator.
    for _ in 0..8 {
        let progress = engine
            .scan_view_work(
                &alice,
                ViewScanBudget {
                    max_events: 1,
                    max_views: 1,
                },
            )
            .unwrap();
        assert!(progress.events_completed <= 1 && progress.view_notifications <= 1);
        drop(engine);
        engine = Engine::open(&path).unwrap();
    }
    for _ in 0..3 {
        engine.drain_view_work(&alice).unwrap().unwrap();
    }
    assert!(engine.drain_view_work(&alice).unwrap().is_none());
    for id in ["a", "b", "c"] {
        assert_eq!(turn(&engine, &alice, id), 2);
        assert_eq!(
            engine
                .read_view(id, None, ViewFreshness::RequireCurrent, &bob)
                .unwrap_err()
                .code,
            "E_FRESHNESS"
        );
    }
    for _ in 0..8 {
        engine
            .scan_view_work(
                &bob,
                ViewScanBudget {
                    max_events: 1,
                    max_views: 1,
                },
            )
            .unwrap();
    }
    for _ in 0..3 {
        engine.drain_view_work(&bob).unwrap().unwrap();
    }
    for id in ["a", "b", "c"] {
        assert_eq!(turn(&engine, &bob, id), 2);
    }
}

#[cfg(feature = "recovery-testing")]
#[test]
fn root_scheduler_scan_and_drain_observer_panics_leave_retryable_work() {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("schedule.db");
    let mut engine = Engine::open(&path).unwrap();
    let alice = owner("alice");
    write(&mut engine, 0);
    enable(&mut engine, &alice, "a");
    engine.drain_view_work(&alice).unwrap().unwrap();
    write(&mut engine, 1);
    let budget = ViewScanBudget {
        max_events: 1,
        max_views: 1,
    };
    assert!(catch_unwind(AssertUnwindSafe(|| {
        let _ =
            engine.scan_view_work_test_before_commit(&alice, budget, || panic!("scan observer"));
    }))
    .is_err());
    assert!(
        engine.drain_view_work(&alice).unwrap().is_none(),
        "rolled-back scan cannot leave work behind"
    );
    assert_eq!(
        engine
            .scan_view_work(&alice, budget)
            .unwrap()
            .view_notifications,
        1
    );
    let generation = engine
        .read_view("a", None, ViewFreshness::AllowStale, &alice)
        .unwrap()
        .generation;
    assert!(catch_unwind(AssertUnwindSafe(|| {
        let _ = engine.drain_view_work_test_before_commit(&alice, || panic!("drain observer"));
    }))
    .is_err());
    let stale = engine
        .read_view("a", None, ViewFreshness::AllowStale, &alice)
        .unwrap();
    assert_eq!(
        stale.generation, generation,
        "result/generation must roll back with ack"
    );
    assert!(!stale.current);
    drop(engine);
    engine = Engine::open(&path).unwrap();
    let outcome = engine.drain_view_work(&alice).unwrap().unwrap();
    assert_eq!(outcome.generation, generation + 1);
    assert_eq!(turn(&engine, &alice, "a"), 1);
    assert!(engine.drain_view_work(&alice).unwrap().is_none());
}
