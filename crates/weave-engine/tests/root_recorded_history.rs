//! Independent history oracles distinguish valid time, branch observation and receipt.
#[path = "support/legacy_storage.rs"]
mod legacy_storage;
use serde_json::json;
use std::sync::Arc;
use weave_contract::*;
use weave_engine::*;

fn host() -> HostContext {
    HostContext::new("alice", ["g".into(), "other".into()])
}
fn query(branch: &str) -> QueryPlan {
    serde_json::from_value(json!({"graph_id":"g","branch_id":branch,"valid_at":7})).unwrap()
}
fn program(branch: &str, expected: Option<&str>, value: i64, private: bool) -> Program {
    serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"g","branch_id":branch,"expected_head":expected,"data":{"nodes":[{"id":"a","entity_id":"a","space_id":"s","readers":if private {vec!["alice"]} else {vec![]}}, {"id":"b","entity_id":"b","space_id":"s","readers":if private {vec!["alice"]} else {vec![]}}],"edges":[{"id":"e","from":"a","to":"b","predicate":"fact","valid_time":{"start":5,"end":8},"properties":{"value":value},"readers":if private {vec!["alice"]} else {vec![]}}]}}]})).unwrap()
}
fn cut(observer: &str, time: i64) -> RecordedCut {
    RecordedCut::AtTime {
        observer: observer.into(),
        unix_millis: time,
    }
}

#[test]
fn late_correction_preserves_valid_and_recorded_axes_and_exact_checkpoint() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    engine
        .execute(&program("main", None, 1, false), &host())
        .unwrap();
    let first = engine
        .recorded_checkpoint_for("g", "main", &host())
        .unwrap();
    clock.set(20);
    engine
        .execute(
            &program("main", Some(&first.graph.revision), 2, false),
            &host(),
        )
        .unwrap();
    clock.set(30);
    let old = engine
        .query_recorded_for(&query("main"), &cut(&first.observer, 15), &host())
        .unwrap();
    let new = engine
        .query_recorded_for(&query("main"), &cut(&first.observer, 25), &host())
        .unwrap();
    assert_eq!(old.observation, first);
    assert_eq!(old.result.graph.edges[0].properties["value"], json!(1));
    assert_eq!(new.result.graph.edges[0].properties["value"], json!(2));
    assert_ne!(old.result.input_snapshots, new.result.input_snapshots);
    let exact = RecordedCut::Checkpoint {
        observer: first.observer,
        checkpoint: first.checkpoint,
    };
    assert_eq!(
        engine
            .query_recorded_for(&query("main"), &exact, &host())
            .unwrap(),
        old
    );
    let mut outside = query("main");
    outside.valid_at = Some(8);
    assert!(engine
        .query_recorded_for(&outside, &exact, &host())
        .unwrap()
        .result
        .graph
        .edges
        .is_empty());
}

#[test]
fn acceptance_uses_its_local_observation_time_and_branch_not_creation_time() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    engine
        .execute(&program("main", None, 1, false), &host())
        .unwrap();
    let old = engine
        .recorded_checkpoint_for("g", "main", &host())
        .unwrap();
    clock.set(20);
    engine
        .execute(
            &program("main", Some(&old.graph.revision), 2, false),
            &host(),
        )
        .unwrap();
    clock.set(30);
    engine.fork_branch(&old.graph, "fork", &host()).unwrap();
    let fork = engine
        .recorded_checkpoint_for("g", "fork", &host())
        .unwrap();
    assert_eq!(fork.kind, ObservationKind::Accepted);
    assert_eq!(fork.recorded_at_ms, 30);
    assert_eq!(engine.recorded_at(&old.graph.revision).unwrap(), 10);
    assert_eq!(
        engine
            .query_recorded_for(&query("fork"), &cut(&old.observer, 29), &host())
            .unwrap_err()
            .code,
        "E_HISTORY_UNAVAILABLE"
    );
    clock.set(40);
    let fork_value = engine
        .query_recorded_for(&query("fork"), &cut(&old.observer, 35), &host())
        .unwrap();
    let main_value = engine
        .query_recorded_for(&query("main"), &cut(&old.observer, 35), &host())
        .unwrap();
    assert_eq!(
        fork_value.result.graph.edges[0].properties["value"],
        json!(1)
    );
    assert_eq!(
        main_value.result.graph.edges[0].properties["value"],
        json!(2)
    );
    let wrong_branch = RecordedCut::Checkpoint {
        observer: fork.observer,
        checkpoint: fork.checkpoint,
    };
    assert_eq!(
        engine
            .query_recorded_for(&query("main"), &wrong_branch, &host())
            .unwrap_err()
            .code,
        "E_HISTORY_UNAVAILABLE"
    );
}

