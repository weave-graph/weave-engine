use serde_json::{json, Value};
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("alice", ["g".into(), "copy".into(), "C".into()])
}
fn space(id: &str) -> Value {
    json!({"id":id,"revision":"1","geometry":{"kind":"physical3d","frame":id,"unit":"metre"}})
}
fn point(id: &str, values: [f64; 3]) -> Value {
    json!({"kind":"coordinates","space":space(id),"role":"position","values":values})
}
fn data() -> GraphData {
    serde_json::from_value(json!({"profile":"explicit","nodes":[{"id":"a","entity_id":"A","space_id":"world"},{"id":"b","entity_id":"B","space_id":"world"}],"structural_edges":[{"id":"sa","predicate":"coordinates","from":"a","to":"a"},{"id":"sb","predicate":"coordinates","from":"b","to":"b"}],"assertions":[{"id":"pa","edge_id":"sa","source":"sensor-a","properties":{"weave.geometry":point("world",[0.,0.,0.])},"valid_time":{"start":0,"end":20}},{"id":"pb","edge_id":"sb","source":"sensor-b","properties":{"weave.geometry":point("world",[3.,4.,0.])},"valid_time":{"start":5,"end":15}}]})).unwrap()
}
fn program(commands: Vec<Command>) -> Program {
    Program {
        version: VERSION.into(),
        source_revisions: vec![],
        commands,
    }
}
fn commit(data: GraphData) -> Command {
    Command::Commit {
        graph_id: "g".into(),
        branch_id: "main".into(),
        expected_head: None,
        data,
    }
}
fn query() -> GraphExpression {
    GraphExpression::Query {
        query: serde_json::from_value(json!({"graph_id":"g"})).unwrap(),
    }
}
fn operand(id: &str) -> GeometryOperand {
    GeometryOperand {
        input: Box::new(query()),
        assertion_id: id.into(),
    }
}
fn distance(at: i64) -> GraphExpression {
    GraphExpression::Geometry {
        operation: GeometryOperation::Distance {
            left: operand("pa"),
            right: operand("pb"),
        },
        valid_at: at,
    }
}
fn value(e: &mut Engine, expression: GraphExpression, host: &HostContext) -> Result<QueryResult> {
    let out = e.execute(
        &program(vec![Command::Evaluate { value: expression }]),
        host,
    )?;
    match out.into_iter().next().unwrap() {
        CommandResult::Queried { result } => Ok(*result),
        _ => panic!("expected graph value"),
    }
}
#[test]
fn graph_distance_has_typed_result_exact_time_and_pinned_proof() {
    let mut e = Engine::memory().unwrap();
    e.execute(&program(vec![commit(data())]), &host()).unwrap();
    let result = value(&mut e, distance(6), &host()).unwrap();
    assert_eq!(result.graph.nodes[0].properties["value"], 5.0);
    assert_eq!(
        result.graph.edges[0].valid_time,
        Interval {
            start: 5,
            end: Some(15)
        }
    );
    assert_eq!(result.provenance.len(), 2);
    assert!(result.provenance.iter().all(|p| p.graph_id == "g"));
    assert!(result.graph.edges[0].readers == ["alice"]);
    assert_eq!(
        result.graph.nodes[0].context_scope,
        Some(ContextSelection::Default)
    );
    assert_eq!(e.event_count().unwrap(), 1);
    let explain = value(
        &mut e,
        GraphExpression::Explain {
            input: Box::new(distance(6)),
        },
        &host(),
    )
    .unwrap();
    assert!(!explain.graph.edges.is_empty());
    assert!(explain.provenance.iter().any(|p| p.assertion_id == "pa"));
}
#[test]
fn geometry_reads_cannot_bypass_private_assertions_or_time_boundaries() {
    let mut e = Engine::memory().unwrap();
    let mut d = data();
    d.assertions[1].readers = vec!["alice".into()];
    e.execute(&program(vec![commit(d)]), &host()).unwrap();
    assert_eq!(
        value(&mut e, distance(6), &HostContext::new("bob", []))
            .unwrap_err()
            .code,
        "E_GEOMETRY_UNAVAILABLE"
    );
    assert_eq!(
        value(&mut e, distance(15), &host()).unwrap_err().code,
        "E_GEOMETRY_UNAVAILABLE"
    );
    assert_eq!(
        value(&mut e, distance(4), &host()).unwrap_err().code,
        "E_GEOMETRY_UNAVAILABLE"
    );
}
#[test]
fn detached_descriptor_negative_claim_and_evidence_wrapper_reject_atomically() {
    for mode in 0..3 {
        let mut e = Engine::memory().unwrap();
        let mut d = data();
        match mode {
            0 => {
                d.assertions[0]
                    .properties
                    .insert("weave.geometry".into(), point("unrelated", [0., 0., 0.]));
                d.assertions[1]
                    .properties
                    .insert("weave.geometry".into(), point("unrelated", [3., 4., 0.]));
            }
            1 => d.assertions[1].polarity = Polarity::Negative,
            _ => {
                d.assertions[0].properties.insert("weave.geometry".into(),json!({"kind":"coordinates","value":point("world",[0.,0.,0.]),"visibility":"public"}));
            }
        }
        let error = e
            .execute(
                &program(vec![commit(d), Command::Evaluate { value: distance(6) }]),
                &host(),
            )
            .unwrap_err();
        assert_eq!(
            error.code,
            match mode {
                0 => "E_GEOMETRY_SPACE",
                1 => "E_GEOMETRY_UNAVAILABLE",
                _ => "E_GEOMETRY_VALUE",
            }
        );
        assert_eq!(e.event_count().unwrap(), 0);
        assert_eq!(e.head("g", "main").unwrap(), None);
    }
}
#[test]
fn contextual_geometry_requires_exact_selection_and_retains_it_after_reuse() {
    let mut e = Engine::memory().unwrap();
    e.execute(
        &program(vec![Command::Commit {
            graph_id: "C".into(),
            branch_id: "main".into(),
            expected_head: None,
            data: GraphData::default(),
        }]),
        &host(),
    )
    .unwrap();
    let c = GraphRef {
        graph_id: "C".into(),
        revision: e.head("C", "main").unwrap().unwrap(),
    };
    let mut d = data();
    for a in &mut d.assertions {
        a.context = Some(c.clone());
    }
    e.execute(&program(vec![commit(d)]), &host()).unwrap();
    assert_eq!(
        value(&mut e, distance(6), &host()).unwrap_err().code,
        "E_CONTEXT_REQUIRED"
    );
    let scoped = |id: &str| GeometryOperand {
        input: Box::new(GraphExpression::Context {
            input: Box::new(query()),
            selection: ContextSelection::Pinned {
                reference: c.clone(),
            },
        }),
        assertion_id: id.into(),
    };
    let result = value(
        &mut e,
        GraphExpression::Geometry {
            operation: GeometryOperation::Distance {
                left: scoped("pa"),
                right: scoped("pb"),
            },
            valid_at: 6,
        },
        &host(),
    )
    .unwrap();
    assert_eq!(
        result.selected_context,
        Some(ContextSelection::Pinned {
            reference: c.clone()
        })
    );
    assert_eq!(result.graph.edges[0].assertion_context, Some(c));
}
#[test]
fn geometry_explain_and_float_schemas_require_new_profile() {
    for expression in [
        distance(6),
        GraphExpression::Explain {
            input: Box::new(query()),
        },
    ] {
        let mut e = Engine::memory().unwrap();
        let mut p = program(vec![
            commit(data()),
            Command::Evaluate { value: expression },
        ]);
        p.version = "0.8.0".into();
        assert_eq!(e.execute(&p, &host()).unwrap_err().code, "E_VERSION");
        assert_eq!(e.event_count().unwrap(), 0);
    }
    let mut e = Engine::memory().unwrap();
    let d:GraphData=serde_json::from_value(json!({"schema":{"id":"float","revision":"1","nodes":{"N":{"properties":{"value":{"value_type":"float"}}}},"edges":{}}})).unwrap();
    let mut p = program(vec![commit(d)]);
    p.version = "0.8.0".into();
    assert_eq!(e.execute(&p, &host()).unwrap_err().code, "E_VERSION");
    assert_eq!(e.event_count().unwrap(), 0);
}

