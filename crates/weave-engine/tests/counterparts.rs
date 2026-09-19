use serde_json::json;
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("alice", ["bridges".into()])
}
fn graph() -> GraphData {
    serde_json::from_value(json!({"profile":"explicit","nodes":[{"id":"physical","entity_id":"equipment","space_id":"physical"},{"id":"operations","entity_id":"equipment","space_id":"operations"}],"structural_edges":[{"id":"bridge","predicate":"counterpart","from":"physical","to":"operations"}],"assertions":[{"id":"mapping","edge_id":"bridge","source":"declared-map","valid_time":{"start":0,"end":10}}]})).unwrap()
}
fn program(commands: Vec<Command>) -> Program {
    Program {
        version: VERSION.into(),
        source_revisions: vec![],
        commands,
    }
}
fn commit(data: GraphData, expected_head: Option<String>) -> Command {
    Command::Commit {
        graph_id: "bridges".into(),
        branch_id: "main".into(),
        expected_head,
        data,
    }
}
fn expression(revision: Option<String>, at: i64) -> GraphExpression {
    GraphExpression::Counterparts {
        input: Box::new(GraphExpression::Query {
            query: serde_json::from_value(json!({"graph_id":"bridges","revision":revision}))
                .unwrap(),
        }),
        selection: CounterpartSelection {
            predicate: "counterpart".into(),
            entity_id: "equipment".into(),
            from_space_id: "physical".into(),
            to_space_id: "operations".into(),
            valid_at: at,
        },
    }
}
fn run(e: &mut Engine, value: GraphExpression, host: &HostContext) -> Result<QueryResult> {
    match e
        .execute(&program(vec![Command::Evaluate { value }]), host)?
        .remove(0)
    {
        CommandResult::Queried { result } => Ok(*result),
        _ => panic!("expected graph"),
    }
}
#[test]
fn counterpart_bridge_is_pinned_temporal_and_does_not_synchronize_state() {
    let mut e = Engine::memory().unwrap();
    let mut d = graph();
    d.nodes[0].properties.insert("x".into(), json!(3));
    e.execute(&program(vec![commit(d, None)]), &host()).unwrap();
    let old = e.head("bridges", "main").unwrap().unwrap();
    let value = run(&mut e, expression(None, 5), &host()).unwrap();
    assert_eq!(value.graph.nodes.len(), 2);
    assert_eq!(value.graph.edges.len(), 1);
    assert_eq!(value.graph.edges[0].id, "mapping");
    assert!(value.edge_origins["mapping"]
        .iter()
        .any(|p| p.revision == old && p.assertion_id == "mapping"));
    assert!(!value
        .graph
        .nodes
        .iter()
        .find(|n| n.id == "operations")
        .unwrap()
        .properties
        .contains_key("x"));
    assert!(run(&mut e, expression(None, 10), &host())
        .unwrap()
        .graph
        .edges
        .is_empty());
    let mut revised = graph();
    revised.assertions.clear();
    e.execute(&program(vec![commit(revised, Some(old.clone()))]), &host())
        .unwrap();
    assert!(run(&mut e, expression(None, 5), &host())
        .unwrap()
        .graph
        .edges
        .is_empty());
    assert_eq!(
        run(&mut e, expression(Some(old), 5), &host())
            .unwrap()
            .graph
            .edges
            .len(),
        1
    );
    assert_eq!(e.event_count().unwrap(), 2);
}
#[test]
fn private_or_missing_counterpart_membership_has_no_hidden_cardinality() {
    let mut e = Engine::memory().unwrap();
    let mut d = graph();
    d.nodes[1].readers = vec!["alice".into()];
    e.execute(&program(vec![commit(d, None)]), &host()).unwrap();
    let hidden = run(&mut e, expression(None, 5), &HostContext::new("bob", [])).unwrap();
    assert!(hidden.graph.nodes.is_empty());
    assert!(hidden.graph.edges.is_empty());
    assert!(hidden.provenance.is_empty());
    let serialized = serde_json::to_string(&hidden).unwrap();
    assert!(!serialized.contains("declared-map"));
    assert!(!serialized.contains("mapping"));
    let mut negative = graph();
    negative.assertions[0].polarity = Polarity::Negative;
    let head = e.head("bridges", "main").unwrap();
    e.execute(&program(vec![commit(negative, head)]), &host())
        .unwrap();
    assert!(run(&mut e, expression(None, 5), &host())
        .unwrap()
        .graph
        .edges
        .is_empty());
}
#[test]
fn counterpart_profile_cannot_be_smuggled_into_older_atomic_program() {
    let mut e = Engine::memory().unwrap();
    let mut p = program(vec![
        commit(graph(), None),
        Command::Evaluate {
            value: expression(None, 5),
        },
    ]);
    p.version = "0.9.0".into();
    assert_eq!(e.execute(&p, &host()).unwrap_err().code, "E_VERSION");
    assert_eq!(e.event_count().unwrap(), 0);
    assert!(e.head("bridges", "main").unwrap().is_none());
}
#[test]
fn contextual_bridge_requires_explicit_scope_and_does_not_broadcast() {
    let mut e = Engine::memory().unwrap();
    let mut d = graph();
    let reference = GraphRef {
        graph_id: "world-description".into(),
        revision: "scope-1".into(),
    };
    d.assertions[0].context = Some(reference.clone());
    e.execute(&program(vec![commit(d, None)]), &host()).unwrap();
    assert_eq!(
        run(&mut e, expression(None, 5), &host()).unwrap_err().code,
        "E_CONTEXT_REQUIRED"
    );
    let mut selected = expression(None, 5);
    if let GraphExpression::Counterparts { input, .. } = &mut selected {
        **input = GraphExpression::Context {
            input: input.clone(),
            selection: ContextSelection::Pinned {
                reference: reference.clone(),
            },
        };
    }
    let result = run(&mut e, selected, &host()).unwrap();
    assert_eq!(
        result.selected_context,
        Some(ContextSelection::Pinned {
            reference: reference.clone()
        })
    );
    assert_eq!(result.graph.edges[0].assertion_context, Some(reference));
}
#[test]
fn prior_explain_wire_profile_remains_accepted() {
    let mut e = Engine::memory().unwrap();
    let mut p = program(vec![commit(graph(), None)]);
    p.version = "0.9.0".into();
    e.execute(&p, &host()).unwrap();
    let explain = GraphExpression::Explain {
        input: Box::new(GraphExpression::Query {
            query: serde_json::from_value(json!({"graph_id":"bridges"})).unwrap(),
        }),
    };
    let mut p = program(vec![Command::Evaluate { value: explain }]);
    p.version = "0.9.0".into();
    assert!(e.execute(&p, &host()).is_ok());
}
#[test]
fn ticked_counterpart_view_retracts_bridge_and_matches_fresh_selection() {
    let mut e = Engine::memory().unwrap();
    e.execute(&program(vec![commit(graph(), None)]), &host())
        .unwrap();
    let definition = ViewDefinition {
        id: "counterparts".into(),
        expression: expression(None, 1),
        clock: ViewClock::Tick,
    };
    let initial = e.register_view(&definition, Some(9), &host()).unwrap();
    assert_eq!(initial.result.graph.edges.len(), 1);
    let expired = e.refresh_view("counterparts", Some(10), &host()).unwrap();
    let mut fresh = expression(None, 10);
    if let GraphExpression::Counterparts { input, .. } = &mut fresh {
        if let GraphExpression::Query { query } = &mut **input {
            query.valid_at = Some(10);
        }
    }
    assert_eq!(expired.result, run(&mut e, fresh, &host()).unwrap());
    assert!(expired.result.graph.nodes.is_empty());
    assert_eq!(
        e.view_changes("counterparts", initial.generation, &host())
            .unwrap()
            .unwrap()
            .removed_edges,
        ["mapping"]
    );
    assert_eq!(e.event_count().unwrap(), 1);
}
