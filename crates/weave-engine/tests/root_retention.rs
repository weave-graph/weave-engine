//! Independent reachability, unavailable-history and transaction-boundary oracles.
use serde_json::json;
use std::sync::Arc;
use weave_contract::*;
use weave_engine::*;

fn host(graphs: &[&str]) -> HostContext {
    HostContext::new("owner", graphs.iter().map(|g| (*g).into()))
}
fn commit(
    engine: &mut Engine,
    graph: &str,
    value: i64,
    metadata: &[GraphRef],
    private: bool,
) -> GraphRef {
    let expected = engine.head(graph, "main").unwrap();
    let program: Program = serde_json::from_value(json!({"version":VERSION,"commands":[{
        "op":"commit","graph_id":graph,"expected_head":expected,"data":{"nodes":[{
            "id":"n","entity_id":"e","space_id":"s","properties":{"value":value},"metadata":metadata,
            "readers":if private {vec!["owner"]} else {vec![]}
        }]}
    }]})).unwrap();
    engine.execute(&program, &host(&[graph])).unwrap();
    GraphRef {
        graph_id: graph.into(),
        revision: engine.head(graph, "main").unwrap().unwrap(),
    }
}
fn read(engine: &Engine, reference: &GraphRef) -> weave_engine::Result<QueryResult> {
    engine.query(&serde_json::from_value(json!({"graph_id":reference.graph_id,"revision":reference.revision,"include_metadata":true,"max_depth":3})).unwrap(),&host(&[]))
}
fn policy(engine: &Engine, time: i64) -> RetentionPolicy {
    RetentionPolicy {
        history_before_ms: time,
        replay_through_sequence: engine
            .events()
            .unwrap()
            .last()
            .map_or(0, |e| e.sequence as i64),
    }
}
#[test]
fn native_audit_literals_are_legal_but_malformed_owned_pin_encodings_block_collection() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("retention.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let pinned = commit(&mut engine, "g", 1, &[], false);
    engine.register_adapter("audit", "g").unwrap();
    let event = engine.events().unwrap()[0].event_id.clone();
    engine.deliver("audit", &event, false).unwrap();
    engine
        .retain_snapshots_for("pin", std::slice::from_ref(&pinned), &host(&[]))
        .unwrap();
    let orphan = commit(&mut engine, "orphan", 2, &[], false);
    engine
        .release_branch_for("orphan", "main", &orphan.revision, &host(&["orphan"]))
        .unwrap();
    clock.set(30);
    let policy = policy(&engine, 20);
    let preview = engine.plan_retention(&policy).unwrap();
    assert_eq!(preview.collect, vec![orphan.clone()]);
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute("UPDATE retention_roots SET refs='{broken'", [])
        .unwrap();
    assert_eq!(
        engine.compact_retention(&preview).unwrap_err().code,
        "E_RETENTION_INTEGRITY"
    );
    assert!(read(&engine, &orphan).is_ok());
    assert_eq!(
        sql.query_row("SELECT count(*) FROM retention_tombstones", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row("SELECT generation FROM retention_policy", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn legal_json_like_identifiers_and_literal_strings_are_not_corrupt_encoded_cells() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    let graph = "{literal";
    let p:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"data":{"nodes":[{"id":"n","entity_id":"e","space_id":"s","properties":{"text":"{not encoded JSON"}}]}}]})).unwrap();
    engine.execute(&p, &host(&[graph])).unwrap();
    let reference = GraphRef {
        graph_id: graph.into(),
        revision: engine.head(graph, "main").unwrap().unwrap(),
    };
    clock.set(30);
    let plan = engine.plan_retention(&policy(&engine, 20)).unwrap();
    assert!(plan.retained.contains(&reference));
    assert!(plan.collect.is_empty());
    engine.compact_retention(&plan).unwrap();
    assert_eq!(
        read(&engine, &reference).unwrap().graph.nodes[0].properties["text"],
        json!("{not encoded JSON")
    );
}
#[test]
fn lowering_the_marker_never_reconstructs_or_defaults_existing_retention_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("retention.db");
    let engine = Engine::open(&path).unwrap();
    drop(engine);
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.pragma_update(None, "user_version", 21).unwrap();
    let error = match Engine::open(&path) {
        Ok(_) => panic!("downgrade accepted"),
        Err(error) => error,
    };
    assert_eq!(error.code, "E_RETENTION_INTEGRITY");
    assert_eq!(
        sql.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        21
    );
    assert_eq!(
        sql.query_row("SELECT generation FROM retention_policy", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn retained_window_preserves_its_start_state_before_the_next_observation() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    let baseline = commit(&mut engine, "g", 1, &[], true);
    clock.set(30);
    commit(&mut engine, "g", 2, &[], true);
    clock.set(40);
    let observer = engine.runtime_source_identity().unwrap();
    let plan = engine.plan_retention(&policy(&engine, 20)).unwrap();
    assert!(
        plan.retained.contains(&baseline),
        "the active branch's value at the lower boundary is a root"
    );
    engine.compact_retention(&plan).unwrap();
    let query: QueryPlan = serde_json::from_value(json!({"graph_id":"g"})).unwrap();
    let selected = engine
        .query_recorded_for(
            &query,
            &RecordedCut::AtTime {
                observer: observer.clone(),
                unix_millis: 25,
            },
            &host(&[]),
        )
        .unwrap();
    assert_eq!(selected.result.graph.nodes[0].properties["value"], json!(1));
    let range = engine
        .query_recorded_range_for(
            &query,
            &observer,
            &Interval {
                start: 20,
                end: Some(35),
            },
            10,
            &host(&[]),
        )
        .unwrap();
    assert_eq!(
        range.start_state.graph.nodes[0].properties["value"],
        json!(1)
    );
    assert_eq!(range.changes.len(), 1);
    assert_eq!(
        range.changes[0].graph.nodes[0].properties["value"],
        json!(2)
    );
}
#[test]
fn unknown_registry_and_corrupt_erasure_anchor_fail_before_further_collection() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("retention.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let old = commit(&mut engine, "g", 1, &[], false);
    engine
        .release_branch_for("g", "main", &old.revision, &host(&["g"]))
        .unwrap();
    clock.set(30);
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch("CREATE TABLE unknown_extension(reference TEXT NOT NULL); INSERT INTO unknown_extension VALUES ('opaque reference encoding');").unwrap();
    let p = policy(&engine, 20);
    assert_eq!(
        engine.plan_retention(&p).unwrap_err().code,
        "E_RETENTION_SCHEMA"
    );
    assert_eq!(
        read(&engine, &old).unwrap().graph.nodes[0].properties["value"],
        json!(1)
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM retention_tombstones", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    sql.execute_batch("DROP TABLE unknown_extension").unwrap();
    let plan = engine.plan_retention(&p).unwrap();
    engine.compact_retention(&plan).unwrap();
    assert_eq!(read(&engine, &old).unwrap_err().code, "E_UNAVAILABLE");
    sql.execute(
        "UPDATE retention_tombstones SET content_digest=?1",
        [format!("sha256:{}", "0".repeat(64))],
    )
    .unwrap();
    assert_eq!(read(&engine, &old).unwrap_err().code, "E_INTEGRITY");
    assert_eq!(engine.plan_retention(&p).unwrap_err().code, "E_INTEGRITY");
    assert_eq!(
        sql.query_row("SELECT generation FROM retention_policy", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn explicit_branch_retirement_never_resurrects_an_old_logical_branch_identity() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock).unwrap();
    let old = commit(&mut engine, "g", 1, &[], false);
    engine
        .release_branch_for("g", "main", &old.revision, &host(&["g"]))
        .unwrap();
    let p: Program = serde_json::from_value(
        json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"g","data":{"nodes":[]}}]}),
    )
    .unwrap();
    assert_eq!(
        engine.execute(&p, &host(&["g"])).unwrap_err().code,
        "E_BRANCH_RETIRED"
    );
    assert_eq!(engine.event_count().unwrap(), 1);
    assert!(engine.head("g", "main").unwrap().is_none());
    assert!(
        read(&engine, &old).is_ok(),
        "retirement does not revoke retained genuine snapshots"
    );
}
#[test]
fn shared_evidence_survives_one_root_release_and_unreachable_payloads_have_anchors() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    let evidence = commit(&mut engine, "evidence", 42, &[], false);
    let a = commit(&mut engine, "a", 1, std::slice::from_ref(&evidence), false);
    let b = commit(&mut engine, "b", 2, std::slice::from_ref(&evidence), false);
    engine
        .release_branch_for("evidence", "main", &evidence.revision, &host(&["evidence"]))
        .unwrap();
    engine
        .release_branch_for("a", "main", &a.revision, &host(&["a"]))
        .unwrap();
    clock.set(30);
    let plan = engine.plan_retention(&policy(&engine, 20)).unwrap();
    assert_eq!(plan.collect, vec![a.clone()]);
    assert!(plan.retained.contains(&evidence) && plan.retained.contains(&b));
    let receipt = engine.compact_retention(&plan).unwrap();
    assert_eq!(receipt.collected_payloads, 1);
    assert!(receipt.payload_bytes > 0);
    assert_eq!(read(&engine, &a).unwrap_err().code, "E_UNAVAILABLE");
    assert_eq!(
        read(&engine, &evidence).unwrap().graph.nodes[0].properties["value"],
        json!(42)
    );
    assert!(!read(&engine, &b).unwrap().metadata_graphs.is_empty());
    assert_eq!(engine.recorded_at(&a.revision).unwrap(), 10);
    engine
        .release_branch_for("b", "main", &b.revision, &host(&["b"]))
        .unwrap();
    let final_plan = engine.plan_retention(&policy(&engine, 20)).unwrap();
    assert_eq!(final_plan.collect.len(), 2);
    engine.compact_retention(&final_plan).unwrap();
    assert_eq!(read(&engine, &evidence).unwrap_err().code, "E_UNAVAILABLE");
    assert_eq!(
        engine.events().unwrap().len(),
        3,
        "event identities and ordering anchors survive"
    );
}
#[test]
fn unreachable_metadata_cycles_collect_but_a_shared_atomic_member_retains_the_batch() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    let program: Program = serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit_batch","batch_id":"cycle","commits":[
        {"graph_id":"a","data":{"nodes":[{"id":"n","entity_id":"a","space_id":"s","metadata":[{"graph_id":"b","revision":"logical:cycle:b"}]}]}},
        {"graph_id":"b","data":{"nodes":[{"id":"n","entity_id":"b","space_id":"s","metadata":[{"graph_id":"a","revision":"logical:cycle:a"}]}]}}
    ]}]})).unwrap();
    engine.execute(&program, &host(&["a", "b"])).unwrap();
    clock.set(30);
    engine
        .release_branch_for("a", "main", "logical:cycle:a", &host(&["a"]))
        .unwrap();
    assert!(engine
        .plan_retention(&policy(&engine, 20))
        .unwrap()
        .collect
        .is_empty());
    engine
        .release_branch_for("b", "main", "logical:cycle:b", &host(&["b"]))
        .unwrap();
    let plan = engine.plan_retention(&policy(&engine, 20)).unwrap();
    assert_eq!(plan.collect.len(), 2);
    engine.compact_retention(&plan).unwrap();
    for graph in ["a", "b"] {
        assert_eq!(
            read(
                &engine,
                &GraphRef {
                    graph_id: graph.into(),
                    revision: format!("logical:cycle:{graph}")
                }
            )
            .unwrap_err()
            .code,
            "E_UNAVAILABLE"
        );
    }
}
#[test]
fn exact_owner_pins_retain_old_history_without_granting_foreign_or_partial_authority() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    let old = commit(&mut engine, "g", 1, &[], true);
    let observer = engine.runtime_source_identity().unwrap();
    let checkpoint = engine
        .recorded_checkpoint_for("g", "main", &host(&[]))
        .unwrap();
    let outsider = HostContext::new("outsider", ["g".into()]);
    assert!(engine
        .retain_snapshots_for("pin", std::slice::from_ref(&old), &outsider)
        .is_err());
    engine
        .retain_snapshots_for("pin", std::slice::from_ref(&old), &host(&[]))
        .unwrap();
    clock.set(20);
    commit(&mut engine, "g", 2, &[], true);
    clock.set(30);
    let p = policy(&engine, 20);
    let plan = engine.plan_retention(&p).unwrap();
    assert!(!plan.collect.contains(&old));
    engine.compact_retention(&plan).unwrap();
    let query: QueryPlan = serde_json::from_value(json!({"graph_id":"g"})).unwrap();
    assert_eq!(
        engine
            .query_recorded_for(
                &query,
                &RecordedCut::AtTime {
                    observer: observer.clone(),
                    unix_millis: 15
                },
                &host(&[])
            )
            .unwrap_err()
            .code,
        "E_HISTORY_EXPIRED"
    );
    assert_ne!(
        engine
            .query_recorded_for(
                &query,
                &RecordedCut::AtTime {
                    observer,
                    unix_millis: 15
                },
                &outsider
            )
            .unwrap_err()
            .code,
        "E_HISTORY_EXPIRED"
    );
    let exact = RecordedCut::Checkpoint {
        observer: checkpoint.observer,
        checkpoint: checkpoint.checkpoint,
    };
    assert_eq!(
        engine
            .query_recorded_for(&query, &exact, &host(&[]))
            .unwrap()
            .result
            .graph
            .nodes[0]
            .properties["value"],
        json!(1)
    );
    engine.release_retention_root_for("pin", &outsider).unwrap();
    assert!(engine.plan_retention(&p).unwrap().collect.is_empty());
    engine
        .release_retention_root_for("pin", &host(&[]))
        .unwrap();
    let collect = engine.plan_retention(&p).unwrap();
    assert_eq!(collect.collect, vec![old.clone()]);
    engine.compact_retention(&collect).unwrap();
    assert_eq!(read(&engine, &old).unwrap_err().code, "E_UNAVAILABLE");
}
#[test]
fn a_new_root_invalidates_a_preview_and_failed_compaction_does_not_erase() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    let old = commit(&mut engine, "g", 1, &[], false);
    clock.set(20);
    commit(&mut engine, "g", 2, &[], false);
    clock.set(30);
    let p = policy(&engine, 20);
    let preview = engine.plan_retention(&p).unwrap();
    assert_eq!(preview.collect, vec![old.clone()]);
    engine
        .retain_snapshots_for("new-pin", std::slice::from_ref(&old), &host(&[]))
        .unwrap();
    assert_eq!(
        engine.compact_retention(&preview).unwrap_err().code,
        "E_CONFLICT"
    );
    assert_eq!(
        read(&engine, &old).unwrap().graph.nodes[0].properties["value"],
        json!(1)
    );
    assert_eq!(engine.plan_retention(&p).unwrap().generation, 0);
    assert!(engine
        .plan_retention(&RetentionPolicy {
            history_before_ms: 31,
            replay_through_sequence: 2
        })
        .is_err());
}
#[cfg(feature = "recovery-testing")]
#[test]
fn panic_before_commit_rolls_back_payload_anchor_and_policy_together_across_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("retention.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let old = commit(&mut engine, "g", 1, &[], false);
    clock.set(20);
    commit(&mut engine, "g", 2, &[], false);
    clock.set(30);
    let plan = engine.plan_retention(&policy(&engine, 20)).unwrap();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || engine.compact_retention_test_before_commit(&plan, || panic!("before commit"))
    ))
    .is_err());
    drop(engine);
    let engine = Engine::open_with_clock(&path, clock).unwrap();
    assert!(read(&engine, &old).is_ok());
    assert_eq!(engine.plan_retention(&plan.policy).unwrap(), plan);
    engine.compact_retention(&plan).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT count(*) FROM retention_tombstones", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM revisions WHERE data=''", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        STORAGE_VERSION
    );
}
