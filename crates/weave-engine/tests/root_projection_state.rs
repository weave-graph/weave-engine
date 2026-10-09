//! Independent state/checkpoint, replay and stale-input transaction oracles.
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use weave_contract::*;
use weave_engine::*;

fn owner() -> HostContext {
    HostContext::new("owner", ["input".into(), "output".into()])
}
fn write(engine: &Engine, value: i64, graph: &str) -> Program {
    serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"expected_head":engine.head(graph,"main").unwrap(),
        "data":{"nodes":[{"id":"n","entity_id":"e","space_id":"s","readers":["owner"],"properties":{"value":value}}]}}]})).unwrap()
}
fn manifest() -> AdapterManifest {
    AdapterManifest {
        id: "projection".into(),
        version: "1".into(),
        artifact_digest: format!("sha256:{}", "a".repeat(64)),
        config_revision: "1".into(),
        principal: "owner".into(),
        subscriptions: vec![SubscriptionScope {
            graph_id: "input".into(),
            branch_id: "main".into(),
        }],
        output_graphs: vec!["output".into()],
        effect_destinations: vec![],
        max_attempts: 3,
        lease_ms: 100,
        max_pending_events: 100,
        projection_replay: true,
    }
}
fn digest(state: &ProjectionRebaseRequest) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(state).unwrap())
    )
}
fn setup(engine: &mut Engine) -> ProjectionRebaseReceipt {
    engine
        .execute(&write(engine, 1, "input"), &owner())
        .unwrap();
    engine.install_adapter(&manifest(), &owner()).unwrap();
    engine.set_adapter_state("projection", "running").unwrap();
    let inputs = engine
        .projection_rebase_inputs_for("projection", &owner())
        .unwrap();
    engine
        .rebase_projection_for(
            &ProjectionRebaseRequest {
                inputs,
                state_revision: "initial".into(),
                state: json!({"total":1}),
            },
            &owner(),
        )
        .unwrap()
}
#[test]
fn a_preexisting_lease_finishes_once_then_requires_the_new_epoch_rebuild() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    let initial = setup(&mut engine);
    clock.set(20);
    engine
        .execute(&write(&engine, 2, "input"), &owner())
        .unwrap();
    let delivery = engine
        .poll_adapter_for("projection", &owner())
        .unwrap()
        .unwrap();
    let completion = request(&engine, &delivery, &initial.binding_digest, 3);
    clock.set(30);
    let plan = engine
        .plan_retention(&RetentionPolicy {
            history_before_ms: 25,
            replay_through_sequence: 2,
        })
        .unwrap();
    engine.compact_retention(&plan).unwrap();
    assert!(
        !engine
            .complete_projection_for(&completion, &owner())
            .unwrap()
            .duplicate
    );
    assert!(
        engine
            .complete_projection_for(&completion, &owner())
            .unwrap()
            .duplicate
    );
    assert_eq!(
        engine
            .poll_adapter_for("projection", &owner())
            .unwrap_err()
            .code,
        "E_CHECKPOINT_EXPIRED"
    );
    let inputs = engine
        .projection_rebase_inputs_for("projection", &owner())
        .unwrap();
    engine
        .rebase_projection_for(
            &ProjectionRebaseRequest {
                inputs,
                state_revision: "rebuilt".into(),
                state: json!({"total":2}),
            },
            &owner(),
        )
        .unwrap();
    assert!(engine
        .poll_adapter_for("projection", &owner())
        .unwrap()
        .is_none());
}
fn request(
    engine: &Engine,
    delivery: &DispatchEnvelope,
    prior: &str,
    value: i64,
) -> ProjectionCompletionRequest {
    ProjectionCompletionRequest {
        adapter: "projection".into(),
        event: delivery.id.clone(),
        lease: delivery.lease.clone(),
        prior_state_digest: prior.into(),
        state_revision: format!("state{value}"),
        state: json!({"total":value}),
        input_snapshots: vec![delivery.graph.clone()],
        program: write(engine, value, "output"),
    }
}
#[test]
fn corrupt_valid_json_state_or_receipt_never_releases_retained_inputs_during_collection() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("projection.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let initial = setup(&mut engine);
    engine
        .execute(
            &write(&engine, 99, "orphan"),
            &HostContext::new("owner", ["orphan".into()]),
        )
        .unwrap();
    let orphan = engine.head("orphan", "main").unwrap().unwrap();
    engine
        .release_branch_for(
            "orphan",
            "main",
            &orphan,
            &HostContext::new("owner", ["orphan".into()]),
        )
        .unwrap();
    clock.set(20);
    engine
        .execute(&write(&engine, 2, "input"), &owner())
        .unwrap();
    let delivery = engine
        .poll_adapter_for("projection", &owner())
        .unwrap()
        .unwrap();
    let completion = request(&engine, &delivery, &initial.binding_digest, 3);
    engine
        .complete_projection_for(&completion, &owner())
        .unwrap();
    clock.set(30);
    let plan = engine
        .plan_retention(&RetentionPolicy {
            history_before_ms: 25,
            replay_through_sequence: engine.events().unwrap().last().unwrap().sequence as i64,
        })
        .unwrap();
    assert_eq!(
        plan.collect,
        vec![GraphRef {
            graph_id: "orphan".into(),
            revision: orphan.clone()
        }]
    );
    let sql = rusqlite::Connection::open(&path).unwrap();
    for table in ["retention_adapter_states", "retention_projection_receipts"] {
        let query = format!("SELECT body FROM {table} WHERE adapter='projection'");
        let original: String = sql.query_row(&query, [], |r| r.get(0)).unwrap();
        let mut changed: serde_json::Value = serde_json::from_str(&original).unwrap();
        if table == "retention_adapter_states" {
            changed["state"] = json!({"total":999});
        } else {
            changed["after"]["inputs"]["snapshots"] = json!([]);
        }
        let update = format!("UPDATE {table} SET body=?1 WHERE adapter='projection'");
        sql.execute(&update, [serde_json::to_string(&changed).unwrap()])
            .unwrap();
        assert_eq!(
            engine.compact_retention(&plan).unwrap_err().code,
            "E_REBASE_INTEGRITY"
        );
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
        assert!(sql
            .query_row(
                "SELECT length(data)>0 FROM revisions WHERE revision=?1",
                [&orphan],
                |r| r.get::<_, bool>(0)
            )
            .unwrap());
        sql.execute(&update, [&original]).unwrap();
    }
    assert_eq!(
        engine.compact_retention(&plan).unwrap().collected_payloads,
        1
    );
}
#[test]
fn detached_or_missing_state_never_replays_or_advances_its_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("projection.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let initial = setup(&mut engine);
    clock.set(20);
    engine
        .execute(&write(&engine, 2, "input"), &owner())
        .unwrap();
    let delivery = engine
        .poll_adapter_for("projection", &owner())
        .unwrap()
        .unwrap();
    let completion = request(&engine, &delivery, &initial.binding_digest, 3);
    engine
        .complete_projection_for(&completion, &owner())
        .unwrap();
    let count = engine.event_count().unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute(
        "UPDATE dispatch_adapters SET checkpoint=checkpoint+1 WHERE id='projection'",
        [],
    )
    .unwrap();
    assert_eq!(
        engine
            .projection_state_for("projection", &owner())
            .unwrap_err()
            .code,
        "E_REBASE_INTEGRITY"
    );
    assert_eq!(
        engine
            .complete_projection_for(&completion, &owner())
            .unwrap_err()
            .code,
        "E_REBASE_INTEGRITY"
    );
    sql.execute("UPDATE dispatch_adapters SET checkpoint=(SELECT checkpoint FROM retention_adapter_states WHERE adapter='projection') WHERE id='projection'",[]).unwrap();
    assert!(engine.projection_state_for("projection", &owner()).is_ok());
    sql.execute(
        "DELETE FROM retention_adapter_states WHERE adapter='projection'",
        [],
    )
    .unwrap();
    assert_eq!(
        engine
            .poll_adapter_for("projection", &owner())
            .unwrap_err()
            .code,
        "E_REBASE_INTEGRITY"
    );
    assert_eq!(
        engine
            .complete_projection_for(&completion, &owner())
            .unwrap_err()
            .code,
        "E_REBASE_INTEGRITY"
    );
    assert_eq!(
        engine
            .complete_handler(
                "projection",
                &delivery.id,
                &delivery.lease,
                &completion.program
            )
            .unwrap_err()
            .code,
        "E_PROJECTION_STATE"
    );
    assert_eq!(engine.event_count().unwrap(), count);
}
#[test]
fn completion_survives_restart_and_old_duplicates_never_rewind_newer_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("projection.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let initial = setup(&mut engine);
    clock.set(20);
    engine
        .execute(&write(&engine, 2, "input"), &owner())
        .unwrap();
    let delivery = engine
        .poll_adapter_for("projection", &owner())
        .unwrap()
        .unwrap();
    let first = request(&engine, &delivery, &initial.binding_digest, 3);
    assert_eq!(
        engine
            .complete_handler("projection", &delivery.id, &delivery.lease, &first.program)
            .unwrap_err()
            .code,
        "E_PROJECTION_STATE"
    );
    assert!(engine.head("output", "main").unwrap().is_none());
    assert!(
        !engine
            .complete_projection_for(&first, &owner())
            .unwrap()
            .duplicate
    );
    let state1 = engine.projection_state_for("projection", &owner()).unwrap();
    assert_eq!(state1.state, json!({"total":3}));
    assert_eq!(
        state1.inputs.snapshots.len(),
        3,
        "initial input, delivered input and output are retained"
    );
    drop(engine);
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    assert_eq!(
        engine.projection_state_for("projection", &owner()).unwrap(),
        state1
    );
    assert!(engine
        .poll_adapter_for("projection", &owner())
        .unwrap()
        .is_none());
    clock.set(30);
    engine
        .execute(&write(&engine, 4, "input"), &owner())
        .unwrap();
    let second_delivery = engine
        .poll_adapter_for("projection", &owner())
        .unwrap()
        .unwrap();
    let mut second = request(&engine, &second_delivery, &initial.binding_digest, 7);
    assert_eq!(
        engine
            .complete_projection_for(&second, &owner())
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    assert_eq!(
        engine.projection_state_for("projection", &owner()).unwrap(),
        state1
    );
    second.prior_state_digest = digest(&state1);
    engine.complete_projection_for(&second, &owner()).unwrap();
    let state2 = engine.projection_state_for("projection", &owner()).unwrap();
    let events = engine.event_count().unwrap();
    assert!(
        engine
            .complete_projection_for(&first, &owner())
            .unwrap()
            .duplicate
    );
    assert_eq!(
        engine.projection_state_for("projection", &owner()).unwrap(),
        state2
    );
    assert_eq!(engine.event_count().unwrap(), events);
    let mut changed = first.clone();
    changed.state = json!({"total":999});
    assert_eq!(
        engine
            .complete_projection_for(&changed, &owner())
            .unwrap_err()
            .code,
        "E_RECEIPT_CONFLICT"
    );
    let outsider = HostContext::new("outsider", ["input".into(), "output".into()]);
    assert!(engine
        .projection_state_for("projection", &outsider)
        .is_err());
    assert!(engine.complete_projection_for(&first, &outsider).is_err());
}
#[test]
fn expired_policy_requires_explicit_rebuild_and_rebase_never_skips_pending_work() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    setup(&mut engine);
    clock.set(30);
    let policy = RetentionPolicy {
        history_before_ms: 20,
        replay_through_sequence: 1,
    };
    let plan = engine.plan_retention(&policy).unwrap();
    engine.compact_retention(&plan).unwrap();
    assert_eq!(
        engine
            .poll_adapter_for("projection", &owner())
            .unwrap_err()
            .code,
        "E_CHECKPOINT_EXPIRED"
    );
    let inputs = engine
        .projection_rebase_inputs_for("projection", &owner())
        .unwrap();
    let rebuilt = ProjectionRebaseRequest {
        inputs,
        state_revision: "rebuilt".into(),
        state: json!({}),
    };
    engine.rebase_projection_for(&rebuilt, &owner()).unwrap();
    clock.set(40);
    engine
        .execute(&write(&engine, 2, "input"), &owner())
        .unwrap();
    let delivery = engine
        .poll_adapter_for("projection", &owner())
        .unwrap()
        .unwrap();
    let inputs = engine
        .projection_rebase_inputs_for("projection", &owner())
        .unwrap();
    let skipped = ProjectionRebaseRequest {
        inputs,
        state_revision: "skipped".into(),
        state: json!({}),
    };
    assert_eq!(
        engine
            .rebase_projection_for(&skipped, &owner())
            .unwrap_err()
            .code,
        "E_REBASE_PENDING"
    );
    assert_eq!(
        engine.projection_state_for("projection", &owner()).unwrap(),
        rebuilt
    );
    let completion = request(&engine, &delivery, &digest(&rebuilt), 2);
    engine
        .complete_projection_for(&completion, &owner())
        .unwrap();
    assert!(engine
        .poll_adapter_for("projection", &owner())
        .unwrap()
        .is_none());
}
#[cfg(feature = "recovery-testing")]
#[test]
fn panic_before_commit_preserves_state_checkpoint_output_and_retry_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("projection.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let initial = setup(&mut engine);
    clock.set(20);
    engine
        .execute(&write(&engine, 2, "input"), &owner())
        .unwrap();
    let delivery = engine
        .poll_adapter_for("projection", &owner())
        .unwrap()
        .unwrap();
    let completion = request(&engine, &delivery, &initial.binding_digest, 3);
    let before = engine.projection_state_for("projection", &owner()).unwrap();
    let count = engine.event_count().unwrap();
    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        engine.complete_projection_test_before_commit(&completion, &owner(), || {
            panic!("before commit")
        })
    }));
    assert!(stopped.is_err());
    drop(engine);
    let mut engine = Engine::open_with_clock(&path, clock).unwrap();
    assert_eq!(
        engine.projection_state_for("projection", &owner()).unwrap(),
        before
    );
    assert_eq!(engine.event_count().unwrap(), count);
    assert!(engine.head("output", "main").unwrap().is_none());
    assert!(
        !engine
            .complete_projection_for(&completion, &owner())
            .unwrap()
            .duplicate
    );
    assert_eq!(engine.event_count().unwrap(), count + 1);
    assert!(
        engine
            .complete_projection_for(&completion, &owner())
            .unwrap()
            .duplicate
    );
}
