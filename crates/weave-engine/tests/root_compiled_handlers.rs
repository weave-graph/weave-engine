//! Independent compiled-handler boundary tests. APIs target the approved0.18 interface.
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::{
    handler_registration::{seal_handler_template, CompiledHandlerTemplate},
    GraphRef, Program, QueryPlan,
};
use weave_engine::{AdapterManifest, Engine, HandlerOutputBinding, HostContext, ManualClock};
fn host() -> HostContext {
    HostContext::new("root-handler", ["input".into(), "output".into()])
}
fn write(engine: &mut Engine, id: &str, data: Value) -> GraphRef {
    let p:Program=serde_json::from_value(json!({"version":weave_contract::VERSION,"commands":[{"op":"commit","graph_id":id,"expected_head":engine.head(id,"main").unwrap(),"data":data}]})).unwrap();
    engine.execute(&p, &host()).unwrap();
    GraphRef {
        graph_id: id.into(),
        revision: engine.head(id, "main").unwrap().unwrap(),
    }
}
fn template() -> CompiledHandlerTemplate {
    seal_handler_template(serde_json::from_value(json!({
        "format":"weave-handler-registration/1","protocol":weave_contract::VERSION,"name":"Identity","revision":"1",
        "input":{"graph_id":"input","branch_id":"main","metadata_depth":0},
        "event_types":["graph.accepted","graph.committed"],"recipe":{"bindings":[],"output":"$event"},
        "output_slot":"result","source_revisions":[],"definition_digest":""
    })).unwrap()).unwrap()
}
fn install(engine: &Engine) {
    let t = template();
    let manifest:AdapterManifest=serde_json::from_value(json!({
        "id":"root-compiled","version":"1","artifact_digest":t.definition_digest,"config_revision":"1","principal":"root-handler",
        "subscriptions":[{"graph_id":"input","branch_id":"main"}],"output_graphs":["output"],"effect_destinations":[],
        "max_attempts":5,"lease_ms":100,"max_pending_events":10,"projection_replay":true
    })).unwrap();
    let binding = HandlerOutputBinding {
        slot: "result".into(),
        graph_id: "output".into(),
        branch_id: "main".into(),
    };
    engine
        .install_compiled_handler(&manifest, &t, &binding, &host())
        .unwrap();
    engine
        .set_adapter_state("root-compiled", "running")
        .unwrap();
}
fn empty_program() -> Program {
    serde_json::from_value(json!({"version":weave_contract::VERSION,"commands":[]})).unwrap()
}
#[test]
fn root_raw_completion_cannot_bypass_compiled_binding_before_or_after_receipt() {
    let mut engine = Engine::memory_with_clock(Arc::new(ManualClock::new(10))).unwrap();
    write(&mut engine, "input", json!({}));
    install(&engine);
    let d = engine.poll_adapter("root-compiled").unwrap().unwrap();
    assert!(engine
        .complete_handler("root-compiled", &d.id, &d.lease, &empty_program())
        .is_err());
    assert_eq!(engine.event_count().unwrap(), 1);
    assert!(engine.head("output", "main").unwrap().is_none());
    let p = engine
        .prepare_compiled_handler("root-compiled", &d.id, &d.lease)
        .unwrap();
    assert!(
        !engine
            .complete_prepared_handler("root-compiled", &d.id, &d.lease, &p.preparation_id)
            .unwrap()
            .duplicate
    );
    assert_eq!(engine.event_count().unwrap(), 2);
    assert!(engine
        .complete_handler("root-compiled", &d.id, &d.lease, &empty_program())
        .is_err());
    assert!(
        engine
            .complete_prepared_handler("root-compiled", &d.id, &d.lease, &p.preparation_id)
            .unwrap()
            .duplicate
    );
    assert_eq!(engine.event_count().unwrap(), 2);
}
#[test]
fn root_renewed_lease_keeps_preparation_and_stale_cas_cannot_acknowledge() {
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::memory_with_clock(clock.clone()).unwrap();
    write(&mut engine, "input", json!({}));
    install(&engine);
    let first = engine.poll_adapter("root-compiled").unwrap().unwrap();
    let p = engine
        .prepare_compiled_handler("root-compiled", &first.id, &first.lease)
        .unwrap();
    clock.set(111);
    let second = engine.poll_adapter("root-compiled").unwrap().unwrap();
    assert_eq!(first.id, second.id);
    assert_ne!(first.lease, second.lease);
    let again = engine
        .prepare_compiled_handler("root-compiled", &second.id, &second.lease)
        .unwrap();
    assert!(again.duplicate);
    assert_eq!(p.preparation_id, again.preparation_id);
    assert!(engine
        .complete_prepared_handler("root-compiled", &first.id, &first.lease, &p.preparation_id)
        .is_err());
    let concurrent = write(&mut engine, "output", json!({}));
    assert!(engine
        .complete_prepared_handler(
            "root-compiled",
            &second.id,
            &second.lease,
            &p.preparation_id
        )
        .is_err());
    assert_eq!(
        engine.head("output", "main").unwrap().unwrap(),
        concurrent.revision
    );
    assert_eq!(engine.event_count().unwrap(), 2);
    clock.set(212);
    let third = engine.poll_adapter("root-compiled").unwrap().unwrap();
    assert_eq!(third.id, first.id);
    assert_eq!(
        engine
            .prepare_compiled_handler("root-compiled", &third.id, &third.lease)
            .unwrap()
            .preparation_id,
        p.preparation_id
    );
    assert!(engine
        .complete_prepared_handler("root-compiled", &third.id, &third.lease, &p.preparation_id)
        .is_err());
    assert_eq!(engine.event_count().unwrap(), 2);
}
#[test]
fn root_empty_input_gate_survives_completion_and_missing_source_denies_replay() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("handler.sqlite");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&db, clock.clone()).unwrap();
    let input = write(&mut engine, "input", json!({}));
    install(&engine);
    let d = engine.poll_adapter("root-compiled").unwrap().unwrap();
    let p = engine
        .prepare_compiled_handler("root-compiled", &d.id, &d.lease)
        .unwrap();
    engine
        .complete_prepared_handler("root-compiled", &d.id, &d.lease, &p.preparation_id)
        .unwrap();
    let output = engine.head("output", "main").unwrap().unwrap();
    let q: QueryPlan = serde_json::from_value(
        json!({"graph_id":"output","revision":output,"include_metadata":true}),
    )
    .unwrap();
    let value = engine.query(&q, &host()).unwrap();
    assert!(value
        .graph
        .influence
        .as_ref()
        .unwrap()
        .snapshots
        .contains(&input));
    assert!(!value.graph.attachments.is_empty());
    assert!(value
        .graph
        .attachments
        .iter()
        .all(|a| a.derived_snapshots.contains(&input)));
    drop(engine);
    // Deliberate unavailable-block fault, not a supported data-removal API.
    let sql = rusqlite::Connection::open(&db).unwrap();
    sql.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    sql.execute("DELETE FROM revisions WHERE graph_id='input'", [])
        .unwrap();
    drop(sql);
    let mut engine = Engine::open_with_clock(&db, clock).unwrap();
    assert!(engine
        .query(&q, &host())
        .unwrap()
        .graph
        .attachments
        .is_empty());
    assert!(engine
        .complete_prepared_handler("root-compiled", &d.id, &d.lease, &p.preparation_id)
        .is_err());
    assert_eq!(engine.event_count().unwrap(), 2);
}
