//! Independent fixed-filter and provenance oracle for incremental refresh.
use serde_json::json;
use weave_contract::*;
use weave_engine::*;
fn host() -> HostContext {
    HostContext::new("alice", ["g".into()])
}
fn graph(turn: i64) -> GraphData {
    let mut d: GraphData = serde_json::from_value(json!({
        "nodes":[{"id":"a","entity_id":"A","space_id":"s","properties":{"turn":turn}}, {"id":"b","entity_id":"B","space_id":"s"},{"id":"isolated","entity_id":"I","space_id":"s"}],
        "edges":[{"id":"early","predicate":"p","from":"a","to":"b","valid_time":{"start":0,"end":10}}, {"id":"late","predicate":"p","from":"a","to":"b","valid_time":{"start":20,"end":30}}]
    })).unwrap();
    if turn % 2 == 1 {
        d.nodes.reverse();
        d.edges.reverse();
    }
    if turn % 3 == 1 {
        d.edges[0].readers = vec!["bob".into()];
    }
    d
}
fn write(e: &mut Engine, turn: i64) {
    let expected_head = e.head("g", "main").unwrap();
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "g".into(),
                branch_id: "main".into(),
                expected_head,
                data: graph(turn),
            }],
        },
        &host(),
    )
    .unwrap();
}
fn query(time: Option<i64>) -> GraphExpression {
    GraphExpression::Query {
        query: serde_json::from_value(json!({"graph_id":"g","valid_at":time})).unwrap(),
    }
}
fn filter(input: GraphExpression, time: Option<i64>) -> GraphExpression {
    GraphExpression::Filter {
        input: Box::new(input),
        predicate: None,
        valid_at: time,
    }
}
fn oracle(e: &mut Engine, expression: GraphExpression) -> QueryResult {
    let CommandResult::Queried { result } = e
        .execute(
            &Program {
                version: VERSION.into(),
                source_revisions: vec![],
                commands: vec![Command::Evaluate { value: expression }],
            },
            &host(),
        )
        .unwrap()
        .remove(0)
    else {
        panic!()
    };
    *result
}
#[test]
fn root_multiple_fixed_filters_and_noop_layers_preserve_full_oracle() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("views.db");
    let mut e = Engine::open(&path).unwrap();
    write(&mut e, 0);
    let expressions = [
        filter(query(Some(5)), Some(25)),
        filter(filter(query(None), Some(5)), None),
    ];
    for (i, expression) in expressions.iter().enumerate() {
        let initial = e
            .register_view(
                &ViewDefinition {
                    id: format!("root-{i}"),
                    expression: expression.clone(),
                    clock: ViewClock::Fixed,
                },
                None,
                &host(),
            )
            .unwrap();
        e.enroll_incremental_view(&format!("root-{i}"), &host())
            .unwrap();
        assert_eq!(initial.result, oracle(&mut e, expression.clone()));
        assert_eq!(initial.result.graph.edges.len(), i);
        assert_eq!(initial.result.graph.nodes.len(), if i == 0 { 0 } else { 2 });
    }
    for turn in 1..12 {
        write(&mut e, turn);
        if turn % 3 == 0 {
            drop(e);
            e = Engine::open(&path).unwrap();
        }
        for (i, expression) in expressions.iter().enumerate() {
            let id = format!("root-{i}");
            assert_eq!(
                e.read_view(&id, None, ViewFreshness::RequireCurrent, &host())
                    .unwrap_err()
                    .code,
                "E_FRESHNESS"
            );
            let refreshed = e.refresh_view(&id, None, &host()).unwrap();
            let exact = oracle(&mut e, expression.clone());
            assert_eq!(refreshed.result, exact, "turn {turn}, profile {i}");
            assert_eq!(refreshed.generation, turn as u64 + 1);
            assert_eq!(
                e.view_changes(&id, refreshed.generation - 1, &host())
                    .unwrap()
                    .unwrap()
                    .result,
                exact
            );
            assert!(refreshed.current);
        }
    }
}
