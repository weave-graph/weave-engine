use std::collections::BTreeMap;
use weave_contract::*;
use weave_engine::{Engine, HostContext};
fn host() -> HostContext {
    HostContext::new("alice", ["g".into(), "evidence".into()])
}
fn node(id: &str, entity: &str, space: &str) -> Node {
    Node {
        derived_nodes: vec![],
        derived_from: vec![],
        context_scope: None,
        type_id: None,
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
        profile: GraphProfile::Legacy,
        structural_edges: vec![],
        assertions: vec![],
        schema: None,
        attachments: vec![],
        nodes: vec![
            node("physical", "device", "physical"),
            node("operational", "device", "operational"),
        ],
        edges: vec![Edge {
            assertion_source: None,
            assertion_context: None,
            structural_ref: None,
            assertion_properties: BTreeMap::new(),
            type_id: None,
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
            derivations: vec![],
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
        source_revisions: vec![],
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
        profile: GraphProfile::Legacy,
        structural_edges: vec![],
        assertions: vec![],
        schema: None,
        attachments: vec![],
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
        profile: GraphProfile::Legacy,
        structural_edges: vec![],
        assertions: vec![],
        schema: None,
        attachments: vec![],
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
        source_revisions: vec![],
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
        source_revisions: vec![],
        version: LEGACY_VERSION.into(),
        commands: vec![Command::Query { query: query("g") }],
    };
    assert!(engine.execute(&p, &host()).is_ok());
}

fn expression(graph: &str) -> GraphExpression {
    GraphExpression::Query {
        query: query(graph),
    }
}
fn joined(left: GraphExpression, right: GraphExpression, predicate: &str) -> GraphExpression {
    GraphExpression::Join {
        left: Box::new(left),
        right: Box::new(right),
        output_predicate: predicate.into(),
        match_on: JoinMatch::EntitySpaceToFrom,
    }
}
#[test]
fn named_graphs_compose_without_persistence_and_keep_leaf_provenance() {
    let mut engine = Engine::memory().unwrap();
    let h = HostContext::new("alice", ["g".into(), "evidence".into(), "third".into()]);
    let mut commands = Vec::new();
    for (graph, start, end) in [("g", "A", "B"), ("evidence", "B", "C"), ("third", "C", "D")] {
        let mut d = data();
        d.nodes[0].entity_id = start.into();
        d.nodes[1].entity_id = end.into();
        d.nodes
            .iter_mut()
            .for_each(|n| n.space_id = "operational".into());
        if graph == "g" {
            d.edges[0].metadata.push(GraphRef {
                graph_id: "missing".into(),
                revision: "r0".into(),
            });
        }
        commands.push(commit(graph, None, d));
    }
    engine.execute(&program(commands), &h).unwrap();
    let before = engine.event_count().unwrap();
    let p = program(vec![
        Command::Bind {
            name: "first".into(),
            value: joined(expression("g"), expression("evidence"), "A_C"),
        },
        Command::Evaluate {
            value: joined(
                GraphExpression::Filter {
                    input: Box::new(GraphExpression::Reference {
                        name: "first".into(),
                    }),
                    predicate: Some("A_C".into()),
                    valid_at: Some(150),
                },
                expression("third"),
                "A_D",
            ),
        },
    ]);
    let result = engine.execute(&p, &h).unwrap();
    let CommandResult::Queried { result } = &result[1] else {
        panic!()
    };
    assert_eq!(result.graph.edges.len(), 1);
    assert_eq!(result.graph.edges[0].derived_from.len(), 3);
    assert_eq!(result.provenance.len(), 3);
    assert_eq!(result.input_snapshots.len(), 3);
    assert_eq!(result.coverage, Coverage::Partial);
    assert_eq!(result.edge_origins[&result.graph.edges[0].id].len(), 3);
    assert_eq!(engine.event_count().unwrap(), before);
    assert!(engine.head("first", "main").unwrap().is_none());
}
#[test]
fn duplicate_unbound_and_excessive_graph_expressions_reject_atomically() {
    let mut engine = Engine::memory().unwrap();
    let p = program(vec![
        commit("g", None, data()),
        Command::Evaluate {
            value: GraphExpression::Reference {
                name: "future".into(),
            },
        },
    ]);
    assert_eq!(engine.execute(&p, &host()).unwrap_err().code, "E_BINDING");
    assert_eq!(engine.event_count().unwrap(), 0);
    engine
        .execute(&program(vec![commit("g", None, data())]), &host())
        .unwrap();
    let bound = Command::Bind {
        name: "same".into(),
        value: expression("g"),
    };
    assert_eq!(
        engine
            .execute(&program(vec![bound.clone(), bound]), &host())
            .unwrap_err()
            .code,
        "E_BINDING"
    );
    let mut value = expression("g");
    for _ in 0..34 {
        value = GraphExpression::Filter {
            input: Box::new(value),
            predicate: None,
            valid_at: None,
        };
    }
    assert_eq!(
        engine
            .execute(&program(vec![Command::Evaluate { value }]), &host())
            .unwrap_err()
            .code,
        "E_BUDGET"
    );
    let old = Program {
        source_revisions: vec![],
        version: "0.2.0".into(),
        commands: vec![Command::Evaluate {
            value: expression("g"),
        }],
    };
    assert_eq!(engine.execute(&old, &host()).unwrap_err().code, "E_VERSION");
}

#[test]
fn malicious_json_cannot_mint_authority_or_bind_itself() {
    let mut engine = Engine::memory().unwrap();
    let self_reference:Program=serde_json::from_str(r#"{"version":"0.3.0","commands":[{"op":"bind","name":"self","value":{"kind":"reference","name":"self"}}]}"#).unwrap();
    assert_eq!(
        engine.execute(&self_reference, &host()).unwrap_err().code,
        "E_BINDING"
    );
    let widened = r#"{"version":"0.3.0","commands":[{"op":"evaluate","value":{"kind":"query","query":{"graph_id":"secret","actor":"admin"}}}]}"#;
    assert!(serde_json::from_str::<Program>(widened).is_err());
    let extra = r#"{"version":"0.3.0","commands":[{"op":"bind","name":"x","value":{"kind":"reference","name":"x","grant":"all"}}]}"#;
    assert!(serde_json::from_str::<Program>(extra).is_err());
    assert_eq!(engine.event_count().unwrap(), 0);
}

#[test]
fn capsule_receive_is_idempotent_quarantined_and_fork_edits_are_independent() {
    let mut source = Engine::memory().unwrap();
    let rev = revision(
        &source
            .execute(&program(vec![commit("g", None, data())]), &host())
            .unwrap(),
    );
    let reference = GraphRef {
        graph_id: "g".into(),
        revision: rev.clone(),
    };
    let capsule = source.export_capsule(&reference, &host()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("offline.db");
    let mut dest = Engine::open(&path).unwrap();
    assert_eq!(dest.receive_capsule(&capsule, &host()).unwrap(), 1);
    assert_eq!(dest.receive_capsule(&capsule, &host()).unwrap(), 0);
    assert!(dest.head("g", "main").unwrap().is_none());
    assert_eq!(dest.event_count().unwrap(), 0);
    dest.accept_revision(&reference, "main", None, &host())
        .unwrap();
    dest.fork_branch(&reference, "offline", &host()).unwrap();
    let mut changed = data();
    changed.nodes[0]
        .properties
        .insert("offline".into(), true.into());
    let command = Command::Commit {
        graph_id: "g".into(),
        branch_id: "offline".into(),
        expected_head: Some(rev.clone()),
        data: changed,
    };
    dest.execute(&program(vec![command]), &host()).unwrap();
    assert_eq!(dest.head("g", "main").unwrap(), Some(rev.clone()));
    assert_ne!(dest.head("g", "offline").unwrap(), Some(rev));
    drop(dest);
    let dest = Engine::open(path).unwrap();
    assert_eq!(dest.event_count().unwrap(), 3);
    assert_eq!(dest.events().unwrap()[0].event_type, "graph.accepted");
}
#[test]
fn capsule_tamper_undeclared_dependency_and_unauthorized_acceptance_fail() {
    let mut source = Engine::memory().unwrap();
    let mut d = data();
    d.edges[0].metadata.push(GraphRef {
        graph_id: "missing".into(),
        revision: "r0".into(),
    });
    let rev = revision(
        &source
            .execute(&program(vec![commit("g", None, d)]), &host())
            .unwrap(),
    );
    let reference = GraphRef {
        graph_id: "g".into(),
        revision: rev,
    };
    let capsule = source.export_capsule(&reference, &host()).unwrap();
    assert_eq!(capsule.external_dependencies.len(), 1);
    let mut dest = Engine::memory().unwrap();
    let mut tampered = capsule.clone();
    tampered.revisions[0].data.nodes[0].entity_id = "tampered".into();
    assert_eq!(
        dest.receive_capsule(&tampered, &host()).unwrap_err().code,
        "E_INTEGRITY"
    );
    let mut hidden_boundary = capsule.clone();
    hidden_boundary.external_dependencies.clear();
    assert_eq!(
        dest.receive_capsule(&hidden_boundary, &host())
            .unwrap_err()
            .code,
        "E_INTEGRITY"
    );
    dest.receive_capsule(&capsule, &host()).unwrap();
    let bob = HostContext::new("bob", Vec::<String>::new());
    assert_eq!(
        dest.accept_revision(&reference, "main", None, &bob)
            .unwrap_err()
            .code,
        "E_FORBIDDEN"
    );
    assert!(dest.head("g", "main").unwrap().is_none());
    assert_eq!(dest.event_count().unwrap(), 0);
}

#[test]
fn repeated_accepted_transitions_have_distinct_events_and_strict_cas() {
    let mut engine = Engine::memory().unwrap();
    let a = revision(
        &engine
            .execute(&program(vec![commit("g", None, data())]), &host())
            .unwrap(),
    );
    let mut changed = data();
    changed.nodes[0]
        .properties
        .insert("change".into(), true.into());
    let b = revision(
        &engine
            .execute(
                &program(vec![commit("g", Some(a.clone()), changed)]),
                &host(),
            )
            .unwrap(),
    );
    let ra = GraphRef {
        graph_id: "g".into(),
        revision: a.clone(),
    };
    let rb = GraphRef {
        graph_id: "g".into(),
        revision: b.clone(),
    };
    engine
        .accept_revision(&ra, "main", Some(&b), &host())
        .unwrap();
    engine
        .accept_revision(&rb, "main", Some(&a), &host())
        .unwrap();
    engine
        .accept_revision(&ra, "main", Some(&b), &host())
        .unwrap();
    engine
        .accept_revision(&rb, "main", Some(&a), &host())
        .unwrap();
    let before = engine.event_count().unwrap();
    assert_eq!(before, 6);
    engine
        .accept_revision(&rb, "main", Some(&b), &host())
        .unwrap();
    assert_eq!(engine.event_count().unwrap(), before);
    assert_eq!(
        engine
            .accept_revision(&ra, "main", Some(&a), &host())
            .unwrap_err()
            .code,
        "E_CONFLICT"
    );
    assert_eq!(engine.event_count().unwrap(), before);
    let events = engine.events().unwrap();
    let ids: std::collections::HashSet<_> = events.iter().map(|e| e.event_id.clone()).collect();
    assert_eq!(ids.len(), 6);
    assert!(events[2..].iter().all(|e| e.event_type == "graph.accepted"));
}

#[test]
fn direct_query_bounds_metadata_bytes_during_expansion() {
    let mut engine = Engine::memory().unwrap();
    let h = HostContext::new("alice", ["g".into(), "m1".into(), "m2".into(), "m3".into()]);
    let mut root = data();
    for name in ["m1", "m2", "m3"] {
        let mut n = node("payload", "payload", "evidence");
        n.properties
            .insert("large".into(), ("x".repeat(12 * 1024 * 1024)).into());
        let rev = revision(
            &engine
                .execute(
                    &program(vec![commit(
                        name,
                        None,
                        GraphData {
                            profile: GraphProfile::Legacy,
                            structural_edges: vec![],
                            assertions: vec![],
                            schema: None,
                            attachments: vec![],
                            nodes: vec![n],
                            edges: vec![],
                        },
                    )]),
                    &h,
                )
                .unwrap(),
        );
        root.nodes[0].metadata.push(GraphRef {
            graph_id: name.into(),
            revision: rev,
        });
    }
    engine
        .execute(&program(vec![commit("g", None, root)]), &h)
        .unwrap();
    assert_eq!(engine.query(&query("g"), &h).unwrap_err().code, "E_BUDGET");
}
#[test]
fn direct_join_bounds_repeated_provenance_before_output_accumulates() {
    let mut engine = Engine::memory().unwrap();
    let left_name = "l".repeat(500);
    let right_name = "r".repeat(500);
    let h = HostContext::new("alice", [left_name.clone(), right_name.clone()]);
    for (name, right) in [(&left_name, false), (&right_name, true)] {
        let mut d = data();
        d.nodes.iter_mut().for_each(|n| n.space_id = "s".into());
        if right {
            d.nodes[0].entity_id = "middle".into();
            d.nodes[1].entity_id = "end".into();
        } else {
            d.nodes[0].entity_id = "start".into();
            d.nodes[1].entity_id = "middle".into();
        }
        let edge = d.edges[0].clone();
        d.edges = (0..200)
            .map(|i| {
                let mut e = edge.clone();
                e.id = format!("{i:03}{}", "e".repeat(497));
                e
            })
            .collect();
        engine
            .execute(&program(vec![commit(name, None, d)]), &h)
            .unwrap();
    }
    assert_eq!(
        engine
            .join(&query(&left_name), &query(&right_name), "path", &h)
            .unwrap_err()
            .code,
        "E_BUDGET"
    );
}

#[test]
fn legacy_hash_and_structural_registry_survive_upgrade() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let mut engine = Engine::open(&path).unwrap();
    let graph = data();
    let legacy_json = serde_json::to_value(&graph).unwrap();
    assert!(legacy_json.get("schema").is_none());
    assert!(legacy_json.get("attachments").is_none());
    assert!(legacy_json["nodes"][0].get("type_id").is_none());
    let expected = format!(
        "sha256:{:x}",
        Sha256::digest(
            serde_json::to_vec(&(
                "weave-revision-v0.1",
                "g",
                "main",
                Option::<String>::None,
                &graph
            ))
            .unwrap()
        )
    );
    let rev = revision(
        &engine
            .execute(&program(vec![commit("g", None, graph)]), &host())
            .unwrap(),
    );
    assert_eq!(rev, expected);
    drop(engine);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("DELETE FROM edge_structures; PRAGMA user_version=3;")
        .unwrap();
    drop(conn);
    let mut engine = Engine::open(&path).unwrap();
    let mut changed = data();
    changed.edges[0].predicate = "retargeted".into();
    assert_eq!(
        engine
            .execute(&program(vec![commit("g", Some(rev), changed)]), &host())
            .unwrap_err()
            .code,
        "E_EDGE_IDENTITY"
    );
    assert_eq!(engine.event_count().unwrap(), 1);
}

#[test]
fn nested_metadata_requires_current_profile_and_rolls_back() {
    let mut engine = Engine::memory().unwrap();
    let mut p = program(vec![
        commit("g", None, data()),
        Command::Evaluate {
            value: GraphExpression::Filter {
                input: Box::new(GraphExpression::Metadata {
                    input: Box::new(GraphExpression::Query { query: query("g") }),
                    host: MetadataHost::Graph,
                    key: "evidence".into(),
                }),
                predicate: None,
                valid_at: None,
            },
        },
    ]);
    p.version = "0.3.0".into();
    assert_eq!(engine.execute(&p, &host()).unwrap_err().code, "E_VERSION");
    assert!(engine.head("g", "main").unwrap().is_none());
}

#[test]
fn alternative_derivations_preserve_visible_support_without_leaking_hidden_group() {
    let mut engine = Engine::memory().unwrap();
    let h = HostContext::new(
        "alice",
        ["private".into(), "public".into(), "result".into()],
    );
    let mut private = data();
    private.edges[0].readers = vec!["alice".into()];
    let a = revision(
        &engine
            .execute(&program(vec![commit("private", None, private)]), &h)
            .unwrap(),
    );
    let b = revision(
        &engine
            .execute(&program(vec![commit("public", None, data())]), &h)
            .unwrap(),
    );
    let refs = vec![
        AssertionRef {
            graph_id: "private".into(),
            revision: a,
            assertion_id: "link".into(),
        },
        AssertionRef {
            graph_id: "public".into(),
            revision: b,
            assertion_id: "link".into(),
        },
    ];
    let mut result = data();
    result.edges[0].derived_from = refs.clone();
    result.edges[0].derivations = refs
        .iter()
        .map(|p| Derivation {
            operator: "rule:test".into(),
            premises: vec![p.clone()],
            parameters: BTreeMap::new(),
            input_snapshots: vec![GraphRef {
                graph_id: p.graph_id.clone(),
                revision: p.revision.clone(),
            }],
        })
        .collect();
    engine
        .execute(&program(vec![commit("result", None, result)]), &h)
        .unwrap();
    let bob = HostContext::new("bob", []);
    let value = engine.query(&query("result"), &bob).unwrap();
    assert_eq!(value.graph.edges.len(), 1);
    assert_eq!(value.graph.edges[0].derivations.len(), 1);
    assert_eq!(value.graph.edges[0].derived_from[0].graph_id, "public");
    assert_eq!(value.coverage, Coverage::Partial);
    assert!(!serde_json::to_string(&value).unwrap().contains("private"));
}