#[test]
fn range_is_half_open_ordered_and_bounded_without_skipping_denied_versions() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    for value in 1..=3 {
        clock.set(value * 10);
        let head = engine.head("g", "main").unwrap();
        engine
            .execute(
                &program("main", head.as_deref(), value, value == 2),
                &host(),
            )
            .unwrap();
    }
    clock.set(40);
    let observer = engine.runtime_source_identity().unwrap();
    let interval = Interval {
        start: 10,
        end: Some(30),
    };
    let range = engine
        .recorded_range_for("g", "main", &observer, &interval, 3, &host())
        .unwrap();
    assert_eq!(
        range
            .changes
            .iter()
            .map(|r| r.recorded_at_ms)
            .collect::<Vec<_>>(),
        vec![10, 20]
    );
    assert_eq!(range.start_state, range.changes[0]);
    assert_eq!(
        engine
            .recorded_range_for("g", "main", &observer, &interval, 1, &host())
            .unwrap_err()
            .code,
        "E_HISTORY_UNAVAILABLE"
    );
    let bob = HostContext::new("bob", []);
    assert_eq!(
        engine
            .recorded_range_for("g", "main", &observer, &interval, 3, &bob)
            .unwrap_err()
            .code,
        "E_HISTORY_UNAVAILABLE"
    );
    assert_eq!(
        engine
            .query_recorded_for(&query("main"), &cut(&observer, 25), &bob)
            .unwrap_err()
            .code,
        "E_HISTORY_UNAVAILABLE"
    );
    assert!(engine
        .query_recorded_for(&query("main"), &cut(&observer, 35), &bob)
        .is_ok());
}

#[test]
fn equal_recording_times_use_append_order_but_noop_and_replay_add_nothing() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    engine
        .execute(&program("main", None, 1, false), &host())
        .unwrap();
    let old = engine
        .recorded_checkpoint_for("g", "main", &host())
        .unwrap();
    let write = program("main", Some(&old.graph.revision), 2, false);
    engine.execute(&write, &host()).unwrap();
    let new = engine
        .recorded_checkpoint_for("g", "main", &host())
        .unwrap();
    assert_ne!(new.checkpoint, old.checkpoint);
    let chosen = engine
        .query_recorded_for(&query("main"), &cut(&old.observer, 10), &host())
        .unwrap();
    assert_eq!(chosen.observation, new);
    let noop = program("main", Some(&new.graph.revision), 2, false);
    engine.execute(&noop, &host()).unwrap();
    engine
        .accept_revision(&new.graph, "main", Some(&new.graph.revision), &host())
        .unwrap();
    assert_eq!(
        engine
            .recorded_checkpoint_for("g", "main", &host())
            .unwrap(),
        new
    );
}

