//! Actual kernel recipe, output, immutable receipt and private checkpoint reconstruction.
use serde_json::json;
use std::sync::Arc;
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("owner", ["input".into(), "output".into()])
}
fn template(revision: &str) -> CompiledHandlerTemplate {
    handler_registration::seal_handler_template(serde_json::from_value(json!({"format":"weave-handler-registration/1","protocol":VERSION,"name":"identity","revision":revision,"input":{"graph_id":"input","branch_id":"main","metadata_depth":0},"event_types":["graph.accepted","graph.committed"],"recipe":{"bindings":[],"output":"$event"},"output_slot":"out","source_revisions":[],"definition_digest":""})).unwrap()).unwrap()
}
fn manifest(id: &str, t: &CompiledHandlerTemplate) -> AdapterManifest {
    serde_json::from_value(json!({"id":id,"version":t.revision,"artifact_digest":t.definition_digest,"config_revision":t.revision,"principal":"owner","subscriptions":[{"graph_id":"input","branch_id":"main"}],"output_graphs":["output"],"effect_destinations":[],"max_attempts":3,"lease_ms":1000,"max_pending_events":100,"projection_replay":true})).unwrap()
}
fn binding() -> HandlerOutputBinding {
    HandlerOutputBinding {
        slot: "out".into(),
        graph_id: "output".into(),
        branch_id: "main".into(),
    }
}
fn write(engine: &mut Engine, graph: &str, value: i64) {
    let p:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":graph,"expected_head":engine.head(graph,"main").unwrap(),"data":{"nodes":[{"id":"n","entity_id":"e","space_id":"s","readers":["owner"],"properties":{"value":value}}]}}]})).unwrap();
    engine.execute(&p, &host()).unwrap();
}
fn setup(engine: &mut Engine) {
    write(engine, "input", 1);
    let t = template("1");
    engine
        .install_compiled_handler(&manifest("worker1", &t), &t, &binding(), &host())
        .unwrap();
    engine
        .set_adapter_state_for("worker1", "paused", &host())
        .unwrap();
}
fn expire(engine: &Engine, clock: &ManualClock) {
    clock.set(30);
    let plan = engine
        .plan_retention(&RetentionPolicy {
            history_before_ms: 20,
            replay_through_sequence: 1,
        })
        .unwrap();
    engine.compact_retention(&plan).unwrap();
}
fn request(engine: &Engine, adapter: &str, nonce: &str) -> CompiledRebuildRequest {
    CompiledRebuildRequest {
        inputs: engine
            .compiled_rebuild_inputs_for(adapter, &host())
            .unwrap(),
        nonce: nonce.into(),
    }
}
fn query(engine: &Engine, reference: Option<&GraphRef>) -> QueryResult {
    engine
        .query(
            &serde_json::from_value(
                json!({"graph_id":"output","revision":reference.map(|r|&r.revision)}),
            )
            .unwrap(),
            &host(),
        )
        .unwrap()
}
fn complete(engine: &mut Engine, adapter: &str) {
    let e = engine.poll_adapter_for(adapter, &host()).unwrap().unwrap();
    let p = engine
        .prepare_compiled_handler_for(adapter, &e.id, &e.lease, &host())
        .unwrap();
    engine
        .complete_prepared_handler_for(adapter, &e.id, &e.lease, &p.preparation_id, &host())
        .unwrap();
}
#[test]
fn actual_recipe_restores_expired_replay_and_later_completion_and_version_transfer_keep_it_ready() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite");
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    setup(&mut e);
    expire(&e, &clock);
    e.set_adapter_state_for("worker1", "running", &host())
        .unwrap();
    assert_eq!(
        e.poll_adapter_for("worker1", &host()).unwrap_err().code,
        "E_CHECKPOINT_EXPIRED"
    );
    e.set_adapter_state_for("worker1", "paused", &host())
        .unwrap();
    let r = request(&e, "worker1", "reconstruct");
    let receipt = e.rebuild_compiled_handler_for(&r, &host()).unwrap();
    assert_eq!(receipt.coverage, Coverage::Complete);
    assert_eq!(query(&e, None).graph.nodes[0].properties["value"], json!(1));
    e.set_adapter_state_for("worker1", "running", &host())
        .unwrap();
    assert!(e.poll_adapter_for("worker1", &host()).unwrap().is_none());
    clock.set(40);
    write(&mut e, "input", 2);
    complete(&mut e, "worker1");
    let head = e.head("output", "main").unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    let checkpoint: i64 = sql
        .query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='worker1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        e.rebuild_compiled_handler_for(&r, &host())
            .unwrap()
            .duplicate
    );
    assert_eq!(e.head("output", "main").unwrap(), head);
    assert_eq!(
        query(&e, Some(&receipt.output)).graph.nodes[0].properties["value"],
        json!(1)
    );
    assert_eq!(
        sql.query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='worker1'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        checkpoint
    );
    e.set_adapter_state_for("worker1", "paused", &host())
        .unwrap();
    let t = template("2");
    let upgrade = CompiledMigrationRequest {
        inputs: e.compiled_migration_inputs_for("worker1", &host()).unwrap(),
        destination: manifest("worker2", &t),
        template: t,
        output: binding(),
        nonce: "upgrade".into(),
        disposition: ProjectionMigrationKind::Upgrade,
    };
    e.migrate_compiled_handler_for(&upgrade, &host()).unwrap();
    drop(e);
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    e.set_adapter_state_for("worker2", "running", &host())
        .unwrap();
    clock.set(50);
    write(&mut e, "input", 3);
    complete(&mut e, "worker2");
    assert_eq!(query(&e, None).graph.nodes[0].properties["value"], json!(3));
    let a: i64 = sql
        .query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='worker2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let b: i64 = sql
        .query_row(
            "SELECT checkpoint FROM compiled_replay_states WHERE adapter='worker2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(a, b);
    assert!(
        e.migrate_compiled_handler_for(&upgrade, &host())
            .unwrap()
            .duplicate
    );
    assert!(e
        .plan_retention(&RetentionPolicy {
            history_before_ms: 20,
            replay_through_sequence: 1
        })
        .is_ok());
}
#[test]
fn private_event_coordinates_do_not_change_reconstruction_inputs_or_public_receipt_identity() {
    let dir = tempfile::tempdir().unwrap();
    let seed = dir.path().join("seed.sqlite");
    let other = dir.path().join("other.sqlite");
    let clock = Arc::new(ManualClock::new(10));
    let mut first = Engine::open_with_clock(&seed, clock.clone()).unwrap();
    setup(&mut first);
    expire(&first, &clock);
    drop(first);
    std::fs::copy(&seed, &other).unwrap();
    let mut first = Engine::open_with_clock(&seed, clock.clone()).unwrap();
    let mut second = Engine::open_with_clock(&other, clock.clone()).unwrap();
    clock.set(40);
    let p:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"secret","data":{"nodes":[{"id":"s","entity_id":"s","space_id":"s","readers":["outsider"]}]}}]})).unwrap();
    second
        .execute(&p, &HostContext::new("outsider", ["secret".into()]))
        .unwrap();
    clock.set(50);
    let r = request(&first, "worker1", "same");
    assert_eq!(r, request(&second, "worker1", "same"));
    let a = first.rebuild_compiled_handler_for(&r, &host()).unwrap();
    let b = second.rebuild_compiled_handler_for(&r, &host()).unwrap();
    assert_eq!(a.receipt_id, b.receipt_id);
    assert_eq!(a.definition_digest, b.definition_digest);
    assert_eq!(query(&first, None).graph, query(&second, None).graph);
    let read = |path| {
        rusqlite::Connection::open(path)
            .unwrap()
            .query_row("SELECT checkpoint FROM compiled_replay_states", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
    };
    assert_eq!(read(&other), read(&seed) + 1);
}
#[test]
fn current_input_output_cas_owner_pending_and_nonce_conflicts_cannot_publish() {
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    setup(&mut e);
    expire(&e, &clock);
    let stale = request(&e, "worker1", "cas");
    clock.set(40);
    write(&mut e, "input", 2);
    assert_eq!(
        e.rebuild_compiled_handler_for(&stale, &host())
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    let stale = request(&e, "worker1", "output-cas");
    write(&mut e, "output", 9);
    assert_eq!(
        e.rebuild_compiled_handler_for(&stale, &host())
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    let r = request(&e, "worker1", "current");
    let outsider = HostContext::new("outsider", ["output".into()]);
    assert_eq!(
        e.rebuild_compiled_handler_for(&r, &outsider)
            .unwrap_err()
            .code,
        "E_HOST_AUTH"
    );
    e.rebuild_compiled_handler_for(&r, &host()).unwrap();
    let head = e.head("output", "main").unwrap();
    let mut changed = r.clone();
    changed.inputs.expected_output = None;
    assert_eq!(
        e.rebuild_compiled_handler_for(&changed, &host())
            .unwrap_err()
            .code,
        "E_RECEIPT_CONFLICT"
    );
    assert_eq!(e.head("output", "main").unwrap(), head);
    e.set_adapter_state_for("worker1", "running", &host())
        .unwrap();
    clock.set(50);
    write(&mut e, "input", 3);
    let delivery = e.poll_adapter_for("worker1", &host()).unwrap().unwrap();
    e.set_adapter_state_for("worker1", "paused", &host())
        .unwrap();
    let r = request(&e, "worker1", "pending");
    assert_eq!(
        e.rebuild_compiled_handler_for(&r, &host())
            .unwrap_err()
            .code,
        "E_REBUILD_PENDING"
    );
    assert!(!delivery.lease.is_empty());
    assert_eq!(e.head("output", "main").unwrap(), head);
}
#[test]
fn cancellation_of_actual_state_bound_stale_work_requires_another_real_materialization() {
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
    setup(&mut e);
    expire(&e, &clock);
    e.rebuild_compiled_handler_for(&request(&e, "worker1", "first"), &host())
        .unwrap();
    e.set_adapter_state_for("worker1", "running", &host())
        .unwrap();
    clock.set(40);
    write(&mut e, "input", 2);
    let d = e.poll_adapter_for("worker1", &host()).unwrap().unwrap();
    let prep = e
        .prepare_compiled_handler_for("worker1", &d.id, &d.lease, &host())
        .unwrap();
    write(&mut e, "output", 99);
    assert_eq!(
        e.complete_prepared_handler_for("worker1", &d.id, &d.lease, &prep.preparation_id, &host())
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    let cancel = DeliveryCancellationRequest {
        adapter: "worker1".into(),
        event: d.id,
        expected_lease: d.lease,
        nonce: "cancel".into(),
        reason: DeliveryCancellationReason::StaleOutput,
    };
    assert!(
        e.cancel_handler_delivery_for(&cancel, &host())
            .unwrap()
            .rebuild_required
    );
    assert_eq!(
        e.poll_adapter_for("worker1", &host()).unwrap_err().code,
        "E_CHECKPOINT_EXPIRED"
    );
    assert_eq!(
        query(&e, None).graph.nodes[0].properties["value"],
        json!(99)
    );
    e.set_adapter_state_for("worker1", "paused", &host())
        .unwrap();
    e.rebuild_compiled_handler_for(&request(&e, "worker1", "again"), &host())
        .unwrap();
    assert_eq!(query(&e, None).graph.nodes[0].properties["value"], json!(2));
    e.set_adapter_state_for("worker1", "running", &host())
        .unwrap();
    assert!(e.poll_adapter_for("worker1", &host()).unwrap().is_none());
}
#[cfg(feature = "recovery-testing")]
#[test]
fn interrupted_reconstruction_has_no_output_receipt_or_checkpoint_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite");
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    setup(&mut e);
    expire(&e, &clock);
    let r = request(&e, "worker1", "death");
    let count = e.event_count().unwrap();
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        e.rebuild_compiled_handler_test_before_commit(&r, &host(), || panic!("before commit"))
    }));
    assert!(failed.is_err());
    drop(e);
    let mut e = Engine::open_with_clock(&path, clock).unwrap();
    assert_eq!(e.event_count().unwrap(), count);
    assert!(e.head("output", "main").unwrap().is_none());
    let sql = rusqlite::Connection::open(&path).unwrap();
    for table in ["compiled_replay_states", "compiled_rebuild_receipts"] {
        assert_eq!(
            sql.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        sql.query_row("SELECT checkpoint FROM dispatch_adapters", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(
        !e.rebuild_compiled_handler_for(&r, &host())
            .unwrap()
            .duplicate
    );
}
#[test]
fn corrupted_bound_checkpoint_and_modern_schema_downgrade_fail_before_collection_or_upgrade() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite");
    let clock = Arc::new(ManualClock::new(10));
    let mut e = Engine::open_with_clock(&path, clock.clone()).unwrap();
    setup(&mut e);
    expire(&e, &clock);
    e.rebuild_compiled_handler_for(&request(&e, "worker1", "current"), &host())
        .unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    let generation: i64 = sql
        .query_row("SELECT generation FROM retention_policy", [], |r| r.get(0))
        .unwrap();
    sql.execute(
        "UPDATE compiled_replay_states SET checkpoint=checkpoint+1",
        [],
    )
    .unwrap();
    assert_eq!(
        e.plan_retention(&RetentionPolicy {
            history_before_ms: 20,
            replay_through_sequence: 1
        })
        .unwrap_err()
        .code,
        "E_REBUILD_INTEGRITY"
    );
    assert_eq!(
        sql.query_row("SELECT generation FROM retention_policy", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        generation
    );
    drop(e);
    sql.pragma_update(None, "user_version", 24).unwrap();
    let error = match Engine::open_with_clock(&path, clock) {
        Ok(_) => panic!("downgrade accepted"),
        Err(error) => error,
    };
    assert_eq!(error.code, "E_REBUILD_INTEGRITY");
    assert_eq!(
        sql.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        24
    );
}
