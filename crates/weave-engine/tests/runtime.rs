use std::collections::BTreeMap;
use weave_contract::*;
use weave_engine::{Engine, HostContext};
fn host() -> HostContext {
    HostContext::new("alice", ["g".into(), "evidence".into()])
}
fn node(id: &str, entity: &str, space: &str) -> Node {
    Node {
        id: id.into(),
        entity_id: entity.into(),
        space_id: space.into(),
        properties: BTreeMap::new(),
        metadata: vec![],
        readers: vec![],
    }
}
fn data() -> GraphData {
    GraphData {
        nodes: vec![
            node("physical", "device", "physical"),
            node("operational", "device", "operational"),
        ],
        edges: vec![Edge {
            id: "link".into(),
            predicate: "counterpart".into(),
            from: "physical".into(),
            to: "operational".into(),
            valid_time: Interval {
                start: 100,
                end: Some(200),
            },
            polarity: Polarity::Positive,
            properties: BTreeMap::new(),
            metadata: vec![],
            readers: vec![],
            derived_from: vec![],
        }],
    }
}
fn commit(graph: &str, head: Option<String>, data: GraphData) -> Command {
    Command::Commit {
        graph_id: graph.into(),
        branch_id: "main".into(),
        expected_head: head,
        data,
    }
}
fn program(commands: Vec<Command>) -> Program {
    Program {
        version: VERSION.into(),
        commands,
    }
}
fn query(graph: &str) -> QueryPlan {
    QueryPlan {
        graph_id: graph.into(),
        revision: None,
        branch_id: "main".into(),
        predicate: None,
        from: None,
        to: None,
        valid_at: None,
        include_metadata: true,
        max_depth: 8,
    }
}
fn revision(results: &[CommandResult]) -> String {
    match &results[0] {
        CommandResult::Committed { revision, .. } => revision.clone(),
        _ => panic!(),
    }
}
#[test]
fn pinned_history_and_outbox_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.db");
    let mut engine = Engine::open(&path).unwrap();
    let old = revision(
        &engine
            .execute(&program(vec![commit("g", None, data())]), &host())
            .unwrap(),
    );
    let mut changed = data();
    changed.edges[0].valid_time.start = 50;
    changed.nodes[0].properties.insert("x".into(), 7.into());
    engine
        .execute(
            &program(vec![commit("g", Some(old.clone()), changed)]),
            &host(),
        )
        .unwrap();
    drop(engine);
    let engine = Engine::open(path).unwrap();
    let mut q = query("g");
    q.valid_at = Some(75);
    assert_eq!(engine.query(&q, &host()).unwrap().graph.edges.len(), 1);
    q.revision = Some(old.clone());
    assert!(engine.query(&q, &host()).unwrap().graph.edges.is_empty());
    q.valid_at = None;
    let result = engine.query(&q, &host()).unwrap();
    assert_eq!(result.provenance[0].revision, old);
    assert_eq!(engine.event_count().unwrap(), 2);
    assert!(engine.recorded_at(&old).unwrap() > 0);
    assert!(result.graph.nodes[1].properties.is_empty());
}
#[test]
fn rejected_program_rolls_back_graph_and_events() {
    let mut engine = Engine::memory().unwrap();
    let result = engine.execute(
        &program(vec![
            commit("g", None, data()),
            commit("not-granted", None, data()),
        ]),
        &host(),
    );
    assert_eq!(result.unwrap_err().code, "E_FORBIDDEN");
    assert_eq!(engine.event_count().unwrap(), 0);
    assert!(engine.head("g", "main").unwrap().is_none());
}
#[test]
fn optimistic_head_prevents_lost_update() {
    let mut engine = Engine::memory().unwrap();
    engine
        .execute(&program(vec![commit("g", None, data())]), &host())
        .unwrap();
    assert_eq!(
        engine
            .execute(&program(vec![commit("g", None, data())]), &host())
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    assert_eq!(engine.event_count().unwrap(), 1);
}
#[test]
fn node_and_edge_metadata_graphs_are_pinned_and_partial_when_missing() {
    let mut engine = Engine::memory().unwrap();
    let evidence = GraphData {
        nodes: vec![node("source", "source", "docs")],
        edges: vec![],
    };
    let rev = revision(
        &engine
            .execute(&program(vec![commit("evidence", None, evidence)]), &host())
            .unwrap(),
    );
    let mut data = data();
    data.nodes[0].metadata.push(GraphRef {
        graph_id: "evidence".into(),
        revision: rev.clone(),
    });
    data.edges[0].metadata.push(GraphRef {
        graph_id: "evidence".into(),
        revision: rev,
    });
    data.edges[0].metadata.push(GraphRef {
        graph_id: "missing".into(),
        revision: "unknown".into(),
    });
    engine
        .execute(&program(vec![commit("g", None, data)]), &host())
        .unwrap();
    let result = engine.query(&query("g"), &host()).unwrap();
    assert_eq!(result.metadata_graphs.len(), 1);
    assert_eq!(result.metadata_graphs[0].graph.nodes[0].id, "source");
    assert_eq!(result.coverage, Coverage::Partial);
    assert_eq!(result.diagnostics[0].code, "E_DEPENDENCY_UNAVAILABLE");
}
#[test]
fn restricted_counterparts_edges_and_provenance_do_not_leak() {
    let mut engine = Engine::memory().unwrap();
    let mut evidence = data();
    evidence
        .nodes
        .iter_mut()
        .for_each(|n| n.readers = vec!["admin".into()]);
    let rev = revision(
        &engine
            .execute(&program(vec![commit("evidence", None, evidence)]), &host())
            .unwrap(),
    );
    let mut graph = data();
    graph.edges[0].derived_from.push(AssertionRef {
        graph_id: "evidence".into(),
        revision: rev,
        assertion_id: "secret-edge".into(),
    });
    engine
        .execute(&program(vec![commit("g", None, graph)]), &host())
        .unwrap();
    assert!(engine
        .query(&query("g"), &host())
        .unwrap()
        .graph
        .edges
        .is_empty());
    let head = engine.head("g", "main").unwrap();
    let mut graph = data();
    graph.nodes[1].readers = vec!["admin".into()];
    graph.nodes[1].metadata.push(GraphRef {
        graph_id: "secret".into(),
        revision: "private".into(),
    });
    engine
        .execute(&program(vec![commit("g", head, graph)]), &host())
        .unwrap();
    let result = engine.query(&query("g"), &host()).unwrap();
    assert_eq!(result.graph.nodes.len(), 1);
    assert!(result.graph.edges.is_empty());
    assert!(result.metadata_graphs.is_empty());
    assert!(result.provenance.is_empty());
    assert_eq!(result.coverage, Coverage::Complete);
}
#[test]
fn hidden_and_missing_metadata_have_same_diagnostic() {
    let mut engine = Engine::memory().unwrap();
    let mut secret = GraphData {
        nodes: vec![node("secret", "secret", "secret")],
        edges: vec![],
    };
    secret.nodes[0].readers = vec!["admin".into()];
    let secret_rev = revision(
        &engine
            .execute(&program(vec![commit("evidence", None, secret)]), &host())
            .unwrap(),
    );
    let mut graph = data();
    graph.nodes[0].metadata.push(GraphRef {
        graph_id: "evidence".into(),
        revision: secret_rev,
    });
    let old = revision(
        &engine
            .execute(&program(vec![commit("g", None, graph.clone())]), &host())
            .unwrap(),
    );
    let first = engine.query(&query("g"), &host()).unwrap().diagnostics;
    graph.nodes[0].metadata[0].revision = "missing".into();
    engine
        .execute(&program(vec![commit("g", Some(old), graph)]), &host())
        .unwrap();
    assert_eq!(
        first,
        engine.query(&query("g"), &host()).unwrap().diagnostics
    );
}
#[test]
fn adapter_delivery_is_durable_deduplicated_and_retries_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bus.db");
    let mut engine = Engine::open(&path).unwrap();
    engine
        .execute(&program(vec![commit("g", None, data())]), &host())
        .unwrap();
    let event = engine.events().unwrap()[0].event_id.clone();
    engine.register_adapter("audit", "g").unwrap();
    assert_eq!(engine.deliver("audit", &event, false).unwrap(), "delivered");
    drop(engine);
    let mut engine = Engine::open(path).unwrap();
    engine.deliver("audit", &event, false).unwrap();
    assert_eq!(engine.effect_count().unwrap(), 1);
    engine.register_adapter("retry", "g").unwrap();
    assert_eq!(engine.deliver("retry", &event, true).unwrap(), "retry");
    assert_eq!(engine.deliver("retry", &event, true).unwrap(), "retry");
    assert_eq!(
        engine.deliver("retry", &event, true).unwrap(),
        "dead_letter"
    );
    assert_eq!(
        engine.deliver("retry", &event, false).unwrap(),
        "dead_letter"
    );
    engine.replay_dead_letter("retry", &event).unwrap();
    assert_eq!(engine.deliver("retry", &event, false).unwrap(), "delivered");
    assert_eq!(engine.effect_count().unwrap(), 2);
    engine.pause_adapter("retry", true).unwrap();
    assert_eq!(
        engine.deliver("retry", &event, false).unwrap_err().code,
        "E_PAUSED"
    );
}
#[test]
fn malformed_intervals_versions_and_host_impersonation_reject() {
    let mut engine = Engine::memory().unwrap();
    let mut graph = data();
    graph.edges[0].valid_time.end = Some(100);
    assert_eq!(
        engine
            .execute(&program(vec![commit("g", None, graph)]), &host())
            .unwrap_err()
            .code,
        "E_INTERVAL"
    );
    assert_eq!(engine.event_count().unwrap(), 0);
    let mut p = program(vec![]);
    p.version = "999".into();
    assert_eq!(engine.execute(&p, &host()).unwrap_err().code, "E_VERSION");
    assert!(serde_json::from_str::<Program>(
        r#"{"version":"0.1.0","actor":"admin","commands":[]}"#
    )
    .is_err());
}
#[test]
fn bounded_metadata_and_interval_end_are_explicit() {
    let mut engine = Engine::memory().unwrap();
    let mut graph = data();
    graph.nodes[0].metadata.push(GraphRef {
        graph_id: "external".into(),
        revision: "r1".into(),
    });
    engine
        .execute(&program(vec![commit("g", None, graph)]), &host())
        .unwrap();
    let mut q = query("g");
    q.max_depth = 0;
    assert_eq!(
        engine.query(&q, &host()).unwrap().diagnostics[0].code,
        "E_BUDGET"
    );
    q.max_depth = 33;
    assert_eq!(engine.query(&q, &host()).unwrap_err().code, "E_BUDGET");
    q.max_depth = 8;
    q.valid_at = Some(200);
    assert!(engine.query(&q, &host()).unwrap().graph.edges.is_empty());
}