#[test]
fn rollback_and_clock_regression_after_reopen_cannot_publish_head_or_observation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let clock = Arc::new(ManualClock::new(100));
    let mut engine = Engine::open_with_clock(&path, clock).unwrap();
    engine
        .execute(&program("main", None, 1, false), &host())
        .unwrap();
    let before = engine
        .recorded_checkpoint_for("g", "main", &host())
        .unwrap();
    drop(engine);
    let clock = Arc::new(ManualClock::new(50));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    assert_eq!(
        engine
            .execute(
                &program("main", Some(&before.graph.revision), 2, false),
                &host()
            )
            .unwrap_err()
            .code,
        "E_HISTORY_CLOCK"
    );
    assert_eq!(
        engine.head("g", "main").unwrap(),
        Some(before.graph.revision.clone())
    );
    clock.set(110);
    assert_eq!(
        engine
            .recorded_checkpoint_for("g", "main", &host())
            .unwrap(),
        before
    );
    let mut batch = program("main", Some(&before.graph.revision), 2, false);
    batch
        .commands
        .push(program("main", Some("stale"), 3, false).commands.remove(0));
    assert_eq!(
        engine.execute(&batch, &host()).unwrap_err().code,
        "E_CONFLICT"
    );
    assert_eq!(
        engine
            .recorded_checkpoint_for("g", "main", &host())
            .unwrap(),
        before
    );
}

#[test]
fn foreign_observer_missing_checkpoint_future_and_dual_selectors_reject() {
    let clock = Arc::new(ManualClock::new(100));
    let mut engine = Engine::memory_with_clock(clock).unwrap();
    engine
        .execute(&program("main", None, 1, false), &host())
        .unwrap();
    let point = engine
        .recorded_checkpoint_for("g", "main", &host())
        .unwrap();
    assert_eq!(
        engine
            .query_recorded_for(&query("main"), &cut("foreign", 100), &host())
            .unwrap_err()
            .code,
        "E_HISTORY_UNAVAILABLE"
    );
    assert_eq!(
        engine
            .query_recorded_for(&query("main"), &cut(&point.observer, 101), &host())
            .unwrap_err()
            .code,
        "E_HISTORY_TIME"
    );
    let missing = RecordedCut::Checkpoint {
        observer: point.observer,
        checkpoint: "observation:missing".into(),
    };
    assert_eq!(
        engine
            .query_recorded_for(&query("main"), &missing, &host())
            .unwrap_err()
            .code,
        "E_HISTORY_UNAVAILABLE"
    );
    let mut pinned = query("main");
    pinned.revision = Some(point.graph.revision);
    assert_eq!(
        engine
            .query_recorded_for(&pinned, &missing, &host())
            .unwrap_err()
            .code,
        "E_HISTORY_SELECTOR"
    );
}

#[test]
fn old_store_migration_baselines_current_heads_without_inventing_past_acceptance() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let mut engine = Engine::open_with_clock(&path, Arc::new(ManualClock::new(10))).unwrap();
    engine
        .execute(&program("main", None, 1, false), &host())
        .unwrap();
    let head = engine.head("g", "main").unwrap();
    drop(engine);
    let connection = rusqlite::Connection::open(&path).unwrap();
    legacy_storage::strip_retention_schema(&connection);
    connection
        .execute_batch("DROP TABLE head_observations; PRAGMA user_version=18;")
        .unwrap();
    drop(connection);
    let engine = Engine::open_with_clock(&path, Arc::new(ManualClock::new(100))).unwrap();
    let baseline = engine
        .recorded_checkpoint_for("g", "main", &host())
        .unwrap();
    assert_eq!(baseline.kind, ObservationKind::Baseline);
    assert_eq!(baseline.recorded_at_ms, 100);
    assert_eq!(Some(baseline.graph.revision), head);
    assert_eq!(
        engine
            .query_recorded_for(&query("main"), &cut(&baseline.observer, 99), &host())
            .unwrap_err()
            .code,
        "E_HISTORY_UNAVAILABLE"
    );
    assert_eq!(engine.event_count().unwrap(), 1);
}

