use serde_json::{json, Value};
use std::sync::Arc;
use weave_contract::*;
use weave_engine::{Engine, HostContext, ManualClock};
fn host(p: &str) -> HostContext {
    HostContext::new(
        p,
        [
            "Source".into(),
            "Saved".into(),
            "A".into(),
            "B".into(),
            "C".into(),
        ],
    )
}
fn engine() -> Engine {
    Engine::memory_with_clock(Arc::new(ManualClock::new(10))).unwrap()
}
fn write(e: &mut Engine, id: &str, value: Value) -> GraphRef {
    let p = Program {
        version: VERSION.into(),
        source_revisions: vec![],
        commands: vec![Command::Commit {
            graph_id: id.into(),
            branch_id: "main".into(),
            expected_head: e.head(id, "main").unwrap(),
            data: serde_json::from_value(value).unwrap(),
        }],
    };
    e.execute(&p, &host("alice")).unwrap();
    GraphRef {
        graph_id: id.into(),
        revision: e.head(id, "main").unwrap().unwrap(),
    }
}
fn read(e: &Engine, r: &GraphRef, p: &str) -> QueryResult {
    e.query(
        &serde_json::from_value(json!({"graph_id":r.graph_id,"revision":r.revision})).unwrap(),
        &host(p),
    )
    .unwrap()
}
#[test]
fn whole_snapshot_is_stricter_than_partial_query_and_preserves_old_pins() {
    let mut e = engine();
    let source = write(
        &mut e,
        "Source",
        json!({"nodes":[{"id":"public","entity_id":"public","space_id":"s"},{"id":"private","entity_id":"private","space_id":"s","readers":["alice"]}]}),
    );
    let saved = write(
        &mut e,
        "Saved",
        json!({"nodes":[{"id":"result","entity_id":"r","space_id":"s","derived_snapshots":[source]}]}),
    );
    assert_eq!(read(&e, &source, "bob").graph.nodes.len(), 1);
    assert!(read(&e, &saved, "bob").graph.nodes.is_empty());
    assert_eq!(read(&e, &saved, "alice").graph.nodes.len(), 1);
    write(&mut e, "Source", json!({}));
    assert!(read(&e, &saved, "bob").graph.nodes.is_empty());
    assert_eq!(read(&e, &saved, "alice").graph.nodes.len(), 1);
}
#[test]
fn mixed_snapshot_node_cycles_fail_but_shared_diamond_is_not_a_cycle() {
    let mut e = engine();
    let c = write(&mut e, "C", json!({}));
    let a = write(&mut e, "A", json!({"influence":{"snapshots":[c]}}));
    let b = write(&mut e, "B", json!({"influence":{"snapshots":[c]}}));
    let saved = write(
        &mut e,
        "Saved",
        json!({"nodes":[{"id":"result","entity_id":"r","space_id":"s","derived_snapshots":[a,b]}]}),
    );
    assert_eq!(read(&e, &saved, "alice").graph.nodes.len(), 1);
    let p:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit_batch","batch_id":"mixed","commits":[{"graph_id":"A","expected_head":a.revision,"data":{"influence":{"snapshots":[{"graph_id":"B","revision":"logical:mixed:B"}]}}},{"graph_id":"B","expected_head":b.revision,"data":{"nodes":[{"id":"n","entity_id":"n","space_id":"s","derived_nodes":[{"graph_id":"A","revision":"logical:mixed:A","node_id":"missing"}]}]}}]}]})).unwrap();
    e.execute(&p, &host("alice")).unwrap();
    let r = GraphRef {
        graph_id: "A".into(),
        revision: "logical:mixed:A".into(),
    };
    let value = read(&e, &r, "alice");
    assert_eq!(value.coverage, Coverage::Partial);
    assert_eq!(value.graph, GraphData::default());
    assert_eq!(read(&e, &saved, "alice").graph.nodes.len(), 1);
}
#[test]
fn capsule_feature_profile_and_old_literal_property_names_are_unambiguous() {
    let mut e = engine();
    let source = write(&mut e, "Source", json!({}));
    let old = e.export_capsule(&source, &host("alice")).unwrap();
    assert_eq!(old.format, "weave-capsule-0.1");
    let saved = write(&mut e, "Saved", json!({"influence":{"snapshots":[source]}}));
    let mut cap = e.export_capsule(&saved, &host("alice")).unwrap();
    assert_eq!(cap.format, "weave-capsule-0.3");
    assert!(cap.revisions.iter().any(|r| r.graph_id == "Source"));
    let mut peer = engine();
    peer.receive_capsule(&cap, &host("alice")).unwrap();
    for format in ["weave-capsule-0.1", "weave-capsule-0.2"] {
        cap.format = format.into();
        assert_eq!(
            peer.receive_capsule(&cap, &host("alice")).unwrap_err().code,
            "E_VERSION"
        );
    }
    let p:Program=serde_json::from_value(json!({"version":"0.16.0","commands":[{"op":"commit","graph_id":"C","data":{"nodes":[{"id":"n","entity_id":"n","space_id":"s","properties":{"derived_snapshots":["inert"],"snapshots":"inert"}}]}}]})).unwrap();
    e.execute(&p, &host("alice")).unwrap();
}
#[test]
fn materialized_copy_keeps_declared_scalar_snapshot_gate() {
    let mut e = engine();
    let source = write(
        &mut e,
        "Source",
        json!({"nodes":[{"id":"private","entity_id":"private","space_id":"s","readers":["alice"]}]}),
    );
    let a = write(
        &mut e,
        "A",
        json!({"nodes":[{"id":"a","entity_id":"a","space_id":"s","derived_snapshots":[source]}]}),
    );
    let input = read(&e, &a, "alice");
    assert!(input.input_snapshots.contains(&source));
    assert_eq!(
        input.graph.influence.as_ref().unwrap().snapshots,
        vec![source.clone()]
    );
    let data = serde_json::to_value(&input.graph).unwrap();
    let saved = write(&mut e, "Saved", data);
    assert!(read(&e, &saved, "bob").graph.nodes.is_empty());
}

