use serde_json::{json, Value};
use weave_contract::{
    Command, CommandResult, GraphData, GraphRef, Program, QueryPlan, QueryResult, VERSION,
};
use weave_engine::{Engine, HostContext};

fn node(id: &str) -> Value {
    json!({"id":id,"entity_id":id,"space_id":"s"})
}
fn commit(e: &mut Engine, g: &str, data: GraphData) -> String {
    let result = e
        .execute(
            &Program {
                version: VERSION.into(),
                source_revisions: vec![],
                commands: vec![Command::Commit {
                    graph_id: g.into(),
                    branch_id: "main".into(),
                    expected_head: None,
                    data,
                }],
            },
            &HostContext::new("alice", [g.into()]),
        )
        .unwrap();
    match &result[0] {
        CommandResult::Committed { revision, .. } => revision.clone(),
        _ => panic!(),
    }
}
fn read(e: &Engine, g: &str, actor: &str) -> QueryResult {
    let q: QueryPlan = serde_json::from_value(json!({"graph_id":g})).unwrap();
    e.query(&q, &HostContext::new(actor, [])).unwrap()
}
fn graph(n: Value) -> GraphData {
    serde_json::from_value(json!({"nodes":[n],"edges":[]})).unwrap()
}

#[test]
fn isolated_node_copies_and_scalar_influence_remain_private_after_repersistence_and_capsules() {
    let mut e = Engine::memory().unwrap();
    let mut secret = node("private");
    secret["readers"] = json!(["alice"]);
    secret["properties"] = json!({"amount":913});
    let source = commit(&mut e, "source", graph(secret));
    let mut copy = read(&e, "source", "alice").graph;
    assert_eq!(copy.nodes[0].derived_nodes.len(), 1);
    assert_eq!(copy.nodes[0].derived_nodes[0].revision, source);
    copy.nodes[0].readers.clear();
    let revision = commit(&mut e, "copy", copy);
    assert_eq!(read(&e, "copy", "alice").graph.nodes.len(), 1);
    assert!(read(&e, "copy", "bob").graph.nodes.is_empty());
    let reference = GraphRef {
        graph_id: "copy".into(),
        revision,
    };
    assert_eq!(
        e.export_capsule(&reference, &HostContext::new("bob", []))
            .unwrap_err()
            .code,
        "E_UNAVAILABLE"
    );
    let capsule = e
        .export_capsule(&reference, &HostContext::new("alice", []))
        .unwrap();
    let mut restored = Engine::memory().unwrap();
    restored
        .receive_capsule(
            &capsule,
            &HostContext::new("alice", ["source".into(), "copy".into()]),
        )
        .unwrap();
    let q =
        serde_json::from_value(json!({"graph_id":"copy","revision":reference.revision})).unwrap();
    assert!(restored
        .query(&q, &HostContext::new("bob", []))
        .unwrap()
        .graph
        .nodes
        .is_empty());
    let mut count = node("count");
    count["properties"] = json!({"count":1});
    count["derived_nodes"] = json!([{"graph_id":"source","revision":source,"node_id":"private"}]);
    commit(&mut e, "aggregate", graph(count));
    assert_eq!(
        read(&e, "aggregate", "alice").graph.nodes[0].properties["count"],
        1
    );
    assert!(read(&e, "aggregate", "bob").graph.nodes.is_empty());
}

#[test]
fn edge_premises_cannot_proxy_a_restricted_node_dependency() {
    let mut e = Engine::memory().unwrap();
    let mut secret = node("n");
    secret["readers"] = json!(["alice"]);
    let r = commit(&mut e, "source", graph(secret));
    let mut proxy = node("proxy");
    proxy["derived_nodes"] = json!([{"graph_id":"source","revision":r,"node_id":"n"}]);
    let proxy=serde_json::from_value(json!({"nodes":[proxy],"edges":[{"id":"witness","predicate":"derived","from":"proxy","to":"proxy","valid_time":{"start":0}}]})).unwrap();
    let r = commit(&mut e, "proxy", proxy);
    let mut scalar = node("value");
    scalar["derived_from"] = json!([{"graph_id":"proxy","revision":r,"assertion_id":"witness"}]);
    commit(&mut e, "scalar", graph(scalar));
    assert_eq!(read(&e, "scalar", "alice").graph.nodes.len(), 1);
    assert!(read(&e, "scalar", "bob").graph.nodes.is_empty());
}

#[test]
fn direct_structural_lookup_enforces_source_node_influences() {
    let mut e = Engine::memory().unwrap();
    let mut secret = node("n");
    secret["readers"] = json!(["alice"]);
    let r = commit(&mut e, "source", graph(secret));
    let mut target = node("target");
    target["derived_nodes"] = json!([{"graph_id":"source","revision":r,"node_id":"n"}]);
    let structural:GraphData=serde_json::from_value(json!({"profile":"explicit","nodes":[target],"structural_edges":[{"id":"relation","predicate":"p","from":"target","to":"target"}],"assertions":[]})).unwrap();
    let revision = commit(&mut e, "structural", structural);
    let reference = weave_contract::StructuralRef {
        graph_id: "structural".into(),
        revision,
        edge_id: "relation".into(),
    };
    assert!(e
        .resolve_structural(&reference, &HostContext::new("alice", []))
        .unwrap()
        .is_some());
    assert!(e
        .resolve_structural(&reference, &HostContext::new("bob", []))
        .unwrap()
        .is_none());
}