#[test]
fn missing_history_table_and_corrupt_checkpoint_fail_closed_without_rebuilding_history() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let mut engine = Engine::open(&path).unwrap();
    engine
        .execute(&program("main", None, 1, false), &host())
        .unwrap();
    drop(engine);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute("UPDATE head_observations SET recorded_at_ms=1", [])
        .unwrap();
    drop(connection);
    let engine = Engine::open(&path).unwrap();
    assert_eq!(
        engine
            .recorded_checkpoint_for("g", "main", &host())
            .unwrap_err()
            .code,
        "E_INTEGRITY"
    );
    drop(engine);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch("DROP TABLE head_observations;")
        .unwrap();
    drop(connection);
    let error = match Engine::open(&path) {
        Ok(_) => panic!("missing history reopened"),
        Err(error) => error,
    };
    assert_eq!(error.code, "E_INTEGRITY");
}

#[test]
fn imported_receipt_is_not_local_branch_knowledge_until_explicit_acceptance() {
    let source_clock = Arc::new(ManualClock::new(10));
    let mut source = Engine::memory_with_clock(source_clock).unwrap();
    source
        .execute(&program("main", None, 1, false), &host())
        .unwrap();
    let original = source
        .recorded_checkpoint_for("g", "main", &host())
        .unwrap();
    let capsule = source.export_capsule(&original.graph, &host()).unwrap();
    let clock = Arc::new(ManualClock::new(20));
    let mut receiver = Engine::memory_with_clock(clock.clone()).unwrap();
    assert_eq!(receiver.receive_capsule(&capsule, &host()).unwrap(), 1);
    assert_eq!(receiver.recorded_at(&original.graph.revision).unwrap(), 20);
    assert_eq!(receiver.head("g", "main").unwrap(), None);
    assert_eq!(
        receiver
            .recorded_checkpoint_for("g", "main", &host())
            .unwrap_err()
            .code,
        "E_HISTORY_UNAVAILABLE"
    );
    clock.set(30);
    receiver
        .accept_revision(&original.graph, "main", None, &host())
        .unwrap();
    let accepted = receiver
        .recorded_checkpoint_for("g", "main", &host())
        .unwrap();
    assert_eq!(accepted.kind, ObservationKind::Accepted);
    assert_eq!(accepted.recorded_at_ms, 30);
    assert_ne!(accepted.observer, original.observer);
    assert_ne!(accepted.checkpoint, original.checkpoint);
    assert_eq!(
        receiver
            .query_recorded_for(&query("main"), &cut(&accepted.observer, 29), &host())
            .unwrap_err()
            .code,
        "E_HISTORY_UNAVAILABLE"
    );
    assert_eq!(
        receiver
            .query_recorded_for(&query("main"), &cut(&accepted.observer, 30), &host())
            .unwrap()
            .observation,
        accepted
    );
    clock.set(40);
    assert_eq!(receiver.receive_capsule(&capsule, &host()).unwrap(), 0);
    assert_eq!(
        receiver
            .recorded_checkpoint_for("g", "main", &host())
            .unwrap(),
        accepted
    );
}

#[test]
fn atomic_logical_batch_records_unchanged_member_head_but_not_a_duplicate_event() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    engine
        .execute(&program("main", None, 1, false), &host())
        .unwrap();
    let empty: Program = serde_json::from_value(
        json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"other","data":{}}]}),
    )
    .unwrap();
    engine.execute(&empty, &host()).unwrap();
    let old = engine
        .recorded_checkpoint_for("other", "main", &host())
        .unwrap();
    let next = program("main", None, 2, false);
    let Command::Commit { data, .. } = &next.commands[0] else {
        unreachable!()
    };
    let batch: Program = serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit_batch","batch_id":"correction","commits":[{"graph_id":"g","expected_head":engine.head("g","main").unwrap(),"data":data},{"graph_id":"other","expected_head":old.graph.revision,"data":{}}]}]})).unwrap();
    clock.set(20);
    engine.execute(&batch, &host()).unwrap();
    let g = engine
        .recorded_checkpoint_for("g", "main", &host())
        .unwrap();
    let other = engine
        .recorded_checkpoint_for("other", "main", &host())
        .unwrap();
    assert_eq!(g.graph.revision, "logical:correction:g");
    assert_eq!(other.graph.revision, "logical:correction:other");
    assert_eq!(other.recorded_at_ms, 20);
    assert_ne!(other.checkpoint, old.checkpoint);
    assert_eq!(engine.event_count().unwrap(), 3);
    clock.set(30);
    engine.execute(&batch, &host()).unwrap();
    assert_eq!(engine.event_count().unwrap(), 3);
    assert_eq!(
        engine
            .recorded_checkpoint_for("g", "main", &host())
            .unwrap(),
        g
    );
    assert_eq!(
        engine
            .recorded_checkpoint_for("other", "main", &host())
            .unwrap(),
        other
    );
}