#[test]
fn broad_ungated_records_do_not_consume_recursive_proof_work() {
    let mut e = engine();
    let nodes: Vec<_> = (0..5000)
        .map(|n| json!({"id":format!("n{n}"),"entity_id":"e","space_id":"s"}))
        .collect();
    let source = write(&mut e, "Source", json!({"nodes":nodes}));
    let value = read(&e, &source, "alice");
    assert_eq!(value.graph.nodes.len(), 5000);
    assert_eq!(value.coverage, Coverage::Complete);
}

#[test]
fn legacy_and_explicit_assertion_snapshot_gates_survive_public_endpoints() {
    for explicit in [false, true] {
        let mut e = engine();
        let source = write(
            &mut e,
            "Source",
            json!({"nodes":[{"id":"private","entity_id":"private","space_id":"s","readers":["alice"]}]}),
        );
        let nodes = json!([{"id":"a","entity_id":"a","space_id":"s"},{"id":"b","entity_id":"b","space_id":"s"}]);
        let data = if explicit {
            json!({"profile":"explicit","nodes":nodes,"structural_edges":[{"id":"shape","predicate":"p","from":"a","to":"b"}],"assertions":[{"id":"claim","edge_id":"shape","source":"label","valid_time":{"start":0},"derived_snapshots":[source]}]})
        } else {
            json!({"nodes":nodes,"edges":[{"id":"claim","predicate":"p","from":"a","to":"b","valid_time":{"start":0},"derived_snapshots":[source]}]})
        };
        let saved = write(&mut e, "Saved", data);
        let denied = read(&e, &saved, "bob");
        assert_eq!(denied.graph.nodes.len(), 2);
        assert!(denied.graph.edges.is_empty());
        assert_eq!(denied.coverage, Coverage::Partial);
        let allowed = read(&e, &saved, "alice");
        assert_eq!(allowed.graph.edges.len(), 1);
        assert_eq!(allowed.graph.edges[0].derived_snapshots, vec![source]);
    }
}
