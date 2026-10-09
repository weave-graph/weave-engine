//! Actual sealed registrations, private checkpoints and immutable output lineage.
use serde_json::json;
use std::sync::Arc;
use weave_contract::*;
use weave_engine::*;
fn owner() -> HostContext {
    HostContext::new("owner", ["input".into(), "output".into()])
}
fn template(revision: &str) -> CompiledHandlerTemplate {
    handler_registration::seal_handler_template(serde_json::from_value(json!({"format":"weave-handler-registration/1","protocol":VERSION,"name":"identity","revision":revision,"input":{"graph_id":"input","branch_id":"main","metadata_depth":0},"event_types":["graph.accepted","graph.committed"],"recipe":{"bindings":[],"output":"$event"},"output_slot":"out","source_revisions":[],"definition_digest":""})).unwrap()).unwrap()
}
fn manifest(id: &str, revision: &str, t: &CompiledHandlerTemplate) -> AdapterManifest {
    AdapterManifest {
        id: id.into(),
        version: revision.into(),
        artifact_digest: t.definition_digest.clone(),
        config_revision: revision.into(),
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
fn output() -> HandlerOutputBinding {
    HandlerOutputBinding {
        slot: "out".into(),
        graph_id: "output".into(),
        branch_id: "main".into(),
    }
}
fn write(engine: &mut Engine, value: i64) {
    let p:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"input","expected_head":engine.head("input","main").unwrap(),"data":{"nodes":[{"id":"n","entity_id":"e","space_id":"s","properties":{"value":value},"readers":["owner"]}]}}]})).unwrap();
    engine.execute(&p, &owner()).unwrap();
}
fn setup(engine: &mut Engine, complete: bool) {
    write(engine, 1);
    let t = template("1");
    engine
        .install_compiled_handler(&manifest("worker1", "1", &t), &t, &output(), &owner())
        .unwrap();
    engine
        .set_adapter_state_for("worker1", "running", &owner())
        .unwrap();
    if complete {
        finish(engine, "worker1");
    }
}
fn finish(engine: &mut Engine, adapter: &str) {
    let delivery = engine.poll_adapter_for(adapter, &owner()).unwrap().unwrap();
    let prep = engine
        .prepare_compiled_handler_for(adapter, &delivery.id, &delivery.lease, &owner())
        .unwrap();
    engine
        .complete_prepared_handler_for(
            adapter,
            &delivery.id,
            &delivery.lease,
            &prep.preparation_id,
            &owner(),
        )
        .unwrap();
}
fn request(engine: &Engine) -> CompiledMigrationRequest {
    let t = template("2");
    CompiledMigrationRequest {
        inputs: engine
            .compiled_migration_inputs_for("worker1", &owner())
            .unwrap(),
        destination: manifest("worker2", "2", &t),
        template: t,
        output: output(),
        nonce: "upgrade1".into(),
        disposition: ProjectionMigrationKind::Upgrade,
    }
}
fn value(engine: &Engine, revision: Option<String>) -> QueryResult {
    engine
        .query(
            &serde_json::from_value(json!({"graph_id":"output","revision":revision})).unwrap(),
            &owner(),
        )
        .unwrap()
}
#[test]
fn actual_completion_after_upgrade_and_rollback_preserves_old_outputs_and_never_rewinds_duplicates()
{
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    setup(&mut engine, true);
    let old_head = engine.head("output", "main").unwrap();
    let old_value = value(&engine, old_head.clone());
    let sql = rusqlite::Connection::open(&path).unwrap();
    let old_preparation: String = sql
        .query_row(
            "SELECT body FROM handler_preparations WHERE adapter='worker1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    clock.set(20);
    write(&mut engine, 2);
    engine
        .set_adapter_state_for("worker1", "paused", &owner())
        .unwrap();
    let request = request(&engine);
    let events = engine.event_count().unwrap();
    engine
        .migrate_compiled_handler_for(&request, &owner())
        .unwrap();
    assert_eq!(engine.event_count().unwrap(), events);
    assert_eq!(engine.head("output", "main").unwrap(), old_head);
    assert_eq!(
        sql.query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='worker2'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        engine
            .set_adapter_state_for("worker1", "running", &owner())
            .unwrap_err()
            .code,
        "E_LIFECYCLE"
    );
    drop(engine);
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    engine
        .set_adapter_state_for("worker2", "running", &owner())
        .unwrap();
    finish(&mut engine, "worker2");
    let new_head = engine.head("output", "main").unwrap();
    let new_value = value(&engine, new_head.clone());
    assert_eq!(new_value.graph.nodes[0].properties["value"], json!(2));
    assert_ne!(old_value.graph.nodes[0].id, new_value.graph.nodes[0].id);
    assert_eq!(value(&engine, old_head.clone()), old_value);
    let checkpoint: i64 = sql
        .query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='worker2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        engine
            .migrate_compiled_handler_for(&request, &owner())
            .unwrap()
            .duplicate
    );
    assert_eq!(
        sql.query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='worker2'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        checkpoint
    );
    assert_eq!(engine.head("output", "main").unwrap(), new_head);
    engine
        .set_adapter_state_for("worker2", "paused", &owner())
        .unwrap();
    let original = template("1");
    let rollback = CompiledMigrationRequest {
        inputs: engine
            .compiled_migration_inputs_for("worker2", &owner())
            .unwrap(),
        destination: manifest("worker3", "1", &original),
        template: original,
        output: output(),
        nonce: "rollback1".into(),
        disposition: ProjectionMigrationKind::Rollback {
            restore_from: "worker1".into(),
        },
    };
    engine
        .migrate_compiled_handler_for(&rollback, &owner())
        .unwrap();
    assert_eq!(engine.head("output", "main").unwrap(), new_head);
    assert_eq!(
        sql.query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='worker3'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    engine
        .set_adapter_state_for("worker3", "running", &owner())
        .unwrap();
    finish(&mut engine, "worker3");
    let final_head = engine.head("output", "main").unwrap();
    assert!(
        engine
            .migrate_compiled_handler_for(&rollback, &owner())
            .unwrap()
            .duplicate
    );
    assert_eq!(engine.head("output", "main").unwrap(), final_head);
    assert_eq!(
        sql.query_row(
            "SELECT body FROM handler_preparations WHERE adapter='worker1'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        old_preparation
    );
    let mut changed = request.clone();
    changed.nonce = "other".into();
    assert_eq!(
        engine
            .migrate_compiled_handler_for(&changed, &owner())
            .unwrap_err()
            .code,
        "E_RECEIPT_CONFLICT"
    );
}
#[test]
fn real_pending_compiled_work_requires_explicit_owner_cleanup_and_incompatible_scopes_cannot_install(
) {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock).unwrap();
    setup(&mut engine, false);
    let delivery = engine
        .poll_adapter_for("worker1", &owner())
        .unwrap()
        .unwrap();
    engine
        .set_adapter_state_for("worker1", "paused", &owner())
        .unwrap();
    let request = request(&engine);
    assert_eq!(
        engine
            .migrate_compiled_handler_for(&request, &owner())
            .unwrap_err()
            .code,
        "E_MIGRATION_PENDING"
    );
    let outsider = HostContext::new("outsider", ["output".into()]);
    assert_eq!(
        engine
            .migrate_compiled_handler_for(&request, &outsider)
            .unwrap_err()
            .code,
        "E_HOST_AUTH"
    );
    engine
        .cancel_handler_delivery_for(
            &DeliveryCancellationRequest {
                adapter: "worker1".into(),
                event: delivery.id,
                expected_lease: delivery.lease,
                nonce: "cleanup".into(),
                reason: DeliveryCancellationReason::StaleOutput,
            },
            &owner(),
        )
        .unwrap();
    for mode in ["scope", "output", "artifact"] {
        let mut changed = request.clone();
        match mode {
            "scope" => changed.template.input.metadata_depth = 1,
            "output" => changed.output.graph_id = "different".into(),
            _ => changed.destination.artifact_digest = format!("sha256:{}", "0".repeat(64)),
        };
        assert!(engine
            .migrate_compiled_handler_for(&changed, &owner())
            .is_err());
    }
    engine
        .migrate_compiled_handler_for(&request, &owner())
        .unwrap();
    engine
        .set_adapter_state_for("worker2", "running", &owner())
        .unwrap();
    assert!(engine
        .poll_adapter_for("worker2", &owner())
        .unwrap()
        .is_none());
}
#[test]
fn unrelated_private_scans_leave_compiled_transfer_inputs_unchanged_and_transfer_the_actual_checkpoint(
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    setup(&mut engine, true);
    let before = engine
        .compiled_migration_inputs_for("worker1", &owner())
        .unwrap();
    clock.set(20);
    let p:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"private","data":{"nodes":[{"id":"n","entity_id":"secret","space_id":"s","readers":["outsider"]}]}}]})).unwrap();
    engine
        .execute(&p, &HostContext::new("outsider", ["private".into()]))
        .unwrap();
    assert!(engine
        .poll_adapter_for("worker1", &owner())
        .unwrap()
        .is_none());
    assert_eq!(
        engine
            .compiled_migration_inputs_for("worker1", &owner())
            .unwrap(),
        before
    );
    engine
        .set_adapter_state_for("worker1", "paused", &owner())
        .unwrap();
    engine
        .migrate_compiled_handler_for(&request(&engine), &owner())
        .unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        sql.query_row(
            "SELECT checkpoint FROM dispatch_adapters WHERE id='worker2'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        3
    );
}
#[cfg(feature = "recovery-testing")]
#[test]
fn interruption_preserves_registry_checkpoint_output_and_receipt_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    setup(&mut engine, true);
    engine
        .set_adapter_state_for("worker1", "paused", &owner())
        .unwrap();
    let request = request(&engine);
    let head = engine.head("output", "main").unwrap();
    let count = engine.event_count().unwrap();
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        engine.migrate_compiled_handler_test_before_commit(&request, &owner(), || {
            panic!("migration boundary")
        })
    }));
    assert!(failed.is_err());
    let sql = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        sql.query_row("SELECT count(*) FROM compiled_handlers", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM compiled_migrations", [], |r| r
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
    assert_eq!(engine.head("output", "main").unwrap(), head);
    assert_eq!(engine.event_count().unwrap(), count);
    engine
        .migrate_compiled_handler_for(&request, &owner())
        .unwrap();
    assert!(
        engine
            .migrate_compiled_handler_for(&request, &owner())
            .unwrap()
            .duplicate
    );
}
#[test]
fn valid_json_record_corruption_prevents_collection_and_modern_schema_cannot_be_downgraded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    setup(&mut engine, true);
    engine
        .set_adapter_state_for("worker1", "paused", &owner())
        .unwrap();
    engine
        .migrate_compiled_handler_for(&request(&engine), &owner())
        .unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute(
        "UPDATE compiled_migrations SET body=json_set(body,'$.after_checkpoint',99)",
        [],
    )
    .unwrap();
    clock.set(30);
    assert_eq!(
        engine
            .plan_retention(&RetentionPolicy {
                history_before_ms: 20,
                replay_through_sequence: 2
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
    drop(engine);
    sql.pragma_update(None, "user_version", 23).unwrap();
    let error = match Engine::open_with_clock(&path, clock) {
        Ok(_) => panic!("downgrade accepted"),
        Err(error) => error,
    };
    assert_eq!(error.code, "E_LIFECYCLE_INTEGRITY");
    assert_eq!(
        sql.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        23
    );
}
#[test]
fn expired_replay_cannot_be_relabelled_as_a_compatible_stateless_checkpoint() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    setup(&mut engine, true);
    engine
        .set_adapter_state_for("worker1", "paused", &owner())
        .unwrap();
    let request = request(&engine);
    clock.set(30);
    let plan = engine
        .plan_retention(&RetentionPolicy {
            history_before_ms: 20,
            replay_through_sequence: 2,
        })
        .unwrap();
    engine.compact_retention(&plan).unwrap();
    assert_eq!(
        engine
            .migrate_compiled_handler_for(&request, &owner())
            .unwrap_err()
            .code,
        "E_CHECKPOINT_EXPIRED"
    );
}

#[test]
fn current_input_cas_and_whole_metadata_authority_are_required_before_version_transfer() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    setup(&mut engine, true);
    engine
        .set_adapter_state_for("worker1", "paused", &owner())
        .unwrap();
    let stale = request(&engine);
    clock.set(20);
    write(&mut engine, 2);
    assert_eq!(
        engine
            .migrate_compiled_handler_for(&stale, &owner())
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    assert!(engine
        .set_adapter_state_for("worker2", "running", &owner())
        .is_err());
    for mode in ["private", "live", "truncated"] {
        let mut e = Engine::memory_with_clock(clock.clone()).unwrap();
        let input_host = HostContext::new("owner", ["input".into(), "meta".into(), "leaf".into()]);
        let leaf: Program = serde_json::from_value(
            json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"leaf","data":{}}]}),
        )
        .unwrap();
        e.execute(&leaf, &input_host).unwrap();
        let leaf = GraphRef {
            graph_id: "leaf".into(),
            revision: e.head("leaf", "main").unwrap().unwrap(),
        };
        let attachment = |value| json!({"id":"detail","host":{"kind":"graph"},"key":"detail","value":value,"valid_time":{"start":0}});
        let data = if mode == "private" {
            json!({"nodes":[{"id":"secret","entity_id":"secret","space_id":"s","readers":["outsider"]}]})
        } else {
            json!({"attachments":[attachment(json!({"kind":"graph","reference":leaf}))]})
        };
        let meta: Program = serde_json::from_value(
            json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"meta","data":data}]}),
        )
        .unwrap();
        e.execute(&meta, &input_host).unwrap();
        let meta = GraphRef {
            graph_id: "meta".into(),
            revision: e.head("meta", "main").unwrap().unwrap(),
        };
        let value = if mode == "live" {
            json!({"kind":"live_graph","graph_id":"meta","branch_id":"main"})
        } else {
            json!({"kind":"graph","reference":meta})
        };
        let input: Program = serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit","graph_id":"input","data":{"attachments":[attachment(value)]}}]})).unwrap();
        e.execute(&input, &input_host).unwrap();
        let mut t = template("1");
        t.input.metadata_depth = 1;
        t.source_revisions.clear();
        t = handler_registration::seal_handler_template(t).unwrap();
        e.install_compiled_handler(&manifest("worker1", "1", &t), &t, &output(), &owner())
            .unwrap();
        let events = e.event_count().unwrap();
        assert!(
            e.compiled_migration_inputs_for("worker1", &owner())
                .is_err(),
            "{mode}"
        );
        assert_eq!(e.event_count().unwrap(), events);
        assert!(e.head("output", "main").unwrap().is_none());
    }
}