#[test]
fn empty_historical_value_keeps_its_whole_snapshot_permission_gate() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    let source: Program = serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"other","data":{"nodes":[{"id":"secret","entity_id":"secret","space_id":"s","readers":["alice"]}]}}]})).unwrap();
    engine.execute(&source, &host()).unwrap();
    let saved: Program = serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"g","data":{"influence":{"snapshots":[{"graph_id":"other","revision":engine.head("other","main").unwrap()}]}}}]})).unwrap();
    engine.execute(&saved, &host()).unwrap();
    let checkpoint = engine
        .recorded_checkpoint_for("g", "main", &host())
        .unwrap();
    let exact = RecordedCut::Checkpoint {
        observer: checkpoint.observer.clone(),
        checkpoint: checkpoint.checkpoint.clone(),
    };
    assert!(engine
        .query_recorded_for(&query("main"), &exact, &host())
        .unwrap()
        .result
        .graph
        .nodes
        .is_empty());
    let outsider = HostContext::new("bob", []);
    clock.set(20);
    for cut in [exact, cut(&checkpoint.observer, 15)] {
        assert_eq!(
            engine
                .query_recorded_for(&query("main"), &cut, &outsider)
                .unwrap_err()
                .code,
            "E_HISTORY_UNAVAILABLE"
        );
    }
    assert_eq!(
        engine
            .recorded_checkpoint_for("g", "main", &outsider)
            .unwrap_err()
            .code,
        "E_HISTORY_UNAVAILABLE"
    );
    assert_eq!(
        engine
            .recorded_range_for(
                "g",
                "main",
                &checkpoint.observer,
                &Interval {
                    start: 10,
                    end: Some(20)
                },
                10,
                &outsider
            )
            .unwrap_err()
            .code,
        "E_HISTORY_UNAVAILABLE"
    );
}

#[test]
fn date_selection_cannot_skip_a_deleted_or_corrupt_intermediate_observation() {
    for delete in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let clock = Arc::new(ManualClock::new(10));
        let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
        for value in 1..=4 {
            clock.set(value * 10);
            engine
                .execute(
                    &program(
                        "main",
                        engine.head("g", "main").unwrap().as_deref(),
                        value,
                        false,
                    ),
                    &host(),
                )
                .unwrap();
        }
        let observer = engine.runtime_source_identity().unwrap();
        drop(engine);
        let connection = rusqlite::Connection::open(&path).unwrap();
        // Independent storage-loss fixture; no public mutation or repair API.
        connection.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        if delete {
            connection
                .execute("DELETE FROM head_observations WHERE recorded_at_ms=20", [])
                .unwrap();
        } else {
            connection
                .execute(
                    "UPDATE head_observations SET recorded_at_ms=21 WHERE recorded_at_ms=20",
                    [],
                )
                .unwrap();
        }
        drop(connection);
        clock.set(50);
        let engine = Engine::open_with_clock(&path, clock).unwrap();
        assert_eq!(
            engine
                .query_recorded_for(&query("main"), &cut(&observer, 15), &host())
                .unwrap_err()
                .code,
            "E_INTEGRITY"
        );
        assert_eq!(
            engine
                .recorded_range_for(
                    "g",
                    "main",
                    &observer,
                    &Interval {
                        start: 10,
                        end: Some(30)
                    },
                    10,
                    &host()
                )
                .unwrap_err()
                .code,
            "E_INTEGRITY"
        );
    }
}
