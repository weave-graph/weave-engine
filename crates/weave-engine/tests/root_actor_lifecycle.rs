//! Independent durable state/effect/output and replay oracles for trusted actors.
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("owner", ["input".into(), "output".into(), "other".into()])
}
fn program(e: &Engine, graph: &str, value: i64) -> Program {
    serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"expected_head":e.head(graph,"main").unwrap(),"data":{"nodes":[{"id":"n","entity_id":"e","space_id":"s","readers":["owner"],"properties":{"value":value}}]}}]})).unwrap()
}
fn write(e: &mut Engine, graph: &str, value: i64) {
    e.execute(&program(e, graph, value), &host()).unwrap();
}
fn definition() -> RecordedActorDefinition {
    let artifact = b"native recorded actor test artifact v1".to_vec();
    RecordedActorDefinition {
        manifest: serde_json::from_value(json!({"id":"actor","version":"1","artifact_digest":format!("sha256:{:x}",Sha256::digest(&artifact)),"config_revision":"1","principal":"owner","subscriptions":[{"graph_id":"input","branch_id":"main"}],"output_graphs":["output"],"effect_destinations":["sink"],"max_attempts":3,"lease_ms":1000,"max_pending_events":100,"projection_replay":false})).unwrap(),
        event_schema: VERSION.into(),
        state_protocol:"weave-recorded-opaque-state/1".into(), metadata_depth: 2, artifact,
    }
}
fn hash(value: &impl serde::Serialize) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value).unwrap())
    )
}
fn bootstrap(e: &Engine) -> RecordedActorBootstrap {
    RecordedActorBootstrap {
        inputs: e.recorded_actor_inputs_for("actor", &host()).unwrap(),
        state_revision: "initial".into(),
        state: json!({"total":1}),
    }
}
fn setup(e: &mut Engine) -> String {
    write(e, "input", 1);
    e.install_recorded_actor_for(&definition(), &host())
        .unwrap();
    let digest = e
        .bootstrap_recorded_actor_for(&bootstrap(e), &host())
        .unwrap();
    e.set_adapter_state_for("actor", "running", &host())
        .unwrap();
    digest
}
fn version(id: &str, v: u32) -> RecordedActorDefinition {
    let mut d = definition();
    d.manifest.id = id.into();
    d.manifest.version = v.to_string();
    d.artifact = format!("native actor actual fixture version {v}").into_bytes();
    d.manifest.artifact_digest = format!("sha256:{:x}", Sha256::digest(&d.artifact));
    d
}
fn state(e: &Engine, id: &str) -> RecordedActorState {
    e.recorded_actor_state_for(id, &host()).unwrap()
}
fn finish(
    e: &mut Engine,
    id: &str,
    event: &DispatchEnvelope,
    value: i64,
    effect: bool,
) -> RecordedActorReceipt {
    if effect {
        let intent = e
            .request_effect(
                id,
                &event.id,
                &event.lease,
                "sink",
                &format!("action-{value}"),
                json!({"value":value}),
            )
            .unwrap();
        e.begin_effect_dispatch(&intent.id).unwrap();
        e.reconcile_effect(
            &intent.id,
            "confirmed",
            json!({"destination_receipt":format!("physical-{value}")}),
        )
        .unwrap();
    }
    let request = RecordedActorCompletion {
        adapter: id.into(),
        event: event.id.clone(),
        lease: event.lease.clone(),
        prior_state_digest: hash(&state(e, id)),
        state_revision: format!("state-{value}"),
        state: json!({"total":value}),
        input_snapshots: vec![event.graph.clone()],
        tool_results: vec![RecordedToolResult {
            name: "sample".into(),
            media_type: "application/json".into(),
            value: json!({"sample":value}),
        }],
        program: program(e, "output", value),
    };
    e.complete_recorded_actor_for(&request, &host()).unwrap()
}
fn transfer_request(
    e: &Engine,
    id: &str,
    destination: RecordedActorDefinition,
    disposition: ProjectionMigrationKind,
) -> RecordedActorMigration {
    RecordedActorMigration {
        inputs: e.recorded_actor_migration_inputs_for(id, &host()).unwrap(),
        nonce: format!("move-{id}-{}", destination.manifest.id),
        destination,
        disposition,
    }
}
fn transfer(
    e: &Engine,
    id: &str,
    destination: RecordedActorDefinition,
    disposition: ProjectionMigrationKind,
) -> (RecordedActorMigration, RecordedActorMigrationReceipt) {
    let r = transfer_request(e, id, destination, disposition);
    let receipt = e.migrate_recorded_actor_for(&r, &host()).unwrap();
    (r, receipt)
}
fn run(e: &Engine, id: &str) {
    e.set_adapter_state_for(id, "running", &host()).unwrap();
}
fn pause(e: &Engine, id: &str) {
    e.set_adapter_state_for(id, "paused", &host()).unwrap();
}
fn observed(e: &Engine, id: &str, event: &DispatchEnvelope) -> RecordedActorObservation {
    RecordedActorObservation {
        adapter: id.into(),
        event: event.id.clone(),
        lease: event.lease.clone(),
        prior_state_digest: hash(&state(e, id)),
        nonce: format!("observe-{id}-{}", event.id),
    }
}
fn history(
    e: &mut Engine,
) -> (
    RecordedActorMigration,
    RecordedActorMigrationReceipt,
    RecordedActorReceipt,
) {
    setup(e);
    write(e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    finish(e, "actor", &event, 2, true);
    pause(e, "actor");
    let (upgrade, receipt) = transfer(
        e,
        "actor",
        version("v2", 2),
        ProjectionMigrationKind::Upgrade,
    );
    run(e, "v2");
    write(e, "input", 3);
    let event = e.poll_adapter_for("v2", &host()).unwrap().unwrap();
    let done = finish(e, "v2", &event, 3, true);
    pause(e, "v2");
    (upgrade, receipt, done)
}
fn rollback(e: &Engine, id: &str, dest: &str) -> RecordedActorMigrationReceipt {
    let mut d = definition();
    d.manifest.id = dest.into();
    transfer(
        e,
        id,
        d,
        ProjectionMigrationKind::Rollback {
            restore_from: "actor".into(),
        },
    )
    .1
}
#[test]
fn upgrade_rollback_observes_original_effect_artifact_and_never_rewinds_later_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut e = Engine::open_with_clock(&path, Arc::new(ManualClock::new(10))).unwrap();
    let (old, upgrade, done) = history(&mut e);
    let output = e.head("output", "main").unwrap();
    rollback(&e, "v2", "restored");
    assert_eq!(state(&e, "restored").state, json!({"total":2}));
    run(&e, "restored");
    let event = e.poll_adapter_for("restored", &host()).unwrap().unwrap();
    assert_eq!(
        e.recorded_actor_run_inputs_for("restored", &event.id, &event.lease, &host())
            .unwrap_err()
            .code,
        "E_ACTOR_REPLAY"
    );
    assert_eq!(
        e.request_effect(
            "restored",
            &event.id,
            &event.lease,
            "sink",
            "repeat",
            json!(null)
        )
        .unwrap_err()
        .code,
        "E_ACTOR_REPLAY"
    );
    let observation = observed(&e, "restored", &event);
    let receipt = e.observe_recorded_actor_for(&observation, &host()).unwrap();
    assert_eq!(receipt.source_adapter, "v2");
    assert_eq!(receipt.source_receipt, done);
    assert_eq!(state(&e, "restored").state, json!({"total":3}));
    assert_eq!(e.head("output", "main").unwrap(), output);
    write(&mut e, "input", 4);
    let next = e.poll_adapter_for("restored", &host()).unwrap().unwrap();
    finish(&mut e, "restored", &next, 4, true);
    let latest = state(&e, "restored");
    let output = e.head("output", "main").unwrap();
    let mut duplicate = observation.clone();
    duplicate.lease = "another-worker".into();
    let again = e.observe_recorded_actor_for(&duplicate, &host()).unwrap();
    assert!(again.handler.duplicate);
    assert_eq!(again.receipt_id, receipt.receipt_id);
    assert_eq!(state(&e, "restored"), latest);
    assert_eq!(e.head("output", "main").unwrap(), output);
    let again = e.migrate_recorded_actor_for(&old, &host()).unwrap();
    assert!(again.duplicate);
    assert_eq!(again.receipt_id, upgrade.receipt_id);
    let sql = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        sql.query_row(
            "SELECT count(*) FROM effect_intents WHERE adapter='restored'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        sql.query_row(
            "SELECT count(*) FROM recorded_actor_observations",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    collect(&e).unwrap();
    drop(e);
    let e = Engine::open_with_clock(&path, Arc::new(ManualClock::new(10))).unwrap();
    assert_eq!(state(&e, "restored"), latest);
    assert_eq!(
        e.recorded_actor_observation_for("restored", &event.id, &host())
            .unwrap(),
        receipt
    );
}
#[test]
fn nested_rollback_reaches_actual_ancestor_before_intermediate_observation() {
    let mut e = Engine::memory().unwrap();
    history(&mut e);
    rollback(&e, "v2", "r1");
    rollback(&e, "r1", "r2");
    run(&e, "r2");
    let event = e.poll_adapter_for("r2", &host()).unwrap().unwrap();
    let request = observed(&e, "r2", &event);
    let out = e.observe_recorded_actor_for(&request, &host()).unwrap();
    assert_eq!(out.source_adapter, "v2");
    assert_eq!(state(&e, "r2").state, json!({"total":3}));
    collect(&e).unwrap();
}
#[test]
fn transfer_requires_drained_compatible_actual_state_and_current_input_cas() {
    let mut e = Engine::memory().unwrap();
    setup(&mut e);
    write(&mut e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    pause(&e, "actor");
    let r = transfer_request(
        &e,
        "actor",
        version("v2", 2),
        ProjectionMigrationKind::Upgrade,
    );
    assert_eq!(
        e.migrate_recorded_actor_for(&r, &host()).unwrap_err().code,
        "E_ACTOR_PENDING"
    );
    run(&e, "actor");
    finish(&mut e, "actor", &event, 2, false);
    pause(&e, "actor");
    let mut r = transfer_request(
        &e,
        "actor",
        version("v2", 2),
        ProjectionMigrationKind::Upgrade,
    );
    r.destination.state_protocol = "foreign-state-abi".into();
    assert_eq!(
        e.migrate_recorded_actor_for(&r, &host()).unwrap_err().code,
        "E_ACTOR_COMPATIBILITY"
    );
    let r = transfer_request(
        &e,
        "actor",
        version("v2", 2),
        ProjectionMigrationKind::Upgrade,
    );
    write(&mut e, "input", 3);
    assert_eq!(
        e.migrate_recorded_actor_for(&r, &host()).unwrap_err().code,
        "E_CONFLICT"
    );
    assert!(e.recorded_actor_state_for("v2", &host()).is_err());
}
#[test]
fn stale_state_nonce_authority_and_missing_history_never_start_new_computation() {
    let mut e = Engine::memory().unwrap();
    history(&mut e);
    rollback(&e, "v2", "restored");
    run(&e, "restored");
    let event = e.poll_adapter_for("restored", &host()).unwrap().unwrap();
    let mut r = observed(&e, "restored", &event);
    r.prior_state_digest = "bad".into();
    assert_eq!(
        e.observe_recorded_actor_for(&r, &host()).unwrap_err().code,
        "E_CONFLICT"
    );
    let r = observed(&e, "restored", &event);
    let wrong = HostContext::new("other", ["output".into()]);
    assert!(e.observe_recorded_actor_for(&r, &wrong).is_err());
    e.observe_recorded_actor_for(&r, &host()).unwrap();
    let mut different = r;
    different.nonce = "different".into();
    assert_eq!(
        e.observe_recorded_actor_for(&different, &host())
            .unwrap_err()
            .code,
        "E_RECEIPT_CONFLICT"
    );
    write(&mut e, "input", 4);
    let next = e.poll_adapter_for("restored", &host()).unwrap().unwrap();
    let r = observed(&e, "restored", &next);
    assert_eq!(
        e.observe_recorded_actor_for(&r, &host()).unwrap_err().code,
        "E_ACTOR_OBSERVATION"
    );
}
#[test]
fn default_state_abi_preserves_old_serialized_definition_hash() {
    let d = definition();
    let json = serde_json::to_value(&d).unwrap();
    assert!(json.get("state_protocol").is_none());
    let restored: RecordedActorDefinition = serde_json::from_value(json).unwrap();
    assert_eq!(restored, d);
    assert_eq!(hash(&restored), hash(&d));
}
#[test]
fn actual_missing_receipt_and_corrupt_observation_stop_replay_and_collection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut e = Engine::open_with_clock(&path, Arc::new(ManualClock::new(10))).unwrap();
    history(&mut e);
    rollback(&e, "v2", "restored");
    run(&e, "restored");
    let event = e.poll_adapter_for("restored", &host()).unwrap().unwrap();
    let r = observed(&e, "restored", &event);
    let sql = rusqlite::Connection::open(&path).unwrap();
    let body: (String, String) = sql
        .query_row(
            "SELECT body,digest FROM recorded_actor_receipts WHERE adapter='v2'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    sql.execute("DELETE FROM recorded_actor_receipts WHERE adapter='v2'", [])
        .unwrap();
    assert_eq!(
        e.observe_recorded_actor_for(&r, &host()).unwrap_err().code,
        "E_REPLAY_UNAVAILABLE"
    );
    assert_eq!(state(&e, "restored").state, json!({"total":2}));
    sql.execute(
        "INSERT INTO recorded_actor_receipts VALUES ('v2',?1,?2,?3)",
        rusqlite::params![event.id, body.0, body.1],
    )
    .unwrap();
    e.observe_recorded_actor_for(&r, &host()).unwrap();
    sql.execute("UPDATE recorded_actor_observations SET nonce='corrupt'", [])
        .unwrap();
    assert_eq!(
        e.recorded_actor_observation_for("restored", &event.id, &host())
            .unwrap_err()
            .code,
        "E_ACTOR_OBSERVATION_INTEGRITY"
    );
    assert!(collect(&e).is_err());
}
#[cfg(feature = "recovery-testing")]
#[test]
fn transfer_and_observation_panics_roll_back_all_state_and_receipts() {
    let mut e = Engine::memory().unwrap();
    history(&mut e);
    let mut d = definition();
    d.manifest.id = "restored".into();
    let r = transfer_request(
        &e,
        "v2",
        d,
        ProjectionMigrationKind::Rollback {
            restore_from: "actor".into(),
        },
    );
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        e.migrate_recorded_actor_test_before_commit(&r, &host(), || panic!("transfer-death"))
    }));
    assert!(failed.is_err());
    assert!(e.recorded_actor_state_for("restored", &host()).is_err());
    e.migrate_recorded_actor_for(&r, &host()).unwrap();
    run(&e, "restored");
    let event = e.poll_adapter_for("restored", &host()).unwrap().unwrap();
    let r = observed(&e, "restored", &event);
    let before = state(&e, "restored");
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || e.observe_recorded_actor_test_before_commit(&r, &host(), || panic!("observation-death"))
    ))
    .is_err());
    assert_eq!(state(&e, "restored"), before);
    assert!(e
        .recorded_actor_observation_for("restored", &event.id, &host())
        .is_err());
    e.observe_recorded_actor_for(&r, &host()).unwrap();
}