#[test]
fn collection_checks_migration_against_actual_registrations_before_erasing_payload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    setup(&mut engine, true);
    engine
        .set_adapter_state_for("worker1", "paused", &owner())
        .unwrap();
    engine
        .migrate_compiled_handler_for(&request(&engine), &owner())
        .unwrap();
    clock.set(30);
    let policy = RetentionPolicy {
        history_before_ms: 20,
        replay_through_sequence: 2,
    };
    engine.plan_retention(&policy).unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute("UPDATE compiled_handlers SET registration=json_set(registration,'$.manifest.config_revision','changed') WHERE adapter='worker2'", []).unwrap();
    assert_eq!(
        engine.plan_retention(&policy).unwrap_err().code,
        "E_LIFECYCLE_INTEGRITY"
    );
    assert_eq!(
        sql.query_row("SELECT generation FROM retention_policy", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM retention_tombstones", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn missing_current_migration_registry_cannot_be_recreated_as_empty_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&path, clock.clone()).unwrap();
    setup(&mut engine, true);
    drop(engine);
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute("DROP TABLE compiled_migrations", []).unwrap();
    let count: i64 = sql
        .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
        .unwrap();
    let error = match Engine::open_with_clock(&path, clock) {
        Ok(_) => panic!("missing registry accepted"),
        Err(error) => error,
    };
    assert_eq!(error.code, "E_LIFECYCLE_INTEGRITY");
    assert_eq!(
        sql.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        24
    );
    assert_eq!(
        sql.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='compiled_migrations'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        count
    );
}
