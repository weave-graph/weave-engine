use serde_json::json;
use weave_contract::*;
use weave_engine::*;
fn host(who: &str) -> HostContext {
    HostContext::new(who, ["input".into(), "saved".into()])
}
fn run(e: &mut Engine, commands: Vec<Command>) -> Vec<CommandResult> {
    e.execute(
        &Program {
            version: VERSION.into(),
            source_revisions: vec![],
            commands,
        },
        &host("alice"),
    )
    .unwrap()
}
#[test]
fn missing_metadata_target_retains_private_attachment_gate_through_unknown_support() {
    let mut e = Engine::memory().unwrap();
    let data:GraphData=serde_json::from_value(json!({"attachments":[{"id":"secret-path","host":{"kind":"graph"},"key":"proof","value":{"kind":"graph","reference":{"graph_id":"unavailable","revision":"r"}},"valid_time":{"start":0,"end":null},"readers":["alice"]}]})).unwrap();
    run(
        &mut e,
        vec![Command::Commit {
            graph_id: "input".into(),
            branch_id: "main".into(),
            expected_head: None,
            data,
        }],
    );
    let query = GraphExpression::Query {
        query: serde_json::from_value(json!({"graph_id":"input","include_metadata":true})).unwrap(),
    };
    let metadata = GraphExpression::Metadata {
        input: Box::new(query),
        host: MetadataHost::Graph,
        key: "proof".into(),
    };
    let value = GraphExpression::Support {
        input: Box::new(metadata),
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
    };
    let results = run(&mut e, vec![Command::Evaluate { value }]);
    let CommandResult::Queried { result } = &results[0] else {
        panic!()
    };
    assert_eq!(result.graph.nodes[0].properties["state"], "unknown");
    assert_eq!(result.coverage, Coverage::Partial);
    assert!(result.graph.nodes[0]
        .derived_from
        .iter()
        .any(|r| r.assertion_id == "secret-path"));
    let mut saved = result.graph.clone();
    saved.influence = None;
    for node in &mut saved.nodes {
        node.readers.clear();
    }
    run(
        &mut e,
        vec![Command::Commit {
            graph_id: "saved".into(),
            branch_id: "main".into(),
            expected_head: None,
            data: saved,
        }],
    );
    let q: QueryPlan = serde_json::from_value(json!({"graph_id":"saved"})).unwrap();
    assert_eq!(e.query(&q, &host("alice")).unwrap().graph.nodes.len(), 1);
    let denied = e.query(&q, &host("bob")).unwrap();
    assert!(denied.graph.nodes.is_empty());
    assert_eq!(denied.coverage, Coverage::Partial);
}

#[test]
fn node_only_cluster_membership_cannot_drop_gates_by_replacing_endpoints() {
    let mut e = Engine::memory().unwrap();
    run(&mut e,vec![Command::Commit {graph_id:"input".into(),branch_id:"main".into(),expected_head:None,data:serde_json::from_value(json!({"nodes":[{"id":"private","entity_id":"private","space_id":"s","readers":["alice"]}]})).unwrap()}]);
    let source = GraphRef {
        graph_id: "input".into(),
        revision: e.head("input", "main").unwrap().unwrap(),
    };
    let navigation = e
        .cluster_navigation(
            &ClusterRequest {
                source,
                context: ContextSelection::Default,
                valid_at: 5,
                predicate: "connected".into(),
                levels: 0,
            },
            &host("alice"),
        )
        .unwrap();
    let mut edge = navigation.graph.edges[0].clone();
    assert!(!edge.derived_nodes.is_empty());
    assert!(edge.derived_from.is_empty());
    edge.readers.clear();
    edge.from = "a".into();
    edge.to = "b".into();
    let mut saved:GraphData=serde_json::from_value(json!({"nodes":[{"id":"a","entity_id":"a","space_id":"s"},{"id":"b","entity_id":"b","space_id":"s"}]})).unwrap();
    saved.edges.push(edge);
    run(
        &mut e,
        vec![Command::Commit {
            graph_id: "saved".into(),
            branch_id: "main".into(),
            expected_head: None,
            data: saved,
        }],
    );
    let q: QueryPlan = serde_json::from_value(json!({"graph_id":"saved"})).unwrap();
    assert_eq!(e.query(&q, &host("alice")).unwrap().graph.edges.len(), 1);
    let denied = e.query(&q, &host("bob")).unwrap();
    assert!(denied.graph.edges.is_empty());
    assert_eq!(denied.coverage, Coverage::Partial);
}

#[test]
fn metadata_path_wrappers_do_not_claim_original_node_identity_and_union_stably() {
    let mut e = Engine::memory().unwrap();
    run(
        &mut e,
        vec![Command::Commit {
            graph_id: "saved".into(),
            branch_id: "main".into(),
            expected_head: None,
            data: serde_json::from_value(
                json!({"nodes":[{"id":"original","entity_id":"entity","space_id":"s"}]}),
            )
            .unwrap(),
        }],
    );
    let pin = e.head("saved", "main").unwrap().unwrap();
    run(&mut e,vec![Command::Commit {graph_id:"input".into(),branch_id:"main".into(),expected_head:None,data:serde_json::from_value(json!({"attachments":[{"id":"path","host":{"kind":"graph"},"key":"proof","value":{"kind":"graph","reference":{"graph_id":"saved","revision":pin}},"valid_time":{"start":0,"end":null},"readers":["alice"]}]})).unwrap()}]);
    let q = |graph: &str| GraphExpression::Query {
        query: serde_json::from_value(json!({"graph_id":graph,"include_metadata":true})).unwrap(),
    };
    let path = GraphExpression::Metadata {
        input: Box::new(q("input")),
        host: MetadataHost::Graph,
        key: "proof".into(),
    };
    let read = run(
        &mut e,
        vec![Command::Evaluate {
            value: path.clone(),
        }],
    );
    let CommandResult::Queried { result } = &read[0] else {
        panic!()
    };
    assert!(result.graph.nodes[0].id.starts_with("metadata-node:"));
    assert!(result.node_origins.values().all(Vec::is_empty));
    assert!(result.graph.nodes[0]
        .derived_nodes
        .iter()
        .any(|r| r.graph_id == "saved" && r.revision == pin && r.node_id == "original"));
    let union = GraphExpression::Union {
        left: Box::new(path),
        right: Box::new(q("saved")),
    };
    let values = run(
        &mut e,
        vec![
            Command::Evaluate {
                value: union.clone(),
            },
            Command::Evaluate {
                value: GraphExpression::Union {
                    left: Box::new(union.clone()),
                    right: Box::new(union),
                },
            },
        ],
    );
    let CommandResult::Queried { result: first } = &values[0] else {
        panic!()
    };
    let CommandResult::Queried { result: repeated } = &values[1] else {
        panic!()
    };
    assert_eq!(first.graph.nodes.len(), 2);
    assert_eq!(first.graph, repeated.graph);
    let plain = e
        .query(
            &serde_json::from_value(json!({"graph_id":"saved"})).unwrap(),
            &host("bob"),
        )
        .unwrap();
    assert_eq!(plain.graph.nodes[0].id, "original");
    assert!(plain.graph.influence.is_none());
}
