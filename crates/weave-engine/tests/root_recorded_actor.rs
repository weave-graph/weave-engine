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
        event_schema: VERSION.into(), metadata_depth: 2, artifact,
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
#[test]
fn actual_outputs_artifacts_state_and_private_checkpoint_reopen_and_old_duplicates_never_rewind() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("actor.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let initial = setup(&mut e);
    write(&mut e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    let r = request(&e, &event, &initial, 2);
    assert_eq!(
        e.complete_handler("actor", &event.id, &event.lease, &r.program)
            .unwrap_err()
            .code,
        "E_ACTOR_STATE"
    );
    let first = e.complete_recorded_actor_for(&r, &host()).unwrap();
    assert!(!first.handler.duplicate);
    assert_eq!(first.artifacts[0].result.value, json!({"sample":2}));
    assert_eq!(first.effects.len(), 0);
    assert_eq!(
        first.state_digest,
        hash(&e.recorded_actor_state_for("actor", &host()).unwrap())
    );
    drop(e);
    let mut e = Engine::open_with_clock(&path, clock).unwrap();
    write(&mut e, "input", 3);
    let next = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    let second = e
        .complete_recorded_actor_for(&request(&e, &next, &first.state_digest, 3), &host())
        .unwrap();
    let head = e.head("output", "main").unwrap();
    let state = e.recorded_actor_state_for("actor", &host()).unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    let cp: i64 = sql
        .query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='actor'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut retry = r.clone();
    retry.lease = "renewed-worker".into();
    let old = e.complete_recorded_actor_for(&retry, &host()).unwrap();
    assert!(old.handler.duplicate);
    assert_eq!(old.receipt_id, first.receipt_id);
    assert_eq!(e.head("output", "main").unwrap(), head);
    assert_eq!(e.recorded_actor_state_for("actor", &host()).unwrap(), state);
    assert_eq!(second.state_digest, hash(&state));
    assert_eq!(
        sql.query_row(
            "SELECT checkpoint FROM recorded_actor_states WHERE adapter='actor'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        cp
    );
    assert_eq!(
        sql.query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='actor'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        cp
    );
    let mut changed = r;
    changed.tool_results[0].value = json!(999);
    assert_eq!(
        e.complete_recorded_actor_for(&changed, &host())
            .unwrap_err()
            .code,
        "E_RECEIPT_CONFLICT"
    );
}
#[test]
fn unknown_physical_effect_survives_restart_and_terminal_ledger_is_bound_to_completion() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("actor.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    let initial = setup(&mut e);
    write(&mut e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    let r = request(&e, &event, &initial, 2);
    let effect = e
        .request_effect(
            "actor",
            &event.id,
            &event.lease,
            "sink",
            "action",
            json!({"device":"test","value":2}),
        )
        .unwrap();
    assert_eq!(
        e.complete_recorded_actor_for(&r, &host()).unwrap_err().code,
        "E_ACTOR_EFFECT_PENDING"
    );
    e.begin_effect_dispatch(&effect.id).unwrap();
    drop(e);
    let mut e = Engine::open_with_clock(&path, clock).unwrap();
    assert_eq!(
        e.effect_intent(&effect.id).unwrap().unwrap().state,
        "unknown"
    );
    assert_eq!(
        e.begin_effect_dispatch(&effect.id).unwrap_err().code,
        "E_EFFECT_UNKNOWN"
    );
    assert_eq!(
        e.complete_recorded_actor_for(&r, &host()).unwrap_err().code,
        "E_ACTOR_EFFECT_PENDING"
    );
    assert!(e.head("output", "main").unwrap().is_none());
    assert_eq!(
        hash(&e.recorded_actor_state_for("actor", &host()).unwrap()),
        initial
    );
    e.reconcile_effect(
        &effect.id,
        "confirmed",
        json!({"destination_receipt":"physical-1"}),
    )
    .unwrap();
    let done = e.complete_recorded_actor_for(&r, &host()).unwrap();
    assert_eq!(done.effects[0].id, effect.id);
    assert_eq!(
        done.effects[0].response,
        Some(json!({"destination_receipt":"physical-1"}))
    );
    assert!(
        e.complete_recorded_actor_for(&r, &host())
            .unwrap()
            .handler
            .duplicate
    );
    let sql = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        sql.query_row("SELECT count(*) FROM effect_intents", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn effect_quota_precedes_new_intent_creation_and_does_not_block_an_existing_key_retry() {
    let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(10))).unwrap();
    setup(&mut e);
    write(&mut e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    let mut first = None;
    for i in 0..32 {
        let effect = e
            .request_effect(
                "actor",
                &event.id,
                &event.lease,
                "sink",
                &format!("key-{i}"),
                json!(i),
            )
            .unwrap();
        if i == 0 {
            first = Some(effect);
        }
    }
    assert_eq!(
        e.request_effect(
            "actor",
            &event.id,
            &event.lease,
            "sink",
            "overflow",
            json!(32)
        )
        .unwrap_err()
        .code,
        "E_BUDGET"
    );
    assert_eq!(
        e.request_effect("actor", &event.id, &event.lease, "sink", "key-0", json!(0))
            .unwrap(),
        first.unwrap()
    );
}
#[test]
fn actor_snapshot_pins_whole_metadata_and_rejects_hidden_or_live_dependencies() {
    for kind in ["visible", "hidden", "live"] {
        let mut e = Engine::memory().unwrap();
        let evidence:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"other","data":{"nodes":[{"id":"proof","entity_id":"proof","space_id":"s","readers":if kind=="hidden" {vec!["someone"]} else {vec!["owner"]}}]}}]})).unwrap();
        e.execute(&evidence, &host()).unwrap();
        let reference = GraphRef {
            graph_id: "other".into(),
            revision: e.head("other", "main").unwrap().unwrap(),
        };
        let value = if kind == "live" {
            json!({"kind":"live_graph","graph_id":"other","branch_id":"main"})
        } else {
            json!({"kind":"graph","reference":reference})
        };
        let input:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"input","data":{"attachments":[{"id":"evidence","host":{"kind":"graph"},"key":"evidence","value":value,"readers":["owner"],"valid_time":{"start":0}}]}}]})).unwrap();
        e.execute(&input, &host()).unwrap();
        e.install_recorded_actor_for(&definition(), &host())
            .unwrap();
        if kind == "visible" {
            let inputs = e.recorded_actor_inputs_for("actor", &host()).unwrap();
            assert_eq!(inputs.input_snapshots.len(), 2);
            assert!(inputs.input_snapshots.contains(&reference));
        } else {
            assert!(e.recorded_actor_inputs_for("actor", &host()).is_err());
        }
    }
}
#[test]
fn stale_state_output_and_snapshot_initialization_compare_and_swap_preserve_all_work() {
    let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(10))).unwrap();
    let initial = setup(&mut e);
    write(&mut e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    let r = request(&e, &event, &initial, 2);
    let mut bad = r.clone();
    bad.prior_state_digest = hash(&json!({"forged":true}));
    assert_eq!(
        e.complete_recorded_actor_for(&bad, &host())
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    write(&mut e, "output", 99);
    let head = e.head("output", "main").unwrap();
    assert_eq!(
        e.complete_recorded_actor_for(&r, &host()).unwrap_err().code,
        "E_CONFLICT"
    );
    assert_eq!(
        hash(&e.recorded_actor_state_for("actor", &host()).unwrap()),
        initial
    );
    assert_eq!(e.head("output", "main").unwrap(), head);
    let mut retry = r;
    retry.program = program(&e, "output", 2);
    e.complete_recorded_actor_for(&retry, &host()).unwrap();
    e.set_adapter_state_for("actor", "paused", &host()).unwrap();
    let stale = bootstrap(&e);
    write(&mut e, "input", 3);
    assert_eq!(
        e.bootstrap_recorded_actor_for(&stale, &host())
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    let current = bootstrap(&e);
    e.bootstrap_recorded_actor_for(&current, &host()).unwrap();
    assert_eq!(
        e.bootstrap_recorded_actor_for(&current, &host())
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
}
#[test]
fn host_scope_and_artifact_bytes_and_existing_class_cannot_be_substituted() {
    let mut e = Engine::memory().unwrap();
    write(&mut e, "input", 1);
    let mut d = definition();
    d.artifact[0] ^= 1;
    assert_eq!(
        e.install_recorded_actor_for(&d, &host()).unwrap_err().code,
        "E_ACTOR_DEFINITION"
    );
    d = definition();
    e.install_recorded_actor_for(&d, &host()).unwrap();
    assert_eq!(
        e.recorded_actor_inputs_for("actor", &HostContext::new("outsider", ["output".into()]))
            .unwrap_err()
            .code,
        "E_HOST_AUTH"
    );
    assert_eq!(
        e.recorded_actor_inputs_for("actor", &HostContext::new("owner", []))
            .unwrap_err()
            .code,
        "E_HOST_AUTH"
    );
    e.bootstrap_recorded_actor_for(&bootstrap(&e), &host())
        .unwrap();
    e.set_adapter_state_for("actor", "running", &host())
        .unwrap();
    write(&mut e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    let state = e.recorded_actor_state_for("actor", &host()).unwrap();
    let r = request(&e, &event, &hash(&state), 2);
    e.complete_recorded_actor_for(&r, &host()).unwrap();
    assert_eq!(
        e.complete_recorded_actor_for(&r, &HostContext::new("owner", []))
            .unwrap_err()
            .code,
        "E_HOST_AUTH"
    );
    let mut d2 = definition();
    d2.manifest.id = "legacy".into();
    e.install_adapter(&d2.manifest, &host()).unwrap();
    assert_eq!(
        e.install_recorded_actor_for(&d2, &host()).unwrap_err().code,
        "E_ACTOR_MODE"
    );
}
#[test]
fn private_unsubscribed_scans_change_only_internal_coordinate_and_digest_pair() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("actor.db");
    let mut e = Engine::open_with_clock(&path, Arc::new(ManualClock::new(10))).unwrap();
    setup(&mut e);
    let before = e.recorded_actor_state_for("actor", &host()).unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    let cp: i64 = sql
        .query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='actor'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    write(&mut e, "other", 99);
    assert!(e.poll_adapter_for("actor", &host()).unwrap().is_none());
    assert_eq!(
        e.recorded_actor_state_for("actor", &host()).unwrap(),
        before
    );
    let cp2: i64 = sql
        .query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='actor'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cp2, cp + 1);
    assert_eq!(
        sql.query_row(
            "SELECT checkpoint FROM recorded_actor_states WHERE adapter='actor'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        cp2
    );
    let mut keys = serde_json::to_value(&before).unwrap();
    assert!(keys.as_object_mut().unwrap().remove("checkpoint").is_none());
}
#[test]
fn retention_epoch_requires_explicit_state_reconstruction_and_preserves_recorded_pins() {
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    let initial = setup(&mut e);
    clock.set(20);
    write(&mut e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    e.complete_recorded_actor_for(&request(&e, &event, &initial, 2), &host())
        .unwrap();
    e.set_adapter_state_for("actor", "paused", &host()).unwrap();
    clock.set(30);
    let plan = e
        .plan_retention(&RetentionPolicy {
            history_before_ms: 25,
            replay_through_sequence: 2,
        })
        .unwrap();
    assert!(plan.collect.is_empty());
    e.compact_retention(&plan).unwrap();
    e.set_adapter_state_for("actor", "running", &host())
        .unwrap();
    assert_eq!(
        e.poll_adapter_for("actor", &host()).unwrap_err().code,
        "E_CHECKPOINT_EXPIRED"
    );
    e.set_adapter_state_for("actor", "paused", &host()).unwrap();
    e.bootstrap_recorded_actor_for(&bootstrap(&e), &host())
        .unwrap();
    e.set_adapter_state_for("actor", "running", &host())
        .unwrap();
    assert!(e.poll_adapter_for("actor", &host()).unwrap().is_none());
}
#[test]
fn typed_record_corruption_stops_collection_and_modern_missing_or_downgraded_schema_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("actor.db");
    let mut e = Engine::open_with_clock(&path, Arc::new(ManualClock::new(10))).unwrap();
    let initial = setup(&mut e);
    write(&mut e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    e.complete_recorded_actor_for(&request(&e, &event, &initial, 2), &host())
        .unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    let plan = e.plan_retention(&RetentionPolicy::default()).unwrap();
    let state_row: (String, String, i64) = sql
        .query_row(
            "SELECT body,digest,checkpoint FROM recorded_actor_states",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    sql.execute("DELETE FROM recorded_actor_states", [])
        .unwrap();
    assert_eq!(
        e.compact_retention(&plan).unwrap_err().code,
        "E_ACTOR_INTEGRITY"
    );
    assert_eq!(
        e.poll_adapter_for("actor", &host()).unwrap_err().code,
        "E_ACTOR_INTEGRITY"
    );
    sql.execute(
        "INSERT INTO recorded_actor_states VALUES ('actor',?1,?2,?3)",
        rusqlite::params![state_row.0, state_row.1, state_row.2],
    )
    .unwrap();
    for table in [
        "recorded_actor_definitions",
        "recorded_actor_states",
        "recorded_actor_receipts",
    ] {
        let original: String = sql
            .query_row(&format!("SELECT body FROM {table}"), [], |r| r.get(0))
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&original).unwrap();
        value["extra_unknown_field"] = json!(true);
        sql.execute(&format!("UPDATE {table} SET body=?1"), [value.to_string()])
            .unwrap();
        assert_eq!(
            e.compact_retention(&plan).unwrap_err().code,
            "E_ACTOR_INTEGRITY"
        );
        sql.execute(&format!("UPDATE {table} SET body=?1"), [original])
            .unwrap();
    }
    let original: String = sql
        .query_row("SELECT results FROM handler_receipts", [], |r| r.get(0))
        .unwrap();
    sql.execute("UPDATE handler_receipts SET results='[]'", [])
        .unwrap();
    assert_eq!(
        e.compact_retention(&plan).unwrap_err().code,
        "E_ACTOR_INTEGRITY"
    );
    sql.execute("UPDATE handler_receipts SET results=?1", [original])
        .unwrap();
    drop(e);
    sql.pragma_update(None, "user_version", 25).unwrap();
    assert_eq!(Engine::open(&path).err().unwrap().code, "E_ACTOR_INTEGRITY");
    assert_eq!(
        sql.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        25
    );
    sql.pragma_update(None, "user_version", 26).unwrap();
    sql.execute("DROP TABLE recorded_actor_receipts", [])
        .unwrap();
    assert_eq!(Engine::open(&path).err().unwrap().code, "E_ACTOR_INTEGRITY");
}
#[cfg(feature = "recovery-testing")]
#[test]
fn panic_before_commit_rolls_back_outputs_receipt_state_and_delivery_acknowledgment() {
    let mut e = Engine::memory_with_clock(Arc::new(ManualClock::new(10))).unwrap();
    let initial = setup(&mut e);
    write(&mut e, "input", 2);
    let event = e.poll_adapter_for("actor", &host()).unwrap().unwrap();
    let r = request(&e, &event, &initial, 2);
    let count = e.event_count().unwrap();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || e.complete_recorded_actor_test_before_commit(&r, &host(), || panic!("host interrupted"))
    ))
    .is_err());
    assert_eq!(e.event_count().unwrap(), count);
    assert!(e.head("output", "main").unwrap().is_none());
    assert_eq!(
        hash(&e.recorded_actor_state_for("actor", &host()).unwrap()),
        initial
    );
    assert!(
        !e.complete_recorded_actor_for(&r, &host())
            .unwrap()
            .handler
            .duplicate
    );
}
