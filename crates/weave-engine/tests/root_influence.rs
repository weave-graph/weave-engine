//! Independent acceptance for the protocol 0.15 influence boundary.
use serde_json::{json, Value};
use weave_contract::*;
use weave_engine::{Engine, HostContext};
fn host(who: &str) -> HostContext {
    HostContext::new(
        who,
        ["secret", "other", "input", "empty", "saved", "marker"].map(String::from),
    )
}
fn write(e: &mut Engine, graph: &str, data: Value) -> GraphRef {
    let p: Program = serde_json::from_value(json!({"version":VERSION,"commands":[{
        "op":"commit","graph_id":graph,"expected_head":e.head(graph,"main").unwrap(),"data":data
    }]}))
    .unwrap();
    e.execute(&p, &host("alice")).unwrap();
    GraphRef {
        graph_id: graph.into(),
        revision: e.head(graph, "main").unwrap().unwrap(),
    }
}
fn node(id: &str) -> Value {
    json!({"id":id,"entity_id":id,"space_id":"s"})
}
fn secret(e: &mut Engine, graph: &str, who: &str) -> NodeRef {
    let mut n = node("private");
    n["readers"] = json!([who, "carol"]);
    let r = write(e, graph, json!({"nodes":[n]}));
    NodeRef {
        graph_id: r.graph_id,
        revision: r.revision,
        node_id: "private".into(),
    }
}
fn query(e: &Engine, graph: &str, who: &str) -> QueryResult {
    e.query(
        &serde_json::from_value(json!({"graph_id":graph})).unwrap(),
        &host(who),
    )
    .unwrap()
}
fn evaluate(e: &mut Engine, value: GraphExpression) -> QueryResult {
    let result = e
        .execute(
            &Program {
                version: VERSION.into(),
                source_revisions: vec![],
                commands: vec![Command::Evaluate { value }],
            },
            &host("alice"),
        )
        .unwrap();
    let CommandResult::Queried { result } = result.into_iter().next().unwrap() else {
        panic!()
    };
    *result
}
fn expression(graph: &str) -> GraphExpression {
    GraphExpression::Query {
        query: serde_json::from_value(json!({"graph_id":graph})).unwrap(),
    }
}
fn support(input: GraphExpression) -> GraphExpression {
    GraphExpression::Support {
        input: Box::new(input),
        predicate: "p".into(),
        from: EntitySpace {
            entity_id: "a".into(),
            space_id: "s".into(),
        },
        to: EntitySpace {
            entity_id: "b".into(),
            space_id: "s".into(),
        },
        valid_at: 5,
    }
}
#[test]
fn private_empty_node_influence_survives_scalar_generation_reader_and_carrier_removal() {
    let mut e = Engine::memory().unwrap();
    let gate = secret(&mut e, "secret", "alice");
    let influenced = write(
        &mut e,
        "input",
        json!({"influence":{"assertions":[],"nodes":[gate]},"nodes":[],"edges":[]}),
    );
    write(&mut e, "empty", json!({"nodes":[],"edges":[]}));
    let empty_capsule = e.export_capsule(&influenced, &host("alice")).unwrap();
    assert!(empty_capsule
        .revisions
        .iter()
        .any(|r| r.graph_id == "secret"));
    let mut empty_peer = Engine::memory().unwrap();
    empty_peer
        .receive_capsule(&empty_capsule, &host("alice"))
        .unwrap();
    let pinned: QueryPlan =
        serde_json::from_value(json!({"graph_id":"input","revision":influenced.revision})).unwrap();
    let peer_value = empty_peer.query(&pinned, &host("alice")).unwrap();
    assert!(serde_json::to_value(peer_value.graph)
        .unwrap()
        .get("influence")
        .is_some());
    let peer_denied = empty_peer.query(&pinned, &host("bob")).unwrap();
    assert_eq!(peer_denied.coverage, Coverage::Partial);
    assert!(serde_json::to_value(peer_denied.graph)
        .unwrap()
        .get("influence")
        .is_none());
    let visible = query(&e, "input", "alice");
    assert!(serde_json::to_value(&visible.graph)
        .unwrap()
        .get("influence")
        .is_some());
    let denied = query(&e, "input", "bob");
    assert_eq!(denied.coverage, Coverage::Partial);
    assert!(denied.graph.nodes.is_empty());
    assert!(serde_json::to_value(&denied.graph)
        .unwrap()
        .get("influence")
        .is_none());
    let result = evaluate(
        &mut e,
        support(GraphExpression::Union {
            left: Box::new(expression("input")),
            right: Box::new(expression("empty")),
        }),
    );
    assert_eq!(result.graph.nodes[0].properties["state"], "unknown");
    assert!(result.graph.nodes[0]
        .derived_nodes
        .iter()
        .any(|r| r == &gate));
    let mut data = serde_json::to_value(result.graph).unwrap();
    data.as_object_mut().unwrap().remove("influence");
    data["edges"] = json!([]);
    for n in data["nodes"].as_array_mut().unwrap() {
        n["readers"] = json!([]);
    }
    let saved = write(&mut e, "saved", data);
    assert_eq!(query(&e, "saved", "alice").graph.nodes.len(), 1);
    assert!(query(&e, "saved", "bob").graph.nodes.is_empty());
    assert!(e.export_capsule(&saved, &host("bob")).is_err());
    let capsule = e.export_capsule(&saved, &host("alice")).unwrap();
    assert!(capsule.revisions.iter().any(|r| r.graph_id == "secret"));
    let mut peer = Engine::memory().unwrap();
    peer.receive_capsule(&capsule, &host("alice")).unwrap();
    let q: QueryPlan =
        serde_json::from_value(json!({"graph_id":"saved","revision":saved.revision})).unwrap();
    assert!(peer.query(&q, &host("bob")).unwrap().graph.nodes.is_empty());
    assert_eq!(peer.query(&q, &host("alice")).unwrap().graph.nodes.len(), 1);
}
#[test]
fn whole_graph_carrier_is_enforced_on_direct_structural_and_assertion_reads() {
    let mut e = Engine::memory().unwrap();
    let gate = secret(&mut e, "secret", "alice");
    let source = write(
        &mut e,
        "input",
        json!({"profile":"explicit","influence":{"assertions":[],"nodes":[gate]},"nodes":[node("a"),node("b")],"structural_edges":[{"id":"edge","from":"a","to":"b","predicate":"p"}],"assertions":[{"id":"claim","edge_id":"edge","source":"root-test","valid_time":{"start":0}}]}),
    );
    let edge = StructuralRef {
        graph_id: source.graph_id.clone(),
        revision: source.revision.clone(),
        edge_id: "edge".into(),
    };
    let claim = AssertionRef {
        graph_id: source.graph_id,
        revision: source.revision,
        assertion_id: "claim".into(),
    };
    assert!(e.resolve_structural(&edge, &host("bob")).unwrap().is_none());
    assert!(e.resolve_assertion(&claim, &host("bob")).unwrap().is_none());
    assert!(e
        .resolve_structural(&edge, &host("alice"))
        .unwrap()
        .is_some());
    assert!(e
        .resolve_assertion(&claim, &host("alice"))
        .unwrap()
        .is_some());
}
#[test]
fn node_only_alternatives_remain_or_groups_and_global_node_gates_remain_and() {
    for explicit in [false, true] {
        let mut e = Engine::memory().unwrap();
        let a = secret(&mut e, "secret", "alice");
        let b = secret(&mut e, "other", "bob");
        let groups = json!([
            {"operator":"root:derive","premises":[],"node_premises":[a],"input_snapshots":[{"graph_id":a.graph_id,"revision":a.revision}]},
            {"operator":"root:derive","premises":[],"node_premises":[b],"input_snapshots":[{"graph_id":b.graph_id,"revision":b.revision}]}
        ]);
        let mut data = if explicit {
            json!({"profile":"explicit","nodes":[node("a"),node("b")],"structural_edges":[{"id":"edge","from":"a","to":"b","predicate":"p"}],"assertions":[{"id":"claim","edge_id":"edge","source":"root-test","valid_time":{"start":0},"derivations":groups}]})
        } else {
            json!({"nodes":[node("a"),node("b")],"edges":[{"id":"claim","from":"a","to":"b","predicate":"p","valid_time":{"start":0},"derivations":groups}]})
        };
        write(&mut e, "input", data.clone());
        for who in ["alice", "bob", "carol"] {
            let view = query(&e, "input", who);
            assert_eq!(view.graph.edges.len(), 1, "{who} explicit={explicit}");
            let count = view.graph.edges[0].derivations.len();
            assert_eq!(count, if who == "carol" { 2 } else { 1 });
        }
        assert!(query(&e, "input", "outsider").graph.edges.is_empty());
        let field = if explicit { "assertions" } else { "edges" };
        data[field][0]["derived_nodes"] = json!([a]);
        let source = write(&mut e, "input", data);
        assert_eq!(query(&e, "input", "alice").graph.edges.len(), 1);
        assert!(query(&e, "input", "bob").graph.edges.is_empty());
        if explicit {
            let claim = AssertionRef {
                graph_id: source.graph_id,
                revision: source.revision,
                assertion_id: "claim".into(),
            };
            assert!(e.resolve_assertion(&claim, &host("bob")).unwrap().is_none());
        }
    }
}
#[test]
fn old_wire_rejection_precedes_marker_writes_for_all_new_influence_positions() {
    let mut e = Engine::memory().unwrap();
    let a = secret(&mut e, "secret", "alice");
    let variants = [
        json!({"influence":{"assertions":[],"nodes":[a]}}),
        json!({"nodes":[node("a"),node("b")],"edges":[{"id":"edge","from":"a","to":"b","predicate":"p","valid_time":{"start":0},"derived_nodes":[a]}]}),
        json!({"nodes":[node("a"),node("b")],"edges":[{"id":"edge","from":"a","to":"b","predicate":"p","valid_time":{"start":0},"derivations":[{"operator":"root:derive","premises":[],"node_premises":[a],"input_snapshots":[{"graph_id":a.graph_id,"revision":a.revision}]}]}]}),
        json!({"profile":"explicit","nodes":[node("a"),node("b")],"structural_edges":[{"id":"edge","from":"a","to":"b","predicate":"p"}],"assertions":[{"id":"claim","edge_id":"edge","source":"root-test","valid_time":{"start":0},"derived_nodes":[a]}]}),
    ];
    let count = e.event_count().unwrap();
    for data in variants {
        let p: Program = serde_json::from_value(json!({"version":"0.14.0","commands":[{"op":"commit","graph_id":"marker","data":{"nodes":[]}},{"op":"commit","graph_id":"input","data":data}]})).unwrap();
        assert_eq!(e.execute(&p, &host("alice")).unwrap_err().code, "E_VERSION");
        assert!(e.head("marker", "main").unwrap().is_none());
        assert_eq!(e.event_count().unwrap(), count);
    }
}

#[test]
fn logical_cross_graph_influence_cycles_fail_closed_without_unbounded_recursion() {
    let mut e = Engine::memory().unwrap();
    let p: Program = serde_json::from_value(json!({"version":VERSION,"commands":[{
        "op":"commit_batch","batch_id":"influence-cycle","commits":[
            {"graph_id":"input","data":{"nodes":[node("a")],"influence":{"assertions":[],"nodes":[{"graph_id":"empty","revision":"logical:influence-cycle:empty","node_id":"b"}]}}},
            {"graph_id":"empty","data":{"nodes":[node("b")],"influence":{"assertions":[],"nodes":[{"graph_id":"input","revision":"logical:influence-cycle:input","node_id":"a"}]}}}
        ]
    }]})).unwrap();
    e.execute(&p, &host("alice")).unwrap();
    for graph in ["input", "empty"] {
        let result = query(&e, graph, "alice");
        assert!(result.graph.nodes.is_empty());
        assert_eq!(result.coverage, Coverage::Partial);
        assert!(serde_json::to_value(result.graph)
            .unwrap()
            .get("influence")
            .is_none());
    }
}
