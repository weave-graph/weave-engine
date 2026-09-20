//! Independent whole-snapshot gate regressions for native authorization.
use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::{Command, GraphData, GraphRef, Program, QueryPlan};
use weave_engine::{Engine, HostContext, ManualClock};
fn host() -> HostContext {
    HostContext::new(
        "root-snapshot-test",
        [
            "Empty".into(),
            "Saved".into(),
            "Cycle".into(),
            "Before".into(),
        ],
    )
}
fn program(commands: Vec<Command>) -> Program {
    Program {
        version: weave_contract::VERSION.into(),
        source_revisions: vec![],
        commands,
    }
}
fn commit(engine: &mut Engine, name: &str, data: Value) -> GraphRef {
    let data: GraphData = serde_json::from_value(data).unwrap();
    engine
        .execute(
            &program(vec![Command::Commit {
                graph_id: name.into(),
                branch_id: "main".into(),
                expected_head: engine.head(name, "main").unwrap(),
                data,
            }]),
            &host(),
        )
        .unwrap();
    GraphRef {
        graph_id: name.into(),
        revision: engine.head(name, "main").unwrap().unwrap(),
    }
}
fn query(engine: &Engine, root: &GraphRef) -> weave_contract::QueryResult {
    let q: QueryPlan = serde_json::from_value(
        json!({"graph_id":root.graph_id,"revision":root.revision,"include_metadata":true}),
    )
    .unwrap();
    engine.query(&q, &host()).unwrap()
}

#[test]
fn root_saved_scalar_and_movable_graph_attachment_keep_empty_source_gate() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.sqlite");
    let clock = Arc::new(ManualClock::new(10));
    let mut engine = Engine::open_with_clock(&db, clock.clone()).unwrap();
    let empty = commit(&mut engine, "Empty", json!({}));
    // No whole-value envelope or readers remain: each movable record must retain its own restriction.
    let saved = commit(
        &mut engine,
        "Saved",
        json!({
            "nodes":[{"id":"count","entity_id":"count","space_id":"result","properties":{"value":0},"derived_snapshots":[empty]}],
            "attachments":[{"id":"summary","host":{"kind":"graph"},"key":"summary","value":{"kind":"literal","value":0},"valid_time":{"start":0},"derived_snapshots":[empty]}]
        }),
    );
    let before = query(&engine, &saved);
    assert_eq!(before.graph.nodes.len(), 1);
    assert_eq!(before.graph.attachments.len(), 1);
    drop(engine);
    let sql = rusqlite::Connection::open(&db).unwrap();
    // Deliberately simulate a missing retained input; this is a temporary fault fixture, not a removal API.
    sql.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    sql.execute("DELETE FROM revisions WHERE graph_id='Empty'", [])
        .unwrap();
    drop(sql);
    let engine = Engine::open_with_clock(&db, clock).unwrap();
    let after = query(&engine, &saved);
    assert!(after.graph.nodes.is_empty());
    assert!(after.graph.attachments.is_empty());
    assert!(engine.export_capsule(&saved, &host()).is_err());
}

#[test]
fn root_ordinary_metadata_cycle_remains_data_but_snapshot_cycle_is_not_evidence() {
    let mut engine = Engine::memory_with_clock(Arc::new(ManualClock::new(10))).unwrap();
    let command:Command=serde_json::from_value(json!({"op":"commit_batch","batch_id":"snapshot-self","commits":[{"graph_id":"Cycle","data":{
        "nodes":[{"id":"secret","entity_id":"secret","space_id":"s"}],
        "influence":{"snapshots":[{"graph_id":"Cycle","revision":"logical:snapshot-self:Cycle"}]}
    }}]})).unwrap();
    engine.execute(&program(vec![command]), &host()).unwrap();
    let root = GraphRef {
        graph_id: "Cycle".into(),
        revision: engine.head("Cycle", "main").unwrap().unwrap(),
    };
    let q: QueryPlan =
        serde_json::from_value(json!({"graph_id":root.graph_id,"revision":root.revision})).unwrap();
    match engine.query(&q, &host()) {
        Err(_) => {}
        Ok(value) => {
            assert!(value.graph.nodes.is_empty());
            assert_ne!(value.coverage, weave_contract::Coverage::Complete);
        }
    }
    assert!(engine.export_capsule(&root, &host()).is_err());
    // An ordinary self metadata pointer carries no circular proof claim.
    let ordinary:Command=serde_json::from_value(json!({"op":"commit_batch","batch_id":"metadata-self","commits":[{"graph_id":"Before","data":{
        "nodes":[{"id":"n","entity_id":"n","space_id":"s","metadata":[{"graph_id":"Before","revision":"logical:metadata-self:Before"}]}]
    }}]})).unwrap();
    engine.execute(&program(vec![ordinary]), &host()).unwrap();
    let ordinary = GraphRef {
        graph_id: "Before".into(),
        revision: engine.head("Before", "main").unwrap().unwrap(),
    };
    assert_eq!(query(&engine, &ordinary).graph.nodes.len(), 1);
}

