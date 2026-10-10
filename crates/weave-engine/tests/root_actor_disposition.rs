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
fn request(
    e: &Engine,
    event: &DispatchEnvelope,
    prior: &str,
    value: i64,
) -> RecordedActorCompletion {
    RecordedActorCompletion {
        adapter: "actor".into(),
        event: event.id.clone(),
        lease: event.lease.clone(),
        prior_state_digest: prior.into(),
        state_revision: format!("state{value}"),
        state: json!({"total":value}),
        input_snapshots: vec![event.graph.clone()],
        tool_results: vec![RecordedToolResult {
            name: "sample".into(),
            media_type: "application/json".into(),
            value: json!({"sample":value}),
        }],
        program: program(e, "output", value),
    }
}
fn cancel(event: &DispatchEnvelope) -> DeliveryCancellationRequest {
    DeliveryCancellationRequest {
        adapter: "actor".into(),
        event: event.id.clone(),
        expected_lease: event.lease.clone(),
        nonce: format!("cancel-{}", event.id),
        reason: DeliveryCancellationReason::OwnerStop,
    }
}
fn rebuild(e: &Engine, value: i64) {
    e.set_adapter_state_for("actor", "paused", &host()).unwrap();
    let request = RecordedActorBootstrap {
        inputs: e.recorded_actor_inputs_for("actor", &host()).unwrap(),
        state_revision: "explicit-new-state".into(),
        state: json!({"total":value}),
    };
    e.bootstrap_recorded_actor_for(&request, &host()).unwrap();
    e.set_adapter_state_for("actor", "running", &host())
        .unwrap();
}
#[test]
fn undispatched_intent_is_terminally_disposed_and_new_work_requires_explicit_initialization() {
    let mut e = Engine::memory().unwrap();
    let initial = setup(&mut e);
    write(&mut e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    let r = request(&e, &event, &initial, 2);
    let intent = e
        .request_effect(
            "actor",
            &event.id,
            &event.lease,
            "sink",
            "not-dispatched",
            json!({"value":2}),
        )
        .unwrap();
    let cancellation = cancel(&event);
    let receipt = e
        .cancel_recorded_actor_delivery_for(&cancellation, &host())
        .unwrap();
    assert!(receipt.rebuild_required && !receipt.duplicate);
    let terminal = e.effect_intent(&intent.id).unwrap().unwrap();
    assert_eq!(terminal.state, "failed");
    assert_eq!(terminal.payload, intent.payload);
    assert_eq!(
        terminal.response,
        Some(json!({"owner_disposition":receipt.receipt_id,"outcome":"not_dispatched"}))
    );
    assert_eq!(
        e.complete_recorded_actor_for(&r, &host()).unwrap_err().code,
        "E_DELIVERY_CANCELED"
    );
    assert_eq!(
        e.complete_handler("actor", &event.id, &event.lease, &r.program)
            .unwrap_err()
            .code,
        "E_DELIVERY_CANCELED"
    );
    assert_eq!(
        e.recorded_actor_state_for("actor", &host())
            .unwrap_err()
            .code,
        "E_CHECKPOINT_EXPIRED"
    );
    e.set_adapter_state_for("actor", "running", &host())
        .unwrap();
    assert_eq!(
        e.poll_adapter_for("actor", &host()).unwrap_err().code,
        "E_CHECKPOINT_EXPIRED"
    );
    assert!(e.begin_effect_dispatch(&intent.id).is_err());
    rebuild(&e, 2);
    write(&mut e, "input", 3);
    let next = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    let prior = hash(&e.recorded_actor_state_for("actor", &host()).unwrap());
    e.complete_recorded_actor_for(&request(&e, &next, &prior, 3), &host())
        .unwrap();
    let current = e.recorded_actor_state_for("actor", &host()).unwrap();
    let head = e.head("output", "main").unwrap();
    assert!(
        e.cancel_recorded_actor_delivery_for(&cancellation, &host())
            .unwrap()
            .duplicate
    );
    assert_eq!(
        e.recorded_actor_state_for("actor", &host()).unwrap(),
        current
    );
    assert_eq!(e.head("output", "main").unwrap(), head);
    assert!(e.begin_effect_dispatch(&intent.id).is_err());
    let plan = e.plan_retention(&RetentionPolicy::default()).unwrap();
    e.compact_retention(&plan).unwrap();
}
#[test]
fn unknown_outcomes_block_disposition_until_actual_terminal_reconciliation() {
    let mut e = Engine::memory().unwrap();
    setup(&mut e);
    write(&mut e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    let intent = e
        .request_effect(
            "actor",
            &event.id,
            &event.lease,
            "sink",
            "unknown",
            json!(2),
        )
        .unwrap();
    e.begin_effect_dispatch(&intent.id).unwrap();
    let before = e.recorded_actor_state_for("actor", &host()).unwrap();
    let cancellation = cancel(&event);
    assert_eq!(
        e.cancel_recorded_actor_delivery_for(&cancellation, &host())
            .unwrap_err()
            .code,
        "E_EFFECT_UNKNOWN"
    );
    assert_eq!(
        e.effect_intent(&intent.id).unwrap().unwrap().state,
        "unknown"
    );
    assert_eq!(
        e.recorded_actor_state_for("actor", &host()).unwrap(),
        before
    );
    let response = json!({"actual_destination_receipt":"physical-2"});
    e.reconcile_effect(&intent.id, "confirmed", response.clone())
        .unwrap();
    e.cancel_recorded_actor_delivery_for(&cancellation, &host())
        .unwrap();
    let terminal = e.effect_intent(&intent.id).unwrap().unwrap();
    assert_eq!(terminal.state, "confirmed");
    assert_eq!(terminal.response, Some(response));
}
#[test]
fn owner_cleanup_survives_source_read_expiry_and_lease_expiry_but_not_foreign_or_stale_cas() {
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    setup(&mut e);
    write(&mut e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    let mut r = cancel(&event);
    let wrong = HostContext::new("other", ["output".into()]);
    assert_eq!(
        e.cancel_recorded_actor_delivery_for(&r, &wrong)
            .unwrap_err()
            .code,
        "E_HOST_AUTH"
    );
    r.expected_lease = "another-worker".into();
    assert_eq!(
        e.cancel_recorded_actor_delivery_for(&r, &host())
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    r.expected_lease = event.lease.clone();
    clock.set(10000);
    let policy = RetentionPolicy {
        history_before_ms: 0,
        replay_through_sequence: i64::try_from(e.event_count().unwrap()).unwrap(),
    };
    let plan = e.plan_retention(&policy).unwrap();
    e.compact_retention(&plan).unwrap();
    assert_eq!(
        e.recorded_actor_state_for("actor", &host())
            .unwrap_err()
            .code,
        "E_CHECKPOINT_EXPIRED"
    );
    let receipt = e.cancel_recorded_actor_delivery_for(&r, &host()).unwrap();
    assert!(receipt.rebuild_required);
}
#[test]
fn actual_terminal_ledger_corruption_and_modern_schema_loss_stop_collection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut e = Engine::open_with_clock(&path, Arc::new(ManualClock::new(10))).unwrap();
    setup(&mut e);
    write(&mut e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    let intent = e
        .request_effect(
            "actor",
            &event.id,
            &event.lease,
            "sink",
            "pending",
            json!(2),
        )
        .unwrap();
    let r = cancel(&event);
    e.cancel_recorded_actor_delivery_for(&r, &host()).unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute(
        "UPDATE effect_intents SET response='null' WHERE id=?1",
        [&intent.id],
    )
    .unwrap();
    assert!(e.plan_retention(&RetentionPolicy::default()).is_err());
    assert!(e.cancel_recorded_actor_delivery_for(&r, &host()).is_err());
    drop(e);
    sql.execute("DROP TABLE recorded_actor_cancellations", [])
        .unwrap();
    assert_eq!(
        Engine::open(&path).err().unwrap().code,
        "E_ACTOR_CANCELLATION_INTEGRITY"
    );
}
#[cfg(feature = "recovery-testing")]
#[test]
fn panic_rolls_back_pending_effect_response_state_pair_receipt_and_rebuild_flag() {
    let mut e = Engine::memory().unwrap();
    setup(&mut e);
    write(&mut e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    let intent = e
        .request_effect(
            "actor",
            &event.id,
            &event.lease,
            "sink",
            "pending",
            json!(2),
        )
        .unwrap();
    let before = e.recorded_actor_state_for("actor", &host()).unwrap();
    let r = cancel(&event);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| e
        .cancel_recorded_actor_delivery_test_before_commit(&r, &host(), || panic!(
            "disposition-death"
        ))))
    .is_err());
    assert_eq!(e.effect_intent(&intent.id).unwrap().unwrap(), intent);
    assert_eq!(
        e.recorded_actor_state_for("actor", &host()).unwrap(),
        before
    );
    e.cancel_recorded_actor_delivery_for(&r, &host()).unwrap();
}
#[test]
fn cleanup_uses_bounded_actual_ledger_witnesses_for_existing_large_broker_payloads() {
    let mut e = Engine::memory().unwrap();
    setup(&mut e);
    write(&mut e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    for i in 0..4 {
        e.request_effect(
            "actor",
            &event.id,
            &event.lease,
            "sink",
            &format!("large-{i}"),
            json!({"payload":"x".repeat(750000)}),
        )
        .unwrap();
    }
    let r = cancel(&event);
    e.cancel_recorded_actor_delivery_for(&r, &host()).unwrap();
    assert!(
        e.cancel_recorded_actor_delivery_for(&r, &host())
            .unwrap()
            .duplicate
    );
    let plan = e.plan_retention(&RetentionPolicy::default()).unwrap();
    e.compact_retention(&plan).unwrap();
}
#[test]
fn historical_cleanup_retry_is_readable_while_a_later_occurrence_has_an_unknown_outcome() {
    let mut e = Engine::memory().unwrap();
    setup(&mut e);
    write(&mut e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    let r = cancel(&event);
    e.cancel_recorded_actor_delivery_for(&r, &host()).unwrap();
    rebuild(&e, 2);
    write(&mut e, "input", 3);
    let next = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    let intent = e
        .request_effect(
            "actor",
            &next.id,
            &next.lease,
            "sink",
            "future-unknown",
            json!(3),
        )
        .unwrap();
    e.begin_effect_dispatch(&intent.id).unwrap();
    assert!(
        e.cancel_recorded_actor_delivery_for(&r, &host())
            .unwrap()
            .duplicate
    );
    assert_eq!(
        e.effect_intent(&intent.id).unwrap().unwrap().state,
        "unknown"
    );
}