#[test]
fn node_cycles_are_unavailable_but_node_and_assertion_namespaces_do_not_collide() {
    let mut e = Engine::memory().unwrap();
    let mut cyclic = node("loop");
    cyclic["derived_nodes"] =
        json!([{"graph_id":"cycle","revision":"logical:cyclic:cycle","node_id":"loop"}]);
    let program:Program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit_batch","batch_id":"cyclic","commits":[{"graph_id":"cycle","data":{"nodes":[cyclic],"edges":[]}}]}]})).unwrap();
    e.execute(&program, &HostContext::new("alice", ["cycle".into()]))
        .unwrap();
    assert!(read(&e, "cycle", "alice").graph.nodes.is_empty());
    let mut a = node("a");
    a["derived_from"] =
        json!([{"graph_id":"valid","revision":"logical:names:valid","assertion_id":"a"}]);
    let program=serde_json::from_value(json!({"version":VERSION,"commands":[{"op":"commit_batch","batch_id":"names","commits":[{"graph_id":"valid","data":{"nodes":[a,node("b")],"edges":[{"id":"a","predicate":"proof","from":"b","to":"b","valid_time":{"start":0}}]}}]}]})).unwrap();
    e.execute(&program, &HostContext::new("alice", ["valid".into()]))
        .unwrap();
    let mut output = node("output");
    output["derived_nodes"] =
        json!([{"graph_id":"valid","revision":"logical:names:valid","node_id":"a"}]);
    commit(&mut e, "output", graph(output));
    assert_eq!(read(&e, "output", "alice").graph.nodes.len(), 1);
}

#[test]
fn old_wire_profile_and_oversized_node_dependencies_fail_atomically() {
    let mut e = Engine::memory().unwrap();
    let mut n = node("n");
    n["derived_nodes"] = json!([{"graph_id":"source","revision":"r","node_id":"n"}]);
    let data = graph(n.clone());
    let program = Program {
        version: "0.10.0".into(),
        source_revisions: vec![],
        commands: vec![Command::Commit {
            graph_id: "old".into(),
            branch_id: "main".into(),
            expected_head: None,
            data,
        }],
    };
    assert_eq!(
        e.execute(&program, &HostContext::new("alice", ["old".into()]))
            .unwrap_err()
            .code,
        "E_VERSION"
    );
    n["derived_nodes"] = Value::Array(vec![n["derived_nodes"][0].clone(); 1001]);
    let program = Program {
        version: VERSION.into(),
        source_revisions: vec![],
        commands: vec![Command::Commit {
            graph_id: "old".into(),
            branch_id: "main".into(),
            expected_head: None,
            data: graph(n),
        }],
    };
    assert_eq!(
        e.execute(&program, &HostContext::new("alice", ["old".into()]))
            .unwrap_err()
            .code,
        "E_PROVENANCE"
    );
    assert!(e.events().unwrap().is_empty());
}

#[test]
fn reads_and_generated_scalar_outputs_respect_the_persistable_influence_boundary() {
    let mut e = Engine::memory().unwrap();
    let source = commit(&mut e, "base", graph(node("base")));
    let mut full = node("full");
    full["derived_nodes"] = Value::Array(vec![
        json!({"graph_id":"base","revision":source,"node_id":"base"});
        1000
    ]);
    commit(&mut e, "full", graph(full));
    let query: QueryPlan = serde_json::from_value(json!({"graph_id":"full"})).unwrap();
    assert_eq!(
        e.query(&query, &HostContext::new("alice", []))
            .unwrap_err()
            .code,
        "E_BUDGET"
    );
    for operation in ["support", "explain"] {
        let mut e = Engine::memory().unwrap();
        let input = json!({"kind":"query","query":{"graph_id":"many"}});
        let value = if operation == "support" {
            json!({"kind":"support","input":input,"predicate":"p","from":{"entity_id":"a","space_id":"s"},"to":{"entity_id":"b","space_id":"s"},"valid_at":0})
        } else {
            json!({"kind":"explain","input":input})
        };
        let p:Program=serde_json::from_value(json!({"version":VERSION,"commands":[
            {"op":"commit","graph_id":"many","data":{"nodes":(0..1001).map(|i|node(&format!("n{i}"))).collect::<Vec<_>>(),"edges":[]}},
            {"op":"evaluate","value":value}]})).unwrap();
        assert_eq!(
            e.execute(&p, &HostContext::new("alice", ["many".into()]))
                .unwrap_err()
                .code,
            "E_PROVENANCE"
        );
        assert!(e.events().unwrap().is_empty());
    }
}