#[test]
fn repersisted_geometry_cannot_declassify_its_private_premise() {
    let mut e = Engine::memory().unwrap();
    let mut d = data();
    d.assertions[1].readers = vec!["alice".into()];
    e.execute(&program(vec![commit(d)]), &host()).unwrap();
    let mut derived = value(&mut e, distance(6), &host()).unwrap().graph;
    for node in &mut derived.nodes {
        node.readers.clear();
    }
    for edge in &mut derived.edges {
        edge.readers.clear();
    }
    e.execute(
        &program(vec![Command::Commit {
            graph_id: "copy".into(),
            branch_id: "main".into(),
            expected_head: None,
            data: derived,
        }]),
        &host(),
    )
    .unwrap();
    let q = GraphExpression::Query {
        query: serde_json::from_value(json!({"graph_id":"copy"})).unwrap(),
    };
    assert_eq!(
        value(&mut e, q.clone(), &host()).unwrap().graph.edges.len(),
        1
    );
    let hidden = value(&mut e, q, &HostContext::new("bob", [])).unwrap();
    assert!(hidden.graph.edges.is_empty());
    assert!(hidden.graph.nodes.is_empty());
}

#[test]
fn node_only_support_projection_retains_private_evidence_gate_after_persistence() {
    let mut e = Engine::memory().unwrap();
    let mut d = data();
    d.assertions[1].readers = vec!["alice".into()];
    e.execute(&program(vec![commit(d)]), &host()).unwrap();
    let support = GraphExpression::Support {
        input: Box::new(query()),
        predicate: "coordinates".into(),
        from: EntitySpace {
            entity_id: "B".into(),
            space_id: "world".into(),
        },
        to: EntitySpace {
            entity_id: "B".into(),
            space_id: "world".into(),
        },
        valid_at: 6,
    };
    let result = value(&mut e, support.clone(), &host()).unwrap();
    let projected = GraphExpression::Project {
        input: Box::new(support),
        node_ids: vec![result.graph.nodes[0].id.clone()],
        edge_ids: vec![],
    };
    let mut graph = value(&mut e, projected, &host()).unwrap().graph;
    assert!(graph.edges.is_empty());
    assert!(!graph.nodes[0].derived_from.is_empty());
    graph.nodes[0].readers.clear();
    e.execute(
        &program(vec![Command::Commit {
            graph_id: "copy".into(),
            branch_id: "main".into(),
            expected_head: None,
            data: graph,
        }]),
        &host(),
    )
    .unwrap();
    let q = GraphExpression::Query {
        query: serde_json::from_value(json!({"graph_id":"copy"})).unwrap(),
    };
    assert_eq!(
        value(&mut e, q.clone(), &host()).unwrap().graph.nodes.len(),
        1
    );
    let denied = value(&mut e, q, &HostContext::new("bob", [])).unwrap();
    assert!(denied.graph.nodes.is_empty());
    let reference = GraphRef {
        graph_id: "copy".into(),
        revision: e.head("copy", "main").unwrap().unwrap(),
    };
    assert_eq!(
        e.export_capsule(&reference, &HostContext::new("bob", []))
            .unwrap_err()
            .code,
        "E_UNAVAILABLE"
    );
    let capsule = e.export_capsule(&reference, &host()).unwrap();
    let mut dest = Engine::memory().unwrap();
    dest.receive_capsule(&capsule, &host()).unwrap();
    let imported = dest
        .query(
            &serde_json::from_value(json!({"graph_id":"copy","revision":reference.revision}))
                .unwrap(),
            &HostContext::new("bob", []),
        )
        .unwrap();
    assert!(imported.graph.nodes.is_empty());
    assert!(denied.provenance.is_empty());
    assert!(!serde_json::to_string(&denied.diagnostics)
        .unwrap()
        .contains("pb"));
}