fn collect(e: &Engine) -> Result<RetentionReceipt> {
    let p = e.plan_retention(&RetentionPolicy::default())?;
    e.compact_retention(&p)
}
#[test]
fn exact_completion_retry_remains_readable_after_retirement_and_cannot_change_computation() {
    let mut e = Engine::memory().unwrap();
    let initial = setup(&mut e);
    write(&mut e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    let mut r = RecordedActorCompletion {
        adapter: "actor".into(),
        event: event.id.clone(),
        lease: event.lease.clone(),
        prior_state_digest: initial,
        state_revision: "original".into(),
        state: json!({"total":2}),
        input_snapshots: vec![event.graph.clone()],
        tool_results: vec![],
        program: program(&e, "output", 2),
    };
    let original = e.complete_recorded_actor_for(&r, &host()).unwrap();
    pause(&e, "actor");
    transfer(
        &e,
        "actor",
        version("v2", 2),
        ProjectionMigrationKind::Upgrade,
    );
    let carried = state(&e, "v2");
    let head = e.head("output", "main").unwrap();
    r.lease = "expired-worker".into();
    let retry = e.complete_recorded_actor_for(&r, &host()).unwrap();
    assert!(retry.handler.duplicate);
    assert_eq!(retry.receipt_id, original.receipt_id);
    assert_eq!(state(&e, "v2"), carried);
    assert_eq!(e.head("output", "main").unwrap(), head);
    r.state = json!(99);
    assert_eq!(
        e.complete_recorded_actor_for(&r, &host()).unwrap_err().code,
        "E_RECEIPT_CONFLICT"
    );
}
#[test]
fn lifecycle_schema_loss_downgrade_and_cross_bound_fence_are_fail_closed() {
    for table in [
        "recorded_actor_migrations",
        "recorded_actor_replay_fences",
        "recorded_actor_observations",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        drop(Engine::open(&path).unwrap());
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute(&format!("DROP TABLE {table}"), []).unwrap();
        assert!(Engine::open(&path).is_err());
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut e = Engine::open_with_clock(&path, Arc::new(ManualClock::new(10))).unwrap();
    history(&mut e);
    rollback(&e, "v2", "restored");
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute(
        "UPDATE recorded_actor_replay_fences SET source_adapter='actor'",
        [],
    )
    .unwrap();
    assert_eq!(collect(&e).unwrap_err().code, "E_ACTOR_LIFECYCLE_INTEGRITY");
    drop(e);
    sql.execute("PRAGMA user_version=26", []).unwrap();
    assert_eq!(
        Engine::open(&path).err().unwrap().code,
        "E_ACTOR_LIFECYCLE_INTEGRITY"
    );
}
#[test]
fn private_scans_leave_transfer_cas_public_identity_unchanged_but_copy_actual_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut e = Engine::open_with_clock(&path, Arc::new(ManualClock::new(10))).unwrap();
    setup(&mut e);
    let initial = e
        .recorded_actor_migration_inputs_for("actor", &host())
        .unwrap();
    write(&mut e, "other", 1);
    assert!(e.poll_adapter_for("actor", &host()).unwrap().is_none());
    let current = e
        .recorded_actor_migration_inputs_for("actor", &host())
        .unwrap();
    assert_eq!(initial, current);
    pause(&e, "actor");
    transfer(
        &e,
        "actor",
        version("v2", 2),
        ProjectionMigrationKind::Upgrade,
    );
    let sql = rusqlite::Connection::open(&path).unwrap();
    let cps: Vec<i64> = sql
        .prepare("SELECT checkpoint FROM dispatch_adapters WHERE id IN ('actor','v2') ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(cps[0], cps[1]);
}
#[test]
fn current_hidden_primary_blocks_observation_and_even_empty_historical_receipt_reads() {
    let mut e = Engine::memory().unwrap();
    history(&mut e);
    rollback(&e, "v2", "restored");
    run(&e, "restored");
    let event = e.poll_adapter_for("restored", &host()).unwrap().unwrap();
    let r = observed(&e, "restored", &event);
    let before = state(&e, "restored");
    let mut p = serde_json::to_value(program(&e, "input", 4)).unwrap();
    p["commands"][0]["data"]["nodes"][0]["readers"] = json!(["other"]);
    e.execute(&serde_json::from_value(p).unwrap(), &host())
        .unwrap();
    assert!(e
        .recorded_actor_delivery_mode_for("restored", &event.id, &event.lease, &host())
        .is_err());
    assert!(e.observe_recorded_actor_for(&r, &host()).is_err());
    assert_eq!(state(&e, "restored"), before);
}