#[test]
fn path_join_intersects_time_and_preserves_both_snapshots_and_identity() {
    let mut engine = Engine::memory().unwrap();
    let mut left = data();
    left.nodes[0].entity_id = "start".into();
    left.nodes[1].entity_id = "middle".into();
    let mut right = data();
    right.nodes[0].entity_id = "middle".into();
    right.nodes[0].space_id = "operational".into();
    right.nodes[1].entity_id = "end".into();
    right.edges[0].valid_time = Interval {
        start: 150,
        end: Some(250),
    };
    engine
        .execute(
            &program(vec![
                commit("g", None, left),
                commit("evidence", None, right),
            ]),
            &host(),
        )
        .unwrap();
    let result = engine
        .join(&query("g"), &query("evidence"), "path", &host())
        .unwrap();
    assert_eq!(result.graph.edges.len(), 1);
    assert_eq!(
        result.graph.edges[0].valid_time,
        Interval {
            start: 150,
            end: Some(200)
        }
    );
    assert_eq!(result.input_snapshots.len(), 2);
    assert_eq!(result.graph.edges[0].derived_from.len(), 2);
    assert_eq!(result.graph.nodes.len(), 2);
    assert_ne!(result.graph.edges[0].from, result.graph.edges[0].to);
    assert_eq!(result.graph.edges[0].readers, vec!["alice"]);
    let again = engine
        .join(&query("g"), &query("evidence"), "path", &host())
        .unwrap();
    assert_eq!(result, again);
}
#[test]
fn path_join_rejects_disjoint_negative_hidden_and_cross_space_premises() {
    for case in ["disjoint", "negative", "hidden", "cross_space"] {
        let mut engine = Engine::memory().unwrap();
        let left = data();
        let mut right = data();
        right.nodes[0].space_id = "operational".into();
        match case {
            "disjoint" => right.edges[0].valid_time.start = 200,
            "negative" => right.edges[0].polarity = Polarity::Negative,
            "hidden" => right.edges[0].readers = vec!["admin".into()],
            "cross_space" => right.nodes[0].space_id = "other".into(),
            _ => unreachable!(),
        }
        if case == "disjoint" {
            right.edges[0].valid_time.end = Some(300);
        }
        engine
            .execute(
                &program(vec![
                    commit("g", None, left),
                    commit("evidence", None, right),
                ]),
                &host(),
            )
            .unwrap();
        assert!(
            engine
                .join(&query("g"), &query("evidence"), "path", &host())
                .unwrap()
                .graph
                .edges
                .is_empty(),
            "{case}"
        );
    }
}
#[test]
fn same_graph_different_revision_join_retains_both_pins_and_legacy_rejects_join() {
    let mut engine = Engine::memory().unwrap();
    let old = revision(
        &engine
            .execute(&program(vec![commit("g", None, data())]), &host())
            .unwrap(),
    );
    let mut changed = data();
    changed.nodes[0].space_id = "operational".into();
    let new = revision(
        &engine
            .execute(
                &program(vec![commit("g", Some(old.clone()), changed)]),
                &host(),
            )
            .unwrap(),
    );
    let mut left = query("g");
    left.revision = Some(old.clone());
    let mut right = query("g");
    right.revision = Some(new.clone());
    let result = engine.join(&left, &right, "path", &host()).unwrap();
    assert_eq!(result.graph.edges.len(), 1);
    assert_eq!(
        result.input_snapshots,
        vec![
            GraphRef {
                graph_id: "g".into(),
                revision: old
            },
            GraphRef {
                graph_id: "g".into(),
                revision: new
            }
        ]
    );
    assert!(!result.snapshots.contains_key("g"));
    let p = Program {
        version: LEGACY_VERSION.into(),
        commands: vec![Command::Join {
            left,
            right,
            output_predicate: "path".into(),
            match_on: JoinMatch::EntitySpaceToFrom,
        }],
    };
    assert_eq!(engine.execute(&p, &host()).unwrap_err().code, "E_VERSION");
    let p = Program {
        version: LEGACY_VERSION.into(),
        commands: vec![Command::Query { query: query("g") }],
    };
    assert!(engine.execute(&p, &host()).is_ok());
}
