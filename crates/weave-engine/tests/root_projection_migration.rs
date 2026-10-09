//! Independent actual artifact/state/checkpoint upgrade and rollback oracles.
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use weave_contract::*;
use weave_engine::*;
fn owner() -> HostContext {
    HostContext::new("owner", ["input".into(), "output".into()])
}
fn digest(state: &ProjectionRebaseRequest) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(state).unwrap())
    )
}
fn manifest(id: &str, version: &str, artifact: char) -> AdapterManifest {
    AdapterManifest {
        id: id.into(),
        version: version.into(),
        artifact_digest: format!("sha256:{}", artifact.to_string().repeat(64)),
        config_revision: version.into(),
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
fn write(engine: &mut Engine, graph: &str, value: i64) {
    let program: Program = serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"expected_head":engine.head(graph,"main").unwrap(),"data":{"nodes":[{"id":"n","entity_id":"e","space_id":"s","readers":["owner"],"properties":{"value":value}}]}}]})).unwrap();
    engine.execute(&program, &owner()).unwrap();
}
fn setup(engine: &mut Engine) -> ProjectionRebaseRequest {
    write(engine, "input", 1);
    engine
        .install_adapter(&manifest("worker1", "1", 'a'), &owner())
        .unwrap();
    let seed = ProjectionRebaseRequest {
        inputs: engine
            .projection_rebase_inputs_for("worker1", &owner())
            .unwrap(),
        state_revision: "seed".into(),
        state: json!({"total":1}),
    };
    engine.rebase_projection_for(&seed, &owner()).unwrap();
    engine
        .set_adapter_state_for("worker1", "paused", &owner())
        .unwrap();
    seed
}
fn upgrade(engine: &Engine) -> ProjectionMigrationRequest {
    ProjectionMigrationRequest {
        inputs: engine
            .projection_migration_inputs_for("worker1", &owner())
            .unwrap(),
        destination: manifest("worker2", "2", 'b'),
        event_schema: VERSION.into(),
        nonce: "upgrade1".into(),
        disposition: ProjectionMigrationKind::Upgrade,
        state_revision: "upgraded".into(),
        state: json!({"sum":1,"algorithm":2}),
    }
}
fn complete(engine: &mut Engine, adapter: &str, value: i64) -> HandlerReceipt {
    let state = engine.projection_state_for(adapter, &owner()).unwrap();
    let delivery = engine.poll_adapter_for(adapter, &owner()).unwrap().unwrap();
    let program: Program = serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"output","expected_head":engine.head("output","main").unwrap(),"data":{"nodes":[{"id":"n","entity_id":"e","space_id":"s","properties":{"value":value},"readers":["owner"]}]}}]})).unwrap();
    engine
        .complete_projection_for(
            &ProjectionCompletionRequest {
                adapter: adapter.into(),
                event: delivery.id,
                lease: delivery.lease,
                prior_state_digest: digest(&state),
                state_revision: "processed".into(),
                state: json!({"sum":value}),
                input_snapshots: vec![delivery.graph],
                program,
            },
            &owner(),
        )
        .unwrap()
}
#[test]
fn upgrade_and_explicit_rollback_restore_actual_pair_without_rewriting_outputs_or_rewinding_duplicates(
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let seed = setup(&mut engine);
    let request = upgrade(&engine);
    let events = engine.event_count().unwrap();
    let receipt = engine.migrate_projection_for(&request, &owner()).unwrap();
    assert!(!receipt.duplicate);
    assert_eq!(engine.event_count().unwrap(), events);
    assert!(engine.head("output", "main").unwrap().is_none());
    assert_eq!(
        engine
            .projection_state_for("worker2", &owner())
            .unwrap()
            .state,
        request.state
    );
    assert_eq!(
        engine
            .set_adapter_state_for("worker1", "running", &owner())
            .unwrap_err()
            .code,
        "E_LIFECYCLE"
    );
    let sql = rusqlite::Connection::open(&path).unwrap();
    let checkpoints: Vec<(String, i64)> = sql
        .prepare("SELECT id,checkpoint FROM dispatch_adapters ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(
        checkpoints,
        vec![("worker1".into(), 1), ("worker2".into(), 1)]
    );
    drop(engine);
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    engine
        .set_adapter_state_for("worker2", "running", &owner())
        .unwrap();
    clock.set(20);
    write(&mut engine, "input", 2);
    complete(&mut engine, "worker2", 3);
    let output = engine.head("output", "main").unwrap();
    let processed = engine.projection_state_for("worker2", &owner()).unwrap();
    assert!(
        engine
            .migrate_projection_for(&request, &owner())
            .unwrap()
            .duplicate
    );
    assert_eq!(
        engine.projection_state_for("worker2", &owner()).unwrap(),
        processed
    );
    assert_eq!(engine.head("output", "main").unwrap(), output);
    engine
        .set_adapter_state_for("worker2", "paused", &owner())
        .unwrap();
    let rollback = ProjectionMigrationRequest {
        inputs: engine
            .projection_migration_inputs_for("worker2", &owner())
            .unwrap(),
        destination: manifest("worker3", "1", 'a'),
        event_schema: VERSION.into(),
        nonce: "rollback1".into(),
        disposition: ProjectionMigrationKind::Rollback {
            restore_from: "worker1".into(),
        },
        state_revision: seed.state_revision.clone(),
        state: seed.state.clone(),
    };
    let mut forged = rollback.clone();
    forged.state = json!({"total":99});
    assert_eq!(
        engine
            .migrate_projection_for(&forged, &owner())
            .unwrap_err()
            .code,
        "E_MIGRATION_COMPATIBILITY"
    );
    let count = engine.event_count().unwrap();
    engine.migrate_projection_for(&rollback, &owner()).unwrap();
    assert_eq!(engine.event_count().unwrap(), count);
    assert_eq!(engine.head("output", "main").unwrap(), output);
    let restored = engine.projection_state_for("worker3", &owner()).unwrap();
    assert_eq!(restored.state, seed.state);
    assert_eq!(restored.inputs.snapshots, seed.inputs.snapshots);
    assert_eq!(
        sql.query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='worker3'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        engine
            .set_adapter_state_for("worker2", "running", &owner())
            .unwrap_err()
            .code,
        "E_LIFECYCLE"
    );
    engine
        .set_adapter_state_for("worker3", "running", &owner())
        .unwrap();
    complete(&mut engine, "worker3", 2);
    let state = engine.projection_state_for("worker3", &owner()).unwrap();
    assert!(
        engine
            .migrate_projection_for(&rollback, &owner())
            .unwrap()
            .duplicate
    );
    assert_eq!(
        engine.projection_state_for("worker3", &owner()).unwrap(),
        state
    );
}
#[test]
fn owner_scopes_schema_and_actual_pending_checkpoint_are_checked_before_installation() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    setup(&mut engine);
    let request = upgrade(&engine);
    let outsider = HostContext::new("outsider", ["output".into()]);
    assert_eq!(
        engine
            .migrate_projection_for(&request, &outsider)
            .unwrap_err()
            .code,
        "E_HOST_AUTH"
    );
    for mode in ["schema", "scope", "effect"] {
        let mut changed = request.clone();
        match mode {
            "schema" => changed.event_schema = "incompatible".into(),
            "scope" => changed.destination.subscriptions[0].graph_id = "extra".into(),
            _ => changed
                .destination
                .effect_destinations
                .push("remote".into()),
        }
        assert_eq!(
            engine
                .migrate_projection_for(&changed, &owner())
                .unwrap_err()
                .code,
            "E_MIGRATION_COMPATIBILITY"
        );
    }
    engine
        .set_adapter_state_for("worker1", "running", &owner())
        .unwrap();
    clock.set(20);
    write(&mut engine, "input", 2);
    let delivery = engine
        .poll_adapter_for("worker1", &owner())
        .unwrap()
        .unwrap();
    engine
        .set_adapter_state_for("worker1", "paused", &owner())
        .unwrap();
    assert_eq!(
        engine
            .migrate_projection_for(&request, &owner())
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    let mut fresh = upgrade(&engine);
    assert_eq!(
        engine
            .migrate_projection_for(&fresh, &owner())
            .unwrap_err()
            .code,
        "E_MIGRATION_PENDING"
    );
    engine
        .cancel_handler_delivery_for(
            &DeliveryCancellationRequest {
                adapter: "worker1".into(),
                event: delivery.id,
                expected_lease: delivery.lease,
                nonce: "dispose".into(),
                reason: DeliveryCancellationReason::OwnerStop,
            },
            &owner(),
        )
        .unwrap();
    assert_eq!(
        engine
            .projection_migration_inputs_for("worker1", &owner())
            .unwrap_err()
            .code,
        "E_CHECKPOINT_EXPIRED"
    );
    let inputs = engine
        .projection_rebase_inputs_for("worker1", &owner())
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
    fresh.inputs = engine
        .projection_migration_inputs_for("worker1", &owner())
        .unwrap();
    fresh.state = json!({"sum":2});
    engine.migrate_projection_for(&fresh, &owner()).unwrap();
    let encoded = serde_json::to_string(&fresh.inputs).unwrap();
    assert!(!encoded.contains("sequence") && !encoded.contains("checkpoint\":"));
    let mut changed = fresh.clone();
    changed.state = json!({"sum":999});
    assert_eq!(
        engine
            .migrate_projection_for(&changed, &owner())
            .unwrap_err()
            .code,
        "E_RECEIPT_CONFLICT"
    );
}
#[test]
fn expired_rollback_pair_cannot_reuse_a_checkpoint_even_after_the_current_worker_rebuilds() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    let seed = setup(&mut engine);
    engine
        .migrate_projection_for(&upgrade(&engine), &owner())
        .unwrap();
    clock.set(30);
    let plan = engine
        .plan_retention(&RetentionPolicy {
            history_before_ms: 20,
            replay_through_sequence: 1,
        })
        .unwrap();
    engine.compact_retention(&plan).unwrap();
    let inputs = engine
        .projection_rebase_inputs_for("worker2", &owner())
        .unwrap();
    engine
        .rebase_projection_for(
            &ProjectionRebaseRequest {
                inputs,
                state_revision: "rebuilt".into(),
                state: json!({"sum":1}),
            },
            &owner(),
        )
        .unwrap();
    let request = ProjectionMigrationRequest {
        inputs: engine
            .projection_migration_inputs_for("worker2", &owner())
            .unwrap(),
        destination: manifest("worker3", "1", 'a'),
        event_schema: VERSION.into(),
        nonce: "rollback".into(),
        disposition: ProjectionMigrationKind::Rollback {
            restore_from: "worker1".into(),
        },
        state_revision: seed.state_revision,
        state: seed.state,
    };
    assert_eq!(
        engine
            .migrate_projection_for(&request, &owner())
            .unwrap_err()
            .code,
        "E_CHECKPOINT_EXPIRED"
    );
}
#[cfg(feature = "recovery-testing")]
#[test]
fn panic_rolls_back_new_namespace_retirement_pair_and_receipt_and_reopen_accepts_one_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    setup(&mut engine);
    let request = upgrade(&engine);
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        engine.migrate_projection_test_before_commit(&request, &owner(), || {
            panic!("migration boundary")
        })
    }));
    assert!(failure.is_err());
    let sql = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        sql.query_row("SELECT count(*) FROM dispatch_adapters", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM projection_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row(
            "SELECT state FROM dispatch_adapters WHERE id='worker1'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "paused"
    );
    drop(engine);
    let engine = Engine::open_with_clock(&path, clock).unwrap();
    engine.migrate_projection_for(&request, &owner()).unwrap();
    assert!(
        engine
            .migrate_projection_for(&request, &owner())
            .unwrap()
            .duplicate
    );
}
#[test]
fn valid_json_migration_corruption_fails_before_retention_mutates_any_payload_or_policy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    setup(&mut engine);
    engine
        .migrate_projection_for(&upgrade(&engine), &owner())
        .unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute(
        "UPDATE projection_migrations SET body=json_set(body,'$.before_checkpoint',999)",
        [],
    )
    .unwrap();
    clock.set(30);
    assert_eq!(
        engine
            .plan_retention(&RetentionPolicy {
                history_before_ms: 20,
                replay_through_sequence: 1
            })
            .unwrap_err()
            .code,
        "E_LIFECYCLE_INTEGRITY"
    );
    assert_eq!(
        sql.query_row("SELECT generation FROM retention_policy", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM revisions WHERE data=''", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn unrelated_private_scan_changes_never_change_public_transfer_inputs_but_actual_cursor_is_preserved(
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    setup(&mut engine);
    engine
        .set_adapter_state_for("worker1", "running", &owner())
        .unwrap();
    let before = engine
        .projection_migration_inputs_for("worker1", &owner())
        .unwrap();
    clock.set(20);
    let private: Program = serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"unrelated-private","expected_head":null,"data":{"nodes":[{"id":"secret","entity_id":"private","space_id":"s","readers":["outsider"]}]}}]})).unwrap();
    engine
        .execute(
            &private,
            &HostContext::new("outsider", ["unrelated-private".into()]),
        )
        .unwrap();
    assert!(engine
        .poll_adapter_for("worker1", &owner())
        .unwrap()
        .is_none());
    assert_eq!(
        engine
            .projection_migration_inputs_for("worker1", &owner())
            .unwrap(),
        before
    );
    engine
        .set_adapter_state_for("worker1", "paused", &owner())
        .unwrap();
    let request = upgrade(&engine);
    engine.migrate_projection_for(&request, &owner()).unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        sql.query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='worker2'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}