#[test]
fn root_old_program_cannot_smuggle_snapshot_gates_after_an_earlier_write() {
    let mut engine = Engine::memory_with_clock(Arc::new(ManualClock::new(10))).unwrap();
    let mut p:Program=serde_json::from_value(json!({"version":"0.16.0","commands":[
        {"op":"commit","graph_id":"Before","data":{}},
        {"op":"commit","graph_id":"Saved","data":{"nodes":[{"id":"n","entity_id":"n","space_id":"s","derived_snapshots":[{"graph_id":"missing","revision":"missing"}]}]}}
    ]})).unwrap();
    assert!(engine.execute(&p, &host()).is_err());
    assert!(engine.head("Before", "main").unwrap().is_none());
    assert_eq!(engine.event_count().unwrap(), 0);
    p.commands.pop();
    engine.execute(&p, &host()).unwrap();
    assert!(engine.head("Before", "main").unwrap().is_some());
}

#[test]
fn root_hidden_record_gate_cannot_suppress_unrelated_public_query() {
    let mut engine = Engine::memory_with_clock(Arc::new(ManualClock::new(10))).unwrap();
    let saved = commit(
        &mut engine,
        "Saved",
        json!({
            "nodes":[
                {"id":"public","entity_id":"public","space_id":"s"},
                {"id":"hidden","entity_id":"hidden","space_id":"s","readers":["different-principal"],"derived_snapshots":[{"graph_id":"never-disclosed","revision":"missing"}]}
            ],
            "attachments":[{"id":"hidden-summary","host":{"kind":"graph"},"key":"private","value":{"kind":"literal","value":"secret"},"valid_time":{"start":0},"readers":["different-principal"],"derived_snapshots":[{"graph_id":"never-disclosed","revision":"missing"}]}]
        }),
    );
    let value = query(&engine, &saved);
    assert_eq!(value.graph.nodes.len(), 1);
    assert_eq!(value.graph.nodes[0].id, "public");
    assert!(value.graph.attachments.is_empty());
    assert!(!serde_json::to_string(&value)
        .unwrap()
        .contains("never-disclosed"));
}

#[test]
fn root_detached_attachment_keeps_assertion_and_node_restrictions_and_new_profile() {
    let mut engine = Engine::memory_with_clock(Arc::new(ManualClock::new(10))).unwrap();
    let source = commit(
        &mut engine,
        "Empty",
        json!({
            "nodes":[{"id":"a","entity_id":"a","space_id":"s"},{"id":"b","entity_id":"b","space_id":"s"},
                     {"id":"private","entity_id":"private","space_id":"s","readers":["root-snapshot-test"]}],
            "edges":[{"id":"proof","from":"a","to":"b","predicate":"evidence","valid_time":{"start":0},"polarity":"positive","readers":["root-snapshot-test"]}]
        }),
    );
    let assertion =
        json!({"graph_id":source.graph_id,"revision":source.revision,"assertion_id":"proof"});
    let node = json!({"graph_id":source.graph_id,"revision":source.revision,"node_id":"private"});
    for (index, gates) in [
        json!({"derived_from":[assertion]}),
        json!({"derived_nodes":[node]}),
    ]
    .into_iter()
    .enumerate()
    {
        let mut attachment = json!({"id":"detached","host":{"kind":"graph"},"key":"summary","value":{"kind":"literal","value":0},"valid_time":{"start":0}});
        attachment
            .as_object_mut()
            .unwrap()
            .extend(gates.as_object().unwrap().clone());
        let data = json!({"attachments":[attachment]});
        let mut old: Program = serde_json::from_value(json!({"version":"0.16.0","commands":[
            {"op":"commit","graph_id":"Before","data":{}},
            {"op":"commit","graph_id":"Saved","data":data}
        ]}))
        .unwrap();
        assert_eq!(engine.execute(&old, &host()).unwrap_err().code, "E_VERSION");
        assert!(engine.head("Before", "main").unwrap().is_none());
        old.commands.clear();
        let saved = commit(&mut engine, "Saved", data);
        assert_eq!(
            query(&engine, &saved).graph.attachments.len(),
            1,
            "gate {index}"
        );
        let outsider = HostContext::new("outsider", ["Empty".into(), "Saved".into()]);
        let q: QueryPlan = serde_json::from_value(
            json!({"graph_id":saved.graph_id,"revision":saved.revision,"include_metadata":true}),
        )
        .unwrap();
        assert!(
            engine
                .query(&q, &outsider)
                .unwrap()
                .graph
                .attachments
                .is_empty(),
            "gate {index}"
        );
        let mut capsule = engine.export_capsule(&saved, &host()).unwrap();
        assert_eq!(capsule.format, "weave-capsule-0.3");
        assert!(capsule
            .revisions
            .iter()
            .any(|r| r.graph_id == source.graph_id && r.revision == source.revision));
        let mut receiver = Engine::memory_with_clock(Arc::new(ManualClock::new(10))).unwrap();
        for old_format in ["weave-capsule-0.1", "weave-capsule-0.2"] {
            capsule.format = old_format.into();
            assert_eq!(
                receiver
                    .receive_capsule(&capsule, &host())
                    .unwrap_err()
                    .code,
                "E_VERSION"
            );
            assert_eq!(receiver.event_count().unwrap(), 0);
        }
    }
}
