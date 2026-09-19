use serde_json::json;
use weave_contract::*;
use weave_engine::*;
#[test]
fn pinned_cluster_expression_matches_native_result_and_rejects_old_nested_wire() {
    let mut e = Engine::memory().unwrap();
    let host = HostContext::new("alice", ["source".into(), "marker".into()]);
    let data=serde_json::from_value(json!({"nodes":[{"id":"a","entity_id":"a","space_id":"s"},{"id":"b","entity_id":"b","space_id":"s"}],"edges":[{"id":"e","from":"a","to":"b","predicate":"linked","valid_time":{"start":0,"end":10}}]})).unwrap();
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands: vec![Command::Commit {
                graph_id: "source".into(),
                branch_id: "main".into(),
                expected_head: None,
                data,
            }],
        },
        &host,
    )
    .unwrap();
    let selection = ClusterRequest {
        source: GraphRef {
            graph_id: "source".into(),
            revision: e.head("source", "main").unwrap().unwrap(),
        },
        context: ContextSelection::Default,
        valid_at: 5,
        predicate: "linked".into(),
        levels: 2,
    };
    let expected = e.cluster_navigation(&selection, &host).unwrap();
    let expression = GraphExpression::Cluster { selection };
    let program = Program {
        version: VERSION.into(),
        source_revisions: vec![],
        commands: vec![Command::Evaluate {
            value: expression.clone(),
        }],
    };
    let results = e.execute(&program, &host).unwrap();
    let CommandResult::Queried { result } = &results[0] else {
        panic!("query result required")
    };
    assert_eq!(**result, expected);
    assert_eq!(e.event_count().unwrap(), 1);
    let old = Program {
        version: "0.12.0".into(),
        source_revisions: vec![],
        commands: vec![
            Command::Commit {
                graph_id: "marker".into(),
                branch_id: "main".into(),
                expected_head: None,
                data: GraphData::default(),
            },
            Command::Evaluate {
                value: GraphExpression::Project {
                    input: Box::new(expression.clone()),
                    node_ids: vec![],
                    edge_ids: vec![],
                },
            },
        ],
    };
    assert_eq!(e.execute(&old, &host).unwrap_err().code, "E_VERSION");
    assert!(e.head("marker", "main").unwrap().is_none());
    let definition = ViewDefinition {
        id: "cluster".into(),
        expression,
        clock: ViewClock::Tick,
    };
    let first = e.register_view(&definition, Some(5), &host).unwrap();
    let expired = e.refresh_view("cluster", Some(10), &host).unwrap();
    assert_ne!(first.result.graph, expired.result.graph);
    assert_eq!(e.event_count().unwrap(), 1);
}
